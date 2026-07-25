//! The replacement table this crate splices into the comrak AST, plus the
//! primitives both consumers ([`crate::ast_splice`] and [`crate::ir`]) use
//! to sequence it.
//!
//! # One coordinate space
//!
//! A 青空文庫 construct is addressed by a byte range into the text the parser
//! measured it against. [`Constructs::build`] tiles that text: the bytes
//! between constructs are copied verbatim, and each construct's range
//! collapses to one of this crate's four PUA sentinels. comrak parses the
//! result; the sentinels survive it untouched, being outside CommonMark's
//! escape set.
//!
//! Both walkers consume the table in document order and never look a
//! construct up by position, so the table is `O(n)` to build and `O(1)` per
//! step to walk.
//!
//! # Which text is tiled
//!
//! The parser canonicalises a document before reading it — it drops a
//! leading BOM, folds `\r` to `\n`, combines accent digraphs inside `〔…〕`,
//! and isolates a decorative rule with a blank line — and publishes that
//! text alongside the ranges. Nearly every document is already its own
//! canonical form, and then there is one text and one coordinate space.
//!
//! When it is not, the canonical text is what comrak must see (it is the
//! text the notation was read from), so this module parses *that* and tiles
//! it. The caller still holds the original, so the ranges published to
//! consumers come from the first read — paired construct for construct with
//! the second, and withheld where the two disagree. A range into a text no
//! consumer holds is a range no consumer can use.
//!
//! # What a construct renders to
//!
//! The same source run answers both questions the walkers ask. Read
//! verbatim it is the literal a markdown code span or link destination
//! needs; handed to [`crate::fragment`] it is the HTML the splice weaves in.
//! The table slices the run per construct and caches the fragment per run,
//! so a document's repeated notation is parsed once per distinct notation.
//!
//! # What counts as one construct
//!
//! A run only renders faithfully on its own if the whole of the notation is
//! inside it. Nearly every node the parser reports is that self-contained —
//! its notation *is* the node — but three shapes are not, and [`coalesce`]
//! folds each into the run that spans it before anything is tiled. A folded
//! group is one construct with one sentinel, so both walkers stay in step
//! without knowing the folding happened.

use core::fmt;
use core::ops::ControlFlow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use aozora::{ContainerKind, NodeKind, Snapshot};
use comrak::nodes::{AstNode, NodeValue};

use crate::diagnostics::{Diagnostic, Span};
use crate::fragment;

/// Inline construct (ruby / bouten / directive / gaiji / TCY / kaeriten).
pub(crate) const INLINE_SENTINEL: char = '\u{E001}';
/// Block-leaf construct (page break, section break, aozora heading, sashie).
pub(crate) const BLOCK_LEAF_SENTINEL: char = '\u{E002}';
/// Paired-container open marker (e.g. `［＃ここから字下げ］`).
pub(crate) const BLOCK_OPEN_SENTINEL: char = '\u{E003}';
/// Paired-container close marker (e.g. `［＃ここで字下げ終わり］`).
pub(crate) const BLOCK_CLOSE_SENTINEL: char = '\u{E004}';

/// A blank line. This crate wraps one around a block sentinel so comrak
/// sees it as a paragraph of its own rather than as inline content of its
/// neighbour.
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

    /// The codepoint that stands for this kind.
    #[inline]
    const fn sentinel(self) -> char {
        match self {
            Self::Leaf => BLOCK_LEAF_SENTINEL,
            Self::Open => BLOCK_OPEN_SENTINEL,
            Self::Close => BLOCK_CLOSE_SENTINEL,
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

/// Which sentinel stands for a construct of this kind, or `None` where the
/// construct belongs to the run of text around it.
///
/// The block kinds are the ones the notation puts on a line of their own: a
/// page break, a section break, the end-of-body marker, a 青空文庫 heading,
/// an illustration reference, and either half of a paired container. Every
/// other notation decorates the text it sits in.
///
/// The upstream enum is `#[non_exhaustive]`, so a kind added by a later
/// spec falls through to the inline arm — the shape a decorating notation
/// has, and the one that keeps the stream in step either way.
#[inline]
pub(crate) const fn block_sentinel_of(kind: NodeKind) -> Option<BlockSentinelKind> {
    match kind {
        NodeKind::ContainerOpen => Some(BlockSentinelKind::Open),
        NodeKind::ContainerClose => Some(BlockSentinelKind::Close),
        NodeKind::PageBreak
        | NodeKind::SectionBreak
        | NodeKind::BodyEnd
        | NodeKind::Heading
        | NodeKind::Illustration => Some(BlockSentinelKind::Leaf),
        _ => None,
    }
}

/// Whether an inline construct is consumed and dropped rather than
/// rendered. Both walkers ask, so neither can decide it differently.
///
/// Two notations never reach the output as themselves:
///
/// * a directive inside a heading. Its fragment is an
///   `aozora-md-directive` wrapper, which Tier C bars from a heading body.
/// * a heading hint, wherever an inline walk reaches one. A hint is a
///   directive about the text around it, and the paragraph case acts on it
///   by promoting the whole paragraph to a heading. Reaching it inline
///   means there is nothing to promote — a markdown heading, a table cell —
///   and rendering the hint there would put a marker into the very heading
///   it was written to name.
pub(crate) const fn inline_is_dropped(kind: NodeKind, in_heading: bool) -> bool {
    match kind {
        NodeKind::HeadingHint => true,
        NodeKind::Directive => in_heading,
        _ => false,
    }
}

// ===================================================================
// What one source run is
// ===================================================================

/// What the parser reports when it reads one source run on its own: the
/// constructs the run contains, in run-relative coordinates, and the HTML
/// the run renders to.
///
/// Both answers come from the same read, because both questions are asked
/// of the same runs — [`coalesce`] asks what a run contains to decide
/// whether it is a whole construct, and the splice asks what it renders to.
#[derive(Debug)]
struct RunFacts {
    nodes: Vec<(NodeKind, Span)>,
    html: String,
}

/// [`RunFacts`] by run, read once per distinct run.
///
/// A document repeats its notation — the same ruby, the same page break,
/// the same container marker — and the folding pass asks about a run more
/// than once besides, so this is what keeps the reads proportional to the
/// notation a document uses rather than to how often it uses it.
#[derive(Debug, Default)]
struct Runs(RefCell<HashMap<String, RunFacts>>);

impl Runs {
    /// Read `run` if it has not been read, then hand `take` its facts.
    fn with<R>(&self, run: &str, take: impl FnOnce(&RunFacts) -> R) -> R {
        if let Some(facts) = self.0.borrow().get(run) {
            return take(facts);
        }
        let facts = read_run(run);
        let mut cache = self.0.borrow_mut();
        take(cache.entry(run.to_owned()).or_insert(facts))
    }

    /// The constructs `run` contains when read on its own.
    fn nodes(&self, run: &str) -> Vec<(NodeKind, Span)> {
        self.with(run, |facts| facts.nodes.clone())
    }

    /// The HTML `run` renders to.
    fn html(&self, run: &str) -> String {
        self.with(run, |facts| facts.html.clone())
    }

    /// Whether `run` renders to nothing at all — the shape a marker has
    /// when it closes something that is not there to be closed.
    fn renders_to_nothing(&self, run: &str) -> bool {
        self.with(run, |facts| facts.html.is_empty())
    }
}

/// Read one source run on its own.
///
/// A run past the parser's span budget reads as nothing: the entry points
/// decline a document that large before any construct is sliced out of it,
/// so this cannot be reached from a render that started.
fn read_run(run: &str) -> RunFacts {
    let Ok(document) = aozora::parse(run.to_owned()) else {
        return RunFacts {
            nodes: Vec::new(),
            html: String::new(),
        };
    };
    let snapshot = document.snapshot();
    RunFacts {
        nodes: nodes_of(&snapshot),
        html: fragment::of(&snapshot),
    }
}

// ===================================================================
// The table
// ===================================================================

/// One construct before it is tiled: what the parser resolved, where its
/// notation sits in the text being tiled, and where it sits in the caller's
/// own text.
#[derive(Debug, Clone, Copy)]
struct Node {
    kind: NodeKind,
    /// The byte range this notation occupies in the text that was tiled —
    /// the one coordinate space this crate can still slice after the fact.
    run: Span,
    /// The byte range this notation occupies in the caller's own text, or
    /// `None` where the parser canonicalised that text and the two reads
    /// could not be paired.
    span: Option<Span>,
}

/// One construct: what the parser resolved, where its notation sits, and
/// the text the author wrote for it.
#[derive(Debug)]
struct Construct {
    kind: NodeKind,
    /// The byte range this notation occupies in the caller's own text, or
    /// `None` where the parser canonicalised that text and the two reads
    /// could not be paired.
    span: Option<Span>,
    /// The byte range this notation occupies in the text that was tiled —
    /// the one coordinate space this crate can still slice after the fact.
    run: Span,
    /// The source run the sentinel stands for.
    literal: String,
}

/// Source-ordered construct table plus the text comrak parses.
#[derive(Debug)]
pub(crate) struct Constructs {
    /// The text comrak parses: the tiled text with every construct
    /// replaced by one sentinel.
    text: String,
    /// The text that was tiled, kept whole. A construct's own run answers
    /// nearly every question about it; a heading hint is the exception —
    /// see [`Constructs::heading_hint_of`].
    tiled: String,
    entries: Vec<Construct>,
    /// What the parser observed, plus this crate's own report of any
    /// construct whose range did not address the text it was measured
    /// against.
    diagnostics: Vec<Diagnostic>,
    /// What each source run contains and renders to.
    runs: Runs,
}

impl Constructs {
    /// Empty table for the markdown-only path (`Options::aozora_enabled =
    /// false`), where no notation is recognised and the caller's own text
    /// goes straight to comrak.
    pub(crate) fn none() -> Self {
        Self::verbatim("")
    }

    /// Parse `source`, then tile it into sentinel-bearing text plus the
    /// construct table.
    pub(crate) fn build(source: &str) -> Self {
        let Ok(document) = aozora::parse(source.to_owned()) else {
            // Beyond the parser's span budget. The entry points guard on
            // that first, so this is unreachable in practice and degrades
            // to "no notation" rather than to a panic.
            return Self::verbatim(source);
        };
        let snapshot = document.snapshot();
        let diagnostics: Vec<Diagnostic> = snapshot
            .diagnostics()
            .iter()
            .map(Diagnostic::from)
            .collect();

        // The common case: the caller already wrote the canonical text, so
        // the ranges the parser reported address it and there is one
        // coordinate space for the whole render.
        if snapshot.normalized_source() == source {
            return Self::from_read(source, &snapshot, None, diagnostics);
        }

        // Otherwise comrak has to see the canonical text — that is what the
        // notation was read from — so it is parsed in its own right and
        // tiled. The ranges published to consumers still come from the read
        // against the caller's text.
        let Ok(canonical) = aozora::parse(snapshot.normalized_source().to_owned()) else {
            return Self::verbatim(source);
        };
        let canonical = canonical.snapshot();
        let published = nodes_of(&snapshot);
        Self::from_read(
            canonical.source(),
            &canonical,
            Some(&published),
            diagnostics,
        )
    }

    /// A table with no constructs whose text is `source` verbatim.
    fn verbatim(source: &str) -> Self {
        Self {
            text: source.to_owned(),
            tiled: source.to_owned(),
            entries: Vec::new(),
            diagnostics: Vec::new(),
            runs: Runs::default(),
        }
    }

    /// Fold `snapshot`'s nodes into constructs and tile `text` with them.
    ///
    /// `published` is the node table of a separate read against the
    /// caller's own text, when that text differed from `text`.
    fn from_read(
        text: &str,
        snapshot: &Snapshot,
        published: Option<&[(NodeKind, Span)]>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        let runs = Runs::default();
        let nodes = pair_reads(&nodes_of(snapshot), published);
        let nodes = coalesce(text, &nodes, snapshot, &runs);
        Self::tile(text, &nodes, diagnostics, runs)
    }

    /// Substitute one sentinel per construct in `text`.
    ///
    /// A range that does not address `text` — out of bounds, out of order,
    /// or landing mid-codepoint — drops its construct from both the tiling
    /// and the table, so the sentinel stream and the table stay in step,
    /// and the render says how many were dropped.
    fn tile(text: &str, nodes: &[Node], mut diagnostics: Vec<Diagnostic>, runs: Runs) -> Self {
        let mut tiled = String::with_capacity(text.len());
        let mut entries = Vec::with_capacity(nodes.len());
        let mut cursor = 0usize;
        let mut lost = 0usize;
        for node in nodes {
            let start = node.run.start as usize;
            let end = node.run.end as usize;
            let piece = (start >= cursor)
                .then(|| text.get(cursor..start))
                .flatten()
                .zip(text.get(start..end));
            let Some((gap, literal)) = piece else {
                lost += 1;
                continue;
            };
            tiled.push_str(gap);
            push_sentinel(&mut tiled, node.kind);
            cursor = end;
            entries.push(Construct {
                kind: node.kind,
                span: node.span,
                run: node.run,
                literal: literal.to_owned(),
            });
        }
        tiled.push_str(text.get(cursor..).unwrap_or_default());
        if lost > 0 {
            diagnostics.push(Diagnostic::constructs_unresolved(lost));
        }
        Self {
            text: tiled,
            tiled: text.to_owned(),
            entries,
            diagnostics,
            runs,
        }
    }

    /// The HTML construct `idx` renders to, or `None` when the run behind
    /// it is empty and there is nothing to render.
    fn fragment_of(&self, idx: usize) -> Option<String> {
        let run = self.entries.get(idx).map(|entry| entry.literal.as_str())?;
        (!run.is_empty()).then(|| self.runs.html(run))
    }

    /// What the parser observed about this document, plus one warning per
    /// render that dropped a construct it could not place.
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The heading level and title construct `idx` names, when it is a
    /// heading hint that names a run of text rather than being one.
    ///
    /// A hint is the one notation whose own run does *not* cover the text
    /// it is about, so — alone among the constructs — the run is widened to
    /// the line it sits on before it is rendered. On that line the hint
    /// reaches the run it names and reports it; off it, the renderer treats
    /// the hint as its own text, which is the answer for a hint that names
    /// nothing and therefore promotes nothing.
    fn heading_hint_of(&self, idx: usize) -> Option<HeadingHint> {
        let entry = self.entries.get(idx)?;
        if entry.kind != NodeKind::HeadingHint {
            return None;
        }
        let line = slice(&self.tiled, line_around(&self.tiled, entry.run))?;
        parse_heading_hint(&self.runs.html(line))
    }

    /// The text comrak parses.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Cursor positioned before the first construct.
    pub(crate) fn cursor(&self) -> ConstructCursor<'_> {
        self.cursor_at(0)
    }

    /// Cursor positioned before construct `idx`. The streaming builder
    /// resumes here between blocks.
    pub(crate) fn cursor_at(&self, idx: usize) -> ConstructCursor<'_> {
        ConstructCursor {
            table: self,
            idx: idx.min(self.entries.len()),
        }
    }
}

/// Pair the read that drives the tiling with the read against the caller's
/// own text, so every node carries both ranges.
///
/// The published range is identity when there was only ever one text.
/// Otherwise it comes from the read against the caller's own, and only
/// where that read found the same construct in the same position — a
/// document whose two reads disagree publishes no range rather than a
/// plausible wrong one.
fn pair_reads(nodes: &[(NodeKind, Span)], published: Option<&[(NodeKind, Span)]>) -> Vec<Node> {
    nodes
        .iter()
        .enumerate()
        .map(|(idx, &(kind, run))| Node {
            kind,
            run,
            span: published.map_or(Some(run), |published| {
                published
                    .get(idx)
                    .filter(|&&(other, _)| other == kind)
                    .map(|&(_, other)| other)
            }),
        })
        .collect()
}

// ===================================================================
// Folding nodes into constructs
// ===================================================================

/// How far past a node the folding pass will look for the rest of its
/// construct.
///
/// A construct that reaches beyond its own node reaches exactly one other —
/// the marker that closes it, or the directive that names it. The bound is
/// slack around that, and it is what keeps the pass linear: a run that has
/// not closed within a few nodes is not one of these shapes.
const FOLD_REACH: usize = 4;

/// Fold the parser's nodes into the units this crate can render one at a
/// time.
///
/// A node is such a unit when its own notation is the whole of it, because
/// then reading that notation on its own reaches the answer the document
/// reached. Nearly every node is. Three shapes are not:
///
/// * a **paired heading** — `［＃中見出し］見出し［＃中見出し終わり］`. The
///   markers bracket a heading *body*, which is phrasing content: `<h1>`–
///   `<h6>` admit no block, so this cannot be a container comrak fills
///   without putting a `<p>` where the HTML content model forbids one.
/// * a **forward reference whose target is not adjacent** —
///   `可哀想な人［＃「可哀想」に傍点］`. The parser reports the referenced text
///   and the directive as two nodes. The first carries no notation at all,
///   so on its own it renders as bare text; the second resolves against its
///   own copy of the target and renders it a second time.
/// * an **inline bracket pair** — `［＃割り注］…［＃割り注終わり］`. The close
///   renders to nothing on its own, having no open to close, and the open
///   renders as an empty element, so the body between them lands outside
///   both.
///
/// Each is folded into the one run that spans it, which then renders as the
/// document renders it. Everything the fold swallows is notation the parser
/// owns anyway, with one deliberate exception: markdown written *between*
/// the two halves of an inline bracket pair or a paired heading is read as
/// 青空文庫 text rather than as markdown. That is the content model talking
/// in the heading case, and in the bracket case it is the price of the body
/// reaching the wrapper at all.
fn coalesce(text: &str, nodes: &[Node], snapshot: &Snapshot, runs: &Runs) -> Vec<Node> {
    let fold = Fold {
        text,
        runs,
        headings: heading_markers(snapshot),
        paired: container_markers(snapshot),
    };
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut idx = 0usize;
    while idx < nodes.len() {
        let (last, kind) = fold
            .heading(nodes, idx)
            .map_or_else(
                || fold.coupled(nodes, idx).map(|last| (last, nodes[idx].kind)),
                |last| Some((last, NodeKind::Heading)),
            )
            .unwrap_or((idx, nodes[idx].kind));
        out.push(Node {
            kind,
            run: Span {
                start: nodes[idx].run.start,
                end: nodes[last].run.end,
            },
            span: nodes[idx]
                .span
                .zip(nodes[last].span)
                .map(|(first, last)| Span {
                    start: first.start,
                    end: last.end,
                }),
        });
        idx = last + 1;
    }
    out
}

/// What the folding pass needs to answer "does this construct reach past
/// its own node, and where does it end".
struct Fold<'a> {
    /// The text being tiled — the one coordinate space the node ranges
    /// address.
    text: &'a str,
    /// What each candidate run contains and renders to.
    runs: &'a Runs,
    /// Where each paired heading's opening marker starts, mapped to where
    /// its closing marker starts.
    headings: HashMap<u32, u32>,
    /// Where every paired container's markers start.
    paired: HashSet<u32>,
}

impl Fold<'_> {
    /// The node that closes the paired heading opening at `idx`, if that is
    /// what `idx` opens.
    fn heading(&self, nodes: &[Node], idx: usize) -> Option<usize> {
        if nodes[idx].kind != NodeKind::ContainerOpen {
            return None;
        }
        let close = *self.headings.get(&nodes[idx].run.start)?;
        nodes
            .iter()
            .enumerate()
            .skip(idx + 1)
            .find(|(_, node)| node.run.start == close)
            .map(|(last, _)| last)
    }

    /// The node that completes the construct starting at `idx`, when that
    /// construct reaches past its own node.
    ///
    /// Two shapes reach: a node whose run carries no notation of its own
    /// (the target half of a forward reference), and a node whose partner
    /// renders to nothing (the open half of an inline bracket pair). Either
    /// way the fold is only taken when the widened run *reproduces the
    /// group* — reading it on its own has to report exactly the nodes the
    /// document reported inside it. That is what keeps a fold from
    /// inventing a construct the document does not have.
    fn coupled(&self, nodes: &[Node], idx: usize) -> Option<usize> {
        let first = nodes[idx];
        if self.paired.contains(&first.run.start) {
            return None;
        }
        // A node that reports itself when read alone carries its own
        // notation, so it only reaches past itself to collect a partner
        // that does not render on its own.
        let self_reporting = self.slice(first.run).is_some_and(|run| {
            self.runs.nodes(run)
                == [(
                    first.kind,
                    Span {
                        start: 0,
                        end: saturating_u32(run.len()),
                    },
                )]
        });
        for last in idx + 1..(idx + 1 + FOLD_REACH).min(nodes.len()) {
            let next = nodes[last];
            if self.paired.contains(&next.run.start) {
                break;
            }
            let group = Span {
                start: first.run.start,
                end: next.run.end,
            };
            // A fold never crosses a line: past one, the text between the
            // two halves is block structure, and block structure is
            // comrak's.
            let Some(run) = self.slice(group).filter(|run| !run.contains('\n')) else {
                break;
            };
            let closes_the_first = next.kind == first.kind
                && self
                    .slice(next.run)
                    .is_some_and(|run| self.runs.renders_to_nothing(run));
            if (!self_reporting || closes_the_first)
                && reproduces(self.runs, run, &nodes[idx..=last])
            {
                return Some(last);
            }
        }
        None
    }

    /// The text `span` addresses.
    fn slice(&self, span: Span) -> Option<&str> {
        slice(self.text, span)
    }
}

/// Where each paired heading's opening marker starts, mapped to where its
/// closing marker starts.
fn heading_markers(snapshot: &Snapshot) -> HashMap<u32, u32> {
    snapshot
        .container_pairs()
        .iter()
        .filter(|pair| pair.kind() == ContainerKind::Heading)
        .map(|pair| (pair.open().start, pair.close().start))
        .collect()
}

/// Where every paired container's markers start — the nodes whose pairing
/// the parser already reports, and which the block sentinels already carry.
fn container_markers(snapshot: &Snapshot) -> HashSet<u32> {
    snapshot
        .container_pairs()
        .iter()
        .flat_map(|pair| [pair.open().start, pair.close().start])
        .collect()
}

/// Whether reading `run` on its own reports exactly the nodes the document
/// reported for `group`, in the same order and at the same offsets into the
/// run.
fn reproduces(runs: &Runs, run: &str, group: &[Node]) -> bool {
    let Some(base) = group.first().map(|node| node.run.start) else {
        return false;
    };
    let expected = group.iter().map(|node| {
        (
            node.kind,
            Span {
                start: node.run.start - base,
                end: node.run.end - base,
            },
        )
    });
    runs.nodes(run).into_iter().eq(expected)
}

/// Every construct a snapshot reports, in document order, as the pair the
/// tiling needs: what it is, and where it sits in the text it was measured
/// against.
fn nodes_of(snapshot: &Snapshot) -> Vec<(NodeKind, Span)> {
    snapshot
        .nodes()
        .iter()
        .map(|node| {
            (
                node.kind(),
                Span {
                    start: node.span().start,
                    end: node.span().end,
                },
            )
        })
        .collect()
}

/// Append the sentinel that stands for a construct of `kind`, padded into a
/// paragraph of its own for the block kinds — a block marker is a line, not
/// a run of inline text, and comrak has to see it that way.
fn push_sentinel(text: &mut String, kind: NodeKind) {
    let Some(block) = block_sentinel_of(kind) else {
        text.push(INLINE_SENTINEL);
        return;
    };
    text.push_str(BLANK_LINE);
    text.push(block.sentinel());
    text.push_str(BLANK_LINE);
}

// ===================================================================
// Heading hints
// ===================================================================

/// The heading a hint asks for: how deep, and what to call it.
///
/// Read off the hint's own rendered fragment rather than from a typed
/// payload, so a spec that grows a new heading style needs no change here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeadingHint {
    pub(crate) level: u8,
    pub(crate) target: String,
}

/// Attribute the fragment carries the outline level in.
const LEVEL_ATTRIBUTE: &str = "data-level=\"";
/// Attribute the fragment carries the heading text in. Present only where
/// the text lives in a run the hint refers back to — which is the only
/// shape a paragraph can be promoted on.
const TARGET_ATTRIBUTE: &str = "data-target=\"";

/// The heading a hint's fragment describes, when there is one to promote
/// to.
///
/// A hint that names a run of text carries it as `data-target`, and the run
/// it names is the heading's body. A hint that is its *own* text carries no
/// target and renders that text visibly instead — it sits mid-line, where a
/// block heading is not valid, so there is nothing to promote and this
/// answers `None`.
fn parse_heading_hint(html: &str) -> Option<HeadingHint> {
    let level = attribute(html, LEVEL_ATTRIBUTE)?.parse::<u8>().ok()?;
    let target = attribute(html, TARGET_ATTRIBUTE)?;
    Some(HeadingHint {
        level,
        target: unescape(target),
    })
}

/// The line `span` sits on, as a range into `text`.
fn line_around(text: &str, span: Span) -> Span {
    let start = text
        .get(..span.start as usize)
        .and_then(|head| head.rfind('\n').map(|at| at + 1))
        .unwrap_or(0);
    let end = text
        .get(span.end as usize..)
        .and_then(|tail| tail.find('\n'))
        .map_or_else(
            || saturating_u32(text.len()),
            |at| span.end.saturating_add(saturating_u32(at)),
        );
    Span {
        start: saturating_u32(start),
        end,
    }
}

/// `text[span]`, or `None` when the range is out of bounds or lands
/// mid-codepoint.
fn slice(text: &str, span: Span) -> Option<&str> {
    text.get(span.start as usize..span.end as usize)
}

/// The value of the attribute `html` opens with `name`.
fn attribute<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let value = html.find(name)? + name.len();
    let rest = html.get(value..)?;
    rest.find('"').and_then(|end| rest.get(..end))
}

/// `text` with the HTML character references a renderer emits read back.
/// Anything else beginning with `&` is the author's own text.
///
/// Both apostrophe spellings are listed because the two renderers in play
/// disagree on it — the parser writes `&#x27;`, comrak writes `&#39;` — and
/// reading either back costs one more row.
fn unescape(text: &str) -> String {
    const ENTITIES: [(&str, char); 6] = [
        ("amp;", '&'),
        ("lt;", '<'),
        ("gt;", '>'),
        ("quot;", '"'),
        ("#39;", '\''),
        ("#x27;", '\''),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        match ENTITIES.iter().find(|(entity, _)| rest.starts_with(entity)) {
            Some(&(entity, ch)) => {
                out.push(ch);
                rest = &rest[entity.len()..];
            }
            None => out.push('&'),
        }
    }
    out.push_str(rest);
    out
}

// ===================================================================
// Cursor
// ===================================================================

/// One entry of the construct stream: what the parser resolved, the byte
/// range it occupied in the source, and — on demand — the HTML it renders
/// to.
///
/// The fragment is asked for rather than carried because a construct does
/// not always reach the output: an orphan close, or a directive inside a
/// heading, is consumed to keep the stream in step and then dropped. Those
/// never pay for a parse.
#[derive(Clone, Copy)]
pub(crate) struct ConstructHit<'t> {
    pub(crate) kind: NodeKind,
    pub(crate) span: Option<Span>,
    table: &'t Constructs,
    idx: usize,
}

impl ConstructHit<'_> {
    /// The HTML this construct renders to, or `None` when there is no run
    /// behind it. A caller that has block structure riding on the answer (a
    /// container marker) must not treat `None` as empty markup: nothing was
    /// rendered, so nothing was opened or closed either.
    pub(crate) fn html(&self) -> Option<String> {
        self.table.fragment_of(self.idx)
    }

    /// The markup that opens this container and the markup that closes it.
    /// `None` when the marker renders to nothing, which opens nothing.
    pub(crate) fn container_halves(&self) -> Option<(String, String)> {
        let fragment = self.html()?;
        let (open, close) = fragment::halves(&fragment);
        (!open.is_empty()).then(|| (open.to_owned(), close.to_owned()))
    }
}

impl fmt::Debug for ConstructHit<'_> {
    /// Elides the table — every hit borrows the same one, and printing it
    /// per hit would bury whatever the caller was debugging.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConstructHit")
            .field("kind", &self.kind)
            .field("span", &self.span)
            .field("idx", &self.idx)
            .finish()
    }
}

/// Cursor over a [`Constructs`] table.
///
/// Both [`crate::ast_splice`] and [`crate::ir`] walk the same table in
/// document order, each with its own cursor, so the two stay in lockstep
/// without being serially coupled.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstructCursor<'t> {
    table: &'t Constructs,
    idx: usize,
}

impl<'t> ConstructCursor<'t> {
    /// The heading the construct at `offset` past the cursor asks for,
    /// when it is a heading hint.
    pub(crate) fn heading_hint(&self, offset: usize) -> Option<HeadingHint> {
        self.table.heading_hint_of(self.idx + offset)
    }

    /// Consume and return the next construct, advancing the cursor.
    pub(crate) fn next(&mut self) -> Option<ConstructHit<'t>> {
        let idx = self.idx;
        let hit = self.table.entries.get(idx).map(|entry| ConstructHit {
            kind: entry.kind,
            span: entry.span,
            table: self.table,
            idx,
        });
        if hit.is_some() {
            self.idx += 1;
        }
        hit
    }

    /// Consume the next construct, returning the source text it stands for.
    /// Used by the splicer / IR builder's literal-context paths.
    pub(crate) fn next_literal(&mut self) -> Option<&'t str> {
        let literal = self
            .table
            .entries
            .get(self.idx)
            .map(|entry| entry.literal.as_str())?;
        self.idx += 1;
        Some(literal)
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
/// the first heading hint.
///
/// Both [`crate::ir`] and [`crate::ast_splice`] need this exact summary to
/// dispatch a paragraph to either heading-hint promotion (Case 2) or
/// ordinary inline processing (Case 3). Computing it here, once, keeps the
/// two walkers in lockstep without duplicating the peek-and-count loop.
#[derive(Debug)]
pub(crate) struct ParaScan {
    /// Total sentinel chars in the paragraph's text descendants.
    /// Equals the number of constructs the paragraph would consume
    /// during inline projection.
    pub(crate) total_sentinels: usize,
    /// The heading the paragraph's first heading hint asks for.
    /// `None` if the paragraph carries no inline heading hint.
    pub(crate) first_heading_hint: Option<HeadingHint>,
}

impl ParaScan {
    pub(crate) fn run<'a>(node: &'a AstNode<'a>, cursor: &ConstructCursor<'_>) -> Self {
        let mut total_sentinels = 0usize;
        let mut first_heading_hint = None;
        for_each_text_descendant(node, |text| {
            for ch in text.chars() {
                if !is_sentinel_char(ch) {
                    continue;
                }
                if first_heading_hint.is_none() {
                    first_heading_hint = cursor.heading_hint(total_sentinels);
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
    /// `check`.
    fn with_constructs<R>(src: &str, check: impl FnOnce(&Constructs) -> R) -> R {
        check(&Constructs::build(src))
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
        for kind in [
            BlockSentinelKind::Leaf,
            BlockSentinelKind::Open,
            BlockSentinelKind::Close,
        ] {
            assert_eq!(BlockSentinelKind::from_char(kind.sentinel()), Some(kind));
        }
        // Inline does NOT count as a block sentinel.
        assert!(BlockSentinelKind::from_char(INLINE_SENTINEL).is_none());
        assert!(BlockSentinelKind::from_char('a').is_none());
    }

    /// The notation this crate puts on a line of its own, and the notation
    /// it leaves in the run of text around it.
    #[test]
    fn block_kinds_are_the_ones_the_notation_gives_a_line_to() {
        for kind in [
            NodeKind::PageBreak,
            NodeKind::SectionBreak,
            NodeKind::BodyEnd,
            NodeKind::Heading,
            NodeKind::Illustration,
        ] {
            assert_eq!(
                block_sentinel_of(kind),
                Some(BlockSentinelKind::Leaf),
                "{kind:?} stands alone"
            );
        }
        assert_eq!(
            block_sentinel_of(NodeKind::ContainerOpen),
            Some(BlockSentinelKind::Open)
        );
        assert_eq!(
            block_sentinel_of(NodeKind::ContainerClose),
            Some(BlockSentinelKind::Close)
        );
        for kind in [
            NodeKind::Ruby,
            NodeKind::Bouten,
            NodeKind::Gaiji,
            NodeKind::Directive,
            NodeKind::HeadingHint,
            NodeKind::AngleQuote,
        ] {
            assert_eq!(block_sentinel_of(kind), None, "{kind:?} decorates its run");
        }
    }

    #[test]
    fn inline_is_dropped_names_the_two_notations_a_heading_bars() {
        assert!(inline_is_dropped(NodeKind::HeadingHint, false));
        assert!(inline_is_dropped(NodeKind::Directive, true));
        assert!(!inline_is_dropped(NodeKind::Directive, false));
        assert!(!inline_is_dropped(NodeKind::Ruby, true));
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

    /// The load-bearing assertion of this crate's design: every construct
    /// slices back to the notation the author wrote for it.
    #[test]
    fn every_construct_slices_back_to_its_notation() {
        for src in TILED {
            with_constructs(src, |constructs| {
                for entry in &constructs.entries {
                    assert!(
                        !entry.literal.is_empty(),
                        "every construct must slice back to its notation for {src:?}"
                    );
                    let span = entry.span.expect("an unrewritten source publishes ranges");
                    assert_eq!(
                        src.get(span.start as usize..span.end as usize),
                        Some(entry.literal.as_str()),
                        "the range must address the caller's own text for {src:?}"
                    );
                }
            });
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
                    first.literal, expected,
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
            assert_eq!(
                SRC.get(span.start as usize..span.end as usize),
                Some("｜青梅《おうめ》")
            );
            assert_eq!(constructs.text(), "前\u{E001}後");
        });
    }

    /// A CRLF or BOM-prefixed document is canonicalised by the parser
    /// before it is read. comrak sees the canonical text, and the ranges
    /// still address what the caller holds.
    #[test]
    fn a_canonicalised_document_keeps_the_callers_ranges() {
        for raw in [
            "前\r\n\r\n`｜青梅《おうめ》`へ",
            "\u{feff}前\n\n`｜青梅《おうめ》`へ",
        ] {
            with_constructs(raw, |constructs| {
                assert!(
                    !constructs.text().contains('\r'),
                    "comrak parses the canonical text: {:?}",
                    constructs.text()
                );
                let mut cursor = constructs.cursor();
                let Some(ConstructHit {
                    span: Some(span), ..
                }) = cursor.next()
                else {
                    panic!("the ruby must be tracked with a range in {raw:?}");
                };
                assert_eq!(
                    raw.get(span.start as usize..span.end as usize),
                    Some("｜青梅《おうめ》"),
                    "the range must address the caller's own text for {raw:?}"
                );
                let mut cursor = constructs.cursor();
                assert_eq!(cursor.next_literal(), Some("｜青梅《おうめ》"));
            });
        }
    }

    /// A decorative rule is isolated by the parser, so comrak reads it as a
    /// rule rather than as the underline of a setext heading — and the
    /// notation after it is still tracked.
    #[test]
    fn a_decorative_rule_keeps_its_own_line() {
        const RAW: &str = "本文\n----------\n彼は`｜青梅《おうめ》`へ";
        with_constructs(RAW, |constructs| {
            assert_eq!(
                constructs.entries.len(),
                1,
                "the ruby is still tracked: {constructs:?}"
            );
            assert!(
                constructs.text().contains(INLINE_SENTINEL),
                "the canonical text drives comrak: {:?}",
                constructs.text()
            );
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next_literal(), Some("｜青梅《おうめ》"));
        });
    }

    /// A document the parser rewrote can still hold two constructs of the
    /// same shape and the same byte length — the norm for CJK notation of
    /// equal character count — and each literal context must get its own.
    #[test]
    fn a_canonicalised_document_tells_two_constructs_of_the_same_shape_apart() {
        const RAW: &str = "本文\r\n----------\r\n`｜A《a》`と`｜B《b》`";
        with_constructs(RAW, |constructs| {
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next_literal(), Some("｜A《a》"));
            assert_eq!(cursor.next_literal(), Some("｜B《b》"));
            assert!(cursor.next_literal().is_none());
        });
    }

    /// The three shapes the parser reports as more than one node, and the
    /// one run each has to be folded into. Without the fold, the notation
    /// between the two nodes reaches neither of them.
    #[test]
    fn a_construct_that_reaches_past_its_node_is_folded_into_one() {
        for (src, expected) in [
            // An inline bracket pair: the body belongs to the wrapper.
            (
                "［＃割り注］うえした［＃割り注終わり］",
                vec![(
                    NodeKind::Directive,
                    "［＃割り注］うえした［＃割り注終わり］",
                )],
            ),
            // A forward reference whose target is not adjacent: the
            // referenced text belongs to the directive that names it.
            (
                "可哀想な人［＃「可哀想」に傍点］",
                vec![(NodeKind::Bouten, "可哀想な人［＃「可哀想」に傍点］")],
            ),
            (
                "この行は［＃「この行」はゴシック体］",
                vec![(NodeKind::Emphasis, "この行は［＃「この行」はゴシック体］")],
            ),
            // A paired heading: the body is phrasing content, so it cannot
            // be a container comrak fills.
            (
                "［＃中見出し］見出し［＃中見出し終わり］",
                vec![(
                    NodeKind::Heading,
                    "［＃中見出し］見出し［＃中見出し終わり］",
                )],
            ),
            // …including across a line break, which the other folds stop at.
            (
                "［＃中見出し］一行目\n二行目［＃中見出し終わり］",
                vec![(
                    NodeKind::Heading,
                    "［＃中見出し］一行目\n二行目［＃中見出し終わり］",
                )],
            ),
        ] {
            with_constructs(src, |constructs| {
                let folded: Vec<(NodeKind, &str)> = constructs
                    .entries
                    .iter()
                    .map(|entry| (entry.kind, entry.literal.as_str()))
                    .collect();
                assert_eq!(folded, expected, "fold of {src:?}");
            });
        }
    }

    /// What the fold must leave alone. A construct whose notation is the
    /// whole of it renders on its own, and folding it would hand comrak's
    /// text to the parser for no gain.
    #[test]
    fn a_self_contained_construct_is_left_on_its_own() {
        for (src, expected) in [
            // Two ruby in a row are two constructs, not one.
            (
                "｜A《a》｜B《b》",
                vec![(NodeKind::Ruby, "｜A《a》"), (NodeKind::Ruby, "｜B《b》")],
            ),
            // A bracket pair whose halves each render on their own stays
            // two constructs, so the text between them stays comrak's.
            (
                "［＃縦中横］20［＃縦中横終わり］",
                vec![
                    (NodeKind::Directive, "［＃縦中横］"),
                    (NodeKind::Directive, "［＃縦中横終わり］"),
                ],
            ),
            // A forward reference already adjacent to its target is one
            // node to begin with.
            (
                "可哀想［＃「可哀想」に傍点］",
                vec![(NodeKind::Bouten, "可哀想［＃「可哀想」に傍点］")],
            ),
        ] {
            with_constructs(src, |constructs| {
                let folded: Vec<(NodeKind, &str)> = constructs
                    .entries
                    .iter()
                    .map(|entry| (entry.kind, entry.literal.as_str()))
                    .collect();
                assert_eq!(folded, expected, "fold of {src:?}");
            });
        }
    }

    /// A container whose body is comrak's — anything but a heading — keeps
    /// its two markers, so markdown written inside it is still markdown.
    #[test]
    fn a_block_container_keeps_its_two_markers() {
        const SRC: &str = "［＃ここから２字下げ］\n\n- a\n\n［＃ここで字下げ終わり］";
        with_constructs(SRC, |constructs| {
            let kinds: Vec<NodeKind> = constructs.entries.iter().map(|e| e.kind).collect();
            assert_eq!(
                kinds,
                vec![NodeKind::ContainerOpen, NodeKind::ContainerClose],
                "a container comrak fills must keep both markers"
            );
        });
    }

    #[test]
    fn cursor_consumes_in_order() {
        const SRC: &str = "｜A《a》\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］";
        with_constructs(SRC, |constructs| {
            let mut cursor = constructs.cursor();
            let Some(hit) = cursor.next() else {
                panic!("the ruby must be tracked");
            };
            assert_eq!(hit.kind, NodeKind::Ruby);
            assert!(hit.span.is_some());
            assert!(format!("{hit:?}").contains("ConstructHit"));
            assert_eq!(cursor.index(), 1);
            assert_eq!(
                cursor.next().map(|hit| hit.kind),
                Some(NodeKind::ContainerOpen)
            );
            cursor.advance(99); // saturating
            assert!(cursor.next().is_none());
            assert!(cursor.next_literal().is_none());
        });
    }

    #[test]
    fn cursor_yields_each_constructs_literal_in_order() {
        const SRC: &str = "｜A《a》と｜B《b》";
        with_constructs(SRC, |constructs| {
            let mut cursor = constructs.cursor();
            assert_eq!(cursor.next_literal(), Some("｜A《a》"));
            assert_eq!(cursor.next_literal(), Some("｜B《b》"));
            assert!(cursor.next_literal().is_none());
        });
    }

    #[test]
    fn markdown_only_table_is_empty() {
        let constructs = Constructs::none();
        assert!(constructs.text().is_empty());
        assert!(constructs.cursor().next().is_none());
        assert!(constructs.diagnostics().is_empty());
    }

    /// A range that does not address the text it was measured against
    /// drops its construct rather than panicking, and the render says how
    /// many it dropped.
    #[test]
    fn tile_drops_ranges_that_do_not_address_the_text() {
        let node = |start: u32, end: u32| {
            let run = Span { start, end };
            Node {
                kind: NodeKind::Ruby,
                run,
                span: Some(run),
            }
        };
        for (nodes, why) in [
            (vec![node(0, 99)], "out of bounds"),
            (vec![node(3, 1)], "inverted"),
            (vec![node(1, 3)], "mid-codepoint"),
            (vec![node(3, 6), node(0, 3)], "out of order"),
        ] {
            let table = Constructs::tile("前後", &nodes, Vec::new(), Runs::default());
            assert!(
                table.entries.len() < nodes.len(),
                "{why} must drop a construct: {table:?}"
            );
            assert!(
                table
                    .diagnostics()
                    .iter()
                    .any(|d| d.code == "aozora-md::constructs_unresolved"),
                "{why} must be reported: {table:?}"
            );
        }
        // A range that fits keeps its construct and reports nothing.
        let table = Constructs::tile("前後", &[node(0, 3)], Vec::new(), Runs::default());
        assert_eq!(table.entries.len(), 1);
        assert_eq!(table.text(), "\u{E001}後");
        assert!(table.diagnostics().is_empty());
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

    #[test]
    fn a_heading_hint_reports_its_level_and_title() {
        with_constructs(
            "第一篇［＃「第一篇」は大見出し］",
            |constructs| {
                assert_eq!(
                    constructs.heading_hint_of(0),
                    Some(HeadingHint {
                        level: 1,
                        target: "第一篇".to_owned(),
                    })
                );
            },
        );
        // Only a hint answers; a ruby is not one.
        with_constructs("｜青梅《おうめ》", |constructs| {
            assert_eq!(constructs.heading_hint_of(0), None);
            assert_eq!(constructs.heading_hint_of(9), None);
        });
    }

    /// A hint whose title is its own visible text carries no `data-target`:
    /// it sits mid-line, so there is no paragraph to promote.
    #[test]
    fn a_self_contained_heading_hint_promotes_nothing() {
        assert_eq!(
            parse_heading_hint(r#"<span class="x" data-level="3">見出し</span>"#),
            None
        );
        assert_eq!(parse_heading_hint("<span></span>"), None);
        assert_eq!(parse_heading_hint(r#"<span data-level="x"></span>"#), None);
        assert_eq!(parse_heading_hint(r#"<span data-level="1""#), None);
    }

    #[test]
    fn a_heading_hints_title_is_read_back_from_its_escaped_form() {
        assert_eq!(
            parse_heading_hint(
                r#"<span data-level="2" data-target="&lt;a&gt; &amp; &quot;b&quot; &#x27;c&#39; &unknown;" hidden></span>"#
            ),
            Some(HeadingHint {
                level: 2,
                target: "<a> & \"b\" 'c' &unknown;".to_owned(),
            })
        );
    }
}
