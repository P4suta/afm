//! Intermediate representation produced by [`crate::render_to_ir`].
//!
//! The shape mirrors the TypeScript `IRDocument` consumed by
//! `aozora-flavored-markdown-obsidian/src/ir/types.ts` (and validated in
//! `aozora-flavored-markdown-obsidian/src/ir/from-wasm.ts`). Keeping the names and field
//! ordering aligned across the FFI boundary makes the
//! `serde-wasm-bindgen` round-trip a pass-through, no shape adapters
//! needed.
//!
//! # Examples
//!
//! ```
//! use aozora_flavored_markdown::ir::{IrBlock, IrInline};
//! use aozora_flavored_markdown::{Options, render_to_ir};
//!
//! let rendered = render_to_ir("｜青梅《おうめ》", &Options::default());
//! let ruby_spans = rendered
//!     .ir
//!     .blocks
//!     .iter()
//!     .filter_map(|block| match block {
//!         IrBlock::Paragraph { children, .. } => Some(children),
//!         _ => None,
//!     })
//!     .flatten()
//!     .filter(|inline| matches!(inline, IrInline::Ruby { .. }))
//!     .count();
//! assert_eq!(ruby_spans, 1);
//! ```
//!
//! # Coverage
//!
//! - **Markdown side**: paragraphs, headings, lists, blockquotes,
//!   fenced code, tables, thematic breaks, images. Inline runs
//!   preserve `Strong`, `Emphasis`, `Link`, `Image`, `Code`,
//!   `LineBreak`, and verbatim `Text`.
//! - **Aozora side**: `Ruby` / `DoubleRuby` / `Bouten` / `Tcy` /
//!   `Gaiji` / `Annotation` (inline) and `Container` / `PageBreak` /
//!   `SectionBreak` (block). Heading hints
//!   (`［＃「X」は大見出し］`) promote their host paragraph to
//!   `IrBlock::Heading` directly, mirroring `crate::ast_splice`.
//!
//! # Module map
//!
//! - `types` — public IR enum/struct definitions (`IrDocument`,
//!   `IrBlock`, `IrInline`, `Range`, ...).
//! - `projection` — pure helpers that convert upstream construct
//!   variants into IR values plus the enum→string mappers and the
//!   sourcepos→range bridge. No walker state.
//! - This file (`mod.rs`) — the stateful walker (`IrWalker`,
//!   `OpenContainer`, `StreamingIrBuilder`) plus the single-descent
//!   `ParaScan` and the public entry points (`build_ir`,
//!   `StreamingIrBuilder::walk_block`).
//!
//! # Architecture
//!
//! The walker is built from three small primitives:
//!
//! 1. `crate::sentinel_stream::SentinelCursor` — the shared construct-stream
//!    cursor. The HTML splicer (`crate::ast_splice`) and this
//!    builder both consume the same source-order sequence of
//!    entries; the cursor abstraction keeps them in lockstep.
//! 2. `ParaScan` — single-descent paragraph profile. One walk per
//!    paragraph computes both the sole-block-sentinel test and the
//!    heading-hint lookahead at once, eliminating the two-scan
//!    redundancy that a naive translation of the HTML splicer would
//!    have.
//! 3. `OpenContainer` — the per-walker container stack. Where the
//!    HTML splicer can stream open/close tags into a string buffer,
//!    the IR demands a tree, so each open container collects
//!    `IrBlock`s into its own `Vec` until the matching close arrives.
//!    Move semantics (no `clone`) carry the children into the closed
//!    `IrBlock::Container`.

mod projection;
mod types;

pub use types::{
    AnnotationKind, BoutenPosition, BoutenStyle, ContainerSubtype, IrBlock, IrDocument, IrInline,
    IrListItem, IrTableAlign, IrTableRow, Position, Range, SectionSubtype,
};

use core::mem;

use aozora::pipeline::BorrowedLexOutput;
use aozora::syntax::ContainerKind;
use aozora::syntax::borrowed::{HeadingHint, NodeRef};
use comrak::nodes::{AstNode, ListType, NodeHeading, NodeList, NodeValue};

use crate::sentinel_stream::{
    BlockSentinelKind, ParaScan, SentinelCursor, is_sentinel_char, paragraph_sole_block_sentinel,
    saturating_u32,
};

use projection::{
    container_indent_level, container_subtype, project_block_leaf, project_inline,
    sourcepos_to_range, table_align,
};

// ===================================================================
// Walker entry points
// ===================================================================

/// Walk a comrak AST root and project it to [`IrDocument`].
///
/// `lex_out` carries the borrowed-AST registry. When `Some`, every
/// PUA sentinel in the comrak text is projected to its matching
/// [`IrBlock`] / [`IrInline`] variant; when `None`, the walker
/// degrades to markdown-only behaviour (used by
/// `Options::aozora_enabled = false`).
/// `sanitized` is the lexer's Phase-0 sanitized source, threaded so a
/// sentinel that landed in a literal markdown context (inline code,
/// link/image destination) projects back to its original Aozora source
/// instead of leaking the PUA char and desyncing the cursor. Must be the
/// sanitized source (not the raw input) — `source_span` coordinates are
/// in sanitized-source bytes.
pub(crate) fn build_ir<'a>(
    root: &'a AstNode<'a>,
    lex_out: Option<&BorrowedLexOutput<'a>>,
    sanitized: &str,
) -> IrDocument {
    let mut walker = IrWalker::new(SentinelCursor::from_lex_out_with_source(lex_out, sanitized));
    walker.walk_root(root);
    IrDocument {
        blocks: walker.finish(),
    }
}

/// Stateful per-block IR builder for streaming mode.
///
/// Materialises the registry once at construction time and threads a
/// shared cursor across successive `walk_block` calls so multi-block
/// inputs preserve the registry's source order. The cursor lives in
/// this struct (not in the walker) so individual `walk_block` calls
/// can be issued lazily — aozora-flavored-markdown-obsidian's chunked-cancellation path
/// (ADR-0009) uses this to checkpoint between blocks.
///
/// Container open/close paragraphs that span multiple top-level
/// blocks emit fragmented `IrBlock::Container` blocks: the open
/// pushes onto a stack that drains at the next `walk_block` boundary
/// (so each block is internally consistent in nesting). Whole-doc
/// `build_ir` remains the canonical path for cross-block nesting.
#[derive(Debug)]
pub struct StreamingIrBuilder<'src> {
    cursor: SentinelCursor<'src>,
}

impl<'src> StreamingIrBuilder<'src> {
    /// Materialise the registry once. `None` produces an empty
    /// builder that degrades to markdown-only projection. `sanitized` is
    /// the lexer's Phase-0 sanitized source, used to project literal-context
    /// sentinels (inline code, link/image URLs) back to their original
    /// Aozora source.
    #[must_use]
    pub fn new(lex_out: Option<&BorrowedLexOutput<'src>>, sanitized: &str) -> Self {
        Self {
            cursor: SentinelCursor::from_lex_out_with_source(lex_out, sanitized),
        }
    }

    /// Walk a single comrak block, advancing the shared cursor.
    /// Streaming-mode containers fragment per-block; for whole-doc
    /// nesting use `build_ir`.
    pub fn walk_block<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<IrBlock> {
        // Move the cursor into a freshly-constructed walker for the
        // duration of this call, then take it back. The walker's
        // `top` / `open` stacks are scoped per-call (streaming-mode
        // containers fragment per-block); the cursor is the only
        // state that threads across calls.
        let cursor = mem::replace(&mut self.cursor, SentinelCursor::from_nodes(Vec::new()));
        let mut walker = IrWalker::new(cursor);
        walker.walk_top(node);
        let (blocks, cursor) = walker.finish_keeping_cursor();
        self.cursor = cursor;
        blocks
    }
}

// ===================================================================
// Walker
// ===================================================================

/// Tree builder that consumes comrak nodes plus a sentinel cursor and
/// emits `IrBlock`s into a stack-balanced container hierarchy.
///
/// The state mirrors `crate::ast_splice`'s splicer for the HTML
/// side: same cursor, same balanced-container model, same
/// orphan-close drain at end-of-document. They differ only in the
/// emit target (rewritten comrak AST vs. tree of `Vec<IrBlock>`).
///
/// Lifetime: `'src` is the arena/source lifetime every borrowed upstream
/// payload references — shared with the owned cursor's payloads and the
/// heading-hint borrows in [`ParagraphAction::HeadingHint`].
///
/// The comrak AST's own lifetime is **independent** (it lives in a
/// different `comrak::Arena`) and elided through `&AstNode<'_>` in
/// every method signature, so a per-method `<'a>` does not have to
/// shadow the struct's `'src`.
struct IrWalker<'src> {
    cursor: SentinelCursor<'src>,
    /// Document-level blocks gathered so far. When a container is
    /// open, new blocks go onto its top-of-stack `children` instead.
    top: Vec<IrBlock>,
    /// Stack of currently-open paired containers. Each frame owns the
    /// blocks gathered between its open and (eventual) close marker.
    open: Vec<OpenContainer>,
    /// Current block/inline nesting depth, bounded by [`MAX_AST_DEPTH`]
    /// so pathologically deep input cannot overflow the recursive
    /// `collect_blocks` / `collect_inlines` descent.
    depth: usize,
}

struct OpenContainer {
    kind: ContainerKind,
    source_line: Option<u32>,
    children: Vec<IrBlock>,
}

/// Maximum IR block/inline nesting depth.
///
/// comrak can emit arbitrarily deep trees from a small input (nested
/// blockquotes — `handle_blockquote` carries no cap — nested list items,
/// nested inline emphasis), and the IR builder's `collect_blocks` /
/// `collect_inlines` recurse over them. Without a bound a crafted input
/// would overflow the call stack and abort the process under the release
/// profile's `panic = "abort"` — a crash on untrusted input that
/// `SECURITY.md` scopes IN as a vulnerability. 256 is far beyond any real
/// document (comrak itself caps list nesting at 100) while leaving the OS
/// stack comfortable; beyond it the IR truncates the over-deep subtree.
/// The HTML splice path is iterative ([`crate::ast_splice`]) and stays
/// complete regardless.
const MAX_AST_DEPTH: usize = 256;

impl<'src> IrWalker<'src> {
    fn new(cursor: SentinelCursor<'src>) -> Self {
        Self {
            cursor,
            top: Vec::new(),
            open: Vec::new(),
            depth: 0,
        }
    }

    /// Drain any unclosed containers (mirror of the HTML splicer's
    /// end-of-document orphan-close pass) and return the document
    /// blocks. Used by `build_ir`.
    fn finish(self) -> Vec<IrBlock> {
        self.finish_keeping_cursor().0
    }

    /// Same as [`Self::finish`] but also returns the consumed cursor
    /// so a streaming caller can thread it into the next per-block
    /// walk. Used by [`StreamingIrBuilder`].
    fn finish_keeping_cursor(mut self) -> (Vec<IrBlock>, SentinelCursor<'src>) {
        while let Some(open) = self.open.pop() {
            let block = open.into_block();
            place_in(&mut self.open, &mut self.top, block);
        }
        (self.top, self.cursor)
    }

    fn walk_root<'a>(&mut self, root: &'a AstNode<'a>) {
        for child in root.children() {
            self.walk_top(child);
        }
    }

    fn walk_top<'a>(&mut self, node: &'a AstNode<'a>) {
        let (source_line, is_paragraph) = top_metadata(node);
        if is_paragraph && let Some(action) = self.classify_paragraph(node) {
            self.dispatch_paragraph(action, source_line);
            return;
        }
        if let Some(block) = self.walk_block(node, true) {
            place_in(&mut self.open, &mut self.top, block);
        }
    }

    /// Run a single descent over `node`'s text descendants, returning
    /// the most specific paragraph action (sole block sentinel or
    /// heading hint promotion) supported by the registry lookahead.
    fn classify_paragraph<'a>(&self, node: &'a AstNode<'a>) -> Option<ParagraphAction<'src>> {
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

    fn dispatch_paragraph(&mut self, action: ParagraphAction<'src>, source_line: u32) {
        match action {
            ParagraphAction::BlockSentinel(kind) => self.handle_block_sentinel(kind, source_line),
            ParagraphAction::HeadingHint {
                hint,
                sentinels_to_consume,
            } => self.handle_heading_hint(hint, sentinels_to_consume, source_line),
        }
    }

    fn handle_block_sentinel(&mut self, kind: BlockSentinelKind, source_line: u32) {
        let Some(node_ref) = self.cursor.next() else {
            return;
        };
        match (kind, node_ref) {
            (BlockSentinelKind::Leaf, NodeRef::BlockLeaf(leaf)) => {
                if let Some(block) = project_block_leaf(leaf, source_line) {
                    place_in(&mut self.open, &mut self.top, block);
                }
            }
            (BlockSentinelKind::Open, NodeRef::BlockOpen(ck)) => {
                self.open.push(OpenContainer {
                    kind: ck,
                    source_line: Some(source_line),
                    children: Vec::new(),
                });
            }
            (BlockSentinelKind::Close, NodeRef::BlockClose(_)) => {
                if let Some(open) = self.open.pop() {
                    let block = open.into_block();
                    place_in(&mut self.open, &mut self.top, block);
                }
                // Orphan close: silently dropped, in lockstep with
                // `splice_aozora_html`'s defensive guard.
            }
            _ => {}
        }
    }

    fn handle_heading_hint(
        &mut self,
        hint: &'src HeadingHint<'src>,
        sentinels_to_consume: usize,
        source_line: u32,
    ) {
        self.cursor.advance(sentinels_to_consume);
        let block = IrBlock::Heading {
            level: hint.level.clamp(1, 6),
            children: vec![IrInline::Text {
                value: hint.target.as_str().to_owned(),
                range: None,
            }],
            source_line: Some(source_line),
            range: None,
        };
        place_in(&mut self.open, &mut self.top, block);
    }

    fn walk_block<'a>(&mut self, node: &'a AstNode<'a>, top_level: bool) -> Option<IrBlock> {
        let data = node.data.borrow();
        let source_line = top_level.then(|| saturating_u32(data.sourcepos.start.line).max(1));
        let range = sourcepos_to_range(&data.sourcepos);
        match &data.value {
            NodeValue::Paragraph => {
                drop(data);
                Some(IrBlock::Paragraph {
                    children: self.collect_inlines(node),
                    source_line,
                    range,
                })
            }
            NodeValue::Heading(NodeHeading { level, .. }) => {
                let level = (*level).clamp(1, 6);
                drop(data);
                Some(IrBlock::Heading {
                    level,
                    children: self.collect_inlines(node),
                    source_line,
                    range,
                })
            }
            NodeValue::BlockQuote => {
                drop(data);
                Some(IrBlock::Blockquote {
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
                Some(IrBlock::List {
                    ordered,
                    start,
                    items: self.collect_list_items(node),
                    source_line,
                    range,
                })
            }
            NodeValue::CodeBlock(code) => {
                let lang = (!code.info.is_empty()).then(|| code.info.clone());
                let value = code.literal.clone();
                drop(data);
                Some(IrBlock::CodeBlock {
                    lang,
                    value,
                    source_line,
                    range,
                })
            }
            NodeValue::ThematicBreak => {
                drop(data);
                Some(IrBlock::ThematicBreak { source_line, range })
            }
            NodeValue::Table(table) => {
                let aligns: Vec<IrTableAlign> =
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

    fn walk_table<'a>(&mut self, node: &'a AstNode<'a>, meta: TableMeta) -> IrBlock {
        let mut rows: Vec<IrTableRow> = Vec::new();
        for child in node.children() {
            rows.push(self.collect_table_row(child));
        }
        let header = rows.first().cloned().unwrap_or(IrTableRow {
            cells: Vec::new(),
            range: None,
        });
        let body = if rows.is_empty() {
            Vec::new()
        } else {
            rows[1..].to_vec()
        };
        IrBlock::Table {
            header,
            rows: body,
            align: meta.align,
            source_line: meta.source_line,
            range: meta.range,
        }
    }

    fn collect_blocks<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<IrBlock> {
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

    fn collect_list_items<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<IrListItem> {
        let mut out = Vec::new();
        for child in node.children() {
            let data = child.data.borrow();
            let is_item = matches!(data.value, NodeValue::Item(_));
            let range = sourcepos_to_range(&data.sourcepos);
            drop(data);
            if !is_item {
                continue;
            }
            out.push(IrListItem {
                children: self.collect_blocks(child),
                range,
            });
        }
        out
    }

    fn collect_table_row<'a>(&mut self, row: &'a AstNode<'a>) -> IrTableRow {
        let data = row.data.borrow();
        let range = sourcepos_to_range(&data.sourcepos);
        drop(data);
        let mut cells = Vec::new();
        for cell in row.children() {
            cells.push(self.collect_inlines(cell));
        }
        IrTableRow { cells, range }
    }

    fn collect_inlines<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<IrInline> {
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

    fn emit_inline<'a>(&mut self, node: &'a AstNode<'a>, out: &mut Vec<IrInline>) {
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
                out.push(IrInline::Code { value, range });
            }
            NodeValue::Strong => {
                drop(data);
                out.push(IrInline::Strong {
                    children: self.collect_inlines(node),
                    range,
                });
            }
            NodeValue::Emph => {
                drop(data);
                out.push(IrInline::Emphasis {
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
                out.push(IrInline::Link {
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
                out.push(IrInline::Image {
                    url,
                    title: (!title.is_empty()).then_some(title),
                    alt,
                    range,
                });
            }
            NodeValue::SoftBreak => {
                drop(data);
                out.push(IrInline::LineBreak { hard: false, range });
            }
            NodeValue::LineBreak => {
                drop(data);
                out.push(IrInline::LineBreak { hard: true, range });
            }
            // Footnote refs, raw HTML, etc. drop quietly.
            _ => {}
        }
    }

    /// Rewrite each sentinel in `s` to the original Aozora source the
    /// lexer collapsed into it, leaving non-sentinel chars untouched, and
    /// advancing the cursor once per sentinel so later entries stay in
    /// lockstep. Used for literal markdown contexts (inline code, link /
    /// image URLs) where a notation must surface as its source text rather
    /// than an interpreted IR node. Mirrors
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
        out: &mut Vec<IrInline>,
    ) {
        // Fast path: no sentinels in this text run.
        if !text.chars().any(is_sentinel_char) {
            if !text.is_empty() {
                out.push(IrInline::Text {
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
                out.push(IrInline::Text {
                    value: head.to_owned(),
                    range,
                });
            }
            cursor = idx + ch.len_utf8();
            let Some(node_ref) = self.cursor.next() else {
                continue;
            };
            // Block sentinels surviving into an inline context (e.g.
            // raw text inside a fenced code block) drop silently —
            // matches `crate::ast_splice::split_text_node`.
            if let NodeRef::Inline(aozora) = node_ref
                && let Some(inline) = project_inline(aozora)
            {
                out.push(inline);
            }
        }
        let tail = &text[cursor..];
        if !tail.is_empty() {
            out.push(IrInline::Text {
                value: tail.to_owned(),
                range,
            });
        }
    }
}

/// Push `block` onto the top-of-stack open container's children, or
/// onto the document's top-level blocks if no container is open.
fn place_in(open: &mut [OpenContainer], top: &mut Vec<IrBlock>, block: IrBlock) {
    if let Some(frame) = open.last_mut() {
        frame.children.push(block);
    } else {
        top.push(block);
    }
}

impl OpenContainer {
    fn into_block(self) -> IrBlock {
        IrBlock::Container {
            subtype: container_subtype(self.kind),
            children: self.children,
            indent_level: container_indent_level(self.kind),
            source_line: self.source_line,
            range: None,
        }
    }
}

struct TableMeta {
    align: Vec<IrTableAlign>,
    source_line: Option<u32>,
    range: Option<Range>,
}

#[derive(Debug, Clone, Copy)]
enum ParagraphAction<'src> {
    BlockSentinel(BlockSentinelKind),
    HeadingHint {
        hint: &'src HeadingHint<'src>,
        sentinels_to_consume: usize,
    },
}

fn top_metadata(node: &AstNode<'_>) -> (u32, bool) {
    let data = node.data.borrow();
    let line = saturating_u32(data.sourcepos.start.line).max(1);
    let is_para = matches!(data.value, NodeValue::Paragraph);
    (line, is_para)
}
