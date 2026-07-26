//! Aozora Flavored Markdown — CommonMark + GFM + 青空文庫記法.
//!
//! ```
//! use aozora_flavored_markdown::{Options, render};
//!
//! let rendered = render("彼は｜青梅《おうめ》に行った。", &Options::default());
//! assert!(rendered.html.contains("<ruby>"));
//! ```
//!
//! The pipeline substitutes one PUA sentinel per 青空文庫 construct
//! (`constructs`), lets a *verbatim* comrak parse the result as vanilla
//! CommonMark + GFM, then splices each sentinel back into the AST as a
//! rendered fragment (`ast_splice`) before formatting. comrak is an unpatched
//! crates.io dependency (ADR-0024), and the boundary with the sibling `aozora`
//! parser is its public API only (ADR-0021).

#![forbid(unsafe_code)]

// Compile every fenced `rust` block in README.md as a doctest (run by
// `just test-doc`) so the published quick-start can't drift from the API —
// the drift this guards against actually happened once. `#[cfg(doctest)]`
// keeps the `include_str!` out of normal builds and `cargo package`.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeDoctests;

mod ast_splice;
mod classes;
mod code_block_mask;
// The CommonMark / GFM spec runners. Inside `src/` rather than `tests/`
// because the spec's expected output is raw HTML, which only a `#[cfg(test)]`
// constructor turns on — an integration test is a separate crate and would
// need that switch to be public. Coverage counts them here too.
#[cfg(test)]
mod conformance;
mod constructs;
pub mod diagnostics;
mod fragment;
pub mod html;
pub mod ir;
mod source_line_anchors;
#[cfg(feature = "theme")]
pub mod theme;
mod verbatim_regions;

/// PUA codepoints this crate substitutes into the source before comrak parses.
///
/// Owned here rather than re-exported from the sibling parser: the
/// substitution is ours to make, so the constants are ours to keep stable.
pub mod sentinels {
    use crate::{code_block_mask, constructs};

    /// Ruby / bouten / annotation / gaiji / TCY / kaeriten.
    pub const INLINE: char = constructs::INLINE_SENTINEL;
    /// Page break, section break, leaf indent, sashie.
    pub const BLOCK_LEAF: char = constructs::BLOCK_LEAF_SENTINEL;
    /// Paired-container open line (e.g. `［＃ここから字下げ］`).
    pub const BLOCK_OPEN: char = constructs::BLOCK_OPEN_SENTINEL;
    /// Paired-container close line (e.g. `［＃ここで字下げ終わり］`).
    pub const BLOCK_CLOSE: char = constructs::BLOCK_CLOSE_SENTINEL;
    /// A 青空文庫 trigger, or a whole region, hidden from the lexer.
    pub const MASK: char = code_block_mask::MASK_CHAR;

    /// Read by the leak checks instead of re-listing codepoints, so a
    /// sentinel added later is covered without editing the checker.
    ///
    /// ```
    /// use aozora_flavored_markdown::sentinels;
    ///
    /// assert!(sentinels::ALL.contains(&sentinels::INLINE));
    /// assert!(sentinels::ALL.iter().all(|c| ('\u{E000}'..='\u{F8FF}').contains(c)));
    /// ```
    pub const ALL: [char; 5] = [INLINE, BLOCK_LEAF, BLOCK_OPEN, BLOCK_CLOSE, MASK];
}

#[doc(inline)]
pub use classes::{AOZORA_MD_CLASSES, is_contract_class};
#[doc(inline)]
pub use diagnostics::{Diagnostic, DiagnosticSource, Severity, Span};

use core::mem;

use comrak::nodes::AstNode;

use crate::constructs::Constructs;

/// Parse-time configuration for [`render`] and friends.
///
/// Every knob is one this crate implements in **both** of its outputs, HTML
/// and IR. comrak's remaining ones stay off the surface because they are not:
/// footnotes and description lists reach the HTML and drop out of the IR of
/// the same call, and `render.sourcepos` collides with
/// [`Options::with_source_line_anchors`]. Raw-HTML passthrough has no setter
/// at all, so no configuration reachable from outside this crate can turn a
/// render into an XSS sink.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Options {
    aozora: bool,
    hardbreaks: bool,
    smart_punctuation: bool,
    cjk_friendly_emphasis: bool,
    source_line_anchors: bool,
    tables: bool,
    strikethrough: bool,
    autolinks: bool,
    task_lists: bool,
    // Raw HTML, and the GFM filter that only bites when raw HTML is passing
    // through, exist for the conformance runners alone. The fields are not
    // compiled into a released build, so there is nothing for a public setter
    // to reach even by accident.
    #[cfg(test)]
    raw_html: bool,
    #[cfg(test)]
    tagfilter: bool,
}

impl Default for Options {
    /// The Aozora dialect — see [`Options::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    /// GFM extensions and 青空文庫 notation on, plus hardbreaks so each
    /// source newline becomes a `<br>` — verse and dialogue boundaries are
    /// load-bearing in 青空文庫 source.
    ///
    /// ```
    /// use aozora_flavored_markdown::Options;
    ///
    /// let opts = Options::new();
    /// assert_eq!(opts, Options::default());
    /// assert_ne!(opts, Options::commonmark());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            aozora: true,
            hardbreaks: true,
            smart_punctuation: false,
            cjk_friendly_emphasis: true,
            source_line_anchors: false,
            tables: true,
            strikethrough: true,
            autolinks: true,
            task_lists: true,
            #[cfg(test)]
            raw_html: false,
            #[cfg(test)]
            tagfilter: false,
        }
    }

    /// Plain CommonMark 0.31.2: no GFM extension, no notation, no hardbreaks
    /// — what the spec's 652 examples are rendered with, so this crate's
    /// superset claim is checked rather than asserted.
    #[must_use]
    pub fn commonmark() -> Self {
        Self {
            aozora: false,
            hardbreaks: false,
            cjk_friendly_emphasis: false,
            tables: false,
            strikethrough: false,
            autolinks: false,
            task_lists: false,
            ..Self::new()
        }
    }

    /// [`Options::commonmark`] plus the four GFM extensions.
    #[must_use]
    pub fn gfm() -> Self {
        Self {
            tables: true,
            strikethrough: true,
            autolinks: true,
            task_lists: true,
            ..Self::commonmark()
        }
    }

    /// Recognise 青空文庫 notation. Off, the source flows straight through
    /// comrak with no lexer pass and no HTML post-processing.
    #[must_use]
    pub fn with_aozora(mut self, on: bool) -> Self {
        self.aozora = on;
        self
    }

    /// Turn each source newline into a `<br>`.
    #[must_use]
    pub fn with_hardbreaks(mut self, on: bool) -> Self {
        self.hardbreaks = on;
        self
    }

    /// Rewrite ASCII quotes, dashes and ellipses to their typographic forms.
    #[must_use]
    pub fn with_smart_punctuation(mut self, on: bool) -> Self {
        self.smart_punctuation = on;
        self
    }

    /// Let emphasis open and close against CJK punctuation, which
    /// CommonMark's flanking rules on their own refuse.
    #[must_use]
    pub fn with_cjk_friendly_emphasis(mut self, on: bool) -> Self {
        self.cjk_friendly_emphasis = on;
        self
    }

    /// Tag every top-level block with `data-aozora-md-source-line="N"`
    /// (1-based), so a host can map a rendered fragment back to a slice of
    /// the source without re-parsing. Costs one extra AST walk plus a
    /// streaming insert, both O(blocks).
    #[must_use]
    pub fn with_source_line_anchors(mut self, on: bool) -> Self {
        self.source_line_anchors = on;
        self
    }

    /// GFM tables.
    #[must_use]
    pub fn with_tables(mut self, on: bool) -> Self {
        self.tables = on;
        self
    }

    /// GFM `~~strikethrough~~`.
    #[must_use]
    pub fn with_strikethrough(mut self, on: bool) -> Self {
        self.strikethrough = on;
        self
    }

    /// GFM bare-URL autolinking.
    #[must_use]
    pub fn with_autolinks(mut self, on: bool) -> Self {
        self.autolinks = on;
        self
    }

    /// GFM `- [ ]` task list items.
    #[must_use]
    pub fn with_task_lists(mut self, on: bool) -> Self {
        self.task_lists = on;
        self
    }

    /// Built per call and never stored: comrak's own options type is neither
    /// comparable nor hashable, and holding one would pull its field set —
    /// pre-1.0, and far wider than this one — into ours.
    fn comrak(&self) -> comrak::Options<'static> {
        let mut comrak = comrak::Options::default();
        comrak.extension.strikethrough = self.strikethrough;
        comrak.extension.table = self.tables;
        comrak.extension.autolink = self.autolinks;
        comrak.extension.tasklist = self.task_lists;
        comrak.extension.cjk_friendly_emphasis = self.cjk_friendly_emphasis;
        comrak.parse.smart = self.smart_punctuation;
        comrak.render.hardbreaks = self.hardbreaks;
        // Rebound rather than mutated under a `#[cfg]` block, which no
        // spelling of satisfies `semicolon_outside_block` and
        // `semicolon_if_nothing_returned` at once.
        #[cfg(test)]
        let comrak = {
            let mut comrak = comrak;
            comrak.extension.tagfilter = self.tagfilter;
            comrak.render.r#unsafe = self.raw_html;
            comrak
        };
        comrak
    }
}

#[cfg(test)]
impl Options {
    // The conformance baseline: [`Options::commonmark`] with raw HTML passing
    // through, because the spec's expected output contains it. Never
    // reachable from outside the crate — that is the point of the `cfg`.
    pub(crate) fn spec_commonmark() -> Self {
        Self {
            raw_html: true,
            ..Self::commonmark()
        }
    }

    // GFM's disallowed-raw-html filter. Only observable with raw HTML on, so
    // it belongs to the runner rather than the public surface.
    pub(crate) fn with_tagfilter(mut self, on: bool) -> Self {
        self.tagfilter = on;
        self
    }
}

/// Output of [`render`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendered {
    /// HTML output, with every Aozora sentinel substituted.
    pub html: String,
    /// Non-fatal lexer observations. Empty on the happy path.
    pub diagnostics: Vec<Diagnostic>,
}

/// Output of [`render_to_ir`].
///
/// The IR lets the wasm bridge's JS renderer pick its own output target (DOM
/// fragment, `CodeMirror` `RangeSet`, semantic tokens, …) from one call.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderedIr {
    pub ir: ir::Document,
    pub html: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Largest source this crate will hand to the sibling parser.
///
/// That parser keys spans on `u32` byte offsets and asserts
/// `source.len() <= u32::MAX` on the way in. Under this workspace's
/// `panic = "abort"` profile that assert is a hard process abort — an
/// in-scope crash per `SECURITY.md` for a hostile 4 GiB input. Every public
/// entry point guards on the boundary *first* and degrades to an empty
/// render instead.
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// Split out from [`source_within_span_budget`] so the boundary arithmetic
/// is testable at `u32::MAX` / `u32::MAX + 1` without allocating gigabytes.
const fn len_within_span_budget(len: usize) -> bool {
    len <= MAX_SOURCE_BYTES
}

const fn source_within_span_budget(input: &str) -> bool {
    len_within_span_budget(input.len())
}

/// Render aozora-flavored-markdown source text to HTML.
///
/// # Examples
///
/// ```
/// use aozora_flavored_markdown::{Options, render};
///
/// let rendered = render("彼は｜青梅《おうめ》に行った。", &Options::default());
/// assert!(rendered.html.contains("<ruby>"));
/// assert!(rendered.diagnostics.is_empty());
/// ```
///
/// Input past `MAX_SOURCE_BYTES` yields an empty `html` and one
/// `source_too_large` diagnostic rather than reaching the lexer.
///
/// # Panics
///
/// Never in practice: `String` cannot fail as a `fmt::Write` sink.
#[must_use]
pub fn render(input: &str, options: &Options) -> Rendered {
    if !source_within_span_budget(input) {
        return Rendered {
            html: String::new(),
            diagnostics: vec![Diagnostic::source_too_large(input.len())],
        };
    }
    let (html, diagnostics, ()) = drive_pipeline(input, options, |_root, _constructs| ());
    Rendered { html, diagnostics }
}

/// Render aozora-flavored-markdown source to a structured IR + HTML + diagnostics.
///
/// Notation that changes the document's *shape* rather than its content is
/// reflected in the IR structure, not as an `Aozora` node: a heading hint
/// (`［＃「X」は大見出し］`) promotes its host paragraph to
/// `Block::Heading`, and an annotation inside a heading body drops out.
/// Both mirror the HTML renderer, so one call's IR and HTML describe the
/// same document.
///
/// # Examples
///
/// ```
/// use aozora_flavored_markdown::ir::Block;
/// use aozora_flavored_markdown::{Options, render_to_ir};
///
/// let rendered = render_to_ir("# 第一章\n\n本文", &Options::default());
/// assert!(matches!(rendered.ir.blocks.first(), Some(Block::Heading { .. })));
/// ```
///
/// Oversized input degrades as in [`render`].
///
/// # Panics
///
/// Never in practice: `String` cannot fail as a `fmt::Write` sink.
#[must_use]
pub fn render_to_ir(input: &str, options: &Options) -> RenderedIr {
    if !source_within_span_budget(input) {
        return RenderedIr {
            ir: ir::Document::default(),
            html: String::new(),
            diagnostics: vec![Diagnostic::source_too_large(input.len())],
        };
    }
    let (html, diagnostics, ir) = drive_pipeline(input, options, ir::build_ir);
    RenderedIr {
        ir,
        html,
        diagnostics,
    }
}

/// `project` runs against the AST *before* splicing, so an IR walker sees
/// the same sentinel-bearing tree the splicer is about to consume.
fn drive_pipeline<F, T>(input: &str, options: &Options, project: F) -> (String, Vec<Diagnostic>, T)
where
    F: for<'a> FnOnce(&'a AstNode<'a>, &Constructs) -> T,
{
    let comrak_options = options.comrak();
    if !options.aozora {
        let comrak_arena = comrak::Arena::new();
        let root = comrak::parse_document(&comrak_arena, input, &comrak_options);
        // No lexer pass, so no constructs and no sentinels: the input goes
        // to comrak as the caller wrote it.
        let extra = project(root, &Constructs::none());
        let html = format_root(root, &comrak_options, options.source_line_anchors, None);
        return (html, Vec::new(), extra);
    }

    // Pre-process: hide aozora trigger characters that live inside a
    // CommonMark fenced code block from the lexer, which is
    // CommonMark-blind by design (ADR-0010), so this lives here. See
    // `code_block_mask` module docs for the masking scheme.
    let (masked_source, mask_originals) = code_block_mask::mask_code_block_triggers(input);

    // A render parses the document once and then each of its constructs
    // again, on its own, to learn what that construct renders to. Building
    // the parser's process-global tables here keeps that cost off the first
    // of those parses; the call is idempotent and free once warm.
    aozora::prewarm();

    // Substitute one sentinel per construct, in source coordinates. The
    // masked source is the single coordinate space from here on: it is
    // char-for-char the caller's input, so a construct's byte range is one
    // the caller can slice (see `crate::constructs`).
    let constructs = Constructs::build(&masked_source);

    let comrak_arena = comrak::Arena::new();
    let root = comrak::parse_document(&comrak_arena, constructs.text(), &comrak_options);

    // Both walkers cursor over the same construct table, each with its own
    // cursor, so they stay in lockstep without serial coupling.
    let extra = project(root, &constructs);

    ast_splice::splice_into_ast(root, &comrak_arena, &constructs);

    let html = format_root(
        root,
        &comrak_options,
        options.source_line_anchors,
        Some(mask_originals.as_slice()),
    );
    (html, constructs.diagnostics().to_vec(), extra)
}

/// Formats per top-level child when `anchors` is on, so each child's first
/// open tag can pick up its `data-aozora-md-source-line`.
fn format_root<'a>(
    root: &'a AstNode<'a>,
    comrak: &comrak::Options<'static>,
    anchors: bool,
    mask_originals: Option<&[char]>,
) -> String {
    let html = if anchors {
        source_line_anchors::format_root_with_anchors(root, comrak)
    } else {
        let mut html = String::new();
        comrak::format_html(root, comrak, &mut html).expect("formatting to a String never fails");
        html
    };
    if let Some(originals) = mask_originals {
        code_block_mask::unmask(&html, originals).into_owned()
    } else {
        html
    }
}

/// One block of [`render_blocks_to_ir`]'s output.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderedBlock {
    /// Usually one block; empty for comrak constructs the IR does not model
    /// (definition lists, footnote refs, raw HTML, …).
    pub ir: Vec<ir::Block>,
    pub html: String,
    /// 1-based line where this block began in the source.
    pub source_line: u32,
}

/// Per-block streaming render, one [`RenderedBlock`] per top-level comrak
/// child in document order.
///
/// Serves the obsidian chunked-cancellation path (ADR-0009): the JS bridge
/// checks its `AbortSignal` between blocks. Diagnostics come back attached
/// to the document rather than per-block, because the lexer pass is not
/// block-scoped.
///
/// A paired container spanning several blocks emits its open and close
/// markers in the blocks they appear in, and one the source never closes is
/// drained into a trailing block matching the closing tag the HTML side
/// appends — so concatenating either output leaves nothing hanging open.
///
/// # Examples
///
/// ```
/// use aozora_flavored_markdown::{Options, render_blocks_to_ir};
///
/// let (blocks, diagnostics) =
///     render_blocks_to_ir("first paragraph\n\n｜second《せかんど》paragraph", &Options::default());
/// assert_eq!(blocks.len(), 2);
/// assert!(diagnostics.is_empty());
/// ```
///
/// Oversized input degrades as in [`render`].
#[must_use]
pub fn render_blocks_to_ir(
    input: &str,
    options: &Options,
) -> (Vec<RenderedBlock>, Vec<Diagnostic>) {
    if !source_within_span_budget(input) {
        return (Vec::new(), vec![Diagnostic::source_too_large(input.len())]);
    }
    if !options.aozora {
        let comrak_arena = comrak::Arena::new();
        let root = comrak::parse_document(&comrak_arena, input, &options.comrak());
        let blocks = collect_rendered_blocks(root, options, Vec::new(), &[]);
        return (blocks, Vec::new());
    }

    let (masked_source, mask_originals) = code_block_mask::mask_code_block_triggers(input);
    aozora::prewarm();
    // The builder owns the construct table; the splice below borrows the
    // same one, so both outputs of this call describe the same document.
    let mut builder = ir::StreamingIrBuilder::new(&masked_source);
    let comrak_arena = comrak::Arena::new();
    let root = comrak::parse_document(
        &comrak_arena,
        builder.constructs().text(),
        &options.comrak(),
    );
    // IR projection runs before AST mutation so it walks the
    // sentinel-bearing Text nodes; AST splicing afterwards rewrites
    // the same nodes for `comrak::format_html` consumption. A single
    // `StreamingIrBuilder` threads its cursor across every top-level
    // child so the construct stream stays in lockstep — a per-call builder
    // would restart the cursor at 0 for every block and misalign
    // Aozora projection against the table.
    let mut blocks_ir: Vec<Vec<ir::Block>> = root
        .children()
        .map(|child| builder.walk_block(child))
        .collect();
    ast_splice::splice_into_ast(root, &comrak_arena, builder.constructs());
    // Read while the builder still owns the table.
    let diagnostics = builder.constructs().diagnostics().to_vec();
    // End-of-document drain. The splicer appends one synthesised close
    // per still-open container as a fresh top-level child, so each one
    // becomes its own `RenderedBlock`; giving the drain the same shape
    // here keeps `ir` and `html` describing the same block.
    blocks_ir.extend(builder.finish().into_iter().map(|block| vec![block]));
    let blocks = collect_rendered_blocks(root, options, blocks_ir, &mask_originals);
    (blocks, diagnostics)
}

fn collect_rendered_blocks<'a>(
    root: &'a AstNode<'a>,
    options: &Options,
    mut blocks_ir: Vec<Vec<ir::Block>>,
    mask_originals: &[char],
) -> Vec<RenderedBlock> {
    // The AST has already been spliced at the document level by the
    // caller (so `format_html` sees no sentinels here), and the IR
    // was already projected from the *pre-splice* AST in source
    // order. We zip them back together one block at a time.
    //
    // Pure-markdown mode (`Options::aozora_enabled = false`) hands
    // us an empty IR vector; we emit `Vec::new()` per block in that
    // case so the per-block IR field stays consistent with the IR
    // builder's no-op behaviour.
    //
    // Masks are restored with a cursor rather than the one pass the
    // document path makes: handing every block the whole slice would replay
    // block 1's originals into block 2.
    let comrak_options = options.comrak();
    let mut blocks = Vec::new();
    let mut mask_cursor = mask_originals;
    for (idx, child) in root.children().enumerate() {
        let data = child.data.borrow();
        let line = constructs::saturating_u32(data.sourcepos.start.line).max(1);
        drop(data);
        let rendered = if options.source_line_anchors {
            source_line_anchors::format_block_with_anchor(child, &comrak_options)
        } else {
            let mut buf = String::new();
            comrak::format_html(child, &comrak_options, &mut buf)
                .expect("formatting a String never fails");
            buf
        };
        let block_html = if mask_cursor.is_empty() {
            rendered
        } else {
            code_block_mask::unmask_from(&rendered, &mut mask_cursor).into_owned()
        };
        let ir_blocks = if idx < blocks_ir.len() {
            mem::take(&mut blocks_ir[idx])
        } else {
            Vec::new()
        };
        blocks.push(RenderedBlock {
            ir: ir_blocks,
            html: block_html,
            source_line: line,
        });
    }
    blocks
}

/// Round-trip source through the parser back to canonical
/// aozora-md-source text.
///
/// Canonicalising, not merely inverse: notation written in a longer form
/// comes back in the shortest spelling that reads the same (below, the
/// ruby's explicit base marker is dropped because the base is unambiguous
/// without it), and the output is a fixed point.
///
/// Only prose is canonicalised: code (fenced, indented, a span), raw HTML, a
/// rule row and a codepoint this crate reserves come back as written, at any
/// container depth. Plain CommonMark therefore passes through verbatim, up to
/// what CommonMark does not itself distinguish and the parser normalises
/// document-wide — CRLF becomes LF, a leading BOM goes, blank lines collapse.
///
/// # Examples
///
/// ```
/// use aozora_flavored_markdown::serialize;
///
/// let canonical = serialize("彼は｜青梅《おうめ》に行った。");
/// assert_eq!(canonical, "彼は青梅《おうめ》に行った。");
/// assert_eq!(serialize(&canonical), canonical);
/// ```
///
/// Past `MAX_SOURCE_BYTES` this returns an empty `String`, so the round-trip
/// is *not* identity there — but such input cannot be lexed at all.
#[must_use]
pub fn serialize(input: &str) -> String {
    if !source_within_span_budget(input) {
        return String::new();
    }
    let Some(mut current) = canonicalise_pass(input) else {
        return String::new();
    };
    for _ in 1..MAX_CANONICAL_PASSES {
        let Some(next) = canonicalise_pass(&current) else {
            return String::new();
        };
        if next == current {
            return current;
        }
        current = next;
    }
    input.to_owned()
}

// A pass reads block structure to decide what to protect and can insert a
// blank line that changes that structure for the next one, so settling is
// checked rather than assumed — every shape seen settles in two. Handing the
// source back is the one answer that stays a fixed point whatever the passes
// were doing, which is why the budget can be small.
const MAX_CANONICAL_PASSES: usize = 4;

// One pass: lift out what comrak claims, canonicalise the prose between,
// splice the originals back; `None` when the source does not lex at all.
// Lifted whole rather than masked character by character as in
// `drive_pipeline`, which has a byte span to keep aligned and this has not —
// so here a region leaves the lexer's sight entirely (`verbatim_regions`).
fn canonicalise_pass(source: &str) -> Option<String> {
    let (protected, originals) = verbatim_regions::protect(source);
    let document = aozora::parse(protected).ok()?;
    let canonical = document.snapshot().to_source();
    Some(verbatim_regions::restore(&canonical, &originals))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_round_trips_through_html() {
        let r = render("hello, world", &Options::default());
        assert!(r.html.contains("hello, world"), "html: {}", r.html);
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn plain_text_serialize_returns_input_unchanged() {
        assert_eq!(serialize("plain text"), "plain text");
    }

    #[test]
    fn fenced_notation_serializes_verbatim() {
        // Unmasked, the lexer canonicalises a fence body like prose and drops
        // the ruby's explicit base marker.
        let src = "```\n｜青梅《おうめ》\n```";
        assert_eq!(serialize(src), src);
    }

    #[test]
    fn serialize_restores_masks_in_source_order_across_fences() {
        // Two fences, different triggers, canonicalised prose between them:
        // a cursor that replayed or skipped would put a character back in the
        // wrong fence rather than lose one, which byte equality catches and a
        // per-fence containment check would not.
        let src = "```\n｜一《いち》\n```\n\n｜二《に》\n\n```\n［＃改ページ］\n```\n";
        assert_eq!(
            serialize(src),
            "```\n｜一《いち》\n```\n\n二《に》\n\n```\n［＃改ページ］\n```\n"
        );
    }

    #[test]
    fn ruby_renders_as_html_ruby_element() {
        let r = render("｜青梅《おうめ》へ", &Options::default());
        assert!(r.html.contains("<ruby>"), "html: {}", r.html);
        assert!(r.html.contains("青梅"));
        assert!(r.html.contains("おうめ"));
        // No bare ［＃ leak (Tier-A canary).
        assert!(!r.html.contains("［＃"));
    }

    #[test]
    fn page_break_promotes_and_does_not_leak_brackets() {
        let r = render("前［＃改ページ］後", &Options::default());
        assert!(!r.html.contains("［＃"), "html: {}", r.html);
    }

    #[test]
    fn unknown_annotation_keeps_brackets_inside_wrapper() {
        let r = render("前［＃ほげふが］後", &Options::default());
        // The annotation HTML carries the original text inside an
        // `aozora-md-directive` wrapper, so the bracket character may
        // appear, but never bare in body text.
        assert!(
            !contains_bare_bracket(&r.html),
            "bare bracket leaked in: {}",
            r.html
        );
    }

    #[test]
    fn commonmark_passes_through_with_heading_intact() {
        let r = render("# Hello\n\nworld", &Options::default());
        assert!(r.html.contains("<h1>Hello</h1>"), "html: {}", r.html);
        assert!(r.html.contains("world"));
    }

    #[test]
    fn gfm_options_have_aozora_disabled_and_gfm_extensions_enabled() {
        let opts = Options::gfm();
        assert!(!opts.aozora, "gfm must skip the aozora pass");
        let comrak = opts.comrak();
        assert!(comrak.extension.strikethrough);
        assert!(comrak.extension.table);
        assert!(comrak.extension.autolink);
        assert!(comrak.extension.tasklist);
        assert!(!comrak.render.r#unsafe, "gfm is safe on untrusted input");
    }

    #[test]
    fn every_builder_flips_exactly_the_knob_it_names() {
        // Each `with_*` is a one-bit edit, so equality against the base with
        // that one knob restored is the whole contract — and it is checkable
        // only because hiding comrak made `Options` comparable.
        let base = Options::default();
        for flipped in [
            base.clone().with_aozora(false),
            base.clone().with_hardbreaks(false),
            base.clone().with_smart_punctuation(true),
            base.clone().with_cjk_friendly_emphasis(false),
            base.clone().with_source_line_anchors(true),
            base.clone().with_tables(false),
            base.clone().with_strikethrough(false),
            base.clone().with_autolinks(false),
            base.clone().with_task_lists(false),
        ] {
            assert_ne!(flipped, base, "a builder that changed nothing");
        }
        assert_eq!(
            base.clone().with_tables(false).with_tables(true),
            base,
            "flipping a knob back must restore the value"
        );
    }

    #[test]
    fn gfm_renders_strikethrough_and_does_not_recognise_ruby() {
        // `gfm`'s contract: GFM extensions on, Aozora pre-pass off.
        // The strikethrough must produce `<del>`; the ruby-shaped
        // `｜...《》` source must survive verbatim because the lexer
        // never ran.
        let opts = Options::gfm();
        let html = render("~~strike~~ ｜青梅《おうめ》", &opts).html;
        assert!(html.contains("<del>strike</del>"), "html: {html}");
        assert!(
            html.contains("｜青梅"),
            "ruby trigger must survive raw: {html}"
        );
        assert!(
            !html.contains("<ruby>"),
            "ruby must NOT render in gfm-only: {html}"
        );
    }

    #[test]
    fn contains_bare_bracket_helper_detects_leaked_marker() {
        // Pins the "bare bracket leaked" branch of the helper itself.
        // The needle appears outside any tag and outside an
        // `aozora-md-directive` wrapper.
        assert!(contains_bare_bracket("plain ［＃ leak"));
        assert!(!contains_bare_bracket(
            "<span class=\"aozora-md-directive\" hidden>［＃</span>"
        ));
        assert!(!contains_bare_bracket("no marker at all"));
    }

    // -------------------------------------------------------------------
    // (a) The spec runners need raw-HTML passthrough, and nothing else may
    // have it. The constructor that supplies it is `#[cfg(test)]`, so these
    // two tests are the whole of what can reach `render.unsafe`.
    // -------------------------------------------------------------------

    #[test]
    fn spec_commonmark_enables_raw_html_and_disables_aozora() {
        let opts = Options::spec_commonmark();
        assert!(
            opts.comrak().render.r#unsafe,
            "spec_commonmark must enable raw-HTML passthrough for the runner"
        );
        assert!(!opts.aozora, "spec_commonmark must skip the aozora pass");
        assert!(
            opts.with_tagfilter(true).comrak().extension.tagfilter,
            "the runner's per-example tagfilter must reach comrak"
        );
    }

    #[test]
    fn no_public_constructor_enables_raw_html() {
        // The XSS-safety contract: raw HTML is reachable from the
        // `#[cfg(test)]` constructor above and from nowhere else.
        for opts in [Options::default(), Options::commonmark(), Options::gfm()] {
            assert!(
                !opts.comrak().render.r#unsafe,
                "{opts:?} must leave raw HTML escaped (no render.unsafe)"
            );
        }
        assert!(
            Options::default().aozora,
            "default must run the aozora pass"
        );
    }

    // -------------------------------------------------------------------
    // (b) Oversized-input boundary guard. The lexer asserts
    // `source.len() <= u32::MAX` and aborts under panic=abort; the aozora-flavored-markdown
    // entry points must degrade to an empty render instead. We cannot
    // allocate a >4 GiB string in a test, so the threshold arithmetic is
    // pinned on the pure `len_within_span_budget` helper, and the entry
    // points are exercised on realistic (in-budget) input.
    // -------------------------------------------------------------------

    #[test]
    fn len_budget_boundary_is_exactly_u32_max() {
        assert!(len_within_span_budget(0));
        assert!(len_within_span_budget(1024));
        assert!(
            len_within_span_budget(MAX_SOURCE_BYTES),
            "exactly u32::MAX bytes is still addressable"
        );
        // `checked_add` keeps the test sound on a hypothetical 32-bit
        // target where `MAX_SOURCE_BYTES == usize::MAX` and `+ 1` would
        // overflow; there, "one past the budget" is unrepresentable, so
        // the over-budget assertion is vacuously satisfied. On the
        // workspace's 64-bit targets `over` is `u32::MAX + 1`, the exact
        // value the core lexer's assert rejects.
        if let Some(over) = MAX_SOURCE_BYTES.checked_add(1) {
            assert!(
                !len_within_span_budget(over),
                "one byte past u32::MAX must be rejected"
            );
        }
    }

    #[test]
    fn in_budget_input_still_renders_normally() {
        // Guard must be transparent for ordinary input.
        let r = render("# hi\n\nbody", &Options::default());
        assert!(r.html.contains("<h1>hi</h1>"), "html: {}", r.html);
        let ir = render_to_ir("para", &Options::default());
        assert!(!ir.ir.blocks.is_empty());
        let (blocks, _) = render_blocks_to_ir("a\n\nb", &Options::default());
        assert_eq!(blocks.len(), 2);
        assert_eq!(serialize("plain"), "plain");
    }

    /// Tier-A canary: every occurrence of `［＃` must be inside an
    /// `aozora-md-directive` wrapper — never in raw body text.
    fn contains_bare_bracket(html: &str) -> bool {
        let needle = "［＃";
        let wrapper_open = "aozora-md-directive";
        let mut pos = 0;
        while let Some(idx) = html[pos..].find(needle) {
            let abs = pos + idx;
            let prefix = &html[..abs];
            let last_open = prefix.rfind('<').unwrap_or(0);
            let last_close = prefix.rfind('>').unwrap_or(0);
            let inside_tag = last_open > last_close;
            let in_wrapper = prefix.contains(wrapper_open);
            if !inside_tag && !in_wrapper {
                return true;
            }
            pos = abs + needle.len();
        }
        false
    }
}
