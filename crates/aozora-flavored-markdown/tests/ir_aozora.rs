//! IR projection tests for the Aozora half of the IR.
//!
//! `ir_coverage.rs` covers the Markdown-side variants. This file pins the
//! collapsed Aozora surface: every notation lands as a single
//! `Inline::Aozora` / `Block::Aozora` carrying its tag, the source span
//! it came from, and the HTML fragment it renders to — and the sentinel
//! stream stays in lockstep with the HTML splicer while doing it.

use aozora_flavored_markdown::ir::{Block, Inline, Span};
use aozora_flavored_markdown::{Options, render, render_to_ir};

fn ir(src: &str) -> Vec<Block> {
    render_to_ir(src, &Options::default()).ir.blocks
}

fn first_paragraph_inlines(blocks: &[Block]) -> &[Inline] {
    match blocks.first().expect("at least one block") {
        Block::Paragraph { children, .. } => children.as_slice(),
        other => panic!("expected paragraph, got {other:?}"),
    }
}

/// `(kind, span, html)` for every Aozora inline in `inlines`, in order.
fn aozora_inlines(inlines: &[Inline]) -> Vec<(&str, Option<Span>, &str)> {
    inlines
        .iter()
        .filter_map(|inline| match inline {
            Inline::Aozora { kind, span, html } => Some((kind.as_str(), *span, html.as_str())),
            _ => None,
        })
        .collect()
}

/// `(kind, span, html, source_line)` for every Aozora block, in order.
fn aozora_blocks(blocks: &[Block]) -> Vec<(&str, Option<Span>, &str, Option<u32>)> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::Aozora {
                kind,
                span,
                html,
                source_line,
            } => Some((kind.as_str(), *span, html.as_str(), *source_line)),
            _ => None,
        })
        .collect()
}

/// The first Aozora inline tagged `kind`, as `(span, html)`.
fn find_inline_kind<'a>(inlines: &'a [Inline], kind: &str) -> (Option<Span>, &'a str) {
    let Some((_, span, html)) = aozora_inlines(inlines)
        .into_iter()
        .find(|(k, ..)| *k == kind)
    else {
        panic!("no {kind} inline in {inlines:#?}")
    };
    (span, html)
}

/// Slice `src` with a projected span.
///
/// The `expect` is part of the assertion: a span is only reported when its
/// offsets address the source the caller passed in, so on the LF /
/// BOM-free fixtures below it must always be there — and slicing must land
/// on the notation, not merely somewhere. See
/// `spans_are_withheld_when_normalisation_moves_the_bytes` for the other
/// side of that contract.
fn slice(src: &str, span: Option<Span>) -> &str {
    let span = span.expect("a projected notation carries its source span");
    &src[span.start as usize..span.end as usize]
}

#[test]
fn ruby_projects_with_its_span_and_rendered_html() {
    const SRC: &str = "彼は｜青梅《おうめ》へ";
    let blocks = ir(SRC);
    let (span, html) = find_inline_kind(first_paragraph_inlines(&blocks), "ruby");
    assert_eq!(
        slice(SRC, span),
        "｜青梅《おうめ》",
        "the span must slice back to the notation the author wrote"
    );
    assert!(html.contains("<ruby>"), "html: {html}");
    assert!(
        html.contains("青梅") && html.contains("おうめ"),
        "html: {html}"
    );
}

#[test]
fn implicit_and_explicit_ruby_share_one_kind() {
    // The `｜` opener used to surface as an `explicit` flag on a typed Ruby
    // variant. That distinction is the notation's, not this crate's: both
    // spellings tag `ruby`, and the span still tells the two apart.
    let explicit = ir("｜青梅《おうめ》");
    let implicit = ir("青梅《おうめ》");
    let (explicit_span, _) = find_inline_kind(first_paragraph_inlines(&explicit), "ruby");
    let (implicit_span, _) = find_inline_kind(first_paragraph_inlines(&implicit), "ruby");
    assert_eq!(slice("｜青梅《おうめ》", explicit_span), "｜青梅《おうめ》");
    assert_eq!(slice("青梅《おうめ》", implicit_span), "青梅《おうめ》");
}

#[test]
fn angle_quote_projects_with_its_own_kind() {
    let blocks = ir("≪強調≫");
    let (_, html) = find_inline_kind(first_paragraph_inlines(&blocks), "angleQuote");
    assert!(html.contains("強調"), "html: {html}");
}

#[test]
fn bouten_projects_with_rebranded_classes() {
    // Forward-reference bouten: the target appears in the same paragraph
    // before the bracket annotation, and both fold into one span.
    const SRC: &str = "対象［＃「対象」に傍点］";
    let blocks = ir(SRC);
    let (span, html) = find_inline_kind(first_paragraph_inlines(&blocks), "bouten");
    assert_eq!(
        slice(SRC, span),
        SRC,
        "a forward reference spans the referenced text as well as the annotation"
    );
    assert!(
        html.contains("aozora-md-") && !html.contains("\"aozora-b"),
        "fragment must carry aozora-md-* classes only (ADR-0011): {html}"
    );
}

#[test]
fn combine_upright_projects_with_its_own_kind() {
    let blocks = ir("20［＃「20」は縦中横］");
    let (_, html) = find_inline_kind(first_paragraph_inlines(&blocks), "combineUpright");
    assert!(html.contains("20"), "html: {html}");
}

#[test]
fn gaiji_projects_with_its_own_kind() {
    // `※［＃...］` is the gaiji shape: a reference mark followed by a
    // description bracket the classifier resolves against the glyph table.
    const SRC: &str = "※［＃二の字点、1-2-22］";
    let blocks = ir(SRC);
    let (span, _) = find_inline_kind(first_paragraph_inlines(&blocks), "gaiji");
    assert_eq!(slice(SRC, span), SRC);
}

#[test]
fn unclassified_annotation_projects_with_its_payload_in_the_html() {
    const SRC: &str = "前［＃ほげふが］後";
    let blocks = ir(SRC);
    let (span, html) = find_inline_kind(first_paragraph_inlines(&blocks), "directive");
    assert_eq!(slice(SRC, span), "［＃ほげふが］");
    assert!(
        html.contains("ほげふが"),
        "the annotation body survives into the fragment: {html}"
    );
}

#[test]
fn leaf_indent_marker_projects_instead_of_dropping() {
    // `［＃地から１字下げ］` (single-line, not paired) had no typed IR
    // variant and used to vanish from the IR while still appearing in the
    // HTML. With the notation vocabulary delegated, it projects like any
    // other construct.
    const SRC: &str = "前［＃地から１字下げ］後";
    let blocks = ir(SRC);
    let inlines = first_paragraph_inlines(&blocks);
    assert!(
        !aozora_inlines(inlines).is_empty(),
        "the leaf marker must reach the IR: {inlines:#?}"
    );
    // Surrounding text still flows around it.
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::Text { value, .. } if value.contains("前")))
    );
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::Text { value, .. } if value.contains("後")))
    );
}

#[test]
fn page_break_projects_as_a_block_with_its_source_line() {
    const SRC: &str = "前\n\n［＃改ページ］\n\n後";
    let blocks = ir(SRC);
    let page_breaks: Vec<_> = aozora_blocks(&blocks)
        .into_iter()
        .filter(|(kind, ..)| *kind == "pageBreak")
        .collect();
    let [(_, span, html, source_line)] = page_breaks.as_slice() else {
        panic!("expected exactly one pageBreak block, got: {blocks:#?}");
    };
    // `source_line` is the marker's line in the text comrak parsed, which
    // the lexer pads around block markers — it is the same coordinate the
    // HTML anchors use, so the two agree, but it is not the raw-source
    // line. `span` is the one that points back at the author's text.
    assert!(source_line.is_some(), "block markers carry a line");
    assert_eq!(slice(SRC, *span), "［＃改ページ］");
    assert!(!html.is_empty(), "a block marker renders to something");
}

#[test]
fn section_break_projects_as_a_block() {
    let blocks = ir("前\n\n［＃改丁］\n\n後");
    assert!(
        aozora_blocks(&blocks)
            .iter()
            .any(|(kind, ..)| *kind == "sectionBreak"),
        "expected a sectionBreak block, got: {blocks:#?}"
    );
}

#[test]
fn heading_hint_promotes_paragraph_to_heading() {
    // The one Aozora construct that is *not* collapsed: a heading hint
    // changes the shape of the document, so it stays a Markdown heading.
    let blocks = ir("第一篇［＃「第一篇」は大見出し］");
    let Block::Heading {
        level, children, ..
    } = &blocks[0]
    else {
        panic!("expected heading promotion, got: {blocks:#?}");
    };
    assert_eq!(*level, 1);
    assert!(matches!(
        children.as_slice(),
        [Inline::Text { value, .. }] if value == "第一篇"
    ));
}

#[test]
fn indent_container_emits_a_matched_open_close_pair_around_its_body() {
    const SRC: &str = "前\n\n［＃ここから２字下げ］\n本文\n\n［＃ここで字下げ終わり］\n\n後";
    let blocks = ir(SRC);
    let markers = aozora_blocks(&blocks);
    let kinds: Vec<&str> = markers.iter().map(|(kind, ..)| *kind).collect();
    assert_eq!(
        kinds,
        ["containerOpen", "containerClose"],
        "one block per marker, in document order: {blocks:#?}"
    );

    let (_, open_span, open_html, _) = markers[0];
    let (_, close_span, close_html, _) = markers[1];
    assert_eq!(slice(SRC, open_span), "［＃ここから２字下げ］");
    assert_eq!(slice(SRC, close_span), "［＃ここで字下げ終わり］");
    assert!(open_html.starts_with("<div"), "open html: {open_html}");
    assert_eq!(close_html, "</div>", "close html: {close_html}");

    // The body sits between the markers as an ordinary block, not nested
    // inside them.
    let open_at = blocks
        .iter()
        .position(|b| matches!(b, Block::Aozora { kind, .. } if kind == "containerOpen"))
        .expect("open marker");
    let close_at = blocks
        .iter()
        .position(|b| matches!(b, Block::Aozora { kind, .. } if kind == "containerClose"))
        .expect("close marker");
    assert!(
        blocks[open_at + 1..close_at]
            .iter()
            .any(|b| matches!(b, Block::Paragraph { .. })),
        "the container body must sit between the markers: {blocks:#?}"
    );
}

#[test]
fn keigakomi_container_pairs_like_any_other() {
    // Keigakomi's paired syntax is `［＃罫囲み］` / `［＃罫囲み終わり］` —
    // no `ここから` prefix (that one is reserved for indent).
    let blocks = ir("前\n\n［＃罫囲み］\n本文\n\n［＃罫囲み終わり］\n\n後");
    let kinds: Vec<&str> = aozora_blocks(&blocks)
        .iter()
        .map(|(kind, ..)| *kind)
        .collect();
    assert_eq!(kinds, ["containerOpen", "containerClose"]);
}

#[test]
fn align_end_container_pairs_like_any_other() {
    let src = "前\n\n［＃ここから地から２字上げ］\n本文\n\n［＃ここで地から２字上げ終わり］\n\n後";
    let blocks = ir(src);
    let kinds: Vec<&str> = aozora_blocks(&blocks)
        .iter()
        .map(|(kind, ..)| *kind)
        .collect();
    assert_eq!(kinds, ["containerOpen", "containerClose"]);
}

#[test]
fn nested_containers_nest_by_document_order() {
    let src = "［＃ここから２字下げ］\n\n［＃罫囲み］\n中\n\n［＃罫囲み終わり］\n\n［＃ここで字下げ終わり］";
    let blocks = ir(src);
    let kinds: Vec<&str> = aozora_blocks(&blocks)
        .iter()
        .map(|(kind, ..)| *kind)
        .collect();
    assert_eq!(
        kinds,
        [
            "containerOpen",
            "containerOpen",
            "containerClose",
            "containerClose"
        ],
        "markers must come out balanced and in order: {blocks:#?}"
    );
}

#[test]
fn orphan_container_close_drops_silently() {
    // No matching open: emitting the close would leave the fragment stream
    // unbalanced, so it is dropped — same guard the HTML splicer applies.
    let blocks = ir("［＃ここで字下げ終わり］");
    assert!(
        aozora_blocks(&blocks).is_empty(),
        "orphan close should emit no block, got: {blocks:#?}"
    );
}

#[test]
fn unclosed_container_at_eof_gets_a_synthesised_close() {
    let blocks = ir("前\n\n［＃ここから２字下げ］\n本文");
    let markers = aozora_blocks(&blocks);
    let kinds: Vec<&str> = markers.iter().map(|(kind, ..)| *kind).collect();
    assert_eq!(kinds, ["containerOpen", "containerClose"]);
    let (_, span, _, source_line) = markers[1];
    assert!(
        span.is_none() && source_line.is_none(),
        "a synthesised close has no source behind it: {markers:#?}"
    );
}

#[test]
fn aozora_disabled_path_emits_no_aozora_ir_variants() {
    let opts = Options::commonmark();
    let result = render_to_ir("｜青梅《おうめ》", &opts);
    let inlines = match &result.ir.blocks[0] {
        Block::Paragraph { children, .. } => children,
        other => panic!("expected paragraph, got {other:?}"),
    };
    assert!(
        aozora_inlines(inlines).is_empty(),
        "aozora_enabled=false must skip the IR projection: {inlines:#?}"
    );
}

#[test]
fn ruby_inside_paragraph_preserves_surrounding_text() {
    let blocks = ir("前｜青梅《おうめ》後");
    let inlines = first_paragraph_inlines(&blocks);
    assert!(matches!(inlines.first(), Some(Inline::Text { value, .. }) if value == "前"));
    assert!(matches!(inlines.last(), Some(Inline::Text { value, .. }) if value == "後"));
    assert_eq!(aozora_inlines(inlines).len(), 1);
}

#[test]
fn registry_lockstep_with_multiple_inline_aozora_in_paragraph() {
    // Two ruby spans in one paragraph: each sentinel must dispatch to its
    // own entry, with its own span and its own rendered reading.
    const SRC: &str = "｜A《a》と｜B《b》の話";
    let blocks = ir(SRC);
    let rubies: Vec<_> = aozora_inlines(first_paragraph_inlines(&blocks))
        .into_iter()
        .filter(|(kind, ..)| *kind == "ruby")
        .collect();
    assert_eq!(rubies.len(), 2, "two ruby spans expected: {blocks:#?}");
    assert_eq!(slice(SRC, rubies[0].1), "｜A《a》");
    assert_eq!(slice(SRC, rubies[1].1), "｜B《b》");
    assert!(rubies[0].2.contains('a') && rubies[1].2.contains('b'));
}

#[test]
fn ruby_inside_markdown_strong_projects_under_strong() {
    let blocks = ir("**｜青梅《おうめ》**");
    let inlines = first_paragraph_inlines(&blocks);
    let Inline::Strong { children, .. } = inlines
        .iter()
        .find(|c| matches!(c, Inline::Strong { .. }))
        .expect("expected Strong wrapper")
    else {
        unreachable!()
    };
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn ruby_inside_markdown_emphasis_projects_under_emphasis() {
    let blocks = ir("*｜青梅《おうめ》*");
    let inlines = first_paragraph_inlines(&blocks);
    let Inline::Emphasis { children, .. } = inlines
        .iter()
        .find(|c| matches!(c, Inline::Emphasis { .. }))
        .expect("expected Emphasis wrapper")
    else {
        unreachable!()
    };
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn ruby_inside_markdown_link_projects_under_link() {
    let blocks = ir("[｜青梅《おうめ》](http://example.com)");
    let inlines = first_paragraph_inlines(&blocks);
    let Inline::Link { children, href, .. } = inlines
        .iter()
        .find(|c| matches!(c, Inline::Link { .. }))
        .expect("expected Link wrapper")
    else {
        unreachable!()
    };
    assert_eq!(href, "http://example.com");
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn inline_code_projects_with_literal_value() {
    let blocks = ir("see `cargo build` here");
    let inlines = first_paragraph_inlines(&blocks);
    let saw_code = inlines
        .iter()
        .any(|c| matches!(c, Inline::Code { value, .. } if value == "cargo build"));
    assert!(saw_code, "expected inline code, got: {inlines:#?}");
}

#[test]
fn ruby_inside_inline_code_projects_literal_source() {
    // A notation written inside backticks is literal markdown: the IR must
    // carry the original source in `Code.value`, not an interpreted node,
    // and must not leak the PUA sentinel.
    let blocks = ir("`｜青梅《おうめ》`");
    let inlines = first_paragraph_inlines(&blocks);
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::Code { value, .. } if value == "｜青梅《おうめ》")),
        "inline code must carry the literal Aozora source, got: {inlines:#?}"
    );
    assert!(
        aozora_inlines(inlines).is_empty(),
        "inline code must not project an interpreted notation: {inlines:#?}"
    );
}

#[test]
fn notation_in_inline_code_does_not_desync_following_ir_node() {
    // Regression: the code-span notation must consume its own registry
    // entry so the trailing ｜B《b》 renders B/b, not the code span's A/a.
    const SRC: &str = "`｜A《a》` then ｜B《b》end";
    let blocks = ir(SRC);
    let inlines = first_paragraph_inlines(&blocks);
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::Code { value, .. } if value == "｜A《a》")),
        "code span keeps its literal, got: {inlines:#?}"
    );
    let (span, html) = find_inline_kind(inlines, "ruby");
    assert_eq!(
        slice(SRC, span),
        "｜B《b》",
        "the ruby must be its OWN span"
    );
    assert!(html.contains('B') && html.contains('b'), "html: {html}");
}

#[test]
fn ruby_inside_blockquote_projects_under_blockquote() {
    let blocks = ir("> ｜青梅《おうめ》");
    let Block::Blockquote { children, .. } = &blocks[0] else {
        panic!("expected Blockquote, got: {blocks:#?}");
    };
    let Block::Paragraph { children, .. } = &children[0] else {
        panic!("expected paragraph inside blockquote, got: {children:#?}");
    };
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn ruby_inside_list_item_projects_under_list_item() {
    let blocks = ir("- ｜青梅《おうめ》");
    let Block::List { items, .. } = &blocks[0] else {
        panic!("expected List, got: {blocks:#?}");
    };
    let Block::Paragraph { children, .. } = &items[0].children[0] else {
        panic!("expected paragraph in list item");
    };
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn aozora_inline_inside_atx_h2_keeps_the_notation() {
    let blocks = ir("## ｜青梅《おうめ》");
    let Block::Heading {
        level, children, ..
    } = &blocks[0]
    else {
        panic!("expected Heading, got: {blocks:#?}");
    };
    assert_eq!(*level, 2);
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn hard_break_inside_paragraph_with_sentinel_preserves_break() {
    // `  \n` is a CommonMark hard line break.
    let blocks = ir("｜A《a》  \n｜B《b》");
    let inlines = first_paragraph_inlines(&blocks);
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::LineBreak { hard: true, .. })),
        "expected hard line break, got: {inlines:#?}"
    );
}

#[test]
fn image_inline_projects_under_aozora_enabled() {
    let blocks = ir("text ![alt](pic.png) tail");
    let inlines = first_paragraph_inlines(&blocks);
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::Image { url, .. } if url == "pic.png"))
    );
    assert!(
        inlines
            .iter()
            .any(|c| matches!(c, Inline::Text { value, .. } if value.contains("text")))
    );
}

/// Every Aozora `html` fragment anywhere in `blocks`, at any nesting depth.
fn all_fragments(blocks: &[Block], out: &mut Vec<String>) {
    fn inlines(nodes: &[Inline], out: &mut Vec<String>) {
        for node in nodes {
            match node {
                Inline::Aozora { html, .. } => out.push(html.clone()),
                Inline::Strong { children, .. }
                | Inline::Emphasis { children, .. }
                | Inline::Link { children, .. } => inlines(children, out),
                Inline::Image { alt, .. } => inlines(alt, out),
                _ => {}
            }
        }
    }
    for block in blocks {
        match block {
            Block::Aozora { html, .. } => out.push(html.clone()),
            Block::Paragraph { children, .. } | Block::Heading { children, .. } => {
                inlines(children, out);
            }
            Block::Blockquote { children, .. } => all_fragments(children, out),
            Block::List { items, .. } => {
                for item in items {
                    all_fragments(&item.children, out);
                }
            }
            _ => {}
        }
    }
}

#[test]
fn every_projected_fragment_appears_in_the_rendered_html() {
    // The IR's `html` and the document's HTML come from one renderer, so
    // each fragment must be findable verbatim in the full render. This is
    // the property that lets a consumer render straight from the IR — and
    // it only holds if the projection also reproduces the splicer's
    // *context* rules, so the cases below deliberately include the two
    // places those rules fire: a heading body (where an annotation is
    // suppressed) and a nested heading hint (where a paragraph is
    // promoted). Feeding this only paragraph-shaped input would pass
    // while the IR carried markup the document does not have.
    for src in [
        "｜青梅《おうめ》へ\n\n［＃改ページ］\n\n対象［＃「対象」に傍点］",
        "# タイトル［＃ほげ］",
        "## ｜青梅《おうめ》と対象［＃「対象」に傍点］",
        "第一篇［＃「第一篇」は大見出し］",
        "> 第一篇［＃「第一篇」は大見出し］",
        "> ［＃改ページ］",
        "- 前［＃地から１字下げ］後",
        "［＃ここから２字下げ］\n\n本文\n\n［＃ここで字下げ終わり］",
    ] {
        let document = render(src, &Options::default()).html;
        let mut fragments = Vec::new();
        all_fragments(&ir(src), &mut fragments);
        for fragment in fragments {
            assert!(
                document.contains(&fragment),
                "fragment {fragment:?} from {src:?} is missing from the rendered document:\n{document}"
            );
        }
    }
}

#[test]
fn annotation_inside_a_heading_is_suppressed_like_the_html_does() {
    // Tier C bars `aozora-md-directive` markup from a heading body, so the
    // splicer drops the notation there. The IR has to make the same call:
    // a consumer rendering the heading from the IR would otherwise put a
    // wrapper inside `<h1>` that `render` never emits.
    const SRC: &str = "# タイトル［＃ほげ］";
    let html = render(SRC, &Options::default()).html;
    assert_eq!(html.trim(), "<h1>タイトル</h1>", "html: {html}");

    let blocks = ir(SRC);
    let Block::Heading { children, .. } = &blocks[0] else {
        panic!("expected Heading, got: {blocks:#?}");
    };
    assert!(
        aozora_inlines(children).is_empty(),
        "the annotation must not reach a heading body: {children:#?}"
    );
}

#[test]
fn notation_allowed_in_a_heading_still_projects() {
    // The suppression above is annotation-shaped only: ruby, bouten and
    // friends are explicitly allowed inside a heading, and must survive.
    let blocks = ir("## ｜青梅《おうめ》");
    let Block::Heading { children, .. } = &blocks[0] else {
        panic!("expected Heading, got: {blocks:#?}");
    };
    assert_eq!(aozora_inlines(children).len(), 1, "{children:#?}");
}

#[test]
fn heading_hint_promotes_a_nested_paragraph_too() {
    // The splicer promotes any paragraph carrying a hint, wherever it
    // sits. Dispatching only at top level used to leave the IR with a
    // blockquote paragraph plus a stray `headingHint` fragment while the
    // HTML had `<blockquote><h1>…`.
    let blocks = ir("> 第一篇［＃「第一篇」は大見出し］");
    let Block::Blockquote { children, .. } = &blocks[0] else {
        panic!("expected Blockquote, got: {blocks:#?}");
    };
    let Block::Heading {
        level,
        children: heading_children,
        ..
    } = &children[0]
    else {
        panic!("expected the hint to promote inside the blockquote: {children:#?}");
    };
    assert_eq!(*level, 1);
    assert!(matches!(
        heading_children.as_slice(),
        [Inline::Text { value, .. }] if value == "第一篇"
    ));
}
/// The parser reads a document against a text it canonicalises first —
/// CRLF folded, a leading BOM dropped — and maps every range it reports
/// back to the text the caller actually holds. So a CRLF or BOM-prefixed
/// source publishes ranges just like its plain twin, and each one slices
/// the notation the author wrote.
#[test]
fn a_canonicalised_source_still_reports_ranges_into_the_callers_text() {
    for src in [
        "前\r\n\r\n｜青梅《おうめ》へ",
        "\u{feff}｜青梅《おうめ》へ",
        "前\n\n｜青梅《おうめ》へ",
    ] {
        let blocks = ir(src);
        let paragraph = blocks
            .iter()
            .find_map(|b| match b {
                Block::Paragraph { children, .. } if !aozora_inlines(children).is_empty() => {
                    Some(children.as_slice())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the ruby projects for {src:?}"));
        let (span, html) = find_inline_kind(paragraph, "ruby");
        assert_eq!(
            slice(src, span),
            "｜青梅《おうめ》",
            "the range must address the caller's own text for {src:?}"
        );
        assert!(
            html.contains("おうめ"),
            "the fragment is unaffected: {html}"
        );
    }
}

#[test]
fn render_blocks_emits_aozora_block_per_top_level_block() {
    use aozora_flavored_markdown::{RenderedBlocks, render_blocks};
    let RenderedBlocks { blocks, .. } =
        render_blocks("｜青梅《おうめ》\n\n［＃改ページ］", &Options::default());
    let saw_ruby_in_first = blocks[0].ir.iter().any(|b| {
        matches!(
            b,
            Block::Paragraph { children, .. }
                if aozora_inlines(children).iter().any(|(kind, ..)| *kind == "ruby")
        )
    });
    let saw_page_break = blocks.iter().any(|b| {
        aozora_blocks(&b.ir)
            .iter()
            .any(|(kind, ..)| *kind == "pageBreak")
    });
    assert!(saw_ruby_in_first, "expected ruby in first block");
    assert!(saw_page_break, "expected page break block");
}

#[test]
fn streaming_blocks_drain_a_container_the_source_never_closed() {
    // The per-block path has no end-of-document event of its own, so the
    // driver has to run the drain. Without it the IR emits `<div …>` and
    // never `</div>`, and a consumer stacking the fragments (obsidian's
    // chunked-cancellation path, ADR-0009) swallows the rest of the note
    // into the open container — while the block `html` of the very same
    // call is balanced.
    use aozora_flavored_markdown::{RenderedBlocks, render_blocks};

    let RenderedBlocks { blocks, .. } =
        render_blocks("［＃ここから２字下げ］\n\n本文", &Options::default());
    let kinds: Vec<&str> = blocks
        .iter()
        .flat_map(|b| aozora_blocks(&b.ir))
        .map(|(kind, ..)| kind)
        .collect();
    assert_eq!(kinds, ["containerOpen", "containerClose"], "{blocks:#?}");

    let from_ir: String = blocks
        .iter()
        .flat_map(|b| aozora_blocks(&b.ir))
        .map(|(_, _, html, _)| html.to_owned())
        .collect();
    let from_html: String = blocks.iter().map(|b| b.html.clone()).collect();
    for stream in [&from_ir, &from_html] {
        assert_eq!(
            stream.matches("<div").count(),
            stream.matches("</div>").count(),
            "both outputs of one call must balance: {stream}"
        );
    }
}

#[test]
fn streaming_container_markers_pair_across_block_boundaries() {
    // The open marker is one top-level block and the close another. The
    // builder threads its container stack across `walk_block` calls, so the
    // close still finds its open instead of being dropped as an orphan.
    use aozora_flavored_markdown::{RenderedBlocks, render_blocks};

    let src = "［＃ここから２字下げ］\n\n本文\n\n［＃ここで字下げ終わり］";
    let RenderedBlocks { blocks, .. } = render_blocks(src, &Options::default());
    let kinds: Vec<&str> = blocks
        .iter()
        .flat_map(|b| aozora_blocks(&b.ir))
        .map(|(kind, ..)| kind)
        .collect();
    assert_eq!(kinds, ["containerOpen", "containerClose"], "{blocks:#?}");
}
