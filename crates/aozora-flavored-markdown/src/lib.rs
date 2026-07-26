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
/// `comrak::Options` is held `'static`: we install neither URL rewriters nor
/// broken-link callbacks — comrak's only non-`'static` fields — so a borrow
/// parameter would be dead weight in the public API.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Options {
    comrak: comrak::Options<'static>,
    aozora_enabled: bool,
    source_line_anchors: bool,
}

impl Default for Options {
    /// GFM extensions on, plus hardbreaks so each source newline becomes a
    /// `<br>` — verse and dialogue boundaries are load-bearing in 青空文庫
    /// source. Raw-HTML passthrough stays off, so this is XSS-safe on
    /// untrusted input.
    ///
    /// ```
    /// use aozora_flavored_markdown::Options;
    ///
    /// let opts = Options::default();
    /// assert!(opts.aozora_enabled());
    /// assert!(opts.comrak().extension.table);
    /// assert!(!opts.source_line_anchors());
    /// ```
    fn default() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.extension.strikethrough = true;
        comrak.extension.table = true;
        comrak.extension.autolink = true;
        comrak.extension.tasklist = true;
        comrak.render.hardbreaks = true;
        Self {
            comrak,
            aozora_enabled: true,
            source_line_anchors: false,
        }
    }
}

impl Options {
    /// Plain CommonMark, for the CommonMark 0.31.2 conformance runner.
    ///
    /// # Security
    ///
    /// **Never use on untrusted input.** The spec's expected output contains
    /// raw HTML, so this turns on comrak's passthrough
    /// (`render.unsafe = true`): raw HTML is emitted verbatim and URLs go
    /// unsanitized, `javascript:` included. That is an XSS sink, hence
    /// `#[doc(hidden)]`. Production callers want [`Options::default`].
    #[doc(hidden)]
    #[must_use]
    pub fn commonmark_only() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.render.r#unsafe = true;
        Self {
            comrak,
            aozora_enabled: false,
            source_line_anchors: false,
        }
    }

    /// Pure GFM, for the GFM 0.29 conformance runner.
    ///
    /// # Security
    ///
    /// Same raw-HTML XSS sink as [`Options::commonmark_only`], for the same
    /// reason. Never use on untrusted input.
    #[doc(hidden)]
    #[must_use]
    pub fn gfm_only() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.extension.strikethrough = true;
        comrak.extension.table = true;
        comrak.extension.autolink = true;
        comrak.extension.tasklist = true;
        comrak.extension.tagfilter = true;
        comrak.render.r#unsafe = true;
        Self {
            comrak,
            aozora_enabled: false,
            source_line_anchors: false,
        }
    }

    /// Tag every top-level block with `data-aozora-md-source-line="N"`
    /// (1-based). The obsidian adapter maps per-block post-processor calls
    /// back to slices of the rendered fragment without re-parsing. Off by
    /// default; costs one extra AST walk plus a streaming insert, both
    /// O(blocks).
    ///
    /// ```
    /// use aozora_flavored_markdown::Options;
    /// let opts = Options::default().with_source_line_anchors(true);
    /// assert!(opts.source_line_anchors());
    /// ```
    #[must_use]
    pub fn with_source_line_anchors(mut self, on: bool) -> Self {
        self.source_line_anchors = on;
        self
    }

    /// With `false`, the input flows straight through comrak with no Aozora
    /// lexing or HTML post-processing — how the spec-conformance runners
    /// check that this wrapper does not perturb upstream behaviour.
    ///
    /// ```
    /// use aozora_flavored_markdown::Options;
    /// let opts = Options::default().with_aozora_enabled(false);
    /// assert!(!opts.aozora_enabled());
    /// ```
    #[must_use]
    pub fn with_aozora_enabled(mut self, on: bool) -> Self {
        self.aozora_enabled = on;
        self
    }

    /// See [`Options::with_aozora_enabled`].
    #[must_use]
    pub fn aozora_enabled(&self) -> bool {
        self.aozora_enabled
    }

    /// See [`Options::with_source_line_anchors`].
    #[must_use]
    pub fn source_line_anchors(&self) -> bool {
        self.source_line_anchors
    }

    /// Inspect a comrak knob directly. The dialect is configured by
    /// [`Options::default`].
    #[must_use]
    pub fn comrak(&self) -> &comrak::Options<'static> {
        &self.comrak
    }

    /// Escape hatch for comrak tuning the `with_*` builders do not cover.
    ///
    /// # Stability
    ///
    /// Comrak's option surface is **not** covered by this crate's `SemVer`
    /// guarantee — a comrak major bump may change these fields.
    pub fn comrak_mut(&mut self) -> &mut comrak::Options<'static> {
        &mut self.comrak
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
    pub ir: ir::IrDocument,
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
/// `IrBlock::Heading`, and an annotation inside a heading body drops out.
/// Both mirror the HTML renderer, so one call's IR and HTML describe the
/// same document.
///
/// # Examples
///
/// ```
/// use aozora_flavored_markdown::ir::IrBlock;
/// use aozora_flavored_markdown::{Options, render_to_ir};
///
/// let rendered = render_to_ir("# 第一章\n\n本文", &Options::default());
/// assert!(matches!(rendered.ir.blocks.first(), Some(IrBlock::Heading { .. })));
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
            ir: ir::IrDocument::default(),
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
    if !options.aozora_enabled {
        let comrak_arena = comrak::Arena::new();
        let root = comrak::parse_document(&comrak_arena, input, &options.comrak);
        // No lexer pass, so no constructs and no sentinels: the input goes
        // to comrak as the caller wrote it.
        let extra = project(root, &Constructs::none());
        let html = format_root(root, options, None);
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
    let root = comrak::parse_document(&comrak_arena, constructs.text(), &options.comrak);

    // Both walkers cursor over the same construct table, each with its own
    // cursor, so they stay in lockstep without serial coupling.
    let extra = project(root, &constructs);

    ast_splice::splice_into_ast(root, &comrak_arena, &constructs);

    let html = format_root(root, options, Some(mask_originals.as_slice()));
    (html, constructs.diagnostics().to_vec(), extra)
}

/// Formats per top-level child when `source_line_anchors` is on, so each
/// child's first open tag can pick up its `data-aozora-md-source-line`.
fn format_root<'a>(
    root: &'a AstNode<'a>,
    options: &Options,
    mask_originals: Option<&[char]>,
) -> String {
    let html = if options.source_line_anchors {
        source_line_anchors::format_root_with_anchors(root, &options.comrak)
    } else {
        let mut html = String::new();
        comrak::format_html(root, &options.comrak, &mut html)
            .expect("formatting to a String never fails");
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
    pub ir: Vec<ir::IrBlock>,
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
    if !options.aozora_enabled {
        let comrak_arena = comrak::Arena::new();
        let root = comrak::parse_document(&comrak_arena, input, &options.comrak);
        let blocks = collect_rendered_blocks(root, options, Vec::new(), &[]);
        return (blocks, Vec::new());
    }

    let (masked_source, mask_originals) = code_block_mask::mask_code_block_triggers(input);
    aozora::prewarm();
    // The builder owns the construct table; the splice below borrows the
    // same one, so both outputs of this call describe the same document.
    let mut builder = ir::StreamingIrBuilder::new(&masked_source);
    let comrak_arena = comrak::Arena::new();
    let root = comrak::parse_document(&comrak_arena, builder.constructs().text(), &options.comrak);
    // IR projection runs before AST mutation so it walks the
    // sentinel-bearing Text nodes; AST splicing afterwards rewrites
    // the same nodes for `comrak::format_html` consumption. A single
    // `StreamingIrBuilder` threads its cursor across every top-level
    // child so the construct stream stays in lockstep — a per-call builder
    // would restart the cursor at 0 for every block and misalign
    // Aozora projection against the table.
    let mut blocks_ir: Vec<Vec<ir::IrBlock>> = root
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
    mut blocks_ir: Vec<Vec<ir::IrBlock>>,
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
    let mut blocks = Vec::new();
    let mut mask_cursor = mask_originals;
    for (idx, child) in root.children().enumerate() {
        let data = child.data.borrow();
        let line = constructs::saturating_u32(data.sourcepos.start.line).max(1);
        drop(data);
        let rendered = if options.source_line_anchors {
            source_line_anchors::format_block_with_anchor(child, &options.comrak)
        } else {
            let mut buf = String::new();
            comrak::format_html(child, &options.comrak, &mut buf)
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
    fn gfm_only_options_have_aozora_disabled_and_gfm_extensions_enabled() {
        let opts = Options::gfm_only();
        assert!(!opts.aozora_enabled, "gfm_only must skip the aozora pass");
        assert!(opts.comrak.extension.strikethrough);
        assert!(opts.comrak.extension.table);
        assert!(opts.comrak.extension.autolink);
        assert!(opts.comrak.extension.tasklist);
        assert!(opts.comrak.extension.tagfilter);
        assert!(opts.comrak.render.r#unsafe);
    }

    #[test]
    fn options_builders_and_getters_round_trip() {
        // Exercise the public builder / getter surface (doctested, but run
        // here too so the coverage gate counts it).
        let opts = Options::default()
            .with_aozora_enabled(false)
            .with_source_line_anchors(true);
        assert!(!opts.aozora_enabled());
        assert!(opts.source_line_anchors());
        assert!(opts.comrak().extension.table);
    }

    #[test]
    fn gfm_only_renders_strikethrough_and_does_not_recognise_ruby() {
        // gfm_only's contract: GFM extensions on, Aozora pre-pass off.
        // The strikethrough must produce `<del>`; the ruby-shaped
        // `｜...《》` source must survive verbatim because the lexer
        // never ran.
        let opts = Options::gfm_only();
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
    // (a) Spec-conformance constructors are #[doc(hidden)] but still
    // wire raw-HTML passthrough on for the spec runners. These tests pin
    // that the hidden constructors keep their unsafe spec config so a
    // future refactor that breaks the spec wiring is caught here.
    // -------------------------------------------------------------------

    #[test]
    fn commonmark_only_enables_raw_html_and_disables_aozora() {
        let opts = Options::commonmark_only();
        assert!(
            opts.comrak.render.r#unsafe,
            "commonmark_only must enable raw-HTML passthrough for the spec runner"
        );
        assert!(
            !opts.aozora_enabled,
            "commonmark_only must skip the aozora pass"
        );
    }

    #[test]
    fn default_does_not_enable_raw_html() {
        // The production constructor must NOT inherit the spec runners'
        // raw-HTML passthrough — that is the XSS-safety contract that
        // motivated hiding commonmark_only / gfm_only.
        let opts = Options::default();
        assert!(
            !opts.comrak.render.r#unsafe,
            "default must leave raw HTML escaped (no render.unsafe)"
        );
        assert!(opts.aozora_enabled, "default must run the aozora pass");
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
