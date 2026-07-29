//! Coverage-driven IR walker tests.
//!
//! Exercises every public `Block` / `Inline` variant the v0.1
//! walker knows how to produce, plus the table-row / list-item /
//! sourcepos-range helpers underneath. The goal is to keep the
//! `aozora-flavored-markdown::ir` and `lib::render_to_ir` paths above the
//! coverage gate without leaning on inline-test scaffolding.

use aozora_flavored_markdown::ir::{Block, Inline, TableAlign};
use aozora_flavored_markdown::{
    Options, RenderedBlocks, diagnose, render, render_blocks, render_to_ir, sentinels,
};

fn ir_for(src: &str) -> Vec<Block> {
    render_to_ir(src, &Options::commonmark()).ir.blocks
}

fn first_inline(block: &Block) -> Option<&Inline> {
    match block {
        Block::Paragraph { children, .. } | Block::Heading { children, .. } => children.first(),
        _ => None,
    }
}

#[test]
fn paragraph_projects_with_text_inline() {
    let blocks = ir_for("hello world\n");
    assert!(matches!(blocks.as_slice(), [Block::Paragraph { .. }]));
    let inline = first_inline(&blocks[0]).expect("paragraph child");
    assert!(matches!(inline, Inline::Text { value, .. } if value == "hello world"));
}

#[test]
fn heading_levels_one_through_six_each_project() {
    for level in 1u8..=6 {
        let prefix = "#".repeat(level as usize);
        let src = format!("{prefix} title\n");
        let blocks = ir_for(&src);
        let level_seen = match blocks.first() {
            Some(Block::Heading { level: l, .. }) => Some(*l),
            _ => None,
        };
        assert_eq!(level_seen, Some(level), "level {level} did not project");
    }
}

#[test]
fn blockquote_projects_with_nested_paragraph() {
    let blocks = ir_for("> quoted\n");
    let Block::Blockquote { children, .. } = &blocks[0] else {
        panic!("expected Blockquote, got {:?}", blocks[0]);
    };
    assert!(matches!(children.as_slice(), [Block::Paragraph { .. }]));
}

#[test]
fn unordered_list_projects_with_items() {
    let blocks = ir_for("- a\n- b\n");
    let Block::List {
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
    let Block::List { ordered, start, .. } = &blocks[0] else {
        panic!("expected List, got {:?}", blocks[0]);
    };
    assert!(*ordered);
    assert_eq!(*start, Some(3));
}

#[test]
fn ordered_list_with_default_start_omits_start() {
    let blocks = ir_for("1. a\n");
    let Block::List { start, .. } = &blocks[0] else {
        panic!("expected List, got {:?}", blocks[0]);
    };
    assert!(start.is_none());
}

#[test]
fn fenced_code_block_with_language_carries_lang() {
    let blocks = ir_for("```rust\nfn x() {}\n```\n");
    let Block::Code { lang, value, .. } = &blocks[0] else {
        panic!("expected a code block, got {:?}", blocks[0]);
    };
    assert_eq!(lang.as_deref(), Some("rust"));
    assert!(value.contains("fn x()"));
}

#[test]
fn fenced_code_block_without_language_omits_lang() {
    let blocks = ir_for("```\nplain\n```\n");
    let Block::Code { lang, .. } = &blocks[0] else {
        panic!("expected a code block, got {:?}", blocks[0]);
    };
    assert!(lang.is_none());
}

#[test]
fn container_fence_restores_info_and_body_in_html_ir_and_streaming() {
    let src = "> ```lang《ignored》 extra《not-rendered》\n> ｜body《literal》\n> ```\n";
    let rendered = render_to_ir(src, &Options::default());
    let [Block::Blockquote { children, .. }] = rendered.ir.blocks.as_slice() else {
        panic!("expected one blockquote, got {:?}", rendered.ir.blocks);
    };
    let [Block::Code { lang, value, .. }] = children.as_slice() else {
        panic!("expected one nested code block, got {children:?}");
    };
    assert_eq!(
        lang.as_deref(),
        Some("lang《ignored》 extra《not-rendered》")
    );
    assert_eq!(value, "｜body《literal》\n");
    assert!(
        rendered.html.contains("language-lang《ignored》"),
        "first info word was not restored: {:?}",
        rendered.html
    );
    assert!(
        rendered.html.contains("｜body《literal》"),
        "code body was not restored: {:?}",
        rendered.html
    );
    assert!(
        !rendered.html.contains('\u{E000}'),
        "mask leaked from HTML: {:?}",
        rendered.html
    );

    let streamed = render_blocks(src, &Options::default());
    let [block] = streamed.blocks.as_slice() else {
        panic!("expected one streamed block, got {:?}", streamed.blocks);
    };
    assert_eq!(block.ir, rendered.ir.blocks);
    assert_eq!(block.html, rendered.html);
}

fn assert_fenced_front_doors_agree(src: &str) -> Vec<Block> {
    let options = Options::default();
    let rendered = render(src, &options);
    let projected = render_to_ir(src, &options);
    let streamed = render_blocks(src, &options);
    let joined_html: String = streamed
        .blocks
        .iter()
        .map(|block| block.html.as_str())
        .collect();
    let joined_ir: Vec<Block> = streamed
        .blocks
        .iter()
        .flat_map(|block| block.ir.iter().cloned())
        .collect();
    let diagnosed = diagnose(src, &options);

    assert_eq!(rendered.html, projected.html, "{src:?}");
    assert_eq!(rendered.html, joined_html, "{src:?}");
    assert_eq!(projected.ir.blocks, joined_ir, "{src:?}");
    assert_eq!(rendered.diagnostics, diagnosed, "{src:?}");
    assert_eq!(projected.diagnostics, diagnosed, "{src:?}");
    assert_eq!(streamed.diagnostics, diagnosed, "{src:?}");
    assert!(
        !rendered.html.contains([
            sentinels::INLINE,
            sentinels::BLOCK_LEAF,
            sentinels::BLOCK_OPEN,
            sentinels::BLOCK_CLOSE,
        ]),
        "a construct sentinel leaked from {src:?}: {:?}",
        rendered.html
    );
    projected.ir.blocks
}

#[test]
fn fence_shapes_and_line_endings_agree_across_every_front_door() {
    let mut sources = vec![
        "> ```lang《quote》\n> ｜body《literal》\n> ```\n".to_owned(),
        "- item\n\n  ~~~lang《list》\n  ［＃literal］\n  ~~~\n".to_owned(),
        "```\n｜first\n```\n\n~~~lang《unclosed》\n［second］\n".to_owned(),
    ];
    for ending in ["\n", "\r\n", "\r"] {
        sources.push(format!(
            "```lang《{ending:?}》{ending}｜body《literal》{ending}```{ending}"
        ));
    }
    for src in sources {
        assert_fenced_front_doors_agree(&src);
    }
}

#[test]
fn unclosed_cr_fence_restores_double_angle_info_without_a_reserved_leak() {
    let src = "~~~≪\u{12}≫\u{80}\r";
    let blocks = assert_fenced_front_doors_agree(src);
    let [Block::Code { lang, value, .. }] = blocks.as_slice() else {
        panic!("expected the unclosed source to stay one code block: {blocks:?}");
    };
    assert_eq!(lang.as_deref(), Some("≪\u{12}≫\u{80}"));
    assert!(value.is_empty());
}

#[test]
fn info_entities_and_raw_reserved_codepoints_follow_the_public_contract() {
    for sentinel in [
        sentinels::INLINE,
        sentinels::BLOCK_LEAF,
        sentinels::BLOCK_OPEN,
        sentinels::BLOCK_CLOSE,
    ] {
        let src = format!(
            "```lang&#x{:X};\nraw{sentinel} ｜body《literal》\n```\n",
            sentinel as u32
        );
        let blocks = assert_fenced_front_doors_agree(&src);
        let [Block::Code { lang, value, .. }] = blocks.as_slice() else {
            panic!("expected one code block, got {blocks:?}");
        };
        assert_eq!(lang.as_deref(), Some("lang�"));
        assert_eq!(value, "raw� ｜body《literal》\n");
    }

    let src = format!(
        "{}\n```\n｜body《literal》{}\n```\n",
        sentinels::MASK,
        sentinels::MASK
    );
    let blocks = assert_fenced_front_doors_agree(&src);
    let Block::Code { value, .. } = &blocks[1] else {
        panic!("expected the second block to be code, got {blocks:?}");
    };
    assert_eq!(value, &format!("｜body《literal》{}\n", sentinels::MASK));
    assert!(
        render(&src, &Options::default())
            .html
            .contains(sentinels::MASK)
    );
}

#[test]
fn raw_mask_stand_down_does_not_let_a_fenced_construct_shift_the_cursor() {
    let src = format!(
        "{}\n```\n｜内《うち》\n```\n\n｜外《そと》\n",
        sentinels::MASK
    );
    let blocks = assert_fenced_front_doors_agree(&src);
    let [
        _,
        Block::Code { value, .. },
        Block::Paragraph { children, .. },
    ] = blocks.as_slice()
    else {
        panic!("expected mask paragraph, code, and ruby paragraph: {blocks:?}");
    };
    assert_eq!(value, "｜内《うち》\n");
    let [Inline::Aozora { kind, html, .. }] = children.as_slice() else {
        panic!("the construct after the fence must project as one ruby: {children:?}");
    };
    assert_eq!(kind, "ruby");
    assert!(
        html.contains('外'),
        "the later ruby lost its base: {html:?}"
    );
    assert!(
        html.contains("そと"),
        "the later ruby lost its reading: {html:?}"
    );
    assert!(
        !html.contains('内'),
        "the fenced ruby consumed the cursor: {html:?}"
    );
    let ruby_html = html.clone();

    let document_html = render(&src, &Options::default()).html;
    assert!(
        document_html.contains("｜内《うち》"),
        "fenced source changed: {document_html:?}"
    );
    assert!(
        document_html.contains(&ruby_html),
        "the projected later ruby differs from HTML: {document_html:?}"
    );
}

#[test]
fn an_ir_depth_cutoff_cannot_shift_a_later_fence_snapshot() {
    let quoted = "> ".repeat(300);
    let src = format!(
        "{quoted}```deep《info》\n{quoted}｜deep《literal》\n{quoted}```\n\n\
         ```later《info》\n［＃「later」に傍点］\n```\n"
    );
    let blocks = assert_fenced_front_doors_agree(&src);
    let Some(Block::Code { lang, value, .. }) = blocks.last() else {
        panic!("the later top-level fence must survive IR truncation: {blocks:?}");
    };
    assert_eq!(lang.as_deref(), Some("later《info》"));
    assert_eq!(value, "［＃「later」に傍点］\n");
}

#[test]
fn thematic_break_projects() {
    let blocks = ir_for("---\n");
    assert!(matches!(blocks[0], Block::ThematicBreak { .. }));
}

#[test]
fn gfm_table_projects_with_alignment_and_rows() {
    // GFM table needs the `table` extension; default has it but
    // commonmark() does not, so use default to force tables on.
    let src = "| a | b | c |\n|---|:--:|--:|\n| 1 | 2 | 3 |\n";
    let result = render_to_ir(src, &Options::default());
    let Block::Table {
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
    assert!(matches!(align[0], TableAlign::Default));
    assert!(matches!(align[1], TableAlign::Center));
    assert!(matches!(align[2], TableAlign::Right));
}

#[test]
fn empty_gfm_table_with_only_header_still_projects() {
    let src = "| a | b |\n|---|---|\n";
    let result = render_to_ir(src, &Options::default());
    let Block::Table { rows, .. } = &result.ir.blocks[0] else {
        panic!("expected Table, got {:?}", result.ir.blocks[0]);
    };
    assert!(rows.is_empty());
}

#[test]
fn strong_inline_projects() {
    let blocks = ir_for("**bold**\n");
    let inline = first_inline(&blocks[0]).expect("paragraph child");
    assert!(matches!(inline, Inline::Strong { .. }));
}

#[test]
fn emphasis_inline_projects() {
    let blocks = ir_for("*italic*\n");
    let inline = first_inline(&blocks[0]).expect("paragraph child");
    assert!(matches!(inline, Inline::Emphasis { .. }));
}

#[test]
fn code_inline_projects_with_literal() {
    let blocks = ir_for("an `inline code` span\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_code = children
        .iter()
        .any(|c| matches!(c, Inline::Code { value, .. } if value == "inline code"));
    assert!(saw_code, "expected an Inline::Code, got {children:?}");
}

#[test]
fn link_with_title_projects_title() {
    let blocks = ir_for("[label](https://example.com \"Hover\")\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_link = children.iter().any(|c| {
        matches!(
            c,
            Inline::Link { href, title, .. }
                if href == "https://example.com" && title.as_deref() == Some("Hover")
        )
    });
    assert!(saw_link, "expected Inline::Link with title");
}

#[test]
fn link_without_title_omits_title_field() {
    let blocks = ir_for("[label](https://example.com)\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_link = children
        .iter()
        .any(|c| matches!(c, Inline::Link { title, .. } if title.is_none()));
    assert!(saw_link, "expected Inline::Link with no title");
}

#[test]
fn soft_break_projects_as_non_hard_line_break() {
    let blocks = ir_for("line one\nline two\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_soft = children
        .iter()
        .any(|c| matches!(c, Inline::LineBreak { hard: false, .. }));
    assert!(saw_soft, "expected soft Inline::LineBreak");
}

#[test]
fn hard_break_projects_as_hard_line_break() {
    let blocks = ir_for("line one  \nline two\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_hard = children
        .iter()
        .any(|c| matches!(c, Inline::LineBreak { hard: true, .. }));
    assert!(saw_hard, "expected hard Inline::LineBreak");
}

#[test]
fn image_inline_projects_with_url_alt_and_optional_title() {
    let blocks = ir_for("![alt text](pic.png \"Caption\")\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_image = children.iter().any(|c| {
        matches!(
            c,
            Inline::Image { url, title, alt, .. }
                if url == "pic.png"
                    && title.as_deref() == Some("Caption")
                    && alt
                        .iter()
                        .any(|a| matches!(a, Inline::Text { value, .. } if value == "alt text"))
        )
    });
    assert!(saw_image, "expected Inline::Image, got: {children:#?}");
}

#[test]
fn image_without_title_omits_title_field() {
    let blocks = ir_for("![alt](pic.png)\n");
    let Block::Paragraph { children, .. } = &blocks[0] else {
        panic!("expected Paragraph");
    };
    let saw_image = children
        .iter()
        .any(|c| matches!(c, Inline::Image { title, .. } if title.is_none()));
    assert!(saw_image, "expected Inline::Image with no title");
}

#[test]
fn aozora_disabled_render_to_ir_runs_commonmark_path() {
    let opts = Options::commonmark().with_source_line_anchors(true);
    let result = render_to_ir("# Heading\n\nbody\n", &opts);
    assert_eq!(result.ir.blocks.len(), 2);
    assert!(matches!(result.ir.blocks[0], Block::Heading { .. }));
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
fn render_blocks_empty_aozora_disabled_path() {
    let opts = Options::commonmark();
    let RenderedBlocks {
        blocks,
        diagnostics,
        ..
    } = render_blocks("", &opts);
    assert!(blocks.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn render_blocks_paragraph_carries_source_line() {
    let RenderedBlocks { blocks, .. } = render_blocks("first\n\nsecond\n", &Options::default());
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
