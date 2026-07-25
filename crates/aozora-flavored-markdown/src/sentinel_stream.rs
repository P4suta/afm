//! Shared primitives for both [`crate::ast_splice`] (HTML splicer)
//! and [`crate::ir`] (IR builder).
//!
//! Both downstream consumers walk the same sentinel-position stream. They
//! differ only in their emit target (string buffer vs. typed tree), not in
//! how they sequence it, so this module owns the sequencing primitives and
//! the two walkers stay in lockstep automatically.
//!
//! Design notes:
//!
//! - `is_sentinel_char` is a single subtract-and-compare on the codepoint
//!   (`ch as u32 - 0xE001 < 4`): every paragraph-text walk touches it per
//!   char, so it is hotter than the `matches!` chain it replaces.
//! - The cursor materialises the constructs into a `Vec` of
//!   (construct, source-text) pairs in source order. Both walkers consume
//!   entries linearly and never look up by position at rewrite time — order
//!   alone is sufficient, so this is `O(n)` with no re-scan.
//! - `paragraph_sole_block_sentinel` walks a comrak paragraph node directly,
//!   allocation-free, returning the kind of block sentinel iff the paragraph
//!   carries exactly one and no other non-whitespace content.

use core::ops::ControlFlow;

use aozora::pipeline::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, BorrowedLexOutput,
    INLINE_SENTINEL,
};
use aozora::syntax::borrowed::{AozoraNode, HeadingHint, NodeRef};
use comrak::nodes::{AstNode, NodeValue};

use crate::diagnostics::Span;

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

/// The lexer's normalised text, plus whether its byte offsets are also
/// valid offsets into the text the caller handed us.
///
/// Every span the lexer reports is measured against its Phase-0 output, and
/// that phase *moves bytes*: it strips leading BOMs, folds `\r\n` to `\n`,
/// decomposes accent digraphs inside `〔…〕`, and inserts a blank line
/// before decorative rules. On such an input a normalised offset addresses a
/// different — possibly non-`char`-boundary — position in the caller's own
/// source, so handing it out as "slice your source with this" would return
/// silently wrong text or panic.
///
/// [`Self::derived`] settles the question once, by comparing the two
/// texts, and the answer rides along to whoever publishes a span
/// (currently [`crate::ir`], which withholds spans it cannot honour).
#[derive(Debug, Clone, Copy)]
pub(crate) struct NormalizedSource<'s> {
    /// The normalised text. Span coordinates are byte offsets into this.
    pub(crate) text: &'s str,
    /// `true` when a span's offsets address `text` *and* the caller's
    /// source, because normalisation changed nothing.
    pub(crate) addresses_source: bool,
}

impl<'s> NormalizedSource<'s> {
    /// For a caller that supplies the normalised text itself: its own
    /// coordinates are, trivially, the ones the spans use.
    pub(crate) const fn verbatim(text: &'s str) -> Self {
        Self {
            text,
            addresses_source: true,
        }
    }

    /// For the internal pipeline, where `normalized` was derived from
    /// `source` by the lexer's Phase 0.
    ///
    /// Byte-equality is the exact test: this crate's code-block mask is
    /// char-for-char length preserving (every trigger and the mask are
    /// 3-byte codepoints), so an unchanged normalisation means the offsets
    /// index the caller's raw input as well.
    pub(crate) fn derived(normalized: &'s str, source: &str) -> Self {
        Self {
            text: normalized,
            addresses_source: normalized == source,
        }
    }
}

/// One entry of the sentinel stream: the construct a sentinel stands for,
/// plus the byte range it occupied in the source.
///
/// `span` is `None` for entries built without a usable source table —
/// [`SentinelCursor::from_nodes`] (unit tests and the streaming cursor
/// swap), or a normalisation that moved the coordinates away from the
/// caller's source (see [`NormalizedSource`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SentinelHit<'src> {
    pub(crate) node: NodeRef<'src>,
    pub(crate) span: Option<Span>,
}

/// Cursor over an owned sentinel-ordered stream of (construct, span,
/// literal) entries, where `literal` is the original source text the lexer
/// collapsed into that sentinel.
///
/// Both [`crate::ast_splice`] and [`crate::ir`] materialise the stream into
/// a `Vec` once, then walk it linearly. The cursor owns that `Vec` so
/// callers don't have to thread a separate slice lifetime through every
/// walker — a single `'src` is enough.
///
/// The owned `literal` lets the splicer's literal-context paths render a
/// sentinel that landed inside a markdown inline code span or a link
/// destination as its *original source*, not as the Aozora HTML. It is
/// owned (not a borrow) because it is sliced from the **normalised**
/// source — a transient buffer the lexer rewrites from the raw input
/// (BOM/CRLF/accent-span normalisation) — which the cursor must not
/// outlive a borrow into. Callers that don't need it (`from_nodes`, the
/// streaming swap) build with an empty literal and read nodes with
/// [`Self::next`] / [`Self::peek`].
#[derive(Debug)]
pub(crate) struct SentinelCursor<'src> {
    nodes: Vec<(SentinelHit<'src>, String)>,
    idx: usize,
}

impl<'src> SentinelCursor<'src> {
    /// Materialise the stream *with* each entry's original source text.
    /// Used by the HTML splicer and IR builder so a sentinel that lands in
    /// a literal markdown context (inline code, link URL) can be rewritten
    /// back to its original Aozora source instead of leaking the PUA char
    /// or rendering interpreted markup where it doesn't belong.
    ///
    /// `src` MUST be the lexer's input-normalisation output, because the
    /// span coordinates are in normalised-source bytes — slicing the raw
    /// input would misalign (or panic) on BOM / CRLF / accent-span inputs.
    /// The two tables are parallel and both source-ordered (pinned by
    /// `source_nodes_parallel_to_registry`), so zipping pairs each entry
    /// with its own span. The `get` guard keeps a span that somehow falls
    /// outside the text from panicking — it degrades to an empty literal
    /// rather than aborting the process.
    ///
    /// A hit only carries its span when [`NormalizedSource::addresses_source`]
    /// says the offsets still address the caller's source; otherwise the
    /// span is withheld rather than published in coordinates no consumer
    /// holds. Literals are unaffected — those are sliced from the
    /// normalised text right here, where the coordinates are always valid.
    pub(crate) fn from_lex_out_with_source(
        lex_out: Option<&BorrowedLexOutput<'src>>,
        src: NormalizedSource<'_>,
    ) -> Self {
        let nodes = lex_out.map_or_else(Vec::new, |lo| {
            lo.registry
                .iter_sorted()
                .zip(lo.source_nodes.iter())
                .map(|((_pos, node), sn)| {
                    let span = sn.source_span;
                    let literal = src
                        .text
                        .get(span.start as usize..span.end as usize)
                        .unwrap_or_default()
                        .to_owned();
                    let hit = SentinelHit {
                        node,
                        span: src.addresses_source.then_some(Span {
                            start: span.start,
                            end: span.end,
                        }),
                    };
                    (hit, literal)
                })
                .collect()
        });
        Self { nodes, idx: 0 }
    }

    /// Construct directly from a `Vec` of registry entries (used
    /// by tests and by the streaming builder which owns the `Vec`).
    /// Literals are empty and spans absent — neither is read on that path.
    pub(crate) fn from_nodes(nodes: Vec<NodeRef<'src>>) -> Self {
        Self {
            nodes: nodes
                .into_iter()
                .map(|node| (SentinelHit { node, span: None }, String::new()))
                .collect(),
            idx: 0,
        }
    }

    /// Peek the registry entry at `offset` past the current cursor.
    /// `peek(0)` returns the next entry that [`Self::next`] would
    /// produce.
    pub(crate) fn peek(&self, offset: usize) -> Option<NodeRef<'src>> {
        self.nodes.get(self.idx + offset).map(|(hit, _)| hit.node)
    }

    /// Consume and return the next entry, advancing the cursor.
    pub(crate) fn next(&mut self) -> Option<SentinelHit<'src>> {
        let n = self.nodes.get(self.idx).map(|(hit, _)| *hit);
        if n.is_some() {
            self.idx += 1;
        }
        n
    }

    /// Consume the next entry, returning its original source text. Used by
    /// the splicer / IR builder's literal-context paths.
    pub(crate) fn next_literal(&mut self) -> Option<&str> {
        if self.idx >= self.nodes.len() {
            return None;
        }
        let i = self.idx;
        self.idx += 1;
        Some(self.nodes[i].1.as_str())
    }

    /// Saturating advance by `n` entries.
    pub(crate) fn advance(&mut self, n: usize) {
        self.idx = self.idx.saturating_add(n).min(self.nodes.len());
    }
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
    /// Equals the number of registry entries the paragraph would
    /// consume during inline projection.
    pub(crate) total_sentinels: usize,
    /// First sentinel that the registry classifies as a heading hint.
    /// `None` if the paragraph carries no inline heading hint.
    pub(crate) first_heading_hint: Option<&'src HeadingHint<'src>>,
}

impl<'src> ParaScan<'src> {
    pub(crate) fn run<'a>(node: &'a AstNode<'a>, cursor: &SentinelCursor<'src>) -> Self {
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

    #[test]
    fn sentinel_cursor_peeks_and_consumes_in_order() {
        // Synthesise a small slice of NodeRefs for cursor mechanics.
        use aozora::syntax::ContainerKind;
        use aozora::syntax::borrowed::AozoraNode;
        let entries: Vec<NodeRef<'static>> = vec![
            NodeRef::Inline(AozoraNode::PageBreak),
            NodeRef::BlockOpen(ContainerKind::Keigakomi),
            NodeRef::BlockClose(ContainerKind::Keigakomi),
        ];
        let mut cursor = SentinelCursor::from_nodes(entries);
        assert!(matches!(
            cursor.peek(0),
            Some(NodeRef::Inline(AozoraNode::PageBreak))
        ));
        assert!(matches!(
            cursor.peek(2),
            Some(NodeRef::BlockClose(ContainerKind::Keigakomi))
        ));
        assert!(cursor.peek(3).is_none());
        let _ = cursor.next();
        assert!(matches!(
            cursor.next(),
            Some(SentinelHit {
                node: NodeRef::BlockOpen(ContainerKind::Keigakomi),
                span: None,
            })
        ));
        cursor.advance(99); // saturating
        assert!(cursor.next().is_none());
    }

    /// The source table is what makes the collapsed IR sliceable: each hit
    /// must come back with the byte range its notation occupied, not just
    /// the construct. Pin it end-to-end through the real lexer, since the
    /// zip in `from_lex_out_with_source` is the only place the two parallel
    /// tables meet.
    #[test]
    fn cursor_carries_the_source_span_of_each_hit() {
        use aozora::pipeline::lex_into_arena;
        use aozora::syntax::borrowed::Arena;

        const SRC: &str = "前｜青梅《おうめ》後";
        let arena = Arena::new();
        let lex_out = lex_into_arena(SRC, &arena);
        let mut cursor = SentinelCursor::from_lex_out_with_source(
            Some(&lex_out),
            NormalizedSource::derived(SRC, SRC),
        );

        let Some(SentinelHit {
            span: Some(span), ..
        }) = cursor.next()
        else {
            panic!("the ruby construct must come back with its source span");
        };
        assert_eq!(
            &SRC[span.start as usize..span.end as usize],
            "｜青梅《おうめ》",
            "span must slice back to the notation the author wrote"
        );
    }

    /// The counterpart: on an input Phase 0 rewrites, the lexer's offsets
    /// stop addressing the caller's source, so the cursor withholds them.
    /// CRLF is the case that matters in practice — 青空文庫 source is
    /// historically Shift_JIS + CRLF — and it is exactly where publishing
    /// the normalised offset would hand a consumer a mid-codepoint index.
    #[test]
    fn cursor_withholds_spans_when_normalisation_moved_the_bytes() {
        use aozora::pipeline::lex_into_arena;
        use aozora::pipeline::lexer::sanitize;
        use aozora::syntax::borrowed::Arena;

        const RAW: &str = "前\r\n\r\n｜青梅《おうめ》へ";
        let sanitized = sanitize(RAW);
        let arena = Arena::new();
        let lex_out = lex_into_arena(RAW, &arena);
        let mut cursor = SentinelCursor::from_lex_out_with_source(
            Some(&lex_out),
            NormalizedSource::derived(&sanitized.text, RAW),
        );

        let Some(hit) = cursor.next() else {
            panic!("the ruby construct is still tracked, span or no span");
        };
        assert!(
            hit.span.is_none(),
            "a normalised offset must not be published as a source offset: {hit:?}"
        );
        // Why it must not: the raw source is two bytes longer per folded
        // CRLF, so the normalised end offset lands inside a codepoint.
        let normalized_end = lex_out.source_nodes[0].source_span.end as usize;
        assert!(
            !RAW.is_char_boundary(normalized_end),
            "the fixture must keep exercising the mid-codepoint case"
        );
    }

    /// `SentinelCursor::from_lex_out_with_source` reads the registry's
    /// `iter_sorted` rather than re-scanning the normalized text. Pin that
    /// `iter_sorted` produces the *same* source-ordered sequence as a full
    /// normalized scan — the invariant the cursor's lockstep with
    /// `split_text_node` / `ParaScan` depends on. Checked on a
    /// sentinel-sparse (representative) and a sentinel-dense (pathological)
    /// document, since a divergence would surface in the dense case.
    #[test]
    fn flatten_matches_normalized_scan() {
        use aozora::NormalizedOffset;
        use aozora::pipeline::lex_into_arena;
        use aozora::syntax::borrowed::Arena;

        const REPRESENTATIVE: &str = "見出し\n\n本文に｜青空《あおぞら》のルビと\
            ［＃「強調」に傍点］を混ぜた段落。\n\n次の段落も｜漢字《かんじ》。";
        const PATHOLOGICAL: &str = "｜A《a》｜B《b》｜C《c》［＃「D」に傍点］｜E《e》";

        for src in [REPRESENTATIVE, PATHOLOGICAL] {
            let arena = Arena::new();
            let lex_out = lex_into_arena(src, &arena);

            // The positions the new `iter_sorted` path yields, in order.
            let via_iter_sorted: Vec<u32> =
                lex_out.registry.iter_sorted().map(|(pos, _)| pos).collect();

            // The positions the old full-normalized-scan path would yield.
            let mut via_scan: Vec<u32> = Vec::new();
            for (idx, ch) in lex_out.normalized.char_indices() {
                if !is_sentinel_char(ch) {
                    continue;
                }
                let pos = u32::try_from(idx).expect("normalized fits u32");
                if lex_out.registry.node_at(NormalizedOffset(pos)).is_some() {
                    via_scan.push(pos);
                }
            }

            assert_eq!(
                via_iter_sorted, via_scan,
                "iter_sorted order must match the normalized-scan order for {src:?}"
            );
            assert_eq!(
                via_iter_sorted.len(),
                lex_out.registry.len(),
                "one node per registry entry for {src:?}"
            );
        }
    }

    /// `from_lex_out_with_source` zips `registry.iter_sorted()` with
    /// `source_nodes` by position, so the two must stay parallel: same
    /// length, same source order. A divergence would pair a sentinel with
    /// the wrong span and silently rewrite literal-context text to the
    /// wrong source slice.
    #[test]
    fn source_nodes_parallel_to_registry() {
        use aozora::pipeline::lex_into_arena;
        use aozora::syntax::borrowed::Arena;

        const REPRESENTATIVE: &str = "本文に｜青空《あおぞら》のルビと\
            ［＃「強調」に傍点］を混ぜた段落。";
        const PATHOLOGICAL: &str = "｜A《a》｜B《b》｜C《c》［＃「D」に傍点］｜E《e》";

        for src in [REPRESENTATIVE, PATHOLOGICAL] {
            let arena = Arena::new();
            let lex_out = lex_into_arena(src, &arena);
            assert_eq!(
                lex_out.registry.len(),
                lex_out.source_nodes.len(),
                "registry and source_nodes must have equal length for {src:?}"
            );
            // Each source_nodes span, sliced from the source, must be a
            // non-empty original-text run (the lexer never tiles an empty
            // span), confirming the parallel table is usable for literal
            // reconstruction.
            for sn in lex_out.source_nodes {
                assert!(
                    !sn.source_span.slice(src).is_empty(),
                    "source span must slice a non-empty run for {src:?}"
                );
            }
        }
    }
}
