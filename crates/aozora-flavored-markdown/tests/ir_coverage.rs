//! Coverage-driven IR walker tests.
//!
//! Exercises every public `IrBlock` / `IrInline` variant the v0.1
//! walker knows how to produce, plus the table-row / list-item /
//! sourcepos-range helpers underneath. The goal is to keep the
//! `aozora-flavored-markdown::ir` and `lib::render_to_ir` paths above the
//! coverage gate without leaning on inline-test scaffolding.

use aozora_flavored_markdown::ir::{IrBlock, IrInline, IrTableAlign};
use aozora_flavored_markdown::{Options, render_blocks_to_ir, render_to_ir};

fn ir_for(src: &str) -> Vec<IrBlock> {
    render_to_ir(src, &Options::commonmark()).ir.blocks
}

fn first_inline(block: &IrBlock) -> Option<&IrInline> {
    match block {
        IrBlock::Paragraph { children, .. } | IrBlock::Heading { children, .. } => children.first(),
        _ => None,
    }
}

#[test]
fn paragraph_projects_with_text_inline() {
    let blocks = ir_for("hello world\n");
    assert!(matches!(blocks.as_slice(), [IrBlock::Paragraph { .. }]));
    let inline = first_inline(&blocks[0]).expect("paragraph child");
    assert!(matches!(inline, IrInline::Text { value, .. } if value == "hello world"));
}

#[test]
fn heading_levels_one_through_six_each_project() {
    for level in 1u8..=6 {
        let prefix = "#".repeat(level as usize);
        let src = format!("{prefix} title\n");
        let blocks = ir_for(&src);
        let level_seen = match blocks.first() {
            Some(IrBlock::Heading { level: l, .. }) => Some(*l),
            _ => None,
        };
        assert_eq!(level_seen, Some(level), "level {level} did not project");
    }
}

#[test]
fn blockquote_projects_with_nested_paragraph() {
    let blocks = ir_for("> quoted\n");
    let IrBlock::Blockquote { children, .. } = &blocks[0] else {
        panic!("expected Blockquote, got {:?}", blocks[0]);
    };
    assert!(matches!(children.as_slice(), [IrBlock::Paragraph { .. }]));
}

#[test]
fn unordered_list_projects_with_items() {
    let blocks = ir_for("- a\n- b\n");
    let IrBlock::List {
        ordered,
        items,
        start,
        ..
    } = &blocks[0]
    else {
        panic!("expected List, got {:?}", blocks[0]);
    };
    assert!(!*ordered);
    assert_eq!(items.len(), 2);
    assert!(start.is_none());
}

#[test]
fn ordered_list_with_nondefault_start_carries_start() {
    let blocks = ir_for("3. a\n4. b\n");
    let IrBlock::List { ordered, start, .. } = &blocks[0] else {
        panic!("expected List, got {:?}", blocks[0]);
    };
    assert!(*ordered);
    assert_eq!(*start, Some(3));
}

#[test]
fn ordered_list_with_default_start_omits_start() {
    let blocks = ir_for("1. a\n");
    let IrBlock::List { start, .. } = &blocks[0] else {
        panic!("expected List, got {:?}", blocks[0]);
    };
    assert!(start.is_none());
}

#[test]
fn fenced_code_block_with_language_carries_lang() {
    let blocks = ir_for("```rust\nfn x() {}\n```\n");
    let IrBlock::CodeBlock { lang, value, .. } = &blocks[0] else {
        panic!("expected CodeBlock, got {:?}", blocks[0]);
    };
    assert_eq!(lang.as_deref(), Some("rust"));
    assert!(value.contains("fn x()"));
}

#[test]
fn fenced_code_block_without_language_omits_lang() {
    let blocks = ir_for("```\nplain\n```\n");
    let IrBlock::CodeBlock { lang, .. } = &blocks[0] else {
        panic!("expected CodeBlock, got {:?}", blocks[0]);
    };
    assert!(lang.is_none());
}

#[test]
fn thematic_break_projects() {
    let blocks = ir_for("---\n");
    assert!(matches!(blocks[0], IrBlock::ThematicBreak { .. }));
}

#[test]
fn gfm_table_projects_with_alignment_and_rows() {
    // GFM table needs the `table` extension; default has it but
    // commonmark() does not, so use default to force tables on.
    let src = "| a | b | c |\n|---|:--:|--:|\n| 1 | 2 | 3 |\n";
    let result = render_to_ir(src, &Options::default());
    let IrBlock::Table {
        header,
        rows,
        align,
        ..
    } = &result.ir.blocks[0]
    else {
        panic!("expected Table, got {:?}", result.ir.blocks[0]);
    };
    assert_eq!(header.cells.len(), 3);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cells.len(), 3);
    assert_eq!(align.len(), 3);
    assert!(matches!(align[0], IrTableAlign::Default));
    assert!(matches!(align[1], IrTableAlign::Center));
    assert!(matches!(align[2], IrTableAlign::Right));
}

#[test]
fn empty_gfm_table_with_only_header_still_projects() {
    let src = "| a | b |\n|---|---|\n";
    let result = render_to_ir(src, &Options::default());
    let IrBlock::Table { rows, .. } = &result.ir.blocks[0] else {
        panic!("expected Table, got {:?}", result.ir.blocks[0]);
    };
    assert!(rows.is_empty());
}

#[test]
fn strong_inline_projects() {
    let blocks = ir_for("**bold**\n");
    let inline = first_inline(&blocks[0]).expect("paragraph child");
    assert!(matches!(inline, IrInline::Strong { .. }));
}

#[test]
fn emphasis_inline_projects() {
    let blocks = ir_for("*italic*\n");
    let inline = first_inline(&blocks[0]).expect("paragraph child");
    assert!(matches!(inline, IrInline::Emphasis { .. }));
}

#[test]
fn code_inline_projects_with_literal() {
    let blocks = ir_for("an `inline code` span\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_code = children
        .iter()
        .any(|c| matches!(c, IrInline::Code { value, .. } if value == "inline code"));
    assert!(saw_code, "expected an IrInline::Code, got {children:?}");
}

#[test]
fn link_with_title_projects_title() {
    let blocks = ir_for("[label](https://example.com \"Hover\")\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_link = children.iter().any(|c| {
        matches!(
            c,
            IrInline::Link { href, title, .. }
                if href == "https://example.com" && title.as_deref() == Some("Hover")
        )
    });
    assert!(saw_link, "expected IrInline::Link with title");
}

#[test]
fn link_without_title_omits_title_field() {
    let blocks = ir_for("[label](https://example.com)\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_link = children
        .iter()
        .any(|c| matches!(c, IrInline::Link { title, .. } if title.is_none()));
    assert!(saw_link, "expected IrInline::Link with no title");
}

#[test]
fn soft_break_projects_as_non_hard_line_break() {
    let blocks = ir_for("line one\nline two\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_soft = children
        .iter()
        .any(|c| matches!(c, IrInline::LineBreak { hard: false, .. }));
    assert!(saw_soft, "expected soft IrInline::LineBreak");
}

#[test]
fn hard_break_projects_as_hard_line_break() {
    let blocks = ir_for("line one  \nline two\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_hard = children
        .iter()
        .any(|c| matches!(c, IrInline::LineBreak { hard: true, .. }));
    assert!(saw_hard, "expected hard IrInline::LineBreak");
}

#[test]
fn image_inline_projects_with_url_alt_and_optional_title() {
    let blocks = ir_for("![alt text](pic.png \"Caption\")\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_image = children.iter().any(|c| {
        matches!(
            c,
            IrInline::Image { url, title, alt, .. }
                if url == "pic.png"
                    && title.as_deref() == Some("Caption")
                    && alt
                        .iter()
                        .any(|a| matches!(a, IrInline::Text { value, .. } if value == "alt text"))
        )
    });
    assert!(saw_image, "expected IrInline::Image, got: {children:#?}");
}

#[test]
fn image_without_title_omits_title_field() {
    let blocks = ir_for("![alt](pic.png)\n");
    let IrBlock::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_image = children
        .iter()
        .any(|c| matches!(c, IrInline::Image { title, .. } if title.is_none()));
    assert!(saw_image, "expected IrInline::Image with no title");
}

#[test]
fn aozora_disabled_render_to_ir_runs_commonmark_path() {
    let opts = Options::commonmark().with_source_line_anchors(true);
    let result = render_to_ir("# Heading\n\nbody\n", &opts);
    assert_eq!(result.ir.blocks.len(), 2);
    assert!(matches!(result.ir.blocks[0], IrBlock::Heading { .. }));
    assert!(result.html.contains("data-aozora-md-source-line=\"1\""));
}

#[test]
fn aozora_enabled_render_to_ir_with_anchors_path() {
    let opts = Options::default().with_source_line_anchors(true);
    let result = render_to_ir("# Heading\n\nbody\n", &opts);
    assert_eq!(result.ir.blocks.len(), 2);
    assert!(result.html.contains("data-aozora-md-source-line=\"1\""));
}

#[test]
fn render_blocks_to_ir_empty_aozora_disabled_path() {
    let opts = Options::commonmark();
    let (blocks, diagnostics) = render_blocks_to_ir("", &opts);
    assert!(blocks.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn render_blocks_to_ir_paragraph_carries_source_line() {
    let (blocks, _) = render_blocks_to_ir("first\n\nsecond\n", &Options::default());
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].source_line, 1);
    assert_eq!(blocks[1].source_line, 3);
}

#[test]
fn options_with_source_line_anchors_builder_toggles_field() {
    // No getter to read the bit back with — `Options` is write-only
    // configuration — so the toggle is observed where it is meant to be
    // observed, in the rendered HTML.
    let on = render_to_ir("p\n", &Options::default().with_source_line_anchors(true));
    assert!(
        on.html.contains("data-aozora-md-source-line"),
        "{}",
        on.html
    );
    let off = render_to_ir("p\n", &Options::default().with_source_line_anchors(false));
    assert!(
        !off.html.contains("data-aozora-md-source-line"),
        "{}",
        off.html
    );
}
