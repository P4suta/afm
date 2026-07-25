//! What a render does with a construct it cannot locate in the source.
//!
//! This crate renders a 青空文庫 construct by handing the construct's own
//! source run back to the parser. Finding that run is free when the parser's
//! ranges were shown to address the caller's source, and a search otherwise
//! — the parser rewrites a document before lexing it, and measures its
//! ranges against the rewrite.
//!
//! The search is good enough that no realistic document defeats it (the
//! fidelity gate in `aozora_parity` carries the shapes real files have, CRLF
//! and decorative rules included). The inputs here defeat it on purpose, by
//! stacking a rewrite this crate reproduces — CRLF — under one it does not:
//! an accent digraph, which combines two characters into one and so leaves
//! the construct's own line unlexable in isolation. That is the last resort,
//! and it has three contracts:
//!
//! * the construct renders as nothing, rather than as a guess;
//! * the block structure around it stays balanced — a container that was
//!   never opened is never closed, and one that was opened is closed even
//!   when the closing marker is the run that went missing;
//! * the render says so, with a diagnostic. Losing an author's text quietly
//!   is the one outcome worth failing this test over.

use aozora_flavored_markdown::{Options, render, render_blocks_to_ir, render_to_ir};
use aozora_flavored_markdown_test_support::check_html_tag_balance;

/// The diagnostic code a lost construct raises.
const UNRESOLVED: &str = "aozora-md::constructs_unresolved";

/// A document that reaches the last resort: CRLF moves every reported
/// offset, and the accent digraph on the notation's own line stops that line
/// from being searched.
const LOST_RUBY: &str = "本文\r\n----------\r\n〔e'tude〕｜青梅《おうめ》";

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
fn a_lost_construct_renders_as_nothing_rather_than_as_a_guess() {
    assert_eq!(
        html_of(LOST_RUBY),
        "<p>本文</p>\n<hr />\n<p>〔étude〕</p>\n"
    );
}

#[test]
fn a_lost_construct_is_reported() {
    assert!(
        codes(LOST_RUBY).contains(&UNRESOLVED),
        "a render that dropped a construct must say so: {:?}",
        codes(LOST_RUBY)
    );
}

#[test]
fn a_document_that_lost_nothing_stays_quiet() {
    // The same shape without the digraph: the run is found, so there is
    // nothing to report. Worth pinning — a diagnostic every CRLF file
    // raises is a diagnostic nobody reads.
    let clean = "本文\r\n----------\r\n｜青梅《おうめ》";
    assert!(html_of(clean).contains("<ruby>青梅"));
    assert!(
        !codes(clean).contains(&UNRESOLVED),
        "nothing was lost here: {:?}",
        codes(clean)
    );
}

#[test]
fn a_container_whose_open_went_missing_is_never_closed() {
    // The open renders to nothing, so it opens nothing — and the close that
    // follows is then an orphan, which the splicer already drops.
    let src = "本文\r\n----------\r\n〔e'tude〕［＃ここから字下げ］\r\n\r\n中\r\n\r\n［＃ここで字下げ終わり］";
    let html = html_of(src);
    assert!(
        !html.contains("<div") && !html.contains("</div>"),
        "no container should survive: {html}"
    );
    assert!(check_html_tag_balance(&html).is_ok(), "{html}");
}

#[test]
fn a_container_whose_close_went_missing_is_closed_anyway() {
    // A close carries no payload — every container closes the same way — so
    // the canonical close notation stands in, and it stands in *here*
    // rather than at the end of the document.
    let src = "本文\r\n----------\r\n［＃ここから字下げ］\r\n\r\n中\r\n\r\n〔e'tude〕［＃ここで字下げ終わり］";
    let html = html_of(src);
    assert!(html.contains("<div class=\"aozora-md-container"), "{html}");
    assert!(check_html_tag_balance(&html).is_ok(), "{html}");
    assert!(
        html.ends_with("</div>"),
        "the close belongs where the marker was: {html}"
    );
}

#[test]
fn a_lost_block_leaf_takes_its_paragraph_with_it() {
    // The sentinel stood alone in its paragraph, so dropping the construct
    // has to drop the paragraph too — leaking the PUA codepoint would break
    // Tier B.
    let src = "本文\r\n----------\r\n〔e'tude〕［＃改ページ］";
    let html = html_of(src);
    assert_eq!(html, "<p>本文</p>\n<hr />\n<p>〔étude〕</p>\n");
    assert!(!html.contains('\u{E002}'), "Tier B: {html}");
}

#[test]
fn both_ir_front_doors_report_what_the_html_one_does() {
    // The IR and the HTML describe the same document, so a construct lost
    // from one is lost from the other — and both say so.
    let whole = render_to_ir(LOST_RUBY, &Options::default());
    assert!(whole.diagnostics.iter().any(|d| d.code == UNRESOLVED));
    assert_eq!(whole.html, html_of(LOST_RUBY));

    let (blocks, diagnostics) = render_blocks_to_ir(LOST_RUBY, &Options::default());
    assert!(diagnostics.iter().any(|d| d.code == UNRESOLVED));
    let streamed: String = blocks.iter().map(|block| block.html.as_str()).collect();
    assert_eq!(streamed, html_of(LOST_RUBY));
    assert!(
        blocks.iter().flat_map(|block| &block.ir).count() > 0,
        "the surviving blocks are still projected"
    );
}
