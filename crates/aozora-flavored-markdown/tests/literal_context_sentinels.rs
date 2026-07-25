//! Aozora notations that land in *literal* markdown contexts — inline
//! code spans and link/image destinations — must render as their original
//! source, never as an interpreted Aozora node, and must never leak the
//! internal PUA sentinel (`U+E001..=U+E004`) into the HTML.
//!
//! The lexer is CommonMark-blind (ADR-0010): it replaces every Aozora
//! notation in the text with a sentinel *before* comrak parses, so a
//! notation written inside backticks or a URL becomes a sentinel that
//! comrak then routes into a `Code` literal or a `Link.url` field — places
//! the splicer used to skip. Skipping leaked the sentinel AND desynced the
//! registry cursor, corrupting *later* notations. These tests pin the fix:
//! the splicer now rewrites such sentinels back to their original source
//! (sliced via the lexer's `source_nodes` span table) and keeps the cursor
//! in lockstep.

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown_test_support::check_no_sentinel_leak;

/// No `U+E001..=U+E004` sentinel may survive into the HTML.
fn assert_no_sentinel(html: &str) {
    if let Err(e) = check_no_sentinel_leak(html) {
        panic!("sentinel leaked: {e:?}\n  html = {html:?}");
    }
}

// ---------------------------------------------------------------------------
// Inline code spans
// ---------------------------------------------------------------------------

#[test]
fn ruby_inside_inline_code_renders_literally() {
    // `｜青梅《おうめ》` inside backticks is literal markdown: the source
    // text must appear verbatim in <code>, NOT as an interpreted <ruby>.
    let html = render_to_string("`｜青梅《おうめ》`");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<code>｜青梅《おうめ》</code>"),
        "inline code must carry the literal Aozora source, got {html:?}"
    );
    assert!(
        !html.contains("<ruby>"),
        "inline code must not interpret the ruby, got {html:?}"
    );
}

#[test]
fn implicit_ruby_inside_inline_code_keeps_base_text() {
    // Implicit ruby (`青梅《おうめ》`, no `｜`): the lexer consumes the
    // base `青梅` into the notation, so the span-sliced literal must
    // restore the full original including the base.
    let html = render_to_string("`青梅《おうめ》`");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<code>青梅《おうめ》</code>"),
        "implicit-ruby literal must include the base text, got {html:?}"
    );
}

#[test]
fn bouten_directive_inside_inline_code_renders_literally() {
    // An *inline* bracket directive (傍点, a forward-reference over the
    // preceding run) inside backticks stays literal source. (Block-level
    // directives like ［＃改ページ］ are `\n\n`-padded by the lexer and so
    // can't sit inside a single-line code span — that's a separate shape.)
    let html = render_to_string("`text［＃「text」に傍点］`");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<code>text［＃「text」に傍点］</code>"),
        "inline directive inside inline code must stay literal, got {html:?}"
    );
}

#[test]
fn sentinel_in_inline_code_does_not_desync_following_notation() {
    // The regression that motivated the fix: a notation inside inline code
    // used to consume nothing, so the *next* real notation grabbed the
    // wrong registry entry. Here the trailing ｜B《b》 must render as B/b,
    // not as A/a from the code span.
    let html = render_to_string("`｜A《a》` then ｜B《b》end");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<code>｜A《a》</code>"),
        "code span keeps its literal, got {html:?}"
    );
    assert!(
        html.contains("<ruby>B") && html.contains("<rt>b</rt>"),
        "trailing notation must render its OWN content (B/b), got {html:?}"
    );
    assert!(
        !html.contains("<ruby>A"),
        "the code span's A must not leak into a rendered ruby, got {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Link / image destinations
// ---------------------------------------------------------------------------

#[test]
fn ruby_trigger_in_link_url_keeps_literal_destination() {
    // A notation inside a link URL must keep the author's literal URL
    // (comrak then percent-encodes the fullwidth chars), not a
    // percent-encoded sentinel.
    let html = render_to_string("[x](http://e.com/｜p《r》)");
    assert_no_sentinel(&html);
    // U+E001 percent-encodes to %EE%80%81; the literal ｜ is %EF%BD%9C.
    assert!(
        !html.contains("%EE%80%81"),
        "the sentinel must not survive (even percent-encoded) in the href, got {html:?}"
    );
    assert!(
        html.contains("%EF%BD%9C"),
        "the literal fullwidth ｜ should be percent-encoded in the href, got {html:?}"
    );
}

#[test]
fn notation_in_link_url_does_not_desync_link_text() {
    // The link text notation and the URL notation must each consume their
    // own registry entry in source order: text first, then url.
    let html = render_to_string("[｜T《t》](http://e.com/｜U《u》)");
    assert_no_sentinel(&html);
    // Link text renders its ruby (T/t)...
    assert!(
        html.contains("<ruby>T") && html.contains("<rt>t</rt>"),
        "link text notation must render as ruby (T/t), got {html:?}"
    );
    // ...and the URL keeps its literal (U/u not interpreted, no sentinel).
    assert!(
        html.contains("href=\"http://e.com/"),
        "link destination must be preserved, got {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Sanitized-coordinate regression: `source_span` is in Phase-0 sanitized
// bytes, so slicing must use the sanitized source. CRLF / BOM inputs shift
// byte offsets — slicing the raw input panicked (out-of-bounds / non-char
// boundary), a SECURITY-scoped crash on untrusted input.
// ---------------------------------------------------------------------------

#[test]
fn crlf_before_notation_does_not_panic_and_renders() {
    // The leading CRLF makes the sanitized source shorter than the raw
    // input, so the ruby's source span only lines up against the sanitized
    // text. Must not panic, and the ruby must still render.
    let html = render_to_string("a\r\n\r\n｜青梅《おうめ》");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<ruby>") && html.contains("青梅") && html.contains("おうめ"),
        "ruby after CRLF must render, got {html:?}"
    );
}

#[test]
fn bom_before_notation_does_not_panic() {
    // A UTF-8 BOM is stripped by sanitize, shifting every later offset.
    let html = render_to_string("\u{feff}｜青梅《おうめ》");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<ruby>"),
        "ruby after BOM must render, got {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Documents the parser rewrites before lexing. Their notations' byte ranges
// address a text nobody else holds, so a literal context has to recover the
// author's source rather than slice it. A decorative rule (`----------`)
// is the cheapest way into that path.
// ---------------------------------------------------------------------------

#[test]
fn rewritten_document_keeps_each_code_span_distinct() {
    // Two rubies of the same shape and the same byte length — the norm for
    // CJK notation of equal character count. Recovering them by shape and
    // length alone cannot tell them apart; the reported offset can, and
    // must, or the author's notation is silently deleted.
    let html = render_to_string("本文\n----------\n`｜A《a》`と`｜B《b》`");
    assert_no_sentinel(&html);
    assert!(
        html.contains("<code>｜A《a》</code>") && html.contains("<code>｜B《b》</code>"),
        "each code span must keep its own literal, got {html:?}"
    );
}

#[test]
fn rewritten_document_keeps_the_link_destination() {
    // An empty recovery is worse than nothing in a URL: it renders as a
    // plausible-looking wrong destination rather than as visibly missing
    // text.
    let html = render_to_string("本文\n----------\n[y](http://e.com/｜A《a》)");
    assert_no_sentinel(&html);
    assert!(
        html.contains("%EF%BD%9C"),
        "the destination must keep the author's ｜, got {html:?}"
    );
}

#[test]
fn rewritten_document_with_many_literal_contexts_stays_linear() {
    // Every literal read used to re-lex the whole enclosing block, which
    // made this input quadratic — reachable from the `parse_render` fuzz
    // target, so a crash-class defect rather than a slow test. The index
    // is built once and shared, so the work is one pass over the source.
    const COUNT: usize = 2_000;
    let mut src = String::from("本文\n----------\n");
    for _ in 0..COUNT {
        // Spaced out: back-to-back backticks would open a double-backtick
        // code span instead of two single ones.
        src.push_str("`｜A《a》` ");
    }
    let html = render_to_string(&src);
    assert_no_sentinel(&html);
    assert_eq!(
        html.matches("<code>｜A《a》</code>").count(),
        COUNT,
        "every code span must recover its literal"
    );
}

// ---------------------------------------------------------------------------
// Fenced code blocks (already masked) stay literal — regression guard.
// ---------------------------------------------------------------------------

#[test]
fn fenced_code_block_still_literal() {
    let html = render_to_string("```\n｜青梅《おうめ》\n```");
    assert_no_sentinel(&html);
    assert!(
        html.contains("｜青梅《おうめ》"),
        "fenced code must keep its literal Aozora source, got {html:?}"
    );
    assert!(
        !html.contains("<ruby>"),
        "fenced code must not interpret the ruby, got {html:?}"
    );
}
