//! Aozora Flavored Markdown — CommonMark + GFM + 青空文庫記法.
//!
//! Layers the sibling `aozora` parser onto a vendored verbatim comrak so a
//! single [`render`] call turns aozora-flavored-markdown source into HTML.
//! Public entry points:
//!
//! - [`render`] — render aozora-flavored-markdown source straight to HTML.
//! - [`serialize`] — aozora-md-source round-trip.
//! - [`Options`] — configuration; [`Options::default`] enables the GFM
//!   extensions aozora-flavored-markdown uses on top of CommonMark.
//! - [`AOZORA_MD_CLASSES`] — every CSS class the rendered HTML can carry,
//!   with matching stylesheets behind the default-off `theme` feature.
//!
//! ```
//! use aozora_flavored_markdown::{Options, render};
//!
//! let rendered = render("彼は｜青梅《おうめ》に行った。", &Options::default());
//! assert!(rendered.html.contains("<ruby>"));
//! ```
//!
//! ## Pipeline
//!
//! ```text
//! source                             ── UTF-8 input
//!   ▼ aozora parse                    ── 青空文庫 constructs + diagnostics
//!   ▼ constructs::build               ── one PUA sentinel per construct,
//!   │                                    substituted in source coordinates
//!   ▼ comrak::parse_document          ── vanilla CommonMark + GFM
//!   │   (PUA sentinels flow through as plain text)
//!   ▼ ast_splice::splice_into_ast     ── sentinel → 青空文庫 HTML fragment
//!   ▼ comrak::format_html             ── vanilla, sentinel-free AST
//! HTML
//! ```
//!
//! Comrak is unmodified: the v0.52.0 verbatim tree carries no Aozora-aware
//! code (ADR-0001 budget = 0). The boundary with `aozora` is its public API
//! only (ADR-0021).

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

/// PUA sentinel codepoints this crate substitutes for 青空文庫
/// constructs before comrak parses.
///
/// Owned here rather than re-exported from the sibling parser, so this
/// crate's public API never republishes a constant it does not control —
/// the substitution is ours to make (see `crate::constructs`), and these
/// are the four codepoints a consumer will see if it ever inspects the
/// intermediate text.
pub mod sentinels {
    use crate::constructs;

    /// Inline Aozora span (ruby / bouten / annotation / gaiji /
    /// TCY / kaeriten).
    pub const INLINE: char = constructs::INLINE_SENTINEL;
    /// Block-leaf Aozora line (page break, section break, leaf
    /// indent, sashie).
    pub const BLOCK_LEAF: char = constructs::BLOCK_LEAF_SENTINEL;
    /// Paired-container open line (e.g. `［＃ここから字下げ］`).
    pub const BLOCK_OPEN: char = constructs::BLOCK_OPEN_SENTINEL;
    /// Paired-container close line (e.g. `［＃ここで字下げ終わり］`).
    pub const BLOCK_CLOSE: char = constructs::BLOCK_CLOSE_SENTINEL;

    /// Every sentinel this crate substitutes, in declaration order.
    ///
    /// A leak check reads the set from here rather than re-listing the
    /// codepoints, so a sentinel added later is checked without the
    /// checker being edited.
    ///
    /// ```
    /// use aozora_flavored_markdown::sentinels;
    ///
    /// assert!(sentinels::ALL.contains(&sentinels::INLINE));
    /// assert!(sentinels::ALL.iter().all(|c| ('\u{E000}'..='\u{F8FF}').contains(c)));
    /// ```
    pub const ALL: [char; 4] = [INLINE, BLOCK_LEAF, BLOCK_OPEN, BLOCK_CLOSE];
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
/// `comrak::Options` is held with a `'static` lifetime: aozora-flavored-markdown doesn't
/// install URL rewriters or broken-link callbacks (which are the
/// only comrak fields that need a non-`'static` lifetime), so the
/// borrow parameter would be dead weight in our public API.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Options {
    comrak: comrak::Options<'static>,
    /// When `true`, run the aozora lex pre-pass and HTML
    /// post-processing. When `false`, the input flows straight into
    /// vanilla `comrak::parse_document` + `format_html` — used by the
    /// CommonMark / GFM spec conformance runners to verify the wrapper
    /// does not perturb upstream behaviour.
    ///
    /// Private: read via [`Options::aozora_enabled`], set via
    /// [`Options::with_aozora_enabled`].
    aozora_enabled: bool,
    /// When `true`, the HTML renderer adds `data-aozora-md-source-line="N"`
    /// (1-based) to every top-level block element it emits. The
    /// aozora-flavored-markdown-obsidian document-mode adapter (Pillar 6 of the plan)
    /// uses these anchors to map per-block post-processor calls back
    /// to slices of the rendered fragment without re-parsing.
    ///
    /// Defaults to `false`. Cost when enabled: one extra walk over
    /// comrak's top-level AST children + a streaming insert pass on
    /// the produced HTML. Both are O(blocks).
    ///
    /// Private: read via [`Options::source_line_anchors`], set via
    /// [`Options::with_source_line_anchors`].
    source_line_anchors: bool,
}

impl Default for Options {
    /// The recommended aozora-flavored-markdown dialect configuration:
    /// GFM extensions on (strikethrough, table, autolink, tasklist),
    /// hardbreaks on so each Aozora source newline becomes a `<br>`
    /// (verse / dialogue boundaries are load-bearing in 青空文庫 source),
    /// and the Aozora pre-pass enabled. Raw-HTML passthrough stays off
    /// (`render.unsafe = false`), so this is XSS-safe on untrusted input.
    ///
    /// # Examples
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
    /// Plain CommonMark (no GFM, no Aozora) with comrak's raw-HTML
    /// passthrough **enabled** (`render.unsafe = true`). Spec-conformance
    /// scaffolding only — it exists so the CommonMark 0.31.2 runner can
    /// verify the wrapper does not perturb comrak's CommonMark behaviour
    /// against a spec whose expected output includes raw HTML.
    ///
    /// Hidden from the published API surface (`#[doc(hidden)]`): this is
    /// not a production configuration. Use [`Options::default`] (which
    /// keeps `render.unsafe = false`) or a hand-built [`Options`] for any
    /// real workload.
    ///
    /// # Security
    ///
    /// **Raw-HTML passthrough — never use on untrusted input.** This adds
    /// no Rust `unsafe`, but it is a security footgun: it turns on
    /// comrak's raw-HTML passthrough (`render.unsafe = true`), so comrak
    /// emits raw HTML verbatim and passes through unsanitized URLs
    /// (`javascript:` schemes included). Feeding attacker-controlled
    /// source through these `Options` is an XSS sink. Reach for
    /// [`Options::default`] instead, which leaves raw HTML escaped.
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

    /// Pure-GFM extension set (no Aozora) with comrak's raw-HTML
    /// passthrough **enabled** (`render.unsafe = true`). Spec-conformance
    /// scaffolding only — it backs the GFM 0.29 conformance runner.
    ///
    /// Hidden from the published API surface (`#[doc(hidden)]`): this is
    /// not a production configuration. Use [`Options::default`] (which
    /// keeps `render.unsafe = false`) or a hand-built [`Options`] for any
    /// real workload.
    ///
    /// # Security
    ///
    /// **Raw-HTML passthrough — never use on untrusted input.** This adds
    /// no Rust `unsafe`, but it is a security footgun: it turns on
    /// comrak's raw-HTML passthrough (`render.unsafe = true`), so comrak
    /// emits raw HTML verbatim and passes through unsanitized URLs
    /// (`javascript:` schemes included). Feeding attacker-controlled
    /// source through these `Options` is an XSS sink. Reach for
    /// [`Options::default`] instead, which leaves raw HTML escaped.
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

    /// Builder-style toggle for source-line anchors. Returns a new
    /// `Options` with `source_line_anchors = on`.
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

    /// Builder-style toggle for the Aozora pre-pass. Returns a new
    /// `Options` with `aozora_enabled = on`. When `false`, the input
    /// flows straight through comrak with no Aozora lexing or HTML
    /// post-processing.
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

    /// Whether the Aozora lex pre-pass / HTML post-pass is enabled
    /// (see [`Options::with_aozora_enabled`]).
    #[must_use]
    pub fn aozora_enabled(&self) -> bool {
        self.aozora_enabled
    }

    /// Whether the renderer tags top-level blocks with
    /// `data-aozora-md-source-line` anchors
    /// (see [`Options::with_source_line_anchors`]).
    #[must_use]
    pub fn source_line_anchors(&self) -> bool {
        self.source_line_anchors
    }

    /// Read access to the underlying [`comrak::Options`]. The standard
    /// dialect configuration is set by [`Options::default`]; reach for
    /// this only to inspect a comrak knob directly.
    #[must_use]
    pub fn comrak(&self) -> &comrak::Options<'static> {
        &self.comrak
    }

    /// Mutable escape hatch to the underlying [`comrak::Options`] for
    /// advanced comrak tuning beyond what the first-class builders cover.
    ///
    /// # Stability
    ///
    /// This re-exposes comrak's own option surface, which is **not**
    /// covered by aozora-flavored-markdown's `SemVer` guarantee: a comrak
    /// major bump may change these fields. Prefer the `with_*` builders
    /// for anything they already cover.
    pub fn comrak_mut(&mut self) -> &mut comrak::Options<'static> {
        &mut self.comrak
    }
}

/// Output of [`render`].
#[derive(Debug)]
#[non_exhaustive]
pub struct Rendered {
    /// HTML output, with every Aozora sentinel substituted.
    pub html: String,
    /// Non-fatal lexer observations (unclosed pairs, PUA collisions,
    /// stray triggers, …). Empty on the happy path.
    pub diagnostics: Vec<Diagnostic>,
}

/// Output of [`render_to_ir`].
///
/// The IR projection alongside the HTML and diagnostics. Used by the
/// `aozora-flavored-markdown-wasm` bridge so the JS-side renderer can pick its own output
/// target (DOM fragment, `CodeMirror` `RangeSet`, semantic tokens, …)
/// from a single source.
#[derive(Debug)]
#[non_exhaustive]
pub struct RenderedIr {
    pub ir: ir::IrDocument,
    pub html: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Largest source this crate will hand to the sibling parser.
///
/// That parser keys every span on a `u32` byte offset and asserts
/// `source.len() <= u32::MAX` on the way in. Under this workspace's
/// `panic = "abort"` release profile that assert is a hard process abort,
/// not a catchable panic — an in-scope crash per `SECURITY.md` for a hostile
/// input above 4 GiB. The public entry points here guard on the boundary
/// *first*, so an oversized input degrades to a graceful empty render
/// instead of aborting the host process.
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// `true` when a source of `len` bytes is within the lexer's
/// addressable `u32` span budget.
///
/// Split out from [`source_within_span_budget`] so the boundary
/// arithmetic is unit-testable at `u32::MAX` / `u32::MAX + 1` without
/// allocating a multi-gigabyte `String`.
const fn len_within_span_budget(len: usize) -> bool {
    len <= MAX_SOURCE_BYTES
}

/// `true` when `input` is within the lexer's addressable `u32` span
/// budget. `false` inputs must not be handed to the core.
const fn source_within_span_budget(input: &str) -> bool {
    len_within_span_budget(input.len())
}

/// Render aozora-flavored-markdown source text to HTML.
///
/// One-stop entry point for the typical caller (aozora-flavored-markdown CLI, aozora-flavored-markdown-epub).
/// Internally: the source is scanned for 青空文庫 constructs and each is
/// replaced by a PUA sentinel; `comrak::parse_document` parses the result
/// (sentinels flow through as plain text, being outside CommonMark's escape
/// set); `ast_splice::splice_into_ast` swaps each sentinel back for its
/// rendered 青空文庫 HTML; `comrak::format_html` emits the spliced AST.
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
/// # Oversized input
///
/// If `input` exceeds `MAX_SOURCE_BYTES` (4 GiB − 1, the lexer's `u32`
/// span budget) this returns an empty [`Rendered`] (`html: ""`, no
/// diagnostics) **without** invoking the core lexer — the core would
/// otherwise `assert!` and abort the process under `panic = "abort"`.
/// See `MAX_SOURCE_BYTES` for the rationale.
///
/// # Panics
///
/// Panics if `comrak::format_html` fails to write into the internal
/// `String` sink — `String` cannot fail as a `fmt::Write`, so this
/// branch is unreachable in normal use.
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
/// Mirrors [`render`] but additionally walks comrak's AST
/// to emit a typed [`ir::IrDocument`]. The IR is the canonical
/// contract between aozora-flavored-markdown-wasm and aozora-flavored-markdown-obsidian's TS renderers.
///
/// The Markdown side is typed (paragraph, heading, blockquote, list,
/// code, thematic break, table, image). Every 青空文庫 notation lands as
/// one `IrBlock::Aozora` / `IrInline::Aozora` carrying its tag, source
/// span, and HTML fragment — except where the notation changes the
/// *shape* of the document rather than its content: a heading hint
/// (`［＃「X」は大見出し］`) promotes its host paragraph to
/// `IrBlock::Heading`, and an annotation inside a heading body drops out.
/// Both mirror what the HTML renderer does, so the IR and the HTML of one
/// call describe the same document.
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
/// # Oversized input
///
/// If `input` exceeds `MAX_SOURCE_BYTES` this returns an empty
/// [`RenderedIr`] (empty IR document, `html: ""`, no diagnostics)
/// without invoking the core lexer. See `MAX_SOURCE_BYTES`.
///
/// # Panics
///
/// Panics if `comrak::format_html` fails to write into the internal
/// `String` sink — `String` cannot fail as a `fmt::Write`, so this
/// branch is unreachable in normal use.
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

/// Internal pipeline driver shared between `render` and `render_to_ir`.
///
/// Runs the full lex → tile → comrak → format → post-process → unmask →
/// anchors chain and threads the AST root + the construct table through
/// `project` *before* HTML formatting starts. The closure returns whatever
/// extra data the caller needs alongside the HTML (`()` for the plain
/// renderer, an `IrDocument` for the IR renderer).
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

    // IR projection sees the AST *before* sentinel splicing — it
    // walks the same Text-with-sentinel-char pre-mutation tree the
    // splicer is about to consume. Both walkers cursor over the same
    // construct table (each with its own cursor) so they stay in lockstep
    // without serial coupling.
    let extra = project(root, &constructs);

    // Mutate the AST: every PUA sentinel becomes a `NodeValue::Raw`
    // node carrying the rendered Aozora HTML. After this returns,
    // the AST contains no sentinel character; `comrak::format_html`
    // emits final HTML in a single verbatim pass. The table carries each
    // construct's source text, so sentinels that landed in literal markdown
    // contexts (inline code, link URLs) are rewritten back to it.
    ast_splice::splice_into_ast(root, &comrak_arena, &constructs);

    let html = format_root(root, options, Some(mask_originals.as_slice()));
    (html, constructs.diagnostics().to_vec(), extra)
}

/// Common HTML finalisation: comrak-format the root (per top-level
/// child when `source_line_anchors` is on, so each child's first
/// open tag picks up its `data-aozora-md-source-line` attribute), then
/// unmask code-block triggers.
///
/// AST-level Aozora sentinel splicing runs in [`drive_pipeline`]
/// before this is called, so by the time we hand the AST to
/// `comrak::format_html` no PUA sentinel remains.
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
        code_block_mask::unmask_html(&html, originals).into_owned()
    } else {
        html
    }
}

/// One block of [`render_blocks_to_ir`]'s output.
///
/// Each entry corresponds to one top-level comrak child. `html` is the
/// rendered HTML for that child (with Aozora sentinels spliced).
/// `ir` is the IR projection — typically a single block, but empty for
/// comrak constructs the IR does not model (definition lists, footnote
/// refs, raw HTML, …).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RenderedBlock {
    pub ir: Vec<ir::IrBlock>,
    pub html: String,
    /// 1-based line where this block began in the source.
    pub source_line: u32,
}

/// Per-block streaming render.
///
/// Produces one [`RenderedBlock`] per top-level comrak child, in
/// document order. Used by aozora-flavored-markdown-obsidian's chunked-cancellation path
/// (ADR-0009): the JS bridge can iterate the returned vector and
/// check its `AbortSignal` between blocks.
///
/// The current implementation parses the document once (a single
/// comrak pass) and renders each top-level block's HTML separately
/// using `comrak::format_html`. Diagnostics from the lexer are
/// returned alongside the blocks, attached to the document as a
/// whole rather than per-block (the lexer pass is non-block-scoped).
///
/// A paired container spanning several top-level blocks emits its open
/// and close markers in the blocks they appear in — the builder threads
/// its container stack across calls, so the pair still matches. A container
/// the source never closes is drained at the end into a trailing block of
/// its own, matching the closing tag the HTML side appends there, so
/// concatenating either output leaves no container hanging open.
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
/// # Oversized input
///
/// If `input` exceeds `MAX_SOURCE_BYTES` this returns
/// `(Vec::new(), Vec::new())` — no blocks, no diagnostics — without
/// invoking the core lexer. See `MAX_SOURCE_BYTES`.
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
        let blocks = collect_rendered_blocks(root, options, Vec::new());
        return (blocks, Vec::new());
    }

    let (masked_source, _mask_originals) = code_block_mask::mask_code_block_triggers(input);
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
    let blocks = collect_rendered_blocks(root, options, blocks_ir);
    (blocks, diagnostics)
}

fn collect_rendered_blocks<'a>(
    root: &'a AstNode<'a>,
    options: &Options,
    mut blocks_ir: Vec<Vec<ir::IrBlock>>,
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
    let mut blocks = Vec::new();
    for (idx, child) in root.children().enumerate() {
        let data = child.data.borrow();
        let line = constructs::saturating_u32(data.sourcepos.start.line).max(1);
        drop(data);
        let mut block_html = String::new();
        comrak::format_html(child, &options.comrak, &mut block_html)
            .expect("formatting a String never fails");
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

/// Round-trip an aozora-flavored-markdown source through the parser and back
/// to canonical aozora-md-source text.
///
/// Delegates to the parser's own formatter — the inverse of the parse, and
/// canonicalising: notation the author wrote in a longer form comes back in
/// the shortest spelling that reads the same (below, the ruby's explicit
/// base marker is dropped because the base is unambiguous without it).
/// Plain CommonMark portions pass through verbatim because the parser
/// leaves them untouched, and the output is a fixed point — serializing it
/// again returns it unchanged.
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
/// # Oversized input
///
/// If `input` exceeds `MAX_SOURCE_BYTES` this returns an empty
/// `String` without invoking the core lexer (which would otherwise
/// `assert!` and abort under `panic = "abort"`). See
/// `MAX_SOURCE_BYTES`. The round-trip is therefore *not* identity on
/// inputs larger than 4 GiB — but such input cannot be lexed at all, so
/// an empty serialization is the only graceful option.
#[must_use]
pub fn serialize(input: &str) -> String {
    if !source_within_span_budget(input) {
        return String::new();
    }
    aozora::parse(input.to_owned())
        .map(|document| document.snapshot().to_source())
        .unwrap_or_default()
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
