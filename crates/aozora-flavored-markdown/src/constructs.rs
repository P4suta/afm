//! The replacement table this crate splices into the comrak AST, plus the
//! primitives both consumers ([`crate::ast_splice`] and [`crate::ir`]) use
//! to sequence it.
//!
//! # One coordinate space
//!
//! A 青空文庫 construct is addressed by a byte range into the source this
//! crate handed the parser — the masked source, which is char-for-char the
//! caller's own input (the code-block mask swaps 3-byte triggers for a
//! 3-byte stand-in). [`Constructs::build`] tiles that source: the bytes
//! between constructs are copied verbatim, and each construct's range
//! collapses to one of this crate's four PUA sentinels. comrak parses the
//! result; the sentinels survive it untouched, being outside CommonMark's
//! escape set.
//!
//! Both walkers consume the table in document order and never look a
//! construct up by position, so the table is `O(n)` to build and `O(1)` per
//! step to walk.
//!
//! # Trusting the tiling
//!
//! The parser measures its ranges against a text it derives from ours
//! before lexing: it drops a leading BOM, folds `\r` to `\n`, combines
//! accent digraphs inside `〔…〕`, isolates decorative rules with a blank
//! line, and neutralises a source-supplied PUA character. On such an input
//! its ranges address *that* text, so slicing ours with them would return a
//! shifted run — silently wrong text, or a panic on a mid-codepoint index.
//!
//! Two of those rewrites are text hygiene rather than notation, and
//! [`text_hygiene`] reproduces them here so a document that only needs them
//! — 青空文庫 source is historically Shift_JIS + CRLF — still tiles. The
//! other three stay the parser's.
//!
//! [`Constructs::build`] settles the question with one exact test: the
//! tiling it produces must equal, byte for byte, the sentinel-bearing text
//! the parser produced from the same input. Equality proves every range
//! addresses the text we tiled, because every byte outside a construct
//! matched and every construct landed at the offset the parser put its
//! sentinel. Those ranges are published only when that text is the caller's
//! own — when no hygiene was needed — since a range into a copy is a range
//! no consumer holds.
//!
//! When the tiling does not match either way, the parser's own text drives
//! comrak — the rendering is identical — and no construct carries a range.
//! What is still needed there is the *source text* of a construct that
//! landed in a literal markdown context (an inline code span, a link
//! destination), and that is recovered by [`SourceIndex`]: every window of
//! the source a sub-parse can trust, lexed once, keyed by what it holds.
//! The index is built on the first literal a document reads and shared by
//! every read after it, so recovery costs one pass over the source however
//! many literals are asked for — never one pass *per* literal.

use core::mem;
use core::ops::ControlFlow;
use std::borrow::Cow;
use std::cell::OnceCell;

use aozora::{AozoraNode, Arena, BorrowedLexOutput, HeadingHint, NodeRef, SourceNode};
use comrak::nodes::{AstNode, NodeValue};

use crate::diagnostics::Span;

/// Inline construct (ruby / bouten / annotation / gaiji / TCY / kaeriten).
pub(crate) const INLINE_SENTINEL: char = '\u{E001}';
/// Block-leaf construct (page break, section break, leaf indent, sashie).
pub(crate) const BLOCK_LEAF_SENTINEL: char = '\u{E002}';
/// Paired-container open marker (e.g. `［＃ここから字下げ］`).
pub(crate) const BLOCK_OPEN_SENTINEL: char = '\u{E003}';
/// Paired-container close marker (e.g. `［＃ここで字下げ終わり］`).
pub(crate) const BLOCK_CLOSE_SENTINEL: char = '\u{E004}';

/// A blank line. It separates one top-level block from the next — which is
/// how the recovery index cuts the source into windows — and this crate
/// wraps one around a block sentinel so comrak sees it as a paragraph of
/// its own rather than as inline content of its neighbour.
const BLANK_LINE: &str = "\n\n";

/// Which paired sentinel a block-sentinel paragraph carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockSentinelKind {
    Leaf,
    Open,
    Close,
}

impl BlockSentinelKind {
    /// Map a char codepoint back to its sentinel kind. `None` for
    /// inline sentinel and non-sentinel chars.
    #[inline]
    pub(crate) const fn from_char(ch: char) -> Option<Self> {
        match ch {
            BLOCK_LEAF_SENTINEL => Some(Self::Leaf),
            BLOCK_OPEN_SENTINEL => Some(Self::Open),
            BLOCK_CLOSE_SENTINEL => Some(Self::Close),
            _ => None,
        }
    }
}

/// Saturating `usize → u32`. Source line / column / byte offsets
/// past `u32::MAX` only happen for files larger than `~4G`, which
/// the rest of the pipeline already declines to handle, so a
/// saturating clamp is the right answer when we have to fit a
/// `usize` into the IR / sourcepos surface.
#[inline]
#[must_use]
pub(crate) fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// True iff `ch` is one of the four PUA sentinel codepoints
/// `U+E001..=U+E004`.
///
/// Implemented as a single subtract-and-compare. The optimiser would
/// likely fold the equivalent `matches!` chain into the same code,
/// but writing it once explicitly keeps the hot path obvious to
/// readers and lets us const-eval it where needed.
#[inline]
pub(crate) const fn is_sentinel_char(ch: char) -> bool {
    (ch as u32).wrapping_sub(INLINE_SENTINEL as u32) < 4
}

// ===================================================================
// The table
// ===================================================================

/// One construct: what the parser resolved, where its notation sits, and
/// the text the author wrote for it.
#[derive(Debug)]
struct Construct<'src> {
    node: NodeRef<'src>,
    /// The byte range the parser reported for this notation. It addresses
    /// the caller's source exactly when the table is tiled (see
    /// [`ranges_address`]), which is when it gets published.
    span: Span,
    /// The source run the sentinel stands for, sliced when the table was
    /// tiled. `None` on the fallback path, where finding it costs a lookup
    /// in the recovery index — see [`Constructs::literal_of`], which only
    /// the literal contexts pay for.
    literal: Option<String>,
}

/// Source-ordered construct table plus the text comrak parses.
#[derive(Debug)]
pub(crate) struct Constructs<'src> {
    /// The text comrak parses: the source with every construct replaced by
    /// one sentinel. Always the parser's own copy — on the tiled path
    /// because ours was just proven byte-equal to it, on the fallback path
    /// because ours could not be proven at all.
    text: &'src str,
    entries: Vec<Construct<'src>>,
    /// Whether the tiled text was the caller's own source rather than a
    /// hygiene copy of it — which is what makes a construct's range one the
    /// caller can slice, and therefore publishable.
    ranges_address_source: bool,
    /// Our copy of the source, kept only when the tiling could not be
    /// trusted: there a literal has to be recovered rather than sliced.
    untiled_source: Option<String>,
    /// Where every construct sits in `untiled_source`, built on the first
    /// literal read and shared by every read after it.
    index: OnceCell<SourceIndex>,
}

impl<'src> Constructs<'src> {
    /// Empty table for the markdown-only path (`Options::aozora_enabled =
    /// false`), where no notation is recognised and the caller's own text
    /// goes straight to comrak.
    pub(crate) fn none() -> Self {
        Self {
            text: "",
            entries: Vec::new(),
            ranges_address_source: true,
            untiled_source: None,
            index: OnceCell::new(),
        }
    }

    /// Tile `source` into sentinel-bearing text plus the construct table.
    ///
    /// `source` MUST be the text handed to the parser that produced
    /// `lex_out`; the ranges are measured against it.
    pub(crate) fn build(source: &str, lex_out: Option<&BorrowedLexOutput<'src>>) -> Self {
        let Some(lex_out) = lex_out else {
            return Self::none();
        };
        // The tiling is trusted only when it reproduces the parser's own
        // sentinel text exactly — see the module docs.
        let hygienic = text_hygiene(source);
        if ranges_address(&hygienic, lex_out) {
            return Self {
                // Byte-equal to the tiling just proven, so the parser's
                // copy serves and ours is dropped rather than kept
                // alongside it for the whole render.
                text: lex_out.normalized,
                entries: tiled(&hygienic, lex_out.source_nodes),
                ranges_address_source: matches!(hygienic, Cow::Borrowed(_)),
                untiled_source: None,
                index: OnceCell::new(),
            };
        }
        Self {
            text: lex_out.normalized,
            entries: untiled(lex_out.source_nodes),
            ranges_address_source: false,
            untiled_source: Some(source.to_owned()),
            index: OnceCell::new(),
        }
    }

    /// The source text a construct's sentinel stands for.
    ///
    /// Free on the tiled path, where the run was sliced when the table was
    /// built. On the fallback path it consults the recovery index — which
    /// is why it is asked for lazily: only a notation that landed in a
    /// literal markdown context (an inline code span, a link destination)
    /// needs its source text, and building the index at all is work a
    /// document without one should not pay for. Empty when the index cannot
    /// place the construct.
    fn literal_of(&self, idx: usize) -> Cow<'_, str> {
        let Some(entry) = self.entries.get(idx) else {
            return Cow::Borrowed("");
        };
        if let Some(literal) = &entry.literal {
            return Cow::Borrowed(literal);
        }
        let Some(source) = &self.untiled_source else {
            return Cow::Borrowed("");
        };
        let index = self.index.get_or_init(|| SourceIndex::build(source));
        resolve_in_source(source, index, entry.span, entry.node)
            .and_then(|span| slice(source, span))
            .map_or(Cow::Borrowed(""), Cow::Borrowed)
    }

    /// The text comrak parses.
    pub(crate) fn text(&self) -> &'src str {
        self.text
    }

    /// Cursor positioned before the first construct.
    pub(crate) fn cursor(&self) -> ConstructCursor<'_, 'src> {
        self.cursor_at(0)
    }

    /// Cursor positioned before construct `idx`. The streaming builder
    /// resumes here between blocks.
    pub(crate) fn cursor_at(&self, idx: usize) -> ConstructCursor<'_, 'src> {
        ConstructCursor { table: self, idx }
    }
}

/// A copy of `source` carrying the two rewrites this crate makes on the
/// parser's behalf: every leading BOM dropped, and every `\r` — plus the
/// `\n` that may follow it — folded to a single `\n`.
///
/// These are text hygiene, not notation. Every Markdown renderer folds line
/// endings, comrak sees the folded text either way, and reproducing them
/// here is what lets a CRLF or BOM-prefixed document be tiled at all, since
/// the parser measures its ranges against a text it folded the same way.
/// The rewrites that *are* notation stay the parser's, and a document
/// carrying one simply fails the tiling test.
///
/// Borrowed — no allocation, no copy — when there is nothing to do, which is
/// the common case.
fn text_hygiene(source: &str) -> Cow<'_, str> {
    let without_bom = source.trim_start_matches('\u{FEFF}');
    if without_bom.len() == source.len() && !source.contains('\r') {
        return Cow::Borrowed(source);
    }
    let mut out = String::with_capacity(without_bom.len());
    let mut chars = without_bom.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\r' {
            out.push(ch);
            continue;
        }
        out.push('\n');
        if chars.peek() == Some(&'\n') {
            chars.next();
        }
    }
    Cow::Owned(out)
}

/// Tile `source` with sentinels, or `None` when a range does not address it
/// (out of bounds, mid-codepoint, out of order).
///
/// Every range boundary is checked by the slice that consumes it: a
/// construct's start by the gap before it, its end by the gap after it (or
/// by the tail), so a range landing mid-codepoint declines rather than
/// panicking.
fn tile(source: &str, nodes: &[SourceNode<'_>]) -> Option<String> {
    let mut text = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for entry in nodes {
        let start = entry.source_span.start as usize;
        let end = entry.source_span.end as usize;
        if start < cursor || end < start {
            return None;
        }
        text.push_str(source.get(cursor..start)?);
        push_sentinel(&mut text, entry.node);
        cursor = end;
    }
    text.push_str(source.get(cursor..)?);
    Some(text)
}

/// Whether the ranges in `lexed` address `text` itself: substituting each
/// one for its sentinel has to reproduce, byte for byte, the sentinel text
/// the parser produced from the same input.
///
/// This is the whole design's one proof — see the module docs. It costs a
/// tiling and a comparison, and it needs no list of the rewrites the parser
/// makes: any of them shows up as a difference.
fn ranges_address(text: &str, lexed: &BorrowedLexOutput<'_>) -> bool {
    tile(text, lexed.source_nodes).is_some_and(|tiled| tiled == lexed.normalized)
}

/// The table for a source the ranges were just proven to address: every
/// construct with the range the parser reported and the run it slices.
fn tiled<'src>(source: &str, nodes: &[SourceNode<'src>]) -> Vec<Construct<'src>> {
    nodes
        .iter()
        .map(|entry| {
            let span = Span {
                start: entry.source_span.start,
                end: entry.source_span.end,
            };
            Construct {
                node: entry.node,
                span,
                literal: slice(source, span).map(str::to_owned),
            }
        })
        .collect()
}

/// Build the table when the tiling could not be trusted: keep every
/// construct — dropping one would desync both walkers — with the range the
/// parser reported, and no range of our own. Nothing is recovered here; the
/// fallback path pays for the index only where a literal is read
/// ([`Constructs::literal_of`]).
fn untiled<'src>(nodes: &[SourceNode<'src>]) -> Vec<Construct<'src>> {
    nodes
        .iter()
        .map(|entry| Construct {
            node: entry.node,
            span: Span {
                start: entry.source_span.start,
                end: entry.source_span.end,
            },
            literal: None,
        })
        .collect()
}

/// Append the sentinel that stands for `node`, padded into a paragraph of
/// its own for the block kinds — a block marker is a line, not a run of
/// inline text, and comrak has to see it that way.
fn push_sentinel(text: &mut String, node: NodeRef<'_>) {
    let block = match node {
        NodeRef::BlockLeaf(_) => Some(BLOCK_LEAF_SENTINEL),
        NodeRef::BlockOpen(_) => Some(BLOCK_OPEN_SENTINEL),
        NodeRef::BlockClose(_) => Some(BLOCK_CLOSE_SENTINEL),
        // `NodeRef::Inline`, and — the upstream enum being
        // `#[non_exhaustive]` — any kind we don't know yet, which is
        // inline-shaped by default so the stream stays in step.
        _ => None,
    };
    let Some(sentinel) = block else {
        text.push(INLINE_SENTINEL);
        return;
    };
    text.push_str(BLANK_LINE);
    text.push(sentinel);
    text.push_str(BLANK_LINE);
}

// ===================================================================
// Recovering a construct's source text
// ===================================================================

/// How many candidates a lookup may try before giving up.
///
/// The two coordinate spaces differ only by what the parser's own pre-lex
/// rewrites inserted, so the answer is a candidate close to the offset it
/// reported. Walking further than a handful would be guessing, and it is
/// also what bounds a lookup's cost on a window packed with same-length
/// notation.
const MAX_PROBES: usize = 16;

/// Where every construct sits in a source whose tiling could not be
/// trusted.
///
/// Built in one pass: the source is cut into the windows a sub-parse can
/// trust and each is lexed once, so the whole index costs about what a
/// single sub-parse of the document costs. Every literal the document reads
/// then answers from it — which is what keeps a document with thousands of
/// notations in literal contexts linear rather than quadratic.
#[derive(Debug)]
struct SourceIndex {
    /// Non-overlapping, in source order, so a lookup binary-searches for
    /// the window holding the offset it was given.
    windows: Vec<IndexedWindow>,
}

/// One window of the source and the constructs lexing it yields.
#[derive(Debug)]
struct IndexedWindow {
    span: Span,
    /// `(byte length, start offset in the source)` per construct, sorted:
    /// a lookup knows the length it wants and roughly the offset, so it
    /// binary-searches for both.
    candidates: Vec<(u32, u32)>,
}

impl SourceIndex {
    /// Lex every window of `source` a sub-parse can trust.
    ///
    /// The windows are the blank-line-delimited blocks, since a notation
    /// resolves against the block it lives in. A block the parser would
    /// itself rewrite is replaced by the lines inside it that it would not
    /// — 青空文庫 source is historically CRLF, and one `\r` should not cost
    /// the whole block.
    fn build(source: &str) -> Self {
        let whole = Span {
            start: 0,
            end: saturating_u32(source.len()),
        };
        let mut windows = Vec::new();
        for block in split_spans(source, whole, BLANK_LINE) {
            match index_window(source, block) {
                Some(window) => windows.push(window),
                None => windows.extend(
                    split_spans(source, block, "\n")
                        .map(|line| trim_carriage_return(source, line))
                        .filter_map(|line| index_window(source, line)),
                ),
            }
        }
        windows.retain(|window| !window.candidates.is_empty());
        Self { windows }
    }

    /// The range in `source` of the construct `span` names.
    ///
    /// Among the candidates in the window holding `span.start` that are
    /// `span`'s byte length, the nearest one that parses on its own to
    /// `node`'s shape wins. `None` when no candidate qualifies — an honest
    /// "unknown" beats a plausible-looking wrong answer.
    fn resolve(&self, source: &str, span: Span, node: NodeRef<'_>) -> Option<Span> {
        let want = span.end.checked_sub(span.start)?;
        let run = self.window_holding(span.start)?.run_of_length(want);
        let pivot = run.partition_point(|&(_, start)| start < span.start);
        let (mut left, mut right) = (pivot, pivot);
        for _ in 0..MAX_PROBES {
            let before = left
                .checked_sub(1)
                .and_then(|i| run.get(i))
                .map(|&(_, start)| start);
            let after = run.get(right).map(|&(_, start)| start);
            // Walk outward from where the reported offset would sit,
            // nearest first.
            let nearer_before = match (before, after) {
                (Some(b), Some(a)) => span.start.abs_diff(b) <= span.start.abs_diff(a),
                (Some(_), None) => true,
                (None, _) => false,
            };
            let stepped = if nearer_before {
                left -= 1;
                before
            } else {
                right += 1;
                after
            };
            let Some(start) = stepped else {
                break;
            };
            let candidate = Span {
                start,
                end: start.saturating_add(want),
            };
            if resolves_alone(source, candidate, node) {
                return Some(candidate);
            }
        }
        None
    }

    /// The indexed window holding `offset`, if any. An offset the parser's
    /// rewrites pushed onto a blank line belongs to no window, and gets no
    /// answer.
    fn window_holding(&self, offset: u32) -> Option<&IndexedWindow> {
        let after = self.windows.partition_point(|w| w.span.start <= offset);
        let window = self.windows.get(after.checked_sub(1)?)?;
        (offset <= window.span.end).then_some(window)
    }
}

impl IndexedWindow {
    /// The candidates of exactly `want` bytes, in source order.
    fn run_of_length(&self, want: u32) -> &[(u32, u32)] {
        let from = self.candidates.partition_point(|&(len, _)| len < want);
        let to = self.candidates.partition_point(|&(len, _)| len <= want);
        self.candidates.get(from..to).unwrap_or_default()
    }
}

/// Lex `source[window]` and record where each construct it holds sits in
/// `source`.
///
/// `None` when the window is one the parser rewrites before lexing, since
/// the offsets a sub-parse reports inside it would then be shifted too.
/// That is decided by the same proof the whole document runs — the window's
/// tiling against the window's own sentinel text — rather than by a list of
/// the rewrites, which are the parser's to make and to change.
fn index_window(source: &str, window: Span) -> Option<IndexedWindow> {
    let text = slice(source, window)?;
    if text.is_empty() {
        return None;
    }
    let arena = Arena::new();
    let lexed = aozora::lex_into_arena(text, &arena);
    if !ranges_address(text, &lexed) {
        return None;
    }
    let mut candidates: Vec<(u32, u32)> = lexed
        .source_nodes
        .iter()
        .map(|entry| {
            (
                entry
                    .source_span
                    .end
                    .saturating_sub(entry.source_span.start),
                window.start.saturating_add(entry.source_span.start),
            )
        })
        .collect();
    candidates.sort_unstable();
    Some(IndexedWindow {
        span: window,
        candidates,
    })
}

/// The range in `source` of the construct at `span`.
///
/// The reported range is tried first: whenever it already addresses
/// `source` — every construct ahead of the parser's first rewrite — its own
/// text parses back to the construct and no index is needed. Otherwise the
/// index answers.
fn resolve_in_source(
    source: &str,
    index: &SourceIndex,
    span: Span,
    node: NodeRef<'_>,
) -> Option<Span> {
    if resolves_alone(source, span, node) {
        return Some(span);
    }
    index.resolve(source, span, node)
}

/// Whether `source[at]`, parsed on its own, is exactly the construct
/// `node`: one notation of the same shape filling the whole slice.
///
/// Demanding the *whole* slice is also what makes a rewritten slice fail
/// closed: the parser reports its ranges against the text it derived, so a
/// slice it shortened (a BOM, a `\r`) or padded reports a notation that no
/// longer spans what we sliced.
fn resolves_alone(source: &str, at: Span, node: NodeRef<'_>) -> bool {
    let Some(text) = slice(source, at) else {
        return false;
    };
    if text.is_empty() {
        return false;
    }
    let want = at.end.saturating_sub(at.start);
    let arena = Arena::new();
    aozora::lex_into_arena(text, &arena)
        .source_nodes
        .iter()
        .any(|entry| {
            entry.source_span.start == 0
                && entry.source_span.end == want
                && same_shape(entry.node, node)
        })
}

/// Whether two constructs are the same shape: same sentinel kind, and for
/// the inline / block-leaf kinds the same notation variant. Container kinds
/// compare by sentinel alone — an open marker can only ever match an open
/// marker, whatever it opens.
fn same_shape(a: NodeRef<'_>, b: NodeRef<'_>) -> bool {
    match (a, b) {
        (NodeRef::Inline(a), NodeRef::Inline(b))
        | (NodeRef::BlockLeaf(a), NodeRef::BlockLeaf(b)) => {
            mem::discriminant(&a) == mem::discriminant(&b)
        }
        (NodeRef::BlockOpen(_), NodeRef::BlockOpen(_))
        | (NodeRef::BlockClose(_), NodeRef::BlockClose(_)) => true,
        _ => false,
    }
}

/// The pieces `source[window]` splits into on `separator`, as ranges into
/// `source`.
fn split_spans<'a>(
    source: &'a str,
    window: Span,
    separator: &'a str,
) -> impl Iterator<Item = Span> + 'a {
    let mut cursor = window.start;
    let step = saturating_u32(separator.len());
    slice(source, window)
        .unwrap_or_default()
        .split(separator)
        .map(move |piece| {
            let start = cursor;
            let end = start.saturating_add(saturating_u32(piece.len()));
            cursor = end.saturating_add(step);
            Span { start, end }
        })
}

/// `line` without the `\r` a CRLF document leaves at its end, so the line
/// itself can still be lexed even though the block around it cannot.
fn trim_carriage_return(source: &str, line: Span) -> Span {
    let trimmed = slice(source, line)
        .unwrap_or_default()
        .trim_end_matches('\r');
    Span {
        start: line.start,
        end: line.start.saturating_add(saturating_u32(trimmed.len())),
    }
}

/// `source[span]`, or `None` when the range is out of bounds or lands
/// mid-codepoint.
fn slice(source: &str, span: Span) -> Option<&str> {
    source.get(span.start as usize..span.end as usize)
}

// ===================================================================
// Cursor
// ===================================================================

/// One entry of the construct stream: what the parser resolved, plus the
/// byte range it occupied in the source.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstructHit<'src> {
    pub(crate) node: NodeRef<'src>,
    pub(crate) span: Option<Span>,
}

/// Cursor over a [`Constructs`] table.
///
/// Both [`crate::ast_splice`] and [`crate::ir`] walk the same table in
/// document order, each with its own cursor, so the two stay in lockstep
/// without being serially coupled.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstructCursor<'t, 'src> {
    table: &'t Constructs<'src>,
    idx: usize,
}

impl<'t, 'src> ConstructCursor<'t, 'src> {
    /// Peek the construct at `offset` past the current cursor.
    /// `peek(0)` returns the next entry that [`Self::next`] would
    /// produce.
    pub(crate) fn peek(&self, offset: usize) -> Option<NodeRef<'src>> {
        self.table
            .entries
            .get(self.idx + offset)
            .map(|entry| entry.node)
    }

    /// Consume and return the next construct, advancing the cursor.
    pub(crate) fn next(&mut self) -> Option<ConstructHit<'src>> {
        let publishable = self.table.ranges_address_source;
        let hit = self.table.entries.get(self.idx).map(|entry| ConstructHit {
            node: entry.node,
            span: publishable.then_some(entry.span),
        });
        if hit.is_some() {
            self.idx += 1;
        }
        hit
    }

    /// Consume the next construct, returning the source text it stands for.
    /// Used by the splicer / IR builder's literal-context paths.
    pub(crate) fn next_literal(&mut self) -> Option<Cow<'t, str>> {
        let idx = self.idx;
        if idx >= self.table.entries.len() {
            return None;
        }
        self.idx += 1;
        Some(self.table.literal_of(idx))
    }

    /// Saturating advance by `n` constructs.
    pub(crate) fn advance(&mut self, n: usize) {
        self.idx = self.idx.saturating_add(n).min(self.table.entries.len());
    }

    /// How many constructs the cursor has consumed. The streaming IR
    /// builder threads this across per-block calls.
    pub(crate) fn index(&self) -> usize {
        self.idx
    }
}

// ===================================================================
// comrak-side traversal primitives
// ===================================================================

/// How [`visit_text_leaves`] handles non-`Text` child nodes
/// (`Strong` / `Emph` / `Link` / `Code` / ...).
#[derive(Debug, Clone, Copy)]
pub(crate) enum InlineDescend {
    /// Bail out the moment a non-`Text` child is encountered. Used
    /// to validate "this paragraph is a single bare block-sentinel
    /// run" without false-positives from emphasis-wrapped content.
    StopAtNonText,
    /// Descend through emphasis / strong / link / code wrappers and
    /// keep visiting their `Text` leaves. The default for paragraph
    /// dispatch (sentinel counting, heading-hint peeking).
    DescendThrough,
}

/// Visit every `Text`-leaf descendant of `node` left-to-right.
///
/// `mode` decides what happens when the walker meets a non-`Text`
/// child (see [`InlineDescend`]). The closure is invoked once per
/// `Text` leaf with the leaf's string slice and may return
/// [`ControlFlow::Break`] to short-circuit the entire walk.
///
/// Returns `Err(())` when:
/// - `mode == StopAtNonText` and a non-`Text` child was encountered,
///   OR
/// - the closure returned `Break` at some point.
///
/// Returns `Ok(())` when the whole subtree was visited and every
/// closure invocation returned `Continue`.
///
/// `core::ops::ControlFlow<()>` is the visitor signal so callers can
/// thread their own early-bail without a bespoke enum.
pub(crate) fn visit_text_leaves<'a, F>(
    node: &'a AstNode<'a>,
    mode: InlineDescend,
    mut visit: F,
) -> Result<(), ()>
where
    F: FnMut(&str) -> ControlFlow<()>,
{
    // Iterative depth-first traversal over an explicit stack rather than
    // recursion. comrak can build arbitrarily deep *inline* nesting from
    // a small input (e.g. deeply nested emphasis / links), and a
    // recursive descent would exhaust the call stack — under the release
    // profile's `panic = "abort"` that is a hard process abort, which
    // both repos' SECURITY.md scope IN as a vulnerability (a crash on
    // untrusted input). The explicit stack moves the unbounded growth to
    // the heap, where it is bounded by the input size, not the OS stack.
    //
    // Ordering: `extend_children_rev` pushes a node's children in reverse
    // so they pop left-to-right, and a `Text` leaf is visited *before* its
    // own descendants are pushed. That reproduces the previous recursion's
    // exact left-to-right pre-order (visit a leaf, then its subtree, then
    // its siblings), which `paragraph_sole_block_sentinel` and `ParaScan`
    // both depend on for their sentinel-count / first-hit semantics.
    let mut stack: Vec<&'a AstNode<'a>> = Vec::new();
    extend_children_rev(&mut stack, node);
    while let Some(child) = stack.pop() {
        let data = child.data.borrow();
        match &data.value {
            NodeValue::Text(s) => {
                // Hold the `child.data` borrow across `visit` rather than
                // cloning the string out. The visitor only ever sees
                // `&str` — it cannot reach `child.data` — and every
                // visitor on this path is read-only (the splice's tree
                // mutation runs in a separate, later walk), so the
                // immutable borrow is sound and the per-leaf `Cow::clone`
                // — an owned-string deep copy on consolidated comrak text
                // — is pure waste.
                let flow = visit(s);
                drop(data);
                if flow == ControlFlow::Break(()) {
                    return Err(());
                }
                // A `Text` node can in principle have children under
                // non-pathological comrak inputs (emphasis splits etc.).
                // Visit them after the leaf itself (pre-order), before any
                // of the leaf's siblings.
                extend_children_rev(&mut stack, child);
            }
            _ => match mode {
                InlineDescend::StopAtNonText => return Err(()),
                InlineDescend::DescendThrough => {
                    drop(data);
                    extend_children_rev(&mut stack, child);
                }
            },
        }
    }
    Ok(())
}

/// Push `parent`'s children onto `stack` in reverse document order, so a
/// `Vec`-as-stack pops them left-to-right. Shared by the iterative
/// [`visit_text_leaves`] traversal.
fn extend_children_rev<'a>(stack: &mut Vec<&'a AstNode<'a>>, parent: &'a AstNode<'a>) {
    let start = stack.len();
    stack.extend(parent.children());
    stack[start..].reverse();
}

/// Walk a comrak paragraph node and return `Some(kind)` iff its
/// body, taken across all `Text`-node descendants, contains exactly
/// one block-sentinel codepoint and otherwise consists only of ASCII
/// whitespace, AND the paragraph has no non-`Text` descendants
/// (which would imply embedded inline structure incompatible with a
/// sole-sentinel paragraph). Allocation-free.
pub(crate) fn paragraph_sole_block_sentinel<'a>(
    node: &'a AstNode<'a>,
) -> Option<BlockSentinelKind> {
    let mut found: Option<BlockSentinelKind> = None;
    let walk_ok = visit_text_leaves(node, InlineDescend::StopAtNonText, |s| {
        for ch in s.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                continue;
            }
            let Some(kind) = BlockSentinelKind::from_char(ch) else {
                return ControlFlow::Break(());
            };
            if found.is_some() {
                return ControlFlow::Break(());
            }
            found = Some(kind);
        }
        ControlFlow::Continue(())
    })
    .is_ok();
    walk_ok.then_some(()).and(found)
}

/// Visit every `Text` descendant of `node` left-to-right, descending
/// through emphasis / strong / link / code wrappers. Unlike the
/// general [`visit_text_leaves`] this never bails — used for the
/// paragraph-level sentinel count + heading-hint peek where every
/// leaf must be observed.
pub(crate) fn for_each_text_descendant<'a, F>(node: &'a AstNode<'a>, mut visit: F)
where
    F: FnMut(&str),
{
    // `DescendThrough` + `Continue` can never short-circuit, so the
    // returned Result is structurally always `Ok(())`; we discard it.
    let _result = visit_text_leaves(node, InlineDescend::DescendThrough, |s| {
        visit(s);
        ControlFlow::Continue(())
    });
}

/// Single-descent paragraph profile: counts sentinel chars and remembers
/// the first heading-hint payload.
///
/// Both [`crate::ir`] and [`crate::ast_splice`] need this exact summary to
/// dispatch a paragraph to either heading-hint promotion (Case 2) or
/// ordinary inline processing (Case 3). Computing it here, once, keeps the
/// two walkers in lockstep without duplicating the peek-and-count loop.
#[derive(Debug)]
pub(crate) struct ParaScan<'src> {
    /// Total sentinel chars in the paragraph's text descendants.
    /// Equals the number of constructs the paragraph would consume
    /// during inline projection.
    pub(crate) total_sentinels: usize,
    /// First construct the parser classified as a heading hint.
    /// `None` if the paragraph carries no inline heading hint.
    pub(crate) first_heading_hint: Option<&'src HeadingHint<'src>>,
}

impl<'src> ParaScan<'src> {
    pub(crate) fn run<'a>(node: &'a AstNode<'a>, cursor: &ConstructCursor<'_, 'src>) -> Self {
        let mut total_sentinels = 0usize;
        let mut first_heading_hint = None;
        for_each_text_descendant(node, |text| {
            for ch in text.chars() {
                if !is_sentinel_char(ch) {
                    continue;
                }
                if first_heading_hint.is_none()
                    && let Some(NodeRef::Inline(AozoraNode::HeadingHint(h))) =
                        cursor.peek(total_sentinels)
                {
                    first_heading_hint = Some(h);
                }
                total_sentinels += 1;
            }
        });
        Self {
            total_sentinels,
            first_heading_hint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the table the pipeline would build for `src` and hand it to
    /// `check`. The arena the constructs borrow from lives for the call,
    /// so the callback shape is what keeps the lifetimes honest.
    fn with_constructs<R>(src: &str, check: impl FnOnce(&Constructs<'_>) -> R) -> R {
        let arena = Arena::new();
        let lex_out = aozora::lex_into_arena(src, &arena);
        check(&Constructs::build(src, Some(&lex_out)))
    }

    #[test]
    fn our_sentinels_are_the_parsers_sentinels() {
        // The fallback path takes the parser's own text verbatim, so the
        // two sentinel alphabets have to be the same codepoints. This is
        // the assertion that lets us own the constants without owning a
        // second alphabet.
        assert_eq!(INLINE_SENTINEL, aozora::INLINE_SENTINEL);
        assert_eq!(BLOCK_LEAF_SENTINEL, aozora::BLOCK_LEAF_SENTINEL);
        assert_eq!(BLOCK_OPEN_SENTINEL, aozora::BLOCK_OPEN_SENTINEL);
        assert_eq!(BLOCK_CLOSE_SENTINEL, aozora::BLOCK_CLOSE_SENTINEL);
    }

    #[test]
    fn is_sentinel_char_recognises_all_four() {
        for ch in [
            INLINE_SENTINEL,
            BLOCK_LEAF_SENTINEL,
            BLOCK_OPEN_SENTINEL,
            BLOCK_CLOSE_SENTINEL,
        ] {
            assert!(is_sentinel_char(ch), "{ch:?} should be a sentinel");
        }
    }

    #[test]
    fn is_sentinel_char_rejects_neighbours() {
        // Codepoints adjacent to the sentinel range must NOT match.
        assert!(!is_sentinel_char('\u{E000}'));
        assert!(!is_sentinel_char('\u{E005}'));
        assert!(!is_sentinel_char('a'));
        assert!(!is_sentinel_char('\0'));
    }

    #[test]
    fn block_sentinel_kind_from_char_round_trips() {
        assert_eq!(
            BlockSentinelKind::from_char(BLOCK_LEAF_SENTINEL),
            Some(BlockSentinelKind::Leaf)
        );
        assert_eq!(
            BlockSentinelKind::from_char(BLOCK_OPEN_SENTINEL),
            Some(BlockSentinelKind::Open)
        );
        assert_eq!(
            BlockSentinelKind::from_char(BLOCK_CLOSE_SENTINEL),
            Some(BlockSentinelKind::Close)
        );
        // Inline does NOT count as a block sentinel.
        assert!(BlockSentinelKind::from_char(INLINE_SENTINEL).is_none());
        assert!(BlockSentinelKind::from_char('a').is_none());
    }

    /// Documents whose notation the tiling has to reproduce exactly.
    /// Deliberately mixed: inline, block-leaf, paired container, forward
    /// reference, and a sentinel-dense pathological line.
    const TILED: &[&str] = &[
        "",
        "hello, world",
        "彼は｜青梅《おうめ》に行った。",
        "可哀想［＃「可哀想」に傍点］だ",
        "前［＃改ページ］後",
        "［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］",
        "見出し\n\n本文に｜青空《あおぞら》のルビと［＃「強調」に傍点］を混ぜた段落。\n\n\
         次の段落も｜漢字《かんじ》。",
        "｜A《a》｜B《b》｜C《c》［＃「D」に傍点］｜E《e》",
        "第一篇［＃「第一篇」は大見出し］\n\n本文",
        "※［＃二の字点、1-2-22］の外字",
        "`｜青梅《おうめ》` と [x](http://e.com/｜p《r》)",
    ];

    /// The load-bearing assertion of this crate's design: the ranges the
    /// parser reports tile the source with no gap and no overlap, so
    /// substituting each one for a sentinel reproduces the parser's own
    /// text byte for byte.
    #[test]
    fn ranges_tile_the_source_exactly() {
        for src in TILED {
            let arena = Arena::new();
            let lex_out = aozora::lex_into_arena(src, &arena);
            assert!(
                ranges_address(src, &lex_out),
                "every range must address the source for {src:?}"
            );
            let entries = tiled(src, lex_out.source_nodes);
            assert_eq!(
                entries.len(),
                lex_out.source_nodes.len(),
                "one entry per construct for {src:?}"
            );
            for entry in &entries {
                assert!(
                    entry.literal.as_deref().is_some_and(|run| !run.is_empty()),
                    "every construct must slice back to its notation for {src:?}"
                );
            }
        }
    }

    /// A forward reference (`可哀想［＃「可哀想」に傍点］`) names text that
    /// precedes the directive. Its range has to cover that text too, or
    /// slicing it would hand back a directive with nothing to point at —
    /// the premise the fragment side is built on.
    #[test]
    fn forward_reference_range_covers_the_text_it_refers_to() {
        for (src, expected) in [
            (
                "可哀想［＃「可哀想」に傍点］だ",
                "可哀想［＃「可哀想」に傍点］",
            ),
            ("20［＃「20」は縦中横］", "20［＃「20」は縦中横］"),
            ("青梅《おうめ》", "青梅《おうめ》"),
        ] {
            with_constructs(src, |constructs| {
                let Some(first) = constructs.entries.first() else {
                    panic!("{src:?} must produce a construct");
                };
                assert_eq!(
                    first.literal.as_deref(),
                    Some(expected),
                    "the range must cover the referenced text for {src:?}"
                );
            });
        }
    }

    #[test]
    fn table_carries_source_ranges_that_slice_back_to_the_notation() {
        const SRC: &str = "前｜青梅《おうめ》後";
        with_constructs(SRC, |constructs| {
            let mut cursor = constructs.cursor();
            let Some(ConstructHit {
                span: Some(span), ..
            }) = cursor.next()
            else {
                panic!("the ruby must be tracked with a range");
            };
            assert_eq!(slice(SRC, span), Some("｜青梅《おうめ》"));
            assert_eq!(constructs.text(), "前\u{E001}後");
        });
    }

    /// A CRLF or BOM-prefixed document tiles against the hygiene copy, so
    /// its constructs keep the text the author wrote for them. The ranges
    /// address that copy rather than the caller's input, so they are not
    /// published — the caller holds the CRLF text, where they would point
    /// one byte per preceding line too far.
    #[test]
    fn hygiene_copy_tiles_a_crlf_document_but_withholds_its_ranges() {
        for raw in [
            "前\r\n\r\n`｜青梅《おうめ》`へ",
            "\u{feff}前\n\n`｜青梅《おうめ》`へ",
        ] {
            with_constructs(raw, |constructs| {
                assert!(
                    !constructs.ranges_address_source,
                    "a range into our copy is not the caller's to slice: {raw:?}"
                );
                assert!(
                    !constructs.text().contains('\r'),
                    "comrak parses the folded text: {:?}",
                    constructs.text()
                );
                let mut cursor = constructs.cursor();
                assert_eq!(cursor.next().and_then(|hit| hit.span), None);
                let mut cursor = constructs.cursor();
                assert_eq!(
                    cursor.next_literal().as_deref(),
                    Some("｜青梅《おうめ》"),
                    "the literal is still sliceable for {raw:?}"
                );
            });
        }
    }

    /// The rewrites this crate does not reproduce — here a decorative rule
    /// gaining a blank line — leave the tiling untrusted. The parser's own
    /// text drives comrak, every construct stays in the stream (dropping one
    /// would desync both walkers), and a literal is recovered rather than
    /// sliced: the reported range lands mid-codepoint, so the index answers.
    #[test]
    fn untiled_document_keeps_its_constructs_and_finds_a_literal() {
        const RAW: &str = "本文\n----------\n彼は`｜青梅《おうめ》`へ";
        with_constructs(RAW, |constructs| {
            assert_eq!(
                constructs.entries.len(),
                1,
                "the ruby is still tracked: {constructs:?}"
            );
            let entry = &constructs.entries[0];
            assert!(
                entry.literal.is_none(),
                "nothing is sliced up front here: {entry:?}"
            );
            assert!(
                constructs.text().contains(INLINE_SENTINEL),
                "the parser's text drives comrak: {:?}",
                constructs.text()
            );
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next().and_then(|hit| hit.span), None);
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next_literal().as_deref(), Some("｜青梅《おうめ》"));
        });
    }

    #[test]
    fn text_hygiene_borrows_when_there_is_nothing_to_do() {
        assert!(matches!(
            text_hygiene("｜青梅《おうめ》\n本文"),
            Cow::Borrowed(_)
        ));
        assert_eq!(text_hygiene("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(text_hygiene("\u{feff}\u{feff}本文"), "本文");
    }

    /// Lex `src` and hand the constructs the parser reported to `check`.
    fn with_nodes<R>(src: &str, check: impl FnOnce(&[SourceNode<'_>]) -> R) -> R {
        let arena = Arena::new();
        let lex_out = aozora::lex_into_arena(src, &arena);
        check(lex_out.source_nodes)
    }

    fn span_of(entry: &SourceNode<'_>) -> Span {
        Span {
            start: entry.source_span.start,
            end: entry.source_span.end,
        }
    }

    /// Resolve `span` the way the fallback path does — the reported range
    /// first, then the recovery index.
    fn recover(src: &str, span: Span, node: NodeRef<'_>) -> Option<Span> {
        resolve_in_source(src, &SourceIndex::build(src), span, node)
    }

    #[test]
    fn recovery_places_a_construct_the_reported_offset_missed() {
        const SRC: &str = "序文\n\n前｜青梅《おうめ》後\nもう一行\n\n終わり";
        with_nodes(SRC, |nodes| {
            let [entry] = nodes else {
                panic!("the fixture holds exactly one construct: {nodes:?}");
            };
            let span = span_of(entry);
            // A range that already parses to the construct is the answer;
            // no index is consulted.
            assert_eq!(recover(SRC, span, entry.node), Some(span));
            // A range of the right length whose start sits elsewhere in the
            // same block — what a pre-lex rewrite leaves behind — still
            // names the construct.
            let shifted = Span {
                start: span.start - 3,
                end: span.end - 3,
            };
            assert_eq!(recover(SRC, shifted, entry.node), Some(span));
        });
    }

    #[test]
    fn recovery_picks_the_candidate_nearest_the_reported_offset() {
        // Two rubies of the same length in one block. Nothing but the
        // reported offset tells them apart, so the nearest candidate wins:
        // declining here would silently drop the author's notation from a
        // code span, or hand a link a plausible-looking wrong URL.
        const SRC: &str = "｜A《a》と｜B《b》";
        with_nodes(SRC, |nodes| {
            let [first, second] = nodes else {
                panic!("the fixture holds two rubies: {nodes:?}");
            };
            let (a, b) = (span_of(first), span_of(second));
            assert_eq!(
                a.end - a.start,
                b.end - b.start,
                "the fixture's two rubies must be the same length"
            );
            assert_eq!(recover(SRC, a, first.node), Some(a));
            assert_eq!(recover(SRC, b, second.node), Some(b));
            // A range that misses by a byte still names the ruby it
            // started in, either side of the gap between them.
            let nudged = Span {
                start: a.start + 1,
                end: a.end + 1,
            };
            assert_eq!(recover(SRC, nudged, first.node), Some(a));
            let nudged = Span {
                start: b.start - 1,
                end: b.end - 1,
            };
            assert_eq!(recover(SRC, nudged, second.node), Some(b));
        });
    }

    /// The index places block markers and container markers, not just
    /// inline notations: a candidate has to carry the same sentinel kind.
    #[test]
    fn recovery_places_block_and_container_markers_too() {
        const SRC: &str =
            "前\n\n［＃改ページ］\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］";
        with_nodes(SRC, |nodes| {
            assert!(nodes.len() >= 3, "the fixture holds three markers");
            for entry in nodes {
                let span = span_of(entry);
                assert_eq!(
                    recover(SRC, span, entry.node),
                    Some(span),
                    "recovery must place {:?}",
                    slice(SRC, span)
                );
            }
        });
    }

    /// Length alone is not enough: an annotation and a page break can be
    /// byte-for-byte the same size, and recovery must not offer one where
    /// the other was asked for.
    #[test]
    fn recovery_will_not_swap_one_construct_for_another_of_the_same_length() {
        const SRC: &str = "前［＃ほげふが］後\n\n［＃改ページ］";
        with_nodes(SRC, |nodes| {
            let [annotation, page_break] = nodes else {
                panic!("the fixture holds an annotation and a page break: {nodes:?}");
            };
            let annotation_span = span_of(annotation);
            let page_break_span = span_of(page_break);
            assert_eq!(
                annotation_span.end - annotation_span.start,
                page_break_span.end - page_break_span.start,
                "the fixture's two constructs must be the same length"
            );
            assert_eq!(
                recover(SRC, page_break_span, annotation.node),
                None,
                "a page break must not answer for an annotation"
            );
            // And a range past the end of the source places nothing.
            let past_end = Span {
                start: saturating_u32(SRC.len()) + 1,
                end: saturating_u32(SRC.len()) + 2,
            };
            assert_eq!(recover(SRC, past_end, annotation.node), None);
        });
    }

    /// A window the parser rewrites before lexing reports shifted offsets,
    /// so it is left out — and the lines inside it that the parser does not
    /// rewrite are indexed instead, which is what keeps a CRLF document or
    /// one carrying a decorative rule recoverable at all.
    #[test]
    fn recovery_indexes_the_lines_of_a_window_the_parser_rewrites() {
        for (src, notation) in [
            ("前\r\n｜青梅《おうめ》", "｜青梅《おうめ》"),
            (
                "本文\n----------\n彼は｜青梅《おうめ》へ",
                "｜青梅《おうめ》",
            ),
            ("\u{feff}前\n｜青梅《おうめ》", "｜青梅《おうめ》"),
        ] {
            let index = SourceIndex::build(src);
            let [window] = index.windows.as_slice() else {
                panic!("one line of {src:?} holds a construct: {index:?}");
            };
            let [(len, start)] = window.candidates.as_slice() else {
                panic!("that line holds exactly one construct: {window:?}");
            };
            assert_eq!(
                slice(
                    src,
                    Span {
                        start: *start,
                        end: start + len
                    }
                ),
                Some(notation),
                "the indexed range must cover the notation in {src:?}"
            );
        }
    }

    /// A document the parser rewrote can still hold two constructs of the
    /// same shape and the same byte length — the norm for CJK notation of
    /// equal character count — and each literal context must get its own.
    #[test]
    fn untiled_document_tells_two_constructs_of_the_same_shape_apart() {
        const RAW: &str = "本文\n----------\n`｜A《a》`と`｜B《b》`";
        with_constructs(RAW, |constructs| {
            assert!(
                constructs.untiled_source.is_some(),
                "the decorative rule must leave the tiling untrusted: {constructs:?}"
            );
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next_literal().as_deref(), Some("｜A《a》"));
            assert_eq!(cursor.next_literal().as_deref(), Some("｜B《b》"));
            assert!(cursor.next_literal().is_none());
        });
    }

    /// Every literal a document reads shares one index, so a document dense
    /// with literal contexts stays linear. Before the index this re-lexed
    /// the whole block per read, which a fuzz target could turn into
    /// minutes of work on tens of kilobytes.
    #[test]
    fn untiled_document_reads_every_literal_from_one_index() {
        const COUNT: usize = 400;
        let mut raw = String::from("本文\n----------\n");
        for _ in 0..COUNT {
            raw.push_str("`｜A《a》`");
        }
        with_constructs(&raw, |constructs| {
            assert!(
                constructs.untiled_source.is_some(),
                "the fixture must take the fallback path"
            );
            assert_eq!(constructs.entries.len(), COUNT);
            let mut cursor = constructs.cursor();
            for i in 0..COUNT {
                assert_eq!(
                    cursor.next_literal().as_deref(),
                    Some("｜A《a》"),
                    "literal {i} must be recovered"
                );
            }
        });
    }

    #[test]
    fn cursor_peeks_and_consumes_in_order() {
        const SRC: &str = "｜A《a》\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］";
        with_constructs(SRC, |constructs| {
            let mut cursor = constructs.cursor();
            assert!(matches!(
                cursor.peek(0),
                Some(NodeRef::Inline(AozoraNode::Ruby(_)))
            ));
            assert!(matches!(cursor.peek(1), Some(NodeRef::BlockOpen(_))));
            assert!(cursor.peek(9).is_none());
            let Some(hit) = cursor.next() else {
                panic!("the ruby must be tracked");
            };
            assert!(hit.span.is_some());
            assert_eq!(cursor.index(), 1);
            cursor.advance(99); // saturating
            assert!(cursor.next().is_none());
        });
    }

    #[test]
    fn cursor_yields_each_constructs_literal_in_order() {
        const SRC: &str = "｜A《a》と｜B《b》";
        with_constructs(SRC, |constructs| {
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next_literal().as_deref(), Some("｜A《a》"));
            assert_eq!(cursor.next_literal().as_deref(), Some("｜B《b》"));
            assert!(cursor.next_literal().is_none());
        });
    }

    #[test]
    fn markdown_only_table_is_empty() {
        let constructs = Constructs::build("plain text", None);
        assert!(constructs.text().is_empty());
        assert!(constructs.cursor().next().is_none());
    }

    #[test]
    fn tile_declines_ranges_that_do_not_address_the_source() {
        // Ranges out of bounds / out of order / mid-codepoint are the three
        // ways a range can fail to address our text; each must degrade to
        // "no tiling" rather than panic.
        let arena = Arena::new();
        let lex_out = aozora::lex_into_arena("｜青梅《おうめ》", &arena);
        let node = lex_out.source_nodes[0].node;
        let bogus = |start: u32, end: u32| {
            vec![SourceNode {
                source_span: aozora::Span { start, end },
                node,
            }]
        };
        assert!(tile("前後", &bogus(0, 99)).is_none(), "out of bounds");
        assert!(tile("前後", &bogus(3, 1)).is_none(), "inverted");
        assert!(tile("前後", &bogus(1, 3)).is_none(), "mid-codepoint");
        assert!(tile("前後", &bogus(0, 3)).is_some(), "a range that fits");
    }

    #[test]
    fn block_constructs_are_padded_into_paragraphs_of_their_own() {
        // A block marker is a line, not inline content: comrak has to see a
        // blank line on each side or the sentinel joins its neighbour's
        // paragraph and the sole-sentinel dispatch stops firing.
        with_constructs("前\n［＃改ページ］\n後", |constructs| {
            let standalone = format!("{BLANK_LINE}{BLOCK_LEAF_SENTINEL}{BLANK_LINE}");
            assert!(
                constructs.text().contains(&standalone),
                "block sentinel must stand alone: {:?}",
                constructs.text()
            );
        });
    }
}
