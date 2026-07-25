//! Intermediate representation produced by [`crate::render_to_ir`].
//!
//! # Examples
//!
//! ```
//! use aozora_flavored_markdown::ir::{IrBlock, IrInline};
//! use aozora_flavored_markdown::{Options, render_to_ir};
//!
//! let rendered = render_to_ir("｜青梅《おうめ》", &Options::default());
//! let ruby_rendered = rendered
//!     .ir
//!     .blocks
//!     .iter()
//!     .filter_map(|block| match block {
//!         IrBlock::Paragraph { children, .. } => Some(children),
//!         _ => None,
//!     })
//!     .flatten()
//!     .any(|inline| {
//!         matches!(inline, IrInline::Aozora { kind, html, .. }
//!             if kind == "ruby" && html.contains("おうめ"))
//!     });
//! assert!(ruby_rendered);
//! ```
//!
//! # Coverage
//!
//! - **Markdown side**: paragraphs, headings, lists, blockquotes,
//!   fenced code, tables, thematic breaks, images. Inline runs
//!   preserve `Strong`, `Emphasis`, `Link`, `Image`, `Code`,
//!   `LineBreak`, and verbatim `Text`.
//! - **Aozora side**: every notation, as [`IrInline::Aozora`] or
//!   [`IrBlock::Aozora`] carrying its tag, source span, and HTML fragment.
//!   Two context rules follow the HTML splicer rather than the notation:
//!   a heading hint (`［＃「X」は大見出し］`) promotes its host paragraph
//!   to [`IrBlock::Heading`] — at any nesting depth, so a hint inside a
//!   blockquote promotes there too — and an annotation inside a heading
//!   body is dropped, because the splicer drops it (Tier C).
//!
//! # Module map
//!
//! - `types` — public IR enum/struct definitions (`IrDocument`,
//!   `IrBlock`, `IrInline`, `Range`, ...).
//! - This file (`mod.rs`) — the stateful walker (`IrWalker`,
//!   `StreamingIrBuilder`) plus the single-descent `ParaScan` dispatch,
//!   the notation-tag mapping, and the public entry points (`build_ir`,
//!   `StreamingIrBuilder::walk_block`).
//!
//! # Architecture
//!
//! The walker is built from two small primitives:
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
//!
//! Both walkers render each notation through the same
//! `crate::ast_splice::render_aozora_html`, so the IR's `html` and the
//! document's HTML cannot drift apart on what a notation *renders to*.
//! Whether a notation renders at all is the other half of that agreement,
//! and it is context-dependent (heading-hint promotion, Tier-C suppression
//! inside headings); this walker reproduces those decisions, and
//! `tests/ir_aozora.rs` pins the result by looking for every projected
//! fragment in the rendered document.

mod types;

pub use types::{
    IrBlock, IrDocument, IrInline, IrListItem, IrTableAlign, IrTableRow, Position, Range, Span,
};

use core::mem;

use aozora::pipeline::BorrowedLexOutput;
use aozora::syntax::borrowed::{AozoraNode, HeadingHint, NodeRef};
use aozora::syntax::{Container, ContainerKind};
use comrak::nodes::{
    AstNode, ListType, NodeHeading, NodeList, NodeValue, Sourcepos, TableAlignment,
};

use crate::ast_splice::render_aozora_html;
use crate::sentinel_stream::{
    BlockSentinelKind, NormalizedSource, ParaScan, SentinelCursor, is_sentinel_char,
    paragraph_sole_block_sentinel, saturating_u32,
};

/// Tag carried by the block that opens a paired container.
const CONTAINER_OPEN: &str = "containerOpen";
/// Tag carried by the block that closes a paired container.
const CONTAINER_CLOSE: &str = "containerClose";

// ===================================================================
// Walker entry points
// ===================================================================

/// Walk a comrak AST root and project it to [`IrDocument`].
///
/// `lex_out` carries the resolved-construct registry. When `Some`, every
/// PUA sentinel in the comrak text is projected to an [`IrBlock::Aozora`] /
/// [`IrInline::Aozora`]; when `None`, the walker degrades to markdown-only
/// behaviour (used by `Options::aozora_enabled = false`).
/// `src` is the lexer's input-normalisation output, threaded so a
/// sentinel that landed in a literal markdown context (inline code,
/// link/image destination) projects back to its original Aozora source
/// instead of leaking the PUA char and desyncing the cursor — and so the
/// projection knows whether the lexer's offsets still address the caller's
/// own text (see `NormalizedSource`).
pub(crate) fn build_ir<'a>(
    root: &'a AstNode<'a>,
    lex_out: Option<&BorrowedLexOutput<'a>>,
    src: NormalizedSource<'_>,
) -> IrDocument {
    let mut walker = IrWalker::new(
        SentinelCursor::from_lex_out_with_source(lex_out, src),
        Vec::new(),
    );
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
/// The open-container stack threads across calls too, so a container that
/// opens in one top-level block and closes in a later one still emits a
/// matched open/close pair. A container the source never closes is drained
/// by [`StreamingIrBuilder::finish`], which the caller invokes when it runs
/// out of blocks — the streaming analogue of the whole-document walker's
/// end-of-document pass. Skipping it leaves the emitted `html` fragments
/// with an unmatched opening tag.
#[derive(Debug)]
pub struct StreamingIrBuilder<'src> {
    cursor: SentinelCursor<'src>,
    open: Vec<ContainerKind>,
}

impl<'src> StreamingIrBuilder<'src> {
    /// Materialise the registry once. `None` produces an empty
    /// builder that degrades to markdown-only projection. `normalized` is
    /// the lexer's input-normalisation output — the same text the sentinels
    /// were embedded in — used to project literal-context sentinels
    /// (inline code, link/image URLs) back to their original Aozora source.
    ///
    /// Because the caller supplies that text, the spans this builder emits
    /// are offsets into it — the caller's own coordinates. (The
    /// whole-document [`crate::render_to_ir`] entry point, whose caller
    /// only ever sees the raw source, withholds a span that normalisation
    /// moved out from under it.)
    #[must_use]
    pub fn new(lex_out: Option<&BorrowedLexOutput<'src>>, normalized: &str) -> Self {
        Self::with_source(lex_out, NormalizedSource::verbatim(normalized))
    }

    /// [`Self::new`] for the in-crate pipeline, which knows whether the
    /// lexer's offsets survived normalisation.
    pub(crate) fn with_source(
        lex_out: Option<&BorrowedLexOutput<'src>>,
        src: NormalizedSource<'_>,
    ) -> Self {
        Self {
            cursor: SentinelCursor::from_lex_out_with_source(lex_out, src),
            open: Vec::new(),
        }
    }

    /// Walk a single comrak block, advancing the shared cursor.
    pub fn walk_block<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<IrBlock> {
        // Move the cursor and container stack into a freshly-constructed
        // walker for the duration of this call, then take them back. The
        // walker's `top` buffer is scoped per-call; the cursor and the
        // stack are the state that threads across calls.
        let cursor = mem::replace(&mut self.cursor, SentinelCursor::from_nodes(Vec::new()));
        let open = mem::take(&mut self.open);
        let mut walker = IrWalker::new(cursor, open);
        walker.walk_top(node);
        let (blocks, cursor, open) = walker.into_parts();
        self.cursor = cursor;
        self.open = open;
        blocks
    }

    /// End-of-document drain: one synthesised close block per container the
    /// source left open, outermost last (LIFO), matching what the HTML
    /// splicer appends to the document in the same situation.
    ///
    /// Call it after the last [`Self::walk_block`]. Without it the emitted
    /// fragments carry an opening `<div>` with no `</div>`, so a consumer
    /// concatenating them — aozora-flavored-markdown-obsidian's
    /// chunked-cancellation path (ADR-0009) inserts them one by one — would
    /// leave the container swallowing everything that follows.
    #[must_use]
    pub fn finish(mut self) -> Vec<IrBlock> {
        let mut out = Vec::with_capacity(self.open.len());
        while let Some(kind) = self.open.pop() {
            out.push(synthesised_close(kind));
        }
        out
    }
}

// ===================================================================
// Walker
// ===================================================================

/// Tree builder that consumes comrak nodes plus a sentinel cursor and
/// emits `IrBlock`s.
///
/// The state mirrors `crate::ast_splice`'s splicer for the HTML
/// side: same cursor, same balanced-container model, same
/// orphan-close drain at end-of-document. They differ only in the
/// emit target (rewritten comrak AST vs. `Vec<IrBlock>`).
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
    /// Blocks gathered so far, in document order.
    top: Vec<IrBlock>,
    /// Kinds of the paired containers currently open, in LIFO order.
    /// Tracking the kind (not just a depth) lets an unclosed container
    /// synthesise its own matching close at end-of-document.
    open: Vec<ContainerKind>,
    /// Number of `Heading` ancestors the walker is currently inside —
    /// the mirror of the splicer's `in_heading_depth`, and used for the
    /// same reason: annotation-shaped notations are dropped from a
    /// heading body (Tier C), so the IR must drop them too or it would
    /// carry a fragment the rendered HTML does not have.
    in_heading: u32,
    /// Current block/inline nesting depth, bounded by [`MAX_AST_DEPTH`]
    /// so pathologically deep input cannot overflow the recursive
    /// `collect_blocks` / `collect_inlines` descent.
    depth: usize,
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
    fn new(cursor: SentinelCursor<'src>, open: Vec<ContainerKind>) -> Self {
        Self {
            cursor,
            top: Vec::new(),
            open,
            in_heading: 0,
            depth: 0,
        }
    }

    /// Close any container the source left open (mirror of the HTML
    /// splicer's end-of-document orphan-close pass) and return the
    /// document blocks. Used by `build_ir`.
    fn finish(mut self) -> Vec<IrBlock> {
        while let Some(kind) = self.open.pop() {
            self.top.push(synthesised_close(kind));
        }
        self.top
    }

    /// Return the blocks plus the cursor and container stack, so a
    /// streaming caller can thread them into the next per-block walk.
    /// Used by [`StreamingIrBuilder`].
    fn into_parts(self) -> (Vec<IrBlock>, SentinelCursor<'src>, Vec<ContainerKind>) {
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

    fn dispatch_paragraph(
        &mut self,
        action: ParagraphAction<'src>,
        source_line: Option<u32>,
    ) -> Option<IrBlock> {
        match action {
            ParagraphAction::BlockSentinel(kind) => self.handle_block_sentinel(kind, source_line),
            ParagraphAction::HeadingHint {
                hint,
                sentinels_to_consume,
            } => Some(self.handle_heading_hint(hint, sentinels_to_consume, source_line)),
        }
    }

    fn handle_block_sentinel(
        &mut self,
        kind: BlockSentinelKind,
        source_line: Option<u32>,
    ) -> Option<IrBlock> {
        let hit = self.cursor.next()?;
        let (tag, html) = match (kind, hit.node) {
            (BlockSentinelKind::Leaf, NodeRef::BlockLeaf(node)) => (
                notation_kind(node).to_owned(),
                render_aozora_html(node, true),
            ),
            (BlockSentinelKind::Open, NodeRef::BlockOpen(ck)) => {
                self.open.push(ck);
                (CONTAINER_OPEN.to_owned(), container_html(ck, true))
            }
            // An orphan close (no matching open) emits nothing, in lockstep
            // with the HTML splicer's guard against unbalanced close tags.
            (BlockSentinelKind::Close, NodeRef::BlockClose(ck)) if self.open.pop().is_some() => {
                (CONTAINER_CLOSE.to_owned(), container_html(ck, false))
            }
            // Registry/AST drift or an orphan close: emit nothing.
            _ => return None,
        };
        Some(IrBlock::Aozora {
            kind: tag,
            span: hit.span,
            html,
            source_line,
        })
    }

    fn handle_heading_hint(
        &mut self,
        hint: &'src HeadingHint<'src>,
        sentinels_to_consume: usize,
        source_line: Option<u32>,
    ) -> IrBlock {
        self.cursor.advance(sentinels_to_consume);
        IrBlock::Heading {
            level: hint.level.clamp(1, 6),
            children: vec![IrInline::Text {
                value: hint.target.as_str().to_owned(),
                range: None,
            }],
            source_line,
            range: None,
        }
    }

    fn walk_block<'a>(&mut self, node: &'a AstNode<'a>, top_level: bool) -> Option<IrBlock> {
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
                Some(IrBlock::Paragraph {
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
                Some(IrBlock::Heading {
                    level,
                    children,
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
            let Some(hit) = self.cursor.next() else {
                continue;
            };
            // Block sentinels surviving into an inline context (e.g.
            // raw text inside a fenced code block) drop silently —
            // matches `crate::ast_splice::split_text_node`.
            if let NodeRef::Inline(aozora) = hit.node {
                // …as does an annotation inside a heading: its fragment is
                // an `aozora-md-annotation` wrapper, which Tier C bars from
                // a heading body, so the splicer drops it. The registry
                // entry is already consumed, so both streams stay in step.
                if self.in_heading > 0 && matches!(aozora, AozoraNode::Annotation(_)) {
                    continue;
                }
                out.push(IrInline::Aozora {
                    kind: notation_kind(aozora).to_owned(),
                    span: hit.span,
                    html: render_aozora_html(aozora, true),
                });
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

/// Opening or closing HTML for one paired-container marker.
fn container_html(kind: ContainerKind, entering: bool) -> String {
    render_aozora_html(AozoraNode::Container(Container { kind }), entering)
}

/// The close block for a container the source never closed. Shared by both
/// drains ([`IrWalker::finish`] for the whole document,
/// [`StreamingIrBuilder::finish`] for the per-block path) so the two cannot
/// describe the same situation differently.
fn synthesised_close(kind: ContainerKind) -> IrBlock {
    IrBlock::Aozora {
        kind: CONTAINER_CLOSE.to_owned(),
        // Synthesised, so there is no source text behind it.
        span: None,
        html: container_html(kind, false),
        source_line: None,
    }
}

/// Opaque tag for a resolved construct, as carried by
/// [`IrInline::Aozora`] / [`IrBlock::Aozora`].
///
/// This is a naming map, not a projection: nothing about the notation's
/// payload is re-modelled here, so a notation the sibling parser grows
/// later needs no change on this side to *render* — it only shows up as
/// `"unknown"` until the tag is named. That is the trailing arm's job, and
/// it is reachable today: the source enum is `#[non_exhaustive]`.
fn notation_kind(node: AozoraNode<'_>) -> &'static str {
    match node {
        AozoraNode::Ruby(_) => "ruby",
        AozoraNode::DoubleRuby(_) => "doubleRuby",
        AozoraNode::Bouten(_) => "bouten",
        AozoraNode::TateChuYoko(_) => "tateChuYoko",
        AozoraNode::Gaiji(_) => "gaiji",
        AozoraNode::Annotation(_) => "annotation",
        AozoraNode::Kaeriten(_) => "kaeriten",
        AozoraNode::Indent(_) => "indent",
        AozoraNode::AlignEnd(_) => "alignEnd",
        AozoraNode::Warichu(_) => "warichu",
        AozoraNode::Keigakomi(_) => "keigakomi",
        AozoraNode::Sashie(_) => "sashie",
        AozoraNode::PageBreak => "pageBreak",
        AozoraNode::SectionBreak(_) => "sectionBreak",
        AozoraNode::AozoraHeading(_) => "aozoraHeading",
        AozoraNode::HeadingHint(_) => "headingHint",
        AozoraNode::Container(_) => "container",
        _ => "unknown",
    }
}

fn table_align(a: TableAlignment) -> IrTableAlign {
    match a {
        TableAlignment::Left => IrTableAlign::Left,
        TableAlignment::Center => IrTableAlign::Center,
        TableAlignment::Right => IrTableAlign::Right,
        TableAlignment::None => IrTableAlign::Default,
    }
}

fn sourcepos_to_range(s: &Sourcepos) -> Option<Range> {
    // comrak source positions are 1-based line / column. Map the
    // pair through `Position` directly — no pseudo-byte arithmetic.
    let start = Position {
        line: saturating_u32(s.start.line),
        column: saturating_u32(s.start.column),
    };
    let end = Position {
        line: saturating_u32(s.end.line),
        column: saturating_u32(s.end.column),
    };
    // `Position` derives `Ord` lexicographically (line first, then
    // column), so the comparison works for malformed inputs where
    // `end` precedes `start`.
    (end >= start).then_some(Range { start, end })
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

#[cfg(test)]
mod tests {
    //! Unit tests for the pure helpers the walker composes.
    //!
    //! The notation-tag map is exercised against the real lexer rather than
    //! synthesised nodes: what matters is that the tag a reader sees in the
    //! IR matches the notation they typed, and only the lexer can say which
    //! construct a given piece of source resolves to.

    use super::*;
    use aozora::pipeline::lex_into_arena;
    use aozora::syntax::borrowed::{Arena, Content};
    use comrak::nodes::LineColumn;

    /// Every notation tag the lexer can produce from a snippet, in source
    /// order, tagged the way the IR would tag it.
    fn kinds_of(src: &str) -> Vec<&'static str> {
        let arena = Arena::new();
        let lex_out = lex_into_arena(src, &arena);
        lex_out
            .registry
            .iter_sorted()
            .map(|(_pos, node)| match node {
                NodeRef::Inline(n) | NodeRef::BlockLeaf(n) => notation_kind(n),
                NodeRef::BlockOpen(_) => CONTAINER_OPEN,
                NodeRef::BlockClose(_) => CONTAINER_CLOSE,
                _ => "unknown",
            })
            .collect()
    }

    #[test]
    fn notation_kind_names_the_inline_notations() {
        for (src, expected) in [
            ("｜青梅《おうめ》", "ruby"),
            ("《《強調》》", "doubleRuby"),
            ("対象［＃「対象」に傍点］", "bouten"),
            ("20［＃「20」は縦中横］", "tateChuYoko"),
            ("※［＃二の字点、1-2-22］", "gaiji"),
            ("［＃ほげふが］", "annotation"),
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
    fn notation_kind_names_the_block_notations() {
        for (src, expected) in [
            ("［＃改ページ］", "pageBreak"),
            ("［＃改丁］", "sectionBreak"),
            ("［＃挿絵（fig1.png）入る］", "sashie"),
            ("［＃地付き］", "alignEnd"),
            ("［＃ここから２字下げ］", CONTAINER_OPEN),
            (
                "［＃ここから２字下げ］\n本文\n\n［＃ここで字下げ終わり］",
                CONTAINER_CLOSE,
            ),
        ] {
            assert!(
                kinds_of(src).contains(&expected),
                "{src:?} should tag a {expected}, got {:?}",
                kinds_of(src)
            );
        }
    }

    /// The constructs above are the ones a document can produce today.
    /// The rest of the map is reached by building the value directly:
    /// these constructs exist in the notation but only ever arrive as a
    /// container payload, so an input-driven test cannot name them — and a
    /// silently wrong tag is exactly what this map must not have.
    #[test]
    fn notation_kind_names_the_container_payload_constructs() {
        use aozora::syntax::borrowed::Warichu;
        use aozora::syntax::{AlignEnd, Indent, Keigakomi};

        let warichu = Warichu {
            upper: Content::EMPTY,
            lower: Content::EMPTY,
        };
        let cases = [
            (AozoraNode::Indent(Indent { amount: 2 }), "indent"),
            (AozoraNode::AlignEnd(AlignEnd { offset: 1 }), "alignEnd"),
            (AozoraNode::Keigakomi(Keigakomi), "keigakomi"),
            (AozoraNode::Warichu(&warichu), "warichu"),
            (
                AozoraNode::Container(Container {
                    kind: ContainerKind::Keigakomi,
                }),
                "container",
            ),
        ];
        for (node, expected) in cases {
            assert_eq!(notation_kind(node), expected);
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
    fn table_align_maps_every_alignment() {
        assert!(matches!(
            table_align(TableAlignment::Left),
            IrTableAlign::Left
        ));
        assert!(matches!(
            table_align(TableAlignment::Center),
            IrTableAlign::Center
        ));
        assert!(matches!(
            table_align(TableAlignment::Right),
            IrTableAlign::Right
        ));
        assert!(matches!(
            table_align(TableAlignment::None),
            IrTableAlign::Default
        ));
    }
}
