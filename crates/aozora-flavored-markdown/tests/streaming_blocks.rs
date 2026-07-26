//! Tests for the per-block streaming render API
//! (`render_blocks`).

use aozora_flavored_markdown::ir::Block;
use aozora_flavored_markdown::{
    Diagnostic, Options, RenderedBlock, RenderedBlocks, render, render_blocks, sentinels,
};
use aozora_flavored_markdown_test_support::check_no_sentinel_leak;

/// Render, then hold every block to Tier B. The per-block path restores the
/// code-block mask one block at a time, so the leak has to be measured on the
/// chunk the caller is handed rather than on a re-joined document.
fn render_blocks_checked(src: &str, options: &Options) -> (Vec<RenderedBlock>, Vec<Diagnostic>) {
    let RenderedBlocks {
        blocks,
        diagnostics,
        ..
    } = render_blocks(src, options);
    for block in &blocks {
        if let Err(e) = check_no_sentinel_leak(src, &block.html) {
            panic!("sentinel leaked: {e:?}\n  block html = {:?}", block.html);
        }
    }
    (blocks, diagnostics)
}

#[test]
fn empty_input_yields_no_blocks() {
    let (blocks, diagnostics) = render_blocks_checked("", &Options::default());
    assert!(blocks.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn each_top_level_block_yields_one_rendered_block() {
    let src = "first\n\nsecond\n\nthird\n";
    let (blocks, _) = render_blocks_checked(src, &Options::default());
    assert_eq!(blocks.len(), 3);
    // Each rendered block has its own HTML chunk.
    for block in &blocks {
        assert!(block.html.starts_with("<p>"));
    }
}

#[test]
fn block_source_lines_are_one_based() {
    let src = "a\n\nb\n\nc\n";
    let (blocks, _) = render_blocks_checked(src, &Options::default());
    assert_eq!(blocks[0].source_line, 1);
    assert_eq!(blocks[1].source_line, 3);
    assert_eq!(blocks[2].source_line, 5);
}

#[test]
fn aozora_inline_renders_inside_per_block_html() {
    let src = "｜漢字《かんじ》\n\nplain second\n";
    let (blocks, diagnostics) = render_blocks_checked(src, &Options::default());
    assert!(diagnostics.is_empty());
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].html.contains("<ruby>"));
    assert!(!blocks[0].html.contains("｜"));
    assert!(!blocks[1].html.contains("<ruby>"));
}

#[test]
fn aozora_disabled_path_skips_lex_pre_pass() {
    let src = "first\n\nsecond\n";
    let opts = Options::commonmark();
    let (blocks, diagnostics) = render_blocks_checked(src, &opts);
    assert_eq!(blocks.len(), 2);
    assert!(diagnostics.is_empty());
}

#[test]
fn heading_blocks_carry_their_kind_in_ir() {
    let src = "# Title\n\nbody\n";
    let (blocks, _) = render_blocks_checked(src, &Options::default());
    assert_eq!(blocks.len(), 2);
    let kind = match blocks[0].ir.first() {
        Some(Block::Heading { level, .. }) => Some(*level),
        _ => None,
    };
    assert_eq!(kind, Some(1));
}

// ---------------------------------------------------------------------------
// fenced code blocks — the mask is restored per block, not per document
// ---------------------------------------------------------------------------

#[test]
fn fenced_aozora_triggers_are_restored_in_per_block_html() {
    // Triggers inside a fence are masked with `sentinels::MASK` before the
    // lexer runs (ADR-0010). The document path restores them on its way out;
    // this path has to do the same or the reader gets the PUA glyph.
    let src = "```\n｜青梅《おうめ》\n```\n";
    let (blocks, _) = render_blocks_checked(src, &Options::default());
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0].html.contains("｜青梅《おうめ》"),
        "fenced code must carry the source the author typed, got {:?}",
        blocks[0].html
    );
    // `render_blocks_checked` already covers this, but pin the codepoint so a
    // regression names the mask rather than "some sentinel".
    assert!(
        !blocks[0].html.contains(sentinels::MASK),
        "mask survived into per-block html: {:?}",
        blocks[0].html
    );
}

#[test]
fn a_later_fence_restores_its_own_triggers_not_an_earlier_fence_s() {
    // Restoration consumes the document's originals in source order, so each
    // block must resume where the last one stopped. The two fences carry
    // *different* trigger sequences and the first one is not block 0, so a
    // restore that rewinds to the start replays ｜《》 over the second
    // fence's ［「」］ instead of leaving it alone.
    let src = "本文\n\n```\n｜青梅《おうめ》\n```\n\n```\n［＃「あ」に傍点］\n```\n";
    let (blocks, _) = render_blocks_checked(src, &Options::default());
    assert_eq!(blocks.len(), 3);
    assert!(
        blocks[1].html.contains("｜青梅《おうめ》"),
        "first fence keeps its own literal, got {:?}",
        blocks[1].html
    );
    assert!(
        blocks[2].html.contains("［＃「あ」に傍点］"),
        "second fence must not replay the first fence's triggers, got {:?}",
        blocks[2].html
    );
}

#[test]
fn concatenated_block_html_matches_the_document_render() {
    // The obsidian bridge streams these chunks into one container, so the
    // per-block path owes the whole-document path byte identity — mask
    // restoration, and its order across two fences, included.
    let src =
        "本文\n\n```\n｜青梅《おうめ》\n```\n\n｜鶴見《つるみ》\n\n```\n［＃「あ」に傍点］\n```\n";
    let (blocks, _) = render_blocks_checked(src, &Options::default());
    let joined: String = blocks.iter().map(|block| block.html.as_str()).collect();
    assert_eq!(joined, render(src, &Options::default()).html);
}

// ---------------------------------------------------------------------------
// source-line anchors on the per-block path
// ---------------------------------------------------------------------------

#[test]
fn source_line_anchors_reach_the_per_block_html() {
    let opts = Options::default().with_source_line_anchors(true);
    let (blocks, _) = render_blocks_checked("一行目\n\n三行目\n", &opts);
    assert_eq!(blocks.len(), 2);
    assert!(
        blocks[0]
            .html
            .contains(r#"<p data-aozora-md-source-line="1">"#),
        "first block must be anchored to line 1, got {:?}",
        blocks[0].html
    );
    assert!(
        blocks[1]
            .html
            .contains(r#"<p data-aozora-md-source-line="3">"#),
        "second block must be anchored to line 3, got {:?}",
        blocks[1].html
    );
}

#[test]
fn per_block_html_carries_no_anchor_unless_asked() {
    let (blocks, _) = render_blocks_checked("一行目\n", &Options::default());
    assert_eq!(blocks.len(), 1);
    assert!(!blocks[0].html.contains("data-aozora-md-source-line"));
}

#[test]
fn an_anchored_fenced_block_is_anchored_and_restored_both() {
    // The anchor injector and the mask restore both rewrite the same buffer;
    // running one must not cost the other.
    let opts = Options::default().with_source_line_anchors(true);
    let src = "本文\n\n```\n｜青梅《おうめ》\n```\n";
    let (blocks, _) = render_blocks_checked(src, &opts);
    assert_eq!(blocks.len(), 2);
    assert!(
        blocks[1]
            .html
            .contains(r#"<pre data-aozora-md-source-line="3">"#),
        "the fence must be anchored to line 3, got {:?}",
        blocks[1].html
    );
    assert!(
        blocks[1].html.contains("｜青梅《おうめ》"),
        "the fence must keep its literal, got {:?}",
        blocks[1].html
    );
}
