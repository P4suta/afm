//! Aozora notations that land in *literal* markdown contexts — inline
//! code spans, link/image destinations, code blocks and unclaimed `［＃…］`
//! runs — must render as their original source, never as an interpreted
//! Aozora node, and must never leak an internal PUA sentinel
//! (`U+E000..=U+E004`) into the HTML.
//!
//! Substitution is CommonMark-blind (ADR-0010): every construct in the
//! text becomes a sentinel *before* comrak parses, so a notation written
//! inside backticks or a URL becomes a sentinel that comrak then routes
//! into a `Code` literal or a `Link.url` field — places the splicer used
//! to skip. Skipping leaked the sentinel AND desynced the construct
//! cursor, corrupting *later* notations. These tests pin the fix: the
//! splicer rewrites such a sentinel back to the source run it stands for
//! and keeps the cursor in lockstep.

use aozora_flavored_markdown::to_html;
use aozora_flavored_markdown_test_support::check_no_sentinel_leak;

/// Render, then hold the output to Tier B: no PUA sentinel may survive into
/// the HTML. Checking at the render site is what gives the predicate the
/// source it reads.
fn render_checked(src: &str) -> String {
    let html = to_html(src);
    let checked = check_no_sentinel_leak(src, &html);
    assert!(
        checked.is_ok(),
        "sentinel leaked: {checked:?}\n  html = {html:?}"
    );
    html
}

// ---------------------------------------------------------------------------
// Inline code spans
// ---------------------------------------------------------------------------

#[test]
fn ruby_inside_inline_code_renders_literally() {
    // `｜青梅《おうめ》` inside backticks is literal markdown: the source
    // text must appear verbatim in <code>, NOT as an interpreted <ruby>.
    let html = render_checked("`｜青梅《おうめ》`");
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
    let html = render_checked("`青梅《おうめ》`");
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
    let html = render_checked("`text［＃「text」に傍点］`");
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
    let html = render_checked("`｜A《a》` then ｜B《b》end");
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
// Unclaimed `［＃…］` runs
//
// A run no notation claimed is hidden behind the directive wrapper and read
// as the author's own bytes — which makes it a literal context like the
// others, and one that can *contain* a claimed construct.
// ---------------------------------------------------------------------------

#[test]
fn a_notation_inside_an_unclaimed_bracket_run_stays_literal() {
    // `［＃改…` is not a notation, so the whole run is hidden as written —
    // including the bouten directive nested inside it, which is one. The
    // splicer used to copy that construct's sentinel into the wrapper's
    // text: a U+E001 in the reader's HTML.
    let html = render_checked("［＃改［＃「あ」に傍点］］");
    assert!(
        html.contains("hidden>［＃改［＃「あ」に傍点］］</span>"),
        "the unclaimed run must be hidden as the author wrote it, got {html:?}"
    );
}

#[test]
fn an_unclaimed_bracket_run_does_not_desync_the_notation_after_it() {
    // The construct inside the run has to be consumed as well as written
    // back, or the next real notation reads this one's entry and renders
    // A/a where the author wrote B/b.
    let html = render_checked("［＃改｜A《a》］\n\nそして｜B《b》です");
    assert!(
        html.contains("hidden>［＃改｜A《a》］</span>"),
        "the run keeps its literal, got {html:?}"
    );
    assert!(
        html.contains("<ruby>B") && html.contains("<rt>b</rt>"),
        "the notation after it must render its OWN content (B/b), got {html:?}"
    );
    assert!(
        !html.contains("<ruby>A"),
        "the run's A must not leak into a rendered ruby, got {html:?}"
    );
}

#[test]
fn an_unclaimed_bracket_run_in_a_heading_does_not_desync_either() {
    // Inside a heading the run is dropped rather than wrapped (Tier C bars
    // the wrapper from a heading body), so nothing is written back — but the
    // construct it swallowed still has to be consumed.
    let html = render_checked("# 見出し［＃改｜A《a》］\n\nそして｜B《b》です");
    assert!(
        html.contains("<h1>見出し</h1>"),
        "the heading keeps only its own text, got {html:?}"
    );
    assert!(
        html.contains("<ruby>B") && html.contains("<rt>b</rt>"),
        "the notation after the heading must render B/b, got {html:?}"
    );
    assert!(
        !html.contains("<ruby>A"),
        "the dropped run's A must not resurface later, got {html:?}"
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
    let html = render_checked("[x](http://e.com/｜p《r》)");
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
    let html = render_checked("[｜T《t》](http://e.com/｜U《u》)");
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
// Code blocks
// ---------------------------------------------------------------------------

#[test]
fn ruby_inside_an_indented_code_block_renders_literally() {
    // A code block is literal markdown for the same reason a code span is.
    // Compiler-derived ranges hide fenced triggers before the lexer runs
    // (ADR-0010), while an indented block is context that mask deliberately
    // does not reproduce and comrak reads one out of any four-space line.
    let html = render_checked("本文\n\n    ｜青梅《おうめ》\n");
    assert!(
        html.contains("<pre><code>｜青梅《おうめ》\n</code></pre>"),
        "the code block must carry the source the author typed, got {html:?}"
    );
}

#[test]
fn a_code_block_reads_the_same_fenced_or_indented() {
    // Two spellings of the same block, one restored from a fence mask and
    // one restored from a construct sentinel, must arrive the same.
    let fenced = to_html("```\n｜青梅《おうめ》\n```\n");
    let indented = to_html("本文\n\n    ｜青梅《おうめ》\n");
    assert!(
        indented.ends_with(fenced.trim_end_matches('\n')) || indented.contains(&fenced),
        "fenced {fenced:?} vs indented {indented:?}"
    );
}

#[test]
fn a_code_block_does_not_desync_the_notation_after_it() {
    // The block consumes its own construct, so the ruby that follows still
    // gets its own.
    let html = render_checked("本文\n\n    ｜青梅《おうめ》\n\n｜鶴見《つるみ》\n");
    assert!(
        html.contains("<pre><code>｜青梅《おうめ》\n</code></pre>"),
        "the block keeps its literal, got {html:?}"
    );
    assert!(
        html.contains("<ruby>鶴見<rp>(</rp><rt>つるみ</rt>"),
        "the ruby after it still renders, got {html:?}"
    );
}

// ---------------------------------------------------------------------------

#[test]
fn crlf_before_notation_does_not_panic_and_renders() {
    // The leading CRLF makes the sanitized source shorter than the raw
    // input, so the ruby's source span only lines up against the sanitized
    // text. Must not panic, and the ruby must still render.
    let html = render_checked("a\r\n\r\n｜青梅《おうめ》");
    assert!(
        html.contains("<ruby>") && html.contains("青梅") && html.contains("おうめ"),
        "ruby after CRLF must render, got {html:?}"
    );
}

#[test]
fn bom_before_notation_does_not_panic() {
    // A UTF-8 BOM is stripped by sanitize, shifting every later offset.
    let html = render_checked("\u{feff}｜青梅《おうめ》");
    assert!(
        html.contains("<ruby>"),
        "ruby after BOM must render, got {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Documents the parser rewrites before lexing. Their notations' byte ranges
// address a text nobody else holds, so a literal context has to recover the
// author's source rather than slice it. CRLF is the cheapest way into that
// path; a decorative rule used to be and no longer is, the row being
// substituted one byte for one now (`verbatim_regions`).
// ---------------------------------------------------------------------------

#[test]
fn rewritten_document_keeps_each_code_span_distinct() {
    // Two rubies of the same shape and the same byte length — the norm for
    // CJK notation of equal character count. Recovering them by shape and
    // length alone cannot tell them apart; the reported offset can, and
    // must, or the author's notation is silently deleted.
    let html = render_checked("本文\r\n----------\r\n`｜A《a》`と`｜B《b》`");
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
    let html = render_checked("本文\r\n----------\r\n[y](http://e.com/｜A《a》)");
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
    let mut src = String::from("本文\r\n----------\r\n");
    for _ in 0..COUNT {
        // Spaced out: back-to-back backticks would open a double-backtick
        // code span instead of two single ones.
        src.push_str("`｜A《a》` ");
    }
    let html = render_checked(&src);
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
    let html = render_checked("```\n｜青梅《おうめ》\n```");
    assert!(
        html.contains("｜青梅《おうめ》"),
        "fenced code must keep its literal Aozora source, got {html:?}"
    );
    assert!(
        !html.contains("<ruby>"),
        "fenced code must not interpret the ruby, got {html:?}"
    );
}
