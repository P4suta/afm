//! What a render does with the documents that are hardest to locate a
//! construct in.
//!
//! This crate renders a 青空文庫 construct by handing the construct's own
//! source run back to the parser. Finding that run means knowing where the
//! notation sits in the text the caller wrote — and the parser canonicalises
//! a document before reading it, so the two are not always the same text.
//!
//! The parser maps every range it reports back through that canonicalisation,
//! so the documents below — CRLF, a BOM, a decorative rule, an accent digraph
//! that combines two characters into one, and every stacking of them — all
//! resolve. This file is the gate on that: each one renders its notation,
//! keeps its block structure balanced, and says nothing.
//!
//! The diagnostic for a construct that *cannot* be placed still exists, for
//! a range that does not address the text it was measured against. Nothing
//! the shipped parser emits reaches it; it is pinned directly in
//! `crate::constructs`'s unit tests instead of through a document here.

use aozora_flavored_markdown::{Options, render, render_blocks_to_ir, render_to_ir};
use aozora_flavored_markdown_test_support::check_html_tag_balance;

/// The diagnostic code a construct that could not be placed would raise.
const UNRESOLVED: &str = "aozora-md::constructs_unresolved";

/// The shapes a real 青空文庫 file has, and the shape that used to defeat
/// this crate: CRLF moves every offset, the decorative rule moves them
/// again, and the accent digraph combines two characters into one on the
/// notation's own line.
const HARD_SOURCES: &[(&str, &str)] = &[
    ("CRLF", "本文\r\n｜青梅《おうめ》"),
    ("BOM", "\u{feff}｜青梅《おうめ》"),
    ("decorative rule", "本文\n----------\n｜青梅《おうめ》"),
    ("accent digraph", "〔e'tude〕｜青梅《おうめ》"),
    (
        "CRLF plus a decorative rule",
        "本文\r\n----------\r\n｜青梅《おうめ》",
    ),
    (
        "all of them at once",
        "\u{feff}本文\r\n----------\r\n〔e'tude〕｜青梅《おうめ》",
    ),
];

fn html_of(src: &str) -> String {
    render(src, &Options::default()).html
}

fn codes(src: &str) -> Vec<&'static str> {
    render(src, &Options::default())
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn every_hard_source_still_renders_its_notation() {
    for (label, src) in HARD_SOURCES {
        let html = html_of(src);
        assert!(
            html.contains("<ruby>青梅"),
            "{label} ({src:?}) lost its ruby: {html}"
        );
        assert!(
            !codes(src).contains(&UNRESOLVED),
            "{label} lost nothing, so it must stay quiet: {:?}",
            codes(src)
        );
    }
}

/// The accent digraph is the parser's own rewrite: two characters become
/// one, and the text comrak reads carries the combined form.
#[test]
fn an_accent_digraph_reaches_the_output_combined() {
    assert_eq!(
        html_of("〔e'tude〕｜青梅《おうめ》"),
        "<p>〔étude〕<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby></p>\n"
    );
}

/// A decorative rule is a rule, not the underline of a setext heading. The
/// parser isolates it before reading, and comrak reads the isolated form.
#[test]
fn a_decorative_rule_does_not_promote_the_line_above_it() {
    let html = html_of("本文\n----------\n｜青梅《おうめ》");
    assert!(!html.contains("<h2>"), "Tier H: {html}");
    assert!(html.contains("<hr />"), "{html}");
}

#[test]
fn a_container_in_a_hard_source_opens_and_closes_once() {
    let src =
        "本文\r\n----------\r\n［＃ここから字下げ］\r\n\r\n中\r\n\r\n［＃ここで字下げ終わり］";
    let html = html_of(src);
    assert_eq!(html.matches("<div").count(), 1, "{html}");
    assert_eq!(html.matches("</div>").count(), 1, "{html}");
    assert!(check_html_tag_balance(&html).is_ok(), "{html}");
}

#[test]
fn a_container_the_source_never_closes_is_closed_at_the_end() {
    let src = "本文\r\n----------\r\n［＃ここから字下げ］\r\n\r\n中";
    let html = html_of(src);
    assert!(html.contains("<div class=\"aozora-md-container"), "{html}");
    assert!(check_html_tag_balance(&html).is_ok(), "{html}");
    assert!(
        html.ends_with("</div>"),
        "the drain closes what the source left open: {html}"
    );
}

#[test]
fn a_block_leaf_in_a_hard_source_gets_its_own_block() {
    let src = "本文\r\n----------\r\n〔e'tude〕\r\n\r\n［＃改ページ］";
    let html = html_of(src);
    assert!(
        html.contains(r#"<div class="aozora-md-page-break"></div>"#),
        "{html}"
    );
    assert!(!html.contains('\u{E002}'), "Tier B: {html}");
}

#[test]
fn both_ir_front_doors_describe_what_the_html_one_does() {
    // The IR and the HTML describe the same document, so what one renders
    // the other projects — and neither reports a loss.
    for (label, src) in HARD_SOURCES {
        let whole = render_to_ir(src, &Options::default());
        assert!(!whole.diagnostics.iter().any(|d| d.code == UNRESOLVED));
        assert_eq!(whole.html, html_of(src), "{label}");

        let (blocks, diagnostics) = render_blocks_to_ir(src, &Options::default());
        assert!(!diagnostics.iter().any(|d| d.code == UNRESOLVED));
        let streamed: String = blocks.iter().map(|block| block.html.as_str()).collect();
        assert_eq!(streamed, html_of(src), "{label}");
        assert!(
            blocks.iter().flat_map(|block| &block.ir).count() > 0,
            "{label}: the blocks are projected"
        );
    }
}
