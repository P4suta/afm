//! Intermediate representation produced by [`crate::render_to_ir`].
//!
//! # Examples
//!
//! ```
//! use aozora_flavored_markdown::ir::{Block, Inline};
//! use aozora_flavored_markdown::{Options, render_to_ir};
//!
//! let rendered = render_to_ir("｜青梅《おうめ》", &Options::default());
//! let ruby_rendered = rendered
//!     .ir
//!     .blocks
//!     .iter()
//!     .filter_map(|block| match block {
//!         Block::Paragraph { children, .. } => Some(children),
//!         _ => None,
//!     })
//!     .flatten()
//!     .any(|inline| {
//!         matches!(inline, Inline::Aozora { kind, html, .. }
//!             if kind == "ruby" && html.contains("おうめ"))
//!     });
//! assert!(ruby_rendered);
//! ```
//!
//! Two context rules follow the HTML splicer rather than the notation: a
//! heading hint (`［＃「X」は大見出し］`) promotes its host paragraph to
//! [`Block::Heading`] at any nesting depth, and an annotation inside a
//! heading body is dropped because the splicer drops it (Tier C).
//!
//! Both walkers read each notation's fragment off the same construct table,
//! so the IR's `html` and the document's HTML cannot drift apart on what a
//! notation renders to. Whether it renders *at all* is the other half of
//! that agreement, and is context-dependent; this walker reproduces the
//! splicer's decisions, and `tests/ir_aozora.rs` pins the result by looking
//! for every projected fragment in the rendered document.

mod types;

pub use types::{Block, Document, Inline, ListItem, Position, Range, Span, TableAlign, TableRow};

use core::mem;

use aozora::NodeKind;
use comrak::nodes::{
    AstNode, ListType, NodeHeading, NodeList, NodeValue, Sourcepos, TableAlignment,
};

use crate::constructs::{
    BlockSentinelKind, ConstructCursor, Constructs, HeadingHint, ParaScan, block_sentinel_of,
    inline_is_dropped, is_sentinel_char, paragraph_sole_block_sentinel, saturating_u32,
};

// ===================================================================
// Walker entry points
// ===================================================================

/// An empty `constructs` table degrades to markdown-only projection.
///
/// A sentinel that landed in a literal markdown context (inline code,
/// link/image destination) projects back to its original Aozora source
/// instead of leaking the PUA char and desyncing the cursor.
pub(crate) fn build_ir<'a>(root: &'a AstNode<'a>, constructs: &Constructs) -> Document {
    let mut walker = IrWalker::new(constructs.cursor(), Vec::new());
    walker.walk_root(root);
    Document {
        blocks: walker.finish(),
    }
}

/// Stateful per-block IR builder for streaming mode.
///
/// The cursor position and open-container stack live here rather than in the
/// walker, so `walk_block` calls can be issued lazily — the obsidian
/// chunked-cancellation path (ADR-0009) checkpoints between blocks — while a
/// container that opens in one block and closes in a later one still emits a
/// matched pair.
#[derive(Debug)]
pub(crate) struct StreamingIrBuilder {
    constructs: Constructs,
    consumed: usize,
    /// Closing markup for each still-open container, innermost last.
    open: Vec<String>,
}

impl StreamingIrBuilder {
    /// `source` is the text handed to the parser, so the spans this builder
    /// emits are offsets into it.
    pub(crate) fn new(source: &str) -> Self {
        Self {
            constructs: Constructs::build(source),
            consumed: 0,
            open: Vec::new(),
        }
    }

    /// The in-crate streaming driver hands this same table to the HTML
    /// splicer, so one table serves both outputs of a call.
    pub(crate) fn constructs(&self) -> &Constructs {
        &self.constructs
    }

    /// Walk a single comrak block, advancing the shared cursor.
    pub(crate) fn walk_block<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<Block> {
        let cursor = self.constructs.cursor_at(self.consumed);
        let mut walker = IrWalker::new(cursor, mem::take(&mut self.open));
        walker.walk_top(node);
        let (blocks, cursor, open) = walker.into_parts();
        self.consumed = cursor.index();
        self.open = open;
        blocks
    }

    /// End-of-document drain, matching what the HTML splicer appends in the
    /// same situation. **Call it after the last [`Self::walk_block`]** —
    /// without it the emitted fragments carry an opening `<div>` with no
    /// `</div>`, and a consumer concatenating them leaves the container
    /// swallowing everything that follows.
    #[must_use]
    pub(crate) fn finish(self) -> Vec<Block> {
        drain_open_containers(self.open)
    }
}

// ===================================================================
// Walker
// ===================================================================

/// Mirrors `crate::ast_splice`'s splicer state — same cursor, same
/// balanced-container model, same orphan-close drain — differing only in the
/// emit target (`Vec<Block>` vs. a rewritten comrak AST).
///
/// The comrak AST's lifetime is independent of `'t` (it lives in a different
/// arena) and stays elided, so a per-method `<'a>` need not shadow it.
struct IrWalker<'t> {
    cursor: ConstructCursor<'t>,
    top: Vec<Block>,
    /// Closing markup for each still-open container, innermost last.
    open: Vec<String>,
    /// Annotation-shaped notations are dropped from a heading body
    /// (Tier C), so the IR must drop them too or carry a fragment the
    /// rendered HTML does not have.
    in_heading: u32,
    depth: usize,
}

/// comrak can emit arbitrarily deep trees from a small input (nested
/// blockquotes carry no cap), and `collect_blocks` / `collect_inlines`
/// recurse over them. Without a bound a crafted input overflows the call
/// stack and aborts under `panic = "abort"` — a crash on untrusted input
/// `SECURITY.md` scopes IN. 256 is far beyond any real document (comrak caps
/// list nesting at 100); past it the IR truncates the over-deep subtree. The
/// HTML splice path is iterative and stays complete regardless.
const MAX_AST_DEPTH: usize = 256;

impl<'t> IrWalker<'t> {
    fn new(cursor: ConstructCursor<'t>, open: Vec<String>) -> Self {
        Self {
            cursor,
            top: Vec::new(),
            open,
            in_heading: 0,
            depth: 0,
        }
    }

    /// Whole-document exit: drains what the source left open.
    fn finish(mut self) -> Vec<Block> {
        let drained = drain_open_containers(mem::take(&mut self.open));
        self.top.extend(drained);
        self.top
    }

    /// Streaming exit: hands back the state [`StreamingIrBuilder`] threads
    /// into the next per-block walk.
    fn into_parts(self) -> (Vec<Block>, ConstructCursor<'t>, Vec<String>) {
        (self.top, self.cursor, self.open)
    }

    fn walk_root<'a>(&mut self, root: &'a AstNode<'a>) {
        for child in root.children() {
            self.walk_top(child);
        }
    }

    fn walk_top<'a>(&mut self, node: &'a AstNode<'a>) {
        if let Some(block) = self.walk_block(node, true) {
            self.top.push(block);
        }
    }

    /// One descent over the text descendants, returning the most specific
    /// action the lookahead supports.
    fn classify_paragraph<'a>(&self, node: &'a AstNode<'a>) -> Option<ParagraphAction> {
        if let Some(kind) = paragraph_sole_block_sentinel(node) {
            return Some(ParagraphAction::BlockSentinel(kind));
        }
        let scan = ParaScan::run(node, &self.cursor);
        if let Some(hint) = scan.first_heading_hint {
            return Some(ParagraphAction::HeadingHint {
                hint,
                sentinels_to_consume: scan.total_sentinels,
            });
        }
        None
    }

    fn dispatch_paragraph(
        &mut self,
        action: ParagraphAction,
        source_line: Option<u32>,
    ) -> Option<Block> {
        match action {
            ParagraphAction::BlockSentinel(kind) => self.handle_block_sentinel(kind, source_line),
            ParagraphAction::HeadingHint {
                hint,
                sentinels_to_consume,
            } => Some(self.handle_heading_hint(&hint, sentinels_to_consume, source_line)),
        }
    }

    fn handle_block_sentinel(
        &mut self,
        kind: BlockSentinelKind,
        source_line: Option<u32>,
    ) -> Option<Block> {
        let hit = self.cursor.next()?;
        let html = match (kind, block_sentinel_of(hit.kind)?) {
            (BlockSentinelKind::Leaf, BlockSentinelKind::Leaf) => hit.html()?,
            (BlockSentinelKind::Open, BlockSentinelKind::Open) => {
                // A marker that renders to nothing opens nothing — the
                // mirror of the splicer's `block_html`, so the two drains
                // owe the document the same number of closes.
                let (open, close) = hit.container_halves()?;
                self.open.push(close);
                open
            }
            // The close the matching open carried. An orphan close (no
            // matching open) emits nothing, in lockstep with the HTML
            // splicer's guard against unbalanced close tags.
            (BlockSentinelKind::Close, BlockSentinelKind::Close) => self.open.pop()?,
            // Table/AST drift: emit nothing.
            _ => return None,
        };
        Some(Block::Aozora {
            kind: hit.kind.as_json_tag().to_owned(),
            span: hit.span,
            html,
            source_line,
        })
    }

    fn handle_heading_hint(
        &mut self,
        hint: &HeadingHint,
        sentinels_to_consume: usize,
        source_line: Option<u32>,
    ) -> Block {
        self.cursor.advance(sentinels_to_consume);
        Block::Heading {
            level: hint.level.clamp(1, 6),
            children: vec![Inline::Text {
                value: hint.target.clone(),
                range: None,
            }],
            source_line,
            range: None,
        }
    }

    fn walk_block<'a>(&mut self, node: &'a AstNode<'a>, top_level: bool) -> Option<Block> {
        let data = node.data.borrow();
        let source_line = top_level.then(|| saturating_u32(data.sourcepos.start.line).max(1));
        let range = sourcepos_to_range(&data.sourcepos);
        match &data.value {
            NodeValue::Paragraph => {
                drop(data);
                // A paragraph the HTML splicer rewrites in place — a sole
                // block marker, or a heading hint promoting it to a heading
                // — takes the same turn here. The test runs at *every*
                // nesting level, because the splicer's does: dispatching
                // only at top level would leave the IR describing a
                // paragraph where the HTML has an `<h1>` or a `<div>`.
                if let Some(action) = self.classify_paragraph(node) {
                    return self.dispatch_paragraph(action, source_line);
                }
                Some(Block::Paragraph {
                    children: self.collect_inlines(node),
                    source_line,
                    range,
                })
            }
            NodeValue::Heading(NodeHeading { level, .. }) => {
                let level = (*level).clamp(1, 6);
                drop(data);
                self.in_heading += 1;
                let children = self.collect_inlines(node);
                self.in_heading -= 1;
                Some(Block::Heading {
                    level,
                    children,
                    source_line,
                    range,
                })
            }
            NodeValue::BlockQuote => {
                drop(data);
                Some(Block::Blockquote {
                    children: self.collect_blocks(node),
                    source_line,
                    range,
                })
            }
            NodeValue::List(NodeList {
                list_type, start, ..
            }) => {
                let ordered = matches!(list_type, ListType::Ordered);
                let start = (*start > 1).then(|| saturating_u32(*start));
                drop(data);
                Some(Block::List {
                    ordered,
                    start,
                    items: self.collect_list_items(node),
                    source_line,
                    range,
                })
            }
            NodeValue::CodeBlock(code) => {
                let lang = (!code.info.is_empty()).then(|| code.info.clone());
                let literal = code.literal.clone();
                drop(data);
                Some(Block::Code {
                    lang,
                    value: self.code_block_value(literal),
                    source_line,
                    range,
                })
            }
            NodeValue::ThematicBreak => {
                drop(data);
                Some(Block::ThematicBreak { source_line, range })
            }
            NodeValue::Table(table) => {
                let aligns: Vec<TableAlign> =
                    table.alignments.iter().copied().map(table_align).collect();
                drop(data);
                Some(self.walk_table(
                    node,
                    TableMeta {
                        align: aligns,
                        source_line,
                        range,
                    },
                ))
            }
            // List items, table rows, and table cells are handled by
            // their parents. Other unhandled block kinds (definition
            // list, footnote refs, etc.) drop from the IR — the HTML
            // still has them.
            _ => None,
        }
    }

    fn walk_table<'a>(&mut self, node: &'a AstNode<'a>, meta: TableMeta) -> Block {
        let mut rows: Vec<TableRow> = Vec::new();
        for child in node.children() {
            rows.push(self.collect_table_row(child));
        }
        let header = rows.first().cloned().unwrap_or(TableRow {
            cells: Vec::new(),
            range: None,
        });
        let body = if rows.is_empty() {
            Vec::new()
        } else {
            rows[1..].to_vec()
        };
        Block::Table {
            header,
            rows: body,
            align: meta.align,
            source_line: meta.source_line,
            range: meta.range,
        }
    }

    fn collect_blocks<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<Block> {
        // Depth-bound the block recursion (`collect_blocks` → `walk_block`
        // → `collect_blocks` for nested blockquotes / list items). Past
        // the bound the over-deep subtree is dropped from the IR rather
        // than overflowing the stack; see [`MAX_AST_DEPTH`].
        if self.depth >= MAX_AST_DEPTH {
            return Vec::new();
        }
        self.depth += 1;
        let mut out = Vec::new();
        for child in node.children() {
            if let Some(block) = self.walk_block(child, false) {
                out.push(block);
            }
        }
        self.depth -= 1;
        out
    }

    fn collect_list_items<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<ListItem> {
        let mut out = Vec::new();
        for child in node.children() {
            let data = child.data.borrow();
            let is_item = matches!(data.value, NodeValue::Item(_));
            let range = sourcepos_to_range(&data.sourcepos);
            drop(data);
            if !is_item {
                continue;
            }
            out.push(ListItem {
                children: self.collect_blocks(child),
                range,
            });
        }
        out
    }

    fn collect_table_row<'a>(&mut self, row: &'a AstNode<'a>) -> TableRow {
        let data = row.data.borrow();
        let range = sourcepos_to_range(&data.sourcepos);
        drop(data);
        let mut cells = Vec::new();
        for cell in row.children() {
            cells.push(self.collect_inlines(cell));
        }
        TableRow { cells, range }
    }

    fn collect_inlines<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<Inline> {
        // Depth-bound the inline recursion (`collect_inlines` →
        // `emit_inline` → `collect_inlines` for nested emphasis / links /
        // images). Past the bound the over-deep inline subtree is dropped
        // rather than overflowing the stack; see [`MAX_AST_DEPTH`].
        if self.depth >= MAX_AST_DEPTH {
            return Vec::new();
        }
        self.depth += 1;
        let mut out = Vec::new();
        for child in node.children() {
            self.emit_inline(child, &mut out);
        }
        self.depth -= 1;
        out
    }

    /// Only an *indented* block reaches this — a fenced one is masked
    /// before the lexer runs (ADR-0010).
    fn code_block_value(&mut self, literal: String) -> String {
        if literal.chars().any(is_sentinel_char) {
            return self.rewrite_literal_context(&literal);
        }
        literal
    }

    fn emit_inline<'a>(&mut self, node: &'a AstNode<'a>, out: &mut Vec<Inline>) {
        let data = node.data.borrow();
        let range = sourcepos_to_range(&data.sourcepos);
        match &data.value {
            NodeValue::Text(s) => {
                let s = s.clone();
                drop(data);
                self.project_text_with_sentinels(&s, range, out);
            }
            NodeValue::Code(c) => {
                let literal = c.literal.clone();
                drop(data);
                // Inline code is literal markdown: a notation written
                // inside backticks projects as its original source, not an
                // interpreted node, and must consume its registry entry so
                // later sentinels stay in lockstep.
                let value = self.rewrite_literal_context(&literal);
                out.push(Inline::Code { value, range });
            }
            NodeValue::Strong => {
                drop(data);
                out.push(Inline::Strong {
                    children: self.collect_inlines(node),
                    range,
                });
            }
            NodeValue::Emph => {
                drop(data);
                out.push(Inline::Emphasis {
                    children: self.collect_inlines(node),
                    range,
                });
            }
            NodeValue::Link(link) => {
                let url = link.url.clone();
                let title = link.title.clone();
                drop(data);
                // Children (link text) first, then url/title — source order,
                // so cursor consumption stays in lockstep.
                let children = self.collect_inlines(node);
                let href = self.rewrite_literal_context(&url);
                let title = self.rewrite_literal_context(&title);
                out.push(Inline::Link {
                    href,
                    title: (!title.is_empty()).then_some(title),
                    children,
                    range,
                });
            }
            NodeValue::Image(image) => {
                let url = image.url.clone();
                let title = image.title.clone();
                drop(data);
                let alt = self.collect_inlines(node);
                let url = self.rewrite_literal_context(&url);
                let title = self.rewrite_literal_context(&title);
                out.push(Inline::Image {
                    url,
                    title: (!title.is_empty()).then_some(title),
                    alt,
                    range,
                });
            }
            NodeValue::SoftBreak => {
                drop(data);
                out.push(Inline::LineBreak { hard: false, range });
            }
            NodeValue::LineBreak => {
                drop(data);
                out.push(Inline::LineBreak { hard: true, range });
            }
            // Footnote refs, raw HTML, etc. drop quietly.
            _ => {}
        }
    }

    /// Literal markdown contexts (inline code, link / image URLs), where a
    /// notation must surface as its source text rather than an interpreted
    /// IR node. Consumes one table entry per sentinel so later entries stay
    /// in lockstep with the splice. Mirrors
    /// `crate::ast_splice::AstSplicer::rewrite_literal_context`.
    fn rewrite_literal_context(&mut self, s: &str) -> String {
        if !s.chars().any(is_sentinel_char) {
            return s.to_owned();
        }
        let mut out = String::with_capacity(s.len());
        for ch in s.chars() {
            if is_sentinel_char(ch) {
                if let Some(literal) = self.cursor.next_literal() {
                    out.push_str(literal);
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn project_text_with_sentinels(
        &mut self,
        text: &str,
        range: Option<Range>,
        out: &mut Vec<Inline>,
    ) {
        // Fast path: no sentinels in this text run.
        if !text.chars().any(is_sentinel_char) {
            if !text.is_empty() {
                out.push(Inline::Text {
                    value: text.to_owned(),
                    range,
                });
            }
            return;
        }
        let mut cursor = 0;
        for (idx, ch) in text.char_indices() {
            if !is_sentinel_char(ch) {
                continue;
            }
            let head = &text[cursor..idx];
            if !head.is_empty() {
                out.push(Inline::Text {
                    value: head.to_owned(),
                    range,
                });
            }
            cursor = idx + ch.len_utf8();
            let Some(hit) = self.cursor.next() else {
                continue;
            };
            // Block sentinels surviving into an inline context (e.g.
            // raw text inside a fenced code block) drop silently —
            // matches `crate::ast_splice::split_text_node`.
            if block_sentinel_of(hit.kind).is_some() {
                continue;
            }
            // …as does a notation the splicer drops rather than renders,
            // for the reasons `inline_is_dropped` gives. The table entry is
            // already consumed, so both streams stay in step.
            if inline_is_dropped(hit.kind, self.in_heading > 0) {
                continue;
            }
            let Some(html) = hit.html() else {
                continue;
            };
            out.push(Inline::Aozora {
                kind: hit.kind.as_json_tag().to_owned(),
                span: hit.span,
                html,
            });
        }
        let tail = &text[cursor..];
        if !tail.is_empty() {
            out.push(Inline::Text {
                value: tail.to_owned(),
                range,
            });
        }
    }
}

/// Shared by both drains so the whole-document and per-block paths cannot
/// describe the same situation differently.
fn drain_open_containers(open: Vec<String>) -> Vec<Block> {
    // Innermost first, so the tags nest the way the source opened them.
    open.into_iter()
        .rev()
        .map(|html| Block::Aozora {
            kind: NodeKind::ContainerClose.as_json_tag().to_owned(),
            // Synthesised, so there is no source text behind it.
            span: None,
            html,
            source_line: None,
        })
        .collect()
}

fn table_align(a: TableAlignment) -> TableAlign {
    match a {
        TableAlignment::Left => TableAlign::Left,
        TableAlignment::Center => TableAlign::Center,
        TableAlignment::Right => TableAlign::Right,
        TableAlignment::None => TableAlign::Default,
    }
}

fn sourcepos_to_range(s: &Sourcepos) -> Option<Range> {
    // comrak source positions are 1-based line / column. Map the
    // pair through `Position` directly — no pseudo-byte arithmetic.
    let start = Position::new(saturating_u32(s.start.line), saturating_u32(s.start.column));
    let end = Position::new(saturating_u32(s.end.line), saturating_u32(s.end.column));
    // `Position` derives `Ord` lexicographically (line first, then
    // column), so the comparison works for malformed inputs where
    // `end` precedes `start`.
    (end >= start).then_some(Range::new(start, end))
}

struct TableMeta {
    align: Vec<TableAlign>,
    source_line: Option<u32>,
    range: Option<Range>,
}

#[derive(Debug, Clone)]
enum ParagraphAction {
    BlockSentinel(BlockSentinelKind),
    HeadingHint {
        hint: HeadingHint,
        sentinels_to_consume: usize,
    },
}

#[cfg(test)]
mod tests {
    //! The notation-tag map is exercised against the real lexer rather than
    //! synthesised nodes: only the lexer can say which construct a given
    //! piece of source resolves to.

    use core::ops::Range as ByteRange;

    use super::*;
    use comrak::nodes::LineColumn;

    /// Every notation tag a snippet produces, in source order, tagged the
    /// way the IR would tag it.
    fn kinds_of(src: &str) -> Vec<&'static str> {
        let document = aozora::parse(src.to_owned()).expect("the fixtures are small");
        document
            .snapshot()
            .nodes()
            .iter()
            .map(|node| node.kind().as_json_tag())
            .collect()
    }

    #[test]
    fn the_tag_names_the_inline_notation_the_author_wrote() {
        for (src, expected) in [
            ("｜青梅《おうめ》", "ruby"),
            ("対象［＃「対象」に傍点］", "bouten"),
            ("20［＃「20」は縦中横］", "combineUpright"),
            ("※［＃二の字点、1-2-22］", "gaiji"),
            ("［＃ほげふが］", "directive"),
            ("天［＃レ］地", "kaeriten"),
            ("第一篇［＃「第一篇」は大見出し］", "headingHint"),
        ] {
            assert!(
                kinds_of(src).contains(&expected),
                "{src:?} should tag a {expected}, got {:?}",
                kinds_of(src)
            );
        }
    }

    #[test]
    fn the_tag_names_the_block_notation_the_author_wrote() {
        for (src, expected) in [
            ("［＃改ページ］", "pageBreak"),
            ("［＃改丁］", "sectionBreak"),
            ("［＃挿絵（fig1.png）入る］", "illustration"),
            ("［＃地付き］", "alignEnd"),
            ("［＃ここから２字下げ］", "containerOpen"),
            (
                "［＃ここから２字下げ］\n本文\n\n［＃ここで字下げ終わり］",
                "containerClose",
            ),
        ] {
            assert!(
                kinds_of(src).contains(&expected),
                "{src:?} should tag a {expected}, got {:?}",
                kinds_of(src)
            );
        }
    }

    #[test]
    fn sourcepos_to_range_returns_some_for_well_ordered_positions() {
        let pos = Sourcepos {
            start: LineColumn { line: 1, column: 1 },
            end: LineColumn { line: 1, column: 5 },
        };
        let range = sourcepos_to_range(&pos).expect("forward range");
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.column, 1);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.column, 5);
        assert!(range.start <= range.end);
    }

    #[test]
    fn sourcepos_to_range_returns_none_for_inverted_positions() {
        // Constructed (impossible) inverted sourcepos: start later
        // than end. The helper guards against negative ranges by
        // returning `None`, which keeps the IR robust under malformed
        // upstream output.
        let pos = Sourcepos {
            start: LineColumn { line: 5, column: 5 },
            end: LineColumn { line: 1, column: 1 },
        };
        assert!(sourcepos_to_range(&pos).is_none());
    }

    #[test]
    fn sourcepos_to_range_preserves_multiline_extent() {
        // Comrak emits ranges that span multiple source lines for
        // block constructs (a fenced code block, a multi-line list
        // item, …). The IR must preserve the line / column pair so
        // editor surfaces can map back to the right slice without
        // doing pseudo-byte arithmetic.
        let pos = Sourcepos {
            start: LineColumn { line: 3, column: 1 },
            end: LineColumn {
                line: 7,
                column: 12,
            },
        };
        let range = sourcepos_to_range(&pos).expect("forward range");
        assert_eq!(range.start.line, 3);
        assert_eq!(range.end.line, 7);
        assert_eq!(range.end.column, 12);
    }

    #[test]
    fn the_streaming_builder_threads_its_cursor_across_blocks() {
        // Two top-level blocks, each with its own inline sentinel: the cursor
        // has to thread, or the second block resolves against the first
        // block's entry. Reached from inside the crate since the builder is
        // `pub(crate)` — the public per-block path is `render_blocks_to_ir`.
        let src = "｜A《a》\n\n｜B《b》";
        let mut builder = StreamingIrBuilder::new(src);
        let arena = comrak::Arena::new();
        let comrak = comrak::Options::default();
        let root = comrak::parse_document(&arena, builder.constructs().text(), &comrak);
        let mut children = root.children();
        let first = builder.walk_block(children.next().expect("first block"));
        let second = builder.walk_block(children.next().expect("second block"));

        for (blocks, expected) in [(&first, "｜A《a》"), (&second, "｜B《b》")] {
            let [Block::Paragraph { children, .. }] = blocks.as_slice() else {
                panic!("expected a single paragraph, got {blocks:#?}");
            };
            let span = children
                .iter()
                .find_map(|inline| match inline {
                    Inline::Aozora { kind, span, .. } if kind == "ruby" => *span,
                    _ => None,
                })
                .expect("a ruby inline carrying its source span");
            assert_eq!(&src[ByteRange::from(span)], expected);
        }
    }

    #[test]
    fn table_align_maps_every_alignment() {
        assert!(matches!(
            table_align(TableAlignment::Left),
            TableAlign::Left
        ));
        assert!(matches!(
            table_align(TableAlignment::Center),
            TableAlign::Center
        ));
        assert!(matches!(
            table_align(TableAlignment::Right),
            TableAlign::Right
        ));
        assert!(matches!(
            table_align(TableAlignment::None),
            TableAlign::Default
        ));
    }
}
