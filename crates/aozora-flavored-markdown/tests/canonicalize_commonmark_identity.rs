//! `canonicalize` against every example in the CommonMark and GFM specs.
//!
//! The README claims this crate is a strict superset of CommonMark. That
//! claim is false for any input `canonicalize` rewrites, and the sibling parser
//! it delegates to is CommonMark-blind by design (ADR-0010) — it mutates the
//! source before anything else runs, without ever asking what CommonMark made
//! of the bytes. Which mutations reach a pure-CommonMark document was, until
//! this file, answered one example at a time.
//!
//! Two gates, because neither alone is sufficient:
//!
//! * [`canonicalize_is_the_identity_on_every_spec_example`] is the *exhaustive*
//!   half — all 652 + 672 examples, each also wrapped in a blockquote and in
//!   a list item, so a mutation that only fires behind a container prefix is
//!   in scope. It settles the "not investigated" question for the corpus.
//! * [`every_source_mutating_step_of_the_sibling_parser_is_protected`] is the
//!   *complete* half. The corpus is necessary but not sufficient: it is
//!   ASCII-shaped and reaches only one of the five rewrites the parser applies
//!   to a source before anything else reads it — it strips a leading BOM,
//!   folds CR and CRLF to LF, composes an accent digraph inside `〔…〕`,
//!   pushes a decorative rule row onto a stanza of its own, and overwrites a
//!   codepoint it reserves for its own lexer. Three of the five have no spec
//!   example at all, and the fourth — a `=`-run — is invisible to the corpus
//!   because the spec never writes one outside a setext position. So the five
//!   are enumerated and pinned one by one.
//!
//! The context matrices below are the second half's teeth: a rule row and a
//! reserved codepoint in every block position CommonMark can put one, each of
//! which the delegate corrupted before DEV-232.

use aozora_flavored_markdown::{Options, canonicalize, render, sentinels};
use pretty_assertions::assert_eq;
use serde::Deserialize;

const COMMONMARK: &str = include_str!("../spec/commonmark-0.31.2.json");
const GFM: &str = include_str!("../spec/gfm-0.29-gfm.json");

/// Measured 2026-07-26 over all 3 972 documents (1 324 examples × 3 shapes).
/// Every one of them is a run of two or more blank lines collapsing to one —
/// CommonMark 221 / 262 / 306 / 307 and their GFM twins 191 / 240 / 286 /
/// 287, each in its bare and its list-item shape (the blockquote shape turns
/// a blank line into a `>` row, so the run never forms). Four unique inputs,
/// e.g. `"aaa\n\n\nbbb\n"` → `"aaa\n\nbbb\n"`.
///
/// Pinned as a count rather than a skip list: every one of them still has to
/// satisfy the two assertions below — the divergence is *exactly* the blank
/// run collapsing, and the rendered HTML does not move — so this number only
/// guards against the set silently growing a member of a different kind.
const EXPECTED_DIVERGENCES: usize = 16;

#[derive(Debug, Deserialize)]
struct SpecExample {
    example: u32,
    section: String,
    markdown: String,
}

/// Every document here is a spec example, bounded and lexable, so an `Err`
/// would be a bug in the guard rather than a divergence this file measures.
fn canonical(src: &str) -> String {
    canonicalize(src).expect("a spec example canonicalises")
}

fn load(fixture: &str) -> Vec<SpecExample> {
    serde_json::from_str(fixture).expect("spec fixture parses as JSON")
}

/// Pure CommonMark, so the comparison reads the spec's examples the way the
/// spec means them rather than through the Aozora dialect.
///
/// No configuration passes raw HTML through any more, and comrak does not
/// escape what it will not emit: every raw block and every raw span collapses
/// to the same `<!-- raw HTML omitted -->`. So this comparison is blind to a
/// rewrite *inside* one raw region that leaves the region count alone — which
/// costs nothing here, because the only rewrite it is ever asked about is the
/// blank-run collapse, and a blank run inside a raw region ends it and moves
/// the count. The teeth are the byte-exact `blank_runs_collapsed` assertion
/// above this one; this is the second opinion.
fn html(src: &str) -> String {
    render(src, &Options::commonmark()).html
}

/// `src` with every run of three or more line breaks cut back to two — the
/// one normalisation the parser applies document-wide that CommonMark does
/// not itself distinguish, spelled out so a divergence can be *matched*
/// against it rather than merely excused.
fn blank_runs_collapsed(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut breaks = 0usize;
    for ch in src.chars() {
        if ch == '\n' {
            breaks += 1;
            if breaks <= 2 {
                out.push(ch);
            }
        } else {
            breaks = 0;
            out.push(ch);
        }
    }
    out
}

/// Every line prefixed with `> `, a blank one with a bare `>` so the quote
/// does not end. Still pure CommonMark, and it puts a container prefix in
/// front of every construct in the corpus.
fn inside_a_blockquote(src: &str) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    for line in src.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        if body.is_empty() {
            out.push('>');
        } else {
            out.push_str("> ");
        }
        out.push_str(body);
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// The same, as one list item: `- ` on the first line, two spaces of
/// continuation after it, and a blank line left blank so the run the corpus
/// carries is still a run.
fn inside_a_list_item(src: &str) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    for (index, line) in src.split_inclusive('\n').enumerate() {
        let body = line.trim_end_matches('\n');
        if !body.is_empty() {
            out.push_str(if index == 0 { "- " } else { "  " });
            out.push_str(body);
        }
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// A `@` in each of these stands for the row under test. Between them they
/// cover every block position CommonMark can put a line in: its own leaf, a
/// setext underline, a paragraph's continuation, a lazy continuation of a
/// list item or a blockquote, a container's own line, a table row, an
/// indented code block, a link reference definition's follower, an HTML
/// block's interior and a fence's interior.
const BLOCK_CONTEXTS: &[&str] = &[
    "@\n",
    "aaa\n@\n",
    "aaa\n\n@\n",
    "aaa\n@\nbbb\n",
    "- aaa\n@\n",
    "- aaa\n  @\n",
    "1. aaa\n   @\n",
    "> aaa\n@\n",
    "> aaa\n> @\n",
    "> @\n",
    "- @\n",
    "| a |\n| - |\n| b |\n@\n",
    "aaa\n    @\n",
    "    aaa\n    @\n",
    "# h\n@\n",
    "[a]: /url\n@\n",
    "<div>\n@\n</div>\n",
    "```\n@\n```\n",
];

/// Both sides of each grammar's threshold. One is a setext underline and
/// nothing else, three is where a thematic break starts, nine is the longest
/// underline anywhere in the spec, ten is where the sibling parser starts
/// reading the row as decoration, and thirty-five is a real 青空文庫
/// separator. The widths are listed rather than derived: naming the sibling's
/// constant here would pin this crate to a number that lives in the other
/// parser, which is the same reason the implementation has no threshold.
const RULE_WIDTHS: &[usize] = &[1, 3, 9, 10, 35];

#[test]
fn canonicalize_is_the_identity_on_every_spec_example() {
    let commonmark = load(COMMONMARK);
    let gfm = load(GFM);
    assert_eq!(commonmark.len(), 652, "re-run `just spec-refresh`");
    assert_eq!(gfm.len(), 672, "re-run `just spec-refresh`");

    let mut divergences = Vec::new();
    for (corpus, examples) in [("CommonMark", &commonmark), ("GFM", &gfm)] {
        for example in examples {
            for (shape, doc) in [
                ("bare", example.markdown.clone()),
                ("in a blockquote", inside_a_blockquote(&example.markdown)),
                ("in a list item", inside_a_list_item(&example.markdown)),
            ] {
                let out = canonical(&doc);
                let where_ = || {
                    format!(
                        "{corpus} example {} ({}), {shape}",
                        example.example, example.section
                    )
                };
                assert_eq!(
                    canonical(&out),
                    out,
                    "I3: canonicalize did not settle for {}\n  src = {doc:?}",
                    where_()
                );
                if out == doc {
                    continue;
                }
                // The corpus carries no 青空文庫 notation, so anything that
                // moved is the delegate rewriting CommonMark. Two things are
                // demanded of it before it may be counted rather than failed:
                // the difference is exactly the one normalisation
                // `canonicalize` documents, and it costs the document nothing.
                assert_eq!(
                    out,
                    blank_runs_collapsed(&doc),
                    "{} was rewritten by something other than the blank-run collapse",
                    where_()
                );
                assert_eq!(
                    html(&doc),
                    html(&out),
                    "{} changed meaning under canonicalize",
                    where_()
                );
                divergences.push(where_());
            }
        }
    }
    assert_eq!(
        divergences.len(),
        EXPECTED_DIVERGENCES,
        "the set of spec examples `canonicalize` does not reproduce verbatim moved:\n  {}",
        divergences.join("\n  "),
    );
}

#[test]
fn every_source_mutating_step_of_the_sibling_parser_is_protected() {
    // The sibling parser (aozora 0.5.0) rewrites a source in five steps
    // before any other stage reads it, and documents all five. Each is
    // pinned here — protected, or named as a normalisation `canonicalize`
    // documents — so the family is closed against the parser's own list
    // rather than against what a corpus happened to contain.

    // 1. A leading BOM run is preserved as source text. comrak ignores one
    //    at render time, but `canonicalize` is a source-to-source API and
    //    does not silently shorten a longer run.
    assert_eq!(canonical("\u{FEFF}abc\n"), "\u{FEFF}abc\n");
    assert_eq!(html("\u{FEFF}abc\n"), html("abc\n"));
    assert_eq!(canonical("a\u{FEFF}b\n"), "a\u{FEFF}b\n");
    assert_eq!(canonical("\u{FEFF}\u{FEFF}abc\n"), "\u{FEFF}\u{FEFF}abc\n");
    assert_eq!(
        html("\u{FEFF}\u{FEFF}abc\n"),
        html(canonical("\u{FEFF}\u{FEFF}abc\n").as_str())
    );

    // 2. CR/LF normalisation. CommonMark does not distinguish the three line
    //    endings, so this changes no document; `canonicalize`'s rustdoc says so.
    assert_eq!(canonical("a\r\nb\r\n"), "a\nb\n");
    assert_eq!(canonical("a\rb\n"), "a\nb\n");

    // 3. Accent decomposition inside `〔…〕`. This one is 青空文庫 notation,
    //    so canonicalising it is the crate's job rather than a defect — and
    //    it is scoped: the same digraph outside the brackets is untouched.
    assert_eq!(canonical("〔e'tude〕\n"), "〔étude〕\n");
    assert_eq!(canonical("e'tude\n"), "e'tude\n");
    assert_eq!(canonical("〔abc〕\n"), "〔abc〕\n");

    // 4 and 5 are the two this crate must protect outright, because both
    //   rewrite bytes CommonMark has already claimed. They are exhaustive
    //   enough to want a matrix each; see the two tests below.
    assert_eq!(canonical("- aaa\n==========\n"), "- aaa\n==========\n");
    assert_eq!(canonical("a\u{E001}b\n"), "a\u{E001}b\n");
}

#[test]
fn a_rule_row_stays_in_the_block_that_owns_it() {
    // Step 4, exhaustively. The sibling isolates a row of ten or more
    // `-`/`=`/`_` by inserting a blank line in front of it — right where
    // CommonMark has not claimed the bytes, and wrong everywhere it has.
    // A3 (#168) protected the two rows comrak reports as nodes of their own,
    // a thematic break and a setext underline. That is not the family: a
    // `=`-run is not a thematic break at all, so it lands as a paragraph's
    // own text, as a lazy continuation, or as a table row, and the blank line
    // splits whichever block owned it.
    //
    // Before DEV-232 eighteen of the combinations below round-tripped wrong,
    // fourteen changing the rendered HTML — among them
    // `- aaa\n==========\n` (the list
    // split), `> aaa\n==========\n` (the blockquote split),
    // `| a |\n| - |\n| b |\n==========\n` (the last row fell out of the
    // table) and `aaa\n    ----------\n` (one paragraph became a paragraph
    // plus an indented code block).
    let mut checked = 0usize;
    for rule in ['-', '=', '_'] {
        for &width in RULE_WIDTHS {
            let row = String::from(rule).repeat(width);
            for context in BLOCK_CONTEXTS {
                let src = context.replace('@', &row);
                assert_eq!(canonical(&src), src, "rule row rewritten in {context:?}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 3 * RULE_WIDTHS.len() * BLOCK_CONTEXTS.len());
}

#[test]
fn every_reserved_codepoint_in_the_source_comes_back_as_written() {
    // Step 5, exhaustively. Four of the five codepoints this crate reserves
    // are rewritten to U+FFFD by the sibling on sight; the fifth is the mask
    // `verbatim_regions` splices with. Before DEV-232 only the mask was
    // lifted, so an author who typed U+E001 got a replacement character back
    // — the source destroyed, in every one of the contexts below.
    //
    // Read off `sentinels::ALL` rather than re-listed, so a codepoint added
    // to the crate's reserved set is covered here without editing this.
    let mut checked = 0usize;
    for reserved in sentinels::ALL {
        for context in BLOCK_CONTEXTS {
            let src = context.replace('@', &format!("a{reserved}b"));
            assert_eq!(
                canonical(&src),
                src,
                "reserved U+{:04X} rewritten in {context:?}",
                reserved as u32
            );
            checked += 1;
        }
        // …and as the whole of a line, where there is no prose around it to
        // keep the line from reading as something else.
        let alone = format!("{reserved}\n");
        assert_eq!(canonical(&alone), alone);
    }
    assert_eq!(checked, sentinels::ALL.len() * BLOCK_CONTEXTS.len());
}
