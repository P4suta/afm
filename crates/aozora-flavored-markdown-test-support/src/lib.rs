//! Test predicates and invariant helpers for the aozora-flavored-markdown
//! integration test suite.
//!
//! This crate hosts the *predicates* that codify every "must never
//! be" output shape the renderer must avoid — see the [Invariant
//! catalog](#invariant-catalog) below. Each predicate returns
//! `Result<(), Violation>` so it can be composed from unit tests,
//! property tests, the corpus sweep, and fuzz harnesses on equal
//! footing. [`assert_invariants`] runs every predicate and aggregates
//! their diagnostics.
//!
//! The matching *generator* strategies live alongside them in
//! [`generators`], with the shared [`ProptestConfig`] in [`config`].
//! Hosting them here — rather than borrowing a parser-side crate — keeps
//! `proptest` out of `aozora-flavored-markdown`'s runtime dependency graph
//! while leaving the pools free to cover this dialect's own mixed
//! CommonMark × Aozora shapes.
//!
//! [`ProptestConfig`]: proptest::prelude::ProptestConfig
//!
//! # Invariant catalog
//!
//! | Tier | Predicate | Shape forbidden in rendered HTML |
//! |------|-----------|-----------------------------------|
//! | A    | [`check_no_bare_bracket`]           | Bare `［＃` outside `aozora-md-directive` wrapper, `<code>` regions excepted (Tier-A canary). |
//! | B    | [`check_no_sentinel_leak`]          | PUA sentinels U+E001–U+E004 (this workspace's own construct markers). |
//! | C    | [`check_heading_integrity`]         | `<h1>`–`<h6>` bodies must not carry `aozora-md-indent`, `aozora-md-container-indent`, or `aozora-md-directive` tokens. |
//! | D    | [`check_html_tag_balance`]          | Tags must balance ([`check_well_formed`] returns `Ok`). |
//! | E    | [`check_directive_wrapper_shape`]  | `aozora-md-directive` wrappers must close, carry `hidden`, and never nest. |
//! | F    | [`check_no_xss_marker`]             | Raw `<script`, `javascript:`, or `on<event>=` must never appear. |
//! | G    | [`check_css_class_contract`]        | Every `aozora-md-*` class token must be in the pinned [`AOZORA_MD_CLASSES`] list. |
//! | I    | [`check_escape_invariants`]         | No double-encoded entities (`&amp;lt;`, `&amp;amp;`, …). |
//! | J    | [`check_content_model`]             | `<rt>` / `<rp>` must appear only inside `<ruby>`. |
//! | K    | [`check_markup_completeness`]       | Every `<ruby>` with an `<rp>(</rp>` must also carry its closing `<rp>)</rp>`. |
//!
//! Tier H (no setext `<h2>` from a decorative rule) and Tier L (no
//! empty promoted heading) are **necessarily unit-test-only**, not a
//! temporary gap: neither has a sound rendered-HTML witness, because
//! each bug-shape is byte-identical to a *legitimate* construct, so a
//! shape-only predicate that fired on it would also fire on valid
//! output under proptest / corpus / fuzz.
//!
//! * Tier H — the failure mode `<h2>prose</h2>` (a decorative rule
//!   that wrongly promoted its preceding paragraph) is indistinguishable
//!   from a legitimate setext heading (`Heading\n---`). An always-on
//!   HTML predicate would have to reject every setext `<h2>`.
//! * Tier L — an empty *promoted* heading `<hN></hN>` is byte-identical
//!   to a legitimate empty ATX heading (`##`), which CommonMark renders
//!   as `<h2></h2>`. A blanket "no empty heading" HTML predicate would
//!   false-positive on that valid input.
//!
//! Both are therefore pinned where the source — and thus the pre-/post-
//! promotion intent — is known: the heading-promotion integration tests
//! in `aozora-flavored-markdown/tests/heading_promotion.rs`.
//!
//! These predicates live in their own crate (an `aozora-flavored-markdown` dev-dependency)
//! so they stay out of the production crate's type-check / lint / `cargo doc`
//! / coverage surface.
//!
//! [`AOZORA_MD_CLASSES`]: aozora_flavored_markdown::AOZORA_MD_CLASSES

#![forbid(unsafe_code)]

pub mod config;
pub mod generators;

use aozora_flavored_markdown::{is_contract_class, sentinels};
use core::error::Error;
use core::fmt;
use std::borrow::Cow;
use std::collections::HashSet;

/// The class a heading the notation asked for carries, marking it as the
/// parser's own markup rather than a heading this crate composed.
const AOZORA_MD_HEADING: &str = "aozora-md-heading";

// ---------------------------------------------------------------------------
// Rendered-HTML post-processing
// ---------------------------------------------------------------------------

const AOZORA_MD_DIRECTIVE_OPEN: &str = r#"<span class="aozora-md-directive" hidden>"#;
const AOZORA_MD_DIRECTIVE_CLOSE: &str = "</span>";

/// Remove `<span class="aozora-md-directive" hidden>…</span>` wrappers from `html`.
///
/// Leaves the caller with "bare" output — useful for asserting that no `［＃`
/// leaked outside a directive wrapper (Tier A invariant). Idempotent: a
/// second pass on already-stripped output returns the same string.
#[must_use]
pub fn strip_directive_wrappers(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(AOZORA_MD_DIRECTIVE_OPEN) {
        out.push_str(&rest[..at]);
        let after_open = &rest[at + AOZORA_MD_DIRECTIVE_OPEN.len()..];
        let Some(close_at) = after_open.find(AOZORA_MD_DIRECTIVE_CLOSE) else {
            // Malformed — preserve remainder so a Tier-A assertion can fire on
            // the leaked bracket.
            out.push_str(rest);
            return out;
        };
        rest = &after_open[close_at + AOZORA_MD_DIRECTIVE_CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

/// Assert `html` carries no bare `［＃` — the Tier A canary in assertion
/// form, for integration tests that want a panic rather than a `Result`.
///
/// Delegates to [`check_no_bare_bracket`] so the tier has exactly one
/// definition; in particular the `<code>` exception applies here too.
///
/// # Panics
///
/// Panics with the [`Violation::BareBracket`] diagnostic (first offset,
/// context snippet, total count) plus the offending HTML.
pub fn assert_no_bare_bracket(html: &str) {
    if let Err(violation) = check_no_bare_bracket(html) {
        panic!("{violation}\n  full html = {html:?}");
    }
}

/// Format a `±window` context snippet around the first `needle` in `haystack`.
///
/// Snaps to UTF-8 boundaries so the excerpt is always losslessly printable.
/// Returns the string `<needle missing>` when the substring is absent.
#[must_use]
pub fn first_occurrence_context(haystack: &str, needle: &str, window: usize) -> String {
    let Some(at) = haystack.find(needle) else {
        return "<needle missing>".to_owned();
    };
    context_window(haystack, at, needle.len(), window)
}

/// Format a `±window` context snippet around a byte offset in `haystack`.
///
/// `len` is the length of the "needle" that starts at `offset`; it scopes
/// the mark so the caller can report a specific tag or class name without
/// having to call the string-based [`first_occurrence_context`]. Used by
/// predicates whose offending location is structural rather than a
/// substring match (tag-balance, heading contamination).
#[must_use]
pub fn first_occurrence_context_bytes(haystack: &str, offset: usize, window: usize) -> String {
    if offset > haystack.len() {
        return "<offset out of range>".to_owned();
    }
    context_window(haystack, offset, 0, window)
}

fn context_window(haystack: &str, at: usize, len: usize, window: usize) -> String {
    let lo = snap_left(haystack, at.saturating_sub(window));
    let hi = snap_right(haystack, (at + len + window).min(haystack.len()));
    format!("...{}...", &haystack[lo..hi])
}

/// Round `i` down to the nearest UTF-8 character boundary in `s`.
#[must_use]
pub const fn snap_left(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `i` up to the nearest UTF-8 character boundary in `s`.
#[must_use]
pub const fn snap_right(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Invariant violations
// ---------------------------------------------------------------------------

/// Every way an aozora-flavored-markdown HTML output can violate a codified invariant.
///
/// One variant per predicate so the aggregator [`assert_invariants`] can
/// route its diagnostics without losing structure. Snippets are short
/// (≤ ±80 bytes around the offending locus) so proptest shrinking does
/// not hold large values in flight during long searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// Tier A — a bare `［＃` leaked outside any `aozora-md-directive` wrapper.
    BareBracket {
        first_offset: usize,
        snippet: String,
        total: usize,
    },
    /// Tier B — a PUA sentinel (U+E001–U+E004) reached the rendered HTML.
    SentinelLeak {
        codepoint: char,
        first_offset: usize,
        snippet: String,
    },
    /// Tier C — a heading (`<h1>`–`<h6>`) body contains a forbidden class.
    HeadingContaminated {
        level: u8,
        forbidden_class: String,
        snippet: String,
    },
    /// Tier D — a tag-balance violation from [`check_well_formed`].
    UnbalancedTag(WellFormedError),
    /// Tier E — the `aozora-md-directive` wrapper shape is malformed.
    DirectiveWrapper {
        violation: &'static str,
        snippet: String,
    },
    /// Tier F — an XSS marker leaked into the HTML.
    XssLeak {
        marker: &'static str,
        first_offset: usize,
        snippet: String,
    },
    /// Tier G — an `aozora-md-*` class token is not in the library's
    /// [`AOZORA_MD_CLASSES`](aozora_flavored_markdown::AOZORA_MD_CLASSES).
    UnknownCssClass { class: String, snippet: String },
    /// Tier I — a double-encoded HTML entity (e.g. `&amp;lt;`) slipped in.
    DoubleEncodedEntity { snippet: String },
    /// Tier J — HTML content-model violation (orphan `<rt>`, `<rp>`, …).
    ContentModel {
        violation: &'static str,
        snippet: String,
    },
    /// Tier K — `<ruby>` element missing its `<rp>(</rp>` ↔ `<rp>)</rp>` pair.
    MarkupIncomplete {
        violation: &'static str,
        snippet: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BareBracket {
                total,
                snippet,
                first_offset,
            } => write!(
                f,
                "Tier A: bare `［＃` leaked outside aozora-md-directive wrapper \
                 ({total} occurrence(s); first near offset {first_offset}): {snippet}",
            ),
            Self::SentinelLeak {
                codepoint,
                first_offset,
                snippet,
            } => write!(
                f,
                "Tier B: lexer PUA sentinel U+{codepoint:04X} leaked to rendered HTML \
                 (first near offset {first_offset}): {snippet}",
                codepoint = *codepoint as u32,
            ),
            Self::HeadingContaminated {
                level,
                forbidden_class,
                snippet,
            } => write!(
                f,
                "Tier C: <h{level}> body carries forbidden class `{forbidden_class}`: {snippet}",
            ),
            Self::UnbalancedTag(e) => write!(f, "Tier D: {e}"),
            Self::DirectiveWrapper { violation, snippet } => {
                write!(
                    f,
                    "Tier E: aozora-md-directive wrapper {violation}: {snippet}"
                )
            }
            Self::XssLeak {
                marker,
                first_offset,
                snippet,
            } => write!(
                f,
                "Tier F: XSS marker `{marker}` leaked (first near offset {first_offset}): {snippet}",
            ),
            Self::UnknownCssClass { class, snippet } => write!(
                f,
                "Tier G: unknown CSS class `{class}` (not in AOZORA_MD_CLASSES): {snippet}",
            ),
            Self::DoubleEncodedEntity { snippet } => write!(
                f,
                "Tier I: double-encoded entity (e.g. `&amp;lt;`) leaked into output: {snippet}",
            ),
            Self::ContentModel { violation, snippet } => write!(
                f,
                "Tier J: content-model violation ({violation}): {snippet}",
            ),
            Self::MarkupIncomplete { violation, snippet } => {
                write!(f, "Tier K: ruby markup incomplete ({violation}): {snippet}")
            }
        }
    }
}

impl Error for Violation {}

// ---------------------------------------------------------------------------
// Predicates — one per tier
// ---------------------------------------------------------------------------

/// Tier A — no bare `［＃` outside `aozora-md-directive` wrappers.
///
/// The Tier-A canary in predicate form, and the single definition of
/// the tier: strip every `<code>` region and every directive wrapper,
/// then assert the remainder contains no `［＃`. Succeeds when the
/// input has no directives at all (stripping is a no-op).
///
/// **`<code>` regions are excepted** — both the fenced `<pre><code>`
/// kind and the inline code span. A code element's body is the user's
/// bytes verbatim (CommonMark §6.1), so notation typed inside one
/// *must* surface unwrapped: that is the renderer restoring a literal
/// context, not leaking an unparsed construct. `literal_context_sentinels`
/// pins the behaviour and `code_block_mask` carries the round-trip
/// properties behind it.
///
/// The exception is deliberately narrower than a blanket skip: an
/// unclosed `<code` keeps the rest of the document in scope, so a leak
/// that happens to follow malformed markup still fires.
///
/// # Errors
///
/// Returns [`Violation::BareBracket`] with a ±80-byte snippet and the
/// total leak count when a bare `［＃` survives the strip.
pub fn check_no_bare_bracket(html: &str) -> Result<(), Violation> {
    const NEEDLE: &str = "［＃";
    let stripped = strip_directive_wrappers(&strip_code_regions(html));
    if let Some(offset) = stripped.find(NEEDLE) {
        let total = stripped.matches(NEEDLE).count();
        return Err(Violation::BareBracket {
            first_offset: offset,
            snippet: first_occurrence_context(&stripped, NEEDLE, 80),
            total,
        });
    }
    Ok(())
}

/// Tier B — rendered HTML contains no PUA sentinel (U+E001–U+E004).
///
/// **Unconditional: no input may legitimately leak one.** A sentinel means
/// "a construct stood here" only inside this workspace's own substitution.
/// An author who types U+E001 into a source does not get one back — the
/// parser reports it *and* overwrites the character with U+FFFD, in a code
/// span and in running text alike. Every sentinel that reaches the output
/// was therefore written by the substitution and never resolved, which is a
/// bug on any input — most of all on one that also produced diagnostics,
/// because the recovery path is where an unresolved construct is likeliest
/// to survive. Callers must not gate this predicate on a clean parse.
///
/// # Errors
///
/// Returns [`Violation::SentinelLeak`] with the offending codepoint
/// and a snippet when any of U+E001, U+E002, U+E003, or U+E004 appears
/// in `html`.
pub fn check_no_sentinel_leak(html: &str) -> Result<(), Violation> {
    // Single source of truth: the substitution is the library's, so the
    // set it publishes is what a leak is measured against. A sentinel added
    // there flows in here automatically instead of silently going unchecked
    // against a hardcoded U+E001..U+E004 copy.
    for &c in &sentinels::ALL {
        let mut buf = [0u8; 4];
        let needle: &str = c.encode_utf8(&mut buf);
        if let Some(offset) = html.find(needle) {
            return Err(Violation::SentinelLeak {
                codepoint: c,
                first_offset: offset,
                snippet: first_occurrence_context_bytes(html, offset, 80),
            });
        }
    }
    Ok(())
}

/// Tier C — heading bodies carry only legitimate classes.
///
/// `<h1>`–`<h6>` bodies must not contain `aozora-md-indent`,
/// `aozora-md-container-indent`, or `aozora-md-directive` as class tokens. Other
/// Aozora markup (bouten, gaiji, tcy, kaeriten) is allowed inside a
/// heading — only indent markers and raw-directive wrappers are
/// bugs here.
///
/// **A heading the parser rendered whole is exempt.**
/// `［＃中見出し］…［＃中見出し終わり］` is a heading the notation asks for, and
/// its body is whatever the parser put there — including the hidden wrapper
/// an editor's note written inside the heading renders to. That is not this
/// crate composing a heading out of parts, which is what the tier is
/// written against; it arrives as one fragment and is held to the parser's
/// own output by `tests/aozora_parity.rs`. Such a heading carries
/// `aozora-md-heading`, which is how it is told apart from a markdown one.
///
/// # Errors
///
/// Returns [`Violation::HeadingContaminated`] on the first offending
/// heading found.
pub fn check_heading_integrity(html: &str) -> Result<(), Violation> {
    const FORBIDDEN: &[&str] = &[
        "aozora-md-indent",
        "aozora-md-container-indent",
        "aozora-md-directive",
    ];
    for level in 1u8..=6 {
        let open_marker = format!("<h{level}");
        let close_marker = format!("</h{level}>");
        let mut search_from = 0usize;
        while let Some(rel) = html[search_from..].find(open_marker.as_str()) {
            let tag_start = search_from + rel;
            // The byte after `<hN` must be `>` or whitespace to avoid
            // matching `<h10>` style tags which don't exist but future-
            // proofs the check.
            let after = tag_start + open_marker.len();
            if after >= html.len() {
                break;
            }
            let b = html.as_bytes()[after];
            if b != b'>' && !b.is_ascii_whitespace() {
                search_from = after;
                continue;
            }
            let Some(gt_rel) = html[tag_start..].find('>') else {
                break;
            };
            let body_start = tag_start + gt_rel + 1;
            let Some(close_rel) = html[body_start..].find(close_marker.as_str()) else {
                break;
            };
            let body_end = body_start + close_rel;
            let body = &html[body_start..body_end];
            // A heading the notation asked for, rendered whole by the
            // parser: its body is the parser's markup, not a composition
            // this tier can speak to.
            if collect_class_tokens(&html[tag_start..body_start]).contains(AOZORA_MD_HEADING) {
                search_from = body_end + close_marker.len();
                continue;
            }
            let tokens = collect_class_tokens(body);
            for &forbidden in FORBIDDEN {
                if tokens.contains(forbidden) {
                    return Err(Violation::HeadingContaminated {
                        level,
                        forbidden_class: forbidden.to_owned(),
                        snippet: first_occurrence_context_bytes(html, tag_start, 80),
                    });
                }
            }
            search_from = body_end + close_marker.len();
        }
    }
    Ok(())
}

/// Tier D — every open tag has a matching close tag (void elements exempted).
///
/// Delegates to [`check_well_formed`] and lifts the first
/// [`WellFormedError`] into a [`Violation::UnbalancedTag`].
///
/// # Errors
///
/// Returns [`Violation::UnbalancedTag`] carrying the first structural
/// error found.
pub fn check_html_tag_balance(html: &str) -> Result<(), Violation> {
    let errors = check_well_formed(html);
    if let Some(first) = errors.into_iter().next() {
        return Err(Violation::UnbalancedTag(first));
    }
    Ok(())
}

/// Tier E — `aozora-md-directive` wrappers are well-shaped.
///
/// Asserts that every `<span class="aozora-md-directive" hidden>` has a
/// matching `</span>`, that no such wrapper is nested inside another
/// (stripping would be non-idempotent), and that every wrapper carries
/// the `hidden` attribute.
///
/// Idempotency is verified by running [`strip_directive_wrappers`]
/// twice and comparing — if the stripper is idempotent on this input,
/// the wrapper shape is sane.
///
/// # Errors
///
/// Returns [`Violation::DirectiveWrapper`] describing the specific
/// shape violation.
pub fn check_directive_wrapper_shape(html: &str) -> Result<(), Violation> {
    let once = strip_directive_wrappers(html);
    let twice = strip_directive_wrappers(&once);
    if once != twice {
        return Err(Violation::DirectiveWrapper {
            violation: "strip_directive_wrappers is not idempotent",
            snippet: first_occurrence_context(html, AOZORA_MD_DIRECTIVE_OPEN, 80),
        });
    }
    // Nested wrapper detection: an open occurring before the next close.
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find(AOZORA_MD_DIRECTIVE_OPEN) {
        let open_at = search_from + rel;
        let after_open = &html[open_at + AOZORA_MD_DIRECTIVE_OPEN.len()..];
        let next_open = after_open.find(AOZORA_MD_DIRECTIVE_OPEN);
        let next_close = after_open.find(AOZORA_MD_DIRECTIVE_CLOSE);
        match (next_open, next_close) {
            (Some(no), Some(nc)) if no < nc => {
                return Err(Violation::DirectiveWrapper {
                    violation: "nested aozora-md-directive open before the enclosing close",
                    snippet: first_occurrence_context_bytes(html, open_at, 80),
                });
            }
            (_, None) => {
                return Err(Violation::DirectiveWrapper {
                    violation: "aozora-md-directive open without matching </span>",
                    snippet: first_occurrence_context_bytes(html, open_at, 80),
                });
            }
            (_, Some(nc)) => {
                search_from =
                    open_at + AOZORA_MD_DIRECTIVE_OPEN.len() + nc + AOZORA_MD_DIRECTIVE_CLOSE.len();
            }
        }
    }
    // Check for `<span class="aozora-md-directive"` *without* the `hidden` attribute
    // — the exact shape is the only one we emit, so anything else is a bug.
    let variant = r#"<span class="aozora-md-directive""#;
    let mut scan_from = 0;
    while let Some(rel) = html[scan_from..].find(variant) {
        let at = scan_from + rel;
        // Check the next non-space content is ` hidden>`.
        let after = &html[at + variant.len()..];
        let trimmed = after.trim_start();
        if !trimmed.starts_with("hidden>") && !trimmed.starts_with("hidden ") {
            return Err(Violation::DirectiveWrapper {
                violation: "aozora-md-directive span missing `hidden` attribute",
                snippet: first_occurrence_context_bytes(html, at, 80),
            });
        }
        scan_from = at + variant.len();
    }
    Ok(())
}

/// Tier F — no XSS markers reach the rendered HTML as *executable*
/// constructs.
///
/// Forbidden shapes:
///
/// * Literal `<script` (case-insensitive) — a raw script-tag opener.
///   Safe as a bare substring check because `aozora_flavored_markdown::aozora::html`
///   always escapes `<` to `&lt;` in text content, and comrak's
///   default `render.unsafe_ = false` suppresses raw-HTML passthrough.
///   So `<script` can appear *only* if aozora-flavored-markdown emits the tag itself, which
///   is always a bug.
/// * `javascript:` inside an attribute value (between `<` and `>`).
///   The substring may legitimately appear in rendered prose (a
///   markdown tutorial discussing JS URIs, for example), so we require
///   it to live inside a tag body — the only position where a browser
///   would act on it.
/// * `on<event>=` (`onerror=`, `onload=`, `onclick=`, …) inside an
///   attribute position. Same tag-context requirement as above — the
///   `onerror=alert(1)` text inside a `<span hidden>` directive
///   wrapper is harmless plain text, not a handler.
///
/// # Errors
///
/// Returns [`Violation::XssLeak`] naming the detected marker and a
/// ±80-byte snippet.
pub fn check_no_xss_marker(html: &str) -> Result<(), Violation> {
    if let Some(offset) = find_ascii_ignore_case(html, "<script") {
        return Err(Violation::XssLeak {
            marker: "<script",
            first_offset: offset,
            snippet: first_occurrence_context_bytes(html, offset, 80),
        });
    }
    if let Some(offset) = find_javascript_uri_in_tag(html) {
        return Err(Violation::XssLeak {
            marker: "javascript:",
            first_offset: offset,
            snippet: first_occurrence_context_bytes(html, offset, 80),
        });
    }
    if let Some(offset) = find_event_handler_attribute(html) {
        return Err(Violation::XssLeak {
            marker: "on<event>=",
            first_offset: offset,
            snippet: first_occurrence_context_bytes(html, offset, 80),
        });
    }
    Ok(())
}

/// Tier G — every `aozora-md-*` class token is recognised.
///
/// A class token is recognised when it is either a direct entry in the
/// library's [`AOZORA_MD_CLASSES`](aozora_flavored_markdown::AOZORA_MD_CLASSES)
/// or of the form `aozora-md-X-N` where `aozora-md-X` is in the
/// list and `N` is a non-negative integer (covers the numeric-suffix
/// modifier classes `aozora-md-indent-N`, `aozora-md-align-end-N`,
/// `aozora-md-container-indent-N`).
///
/// Non-`aozora-md-` classes (e.g. comrak's own `language-rust` on code
/// blocks) are ignored. Code-block info-strings the user supplied
/// surface as `class="language-X"` where `X` is arbitrary user
/// markup, so the check strips `<pre><code>...</code></pre>` regions
/// before scanning — the contents (including the open tag's class
/// attribute) belong to the user, not aozora-flavored-markdown.
///
/// # Errors
///
/// Returns [`Violation::UnknownCssClass`] on the first unrecognised
/// aozora-flavored-markdown class.
pub fn check_css_class_contract(html: &str) -> Result<(), Violation> {
    let scope = strip_pre_code_blocks(html);
    let tokens = collect_class_tokens(&scope);
    for token in &tokens {
        if !token.starts_with("aozora-md-") {
            continue;
        }
        if is_recognised_class(token) {
            continue;
        }
        return Err(Violation::UnknownCssClass {
            class: token.clone(),
            snippet: first_occurrence_context(&scope, token, 80),
        });
    }
    Ok(())
}

/// Tier I — no double-encoded HTML entities.
///
/// Detects the patterns that indicate the escape pass ran twice —
/// `&amp;lt;`, `&amp;gt;`, `&amp;amp;`, `&amp;quot;`, `&amp;#x27;` —
/// any of which means `&lt;` was re-escaped into `&amp;lt;`.
///
/// **Code blocks are excluded.** A `<pre><code>` body literally
/// contains the user's bytes verbatim, so a body that originally
/// carried `&amp;` MUST surface as `&amp;amp;` once HTML-escaped —
/// that is the spec, not a double-encode bug. The check therefore
/// strips `<pre><code>...</code></pre>` regions before scanning.
///
/// # Errors
///
/// Returns [`Violation::DoubleEncodedEntity`] at the first offender.
pub fn check_escape_invariants(html: &str) -> Result<(), Violation> {
    const DOUBLE_ENCODED: &[&str] = &[
        "&amp;lt;",
        "&amp;gt;",
        "&amp;amp;",
        "&amp;quot;",
        "&amp;#x27;",
        "&amp;#39;",
    ];
    let scope = strip_pre_code_blocks(html);
    for &needle in DOUBLE_ENCODED {
        if let Some(offset) = scope.find(needle) {
            return Err(Violation::DoubleEncodedEntity {
                snippet: first_occurrence_context_bytes(&scope, offset, 80),
            });
        }
    }
    Ok(())
}

/// Run every always-on invariant check against `html` and panic with
/// a rich, copy-pastable diagnostic if any one fails.
///
/// The panic message includes the source bytes (Debug-formatted), the
/// produced HTML (truncated to keep the message shell-friendly), and
/// the failing tier label. Fuzz targets and regression tests both call
/// this so a libFuzzer crash artifact translates straight into
/// "tier + source + html" without manual triage. Tier I is gated on
/// [`source_contains_html_entity_literal`]: a source carrying entity
/// literals legitimately produces `&amp;{lt,gt,amp,…};` on output.
///
/// Tier A is the one predicate deliberately left out: a bare `［＃` is
/// legitimate output for a source whose bracket pairing is malformed, so
/// its precondition belongs to the caller. Every other tier here holds on
/// arbitrary bytes, [`check_no_sentinel_leak`] included.
///
/// # Panics
///
/// Panics on the first invariant violation. The panic message is
/// engineered for `cargo test` / `cargo fuzz` output — it reads
/// linearly without needing a stack-trace pass.
pub fn assert_html_invariants(src: &str, html: &str) {
    let context = || {
        let html_excerpt = if html.len() > 600 {
            format!(
                "{:?}…[+{} more bytes]",
                &html[..html.char_indices().nth(160).map_or(html.len(), |(i, _)| i)],
                html.len() - 600
            )
        } else {
            format!("{html:?}")
        };
        format!("\n  src = {src:?}\n  html = {html_excerpt}")
    };
    let report = |tier: &str, e: Violation| -> ! {
        panic!("{tier} violated:{}\n  details = {e:?}", context())
    };
    if let Err(e) = check_no_sentinel_leak(html) {
        report("Tier B (PUA sentinel leak)", e);
    }
    if let Err(e) = check_html_tag_balance(html) {
        report("Tier D (tag balance)", e);
    }
    if let Err(e) = check_directive_wrapper_shape(html) {
        report("Tier E (directive wrapper)", e);
    }
    if let Err(e) = check_no_xss_marker(html) {
        report("Tier F (xss marker)", e);
    }
    if let Err(e) = check_css_class_contract(html) {
        report("Tier G (css class)", e);
    }
    if !source_contains_html_entity_literal(src)
        && let Err(e) = check_escape_invariants(html)
    {
        report("Tier I (double-encoded entity)", e);
    }
    if let Err(e) = check_content_model(html) {
        report("Tier J (content model)", e);
    }
    if let Err(e) = check_markup_completeness(html) {
        report("Tier K (markup completeness)", e);
    }
    if let Err(e) = check_heading_integrity(html) {
        report("Tier C (heading integrity)", e);
    }
}

/// True iff `src` would let comrak's escape pass legitimately
/// surface a `&amp;{lt,gt,amp,…};` on output.
///
/// Matches an entity-prefix literal (`&lt`, `&gt`, …) or a PUA
/// sentinel character (U+E000..U+E004).
///
/// Callers gate [`check_escape_invariants`] on the negation of this.
/// Two distinct sources of spec-correct apparent double-encodes:
///
/// 1. The source contains `&amp;`, `&lt;`, etc. (or CommonMark-escaped
///    variants like `&lt\;` that comrak resolves to the same character
///    sequence). The escape pass turns the leading `&` into `&amp;`
///    and the rest is preserved, surfacing as `&amp;{lt,gt,amp,…};`.
/// 2. The source contains a PUA sentinel (U+E000..U+E004). The lexer
///    flags this with a `SourceContainsPua` diagnostic, but the AST
///    splicer still walks the resulting Text nodes; the sentinel
///    splits the text around itself and the leading half can collapse
///    to a `&` adjacent to a literal `amp;` substring, which comrak
///    then escapes to `&amp;amp;`. Apparent double-encode, structural
///    cause — not an aozora-md-side bug.
///
/// The check is intentionally **permissive**: false negatives (missed
/// aozora-md-side double-escape because the source happened to contain
/// `&amp` somewhere) are preferable to false positives (fuzz wedged
/// on comrak-correct output). Tier I targets aozora-md-side regressions,
/// not a forensic accounting of every entity in the output.
#[must_use]
pub fn source_contains_html_entity_literal(src: &str) -> bool {
    const HINTS: &[&str] = &["&lt", "&gt", "&amp", "&quot", "&apos", "&#"];
    if HINTS.iter().any(|hint| src.contains(hint)) {
        return true;
    }
    src.chars()
        .any(|ch| ('\u{E000}'..='\u{E004}').contains(&ch))
}

/// What a scrub does with a region whose closing tag never arrives.
///
/// Malformed markup has no right answer, so each caller picks the error
/// it prefers rather than inheriting one.
#[derive(Clone, Copy)]
enum Unclosed {
    /// Drop the remainder — the caller would rather miss a violation
    /// than report one against markup it cannot delimit.
    Swallow,
    /// Keep the remainder in scope — the caller would rather report.
    Retain,
}

/// Remove every `open`…`close` region from `html`.
///
/// Substring scanning, not parsing: these predicates read renderer
/// output, where the tags of interest are emitted in one fixed shape.
fn strip_regions<'a>(html: &'a str, open: &str, close: &str, unclosed: Unclosed) -> Cow<'a, str> {
    if !html.contains(open) {
        return Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(rel_open) = html[cursor..].find(open) {
        let abs_open = cursor + rel_open;
        out.push_str(&html[cursor..abs_open]);
        let after_open = abs_open + open.len();
        let Some(rel_close) = html[after_open..].find(close) else {
            if matches!(unclosed, Unclosed::Retain) {
                out.push_str(&html[after_open..]);
            }
            return Cow::Owned(out);
        };
        cursor = after_open + rel_close + close.len();
    }
    out.push_str(&html[cursor..]);
    Cow::Owned(out)
}

/// Strip every `<pre><code...>...</code></pre>` region from `html`.
/// The double-encoded-entity check defers to this helper because code
/// blocks legitimately host `&amp;amp;` and friends in their literal
/// payload (CommonMark §6.1: code-block content is verbatim, then
/// escape-pass-once for output safety). An unclosed block swallows the
/// remainder so a stray `&amp;amp;` behind it cannot false-positive
/// Tier I.
fn strip_pre_code_blocks(html: &str) -> Cow<'_, str> {
    strip_regions(html, "<pre><code", "</code></pre>", Unclosed::Swallow)
}

/// Strip every `<code...>...</code>` region from `html` — the fenced
/// `<pre><code>` kind and the inline code span alike.
///
/// Tier A defers to this helper: both kinds carry the user's bytes
/// verbatim, so a `［＃` inside one is literal text rather than a
/// leaked construct. Unlike [`strip_pre_code_blocks`] an unclosed
/// `<code` retains the remainder — Tier A is the canary that must not
/// go quiet.
fn strip_code_regions(html: &str) -> Cow<'_, str> {
    strip_regions(html, "<code", "</code>", Unclosed::Retain)
}

/// Tier J — HTML content-model correctness for the handful of elements
/// the aozora-flavored-markdown renderer emits under structural constraints.
///
/// Specifically: every `<rt>` and `<rp>` must appear inside an open
/// `<ruby>` element. A stray `<rt>` outside `<ruby>` indicates the
/// renderer emitted a fragment without its containing element (e.g. a
/// post-process bug that detached the base).
///
/// # Errors
///
/// Returns [`Violation::ContentModel`] naming the orphaned element.
pub fn check_content_model(html: &str) -> Result<(), Violation> {
    let mut ruby_depth: i32 = 0;
    let mut i = 0usize;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(gt) = html[i..].find('>') else {
            break;
        };
        let inside = &html[i + 1..i + gt];
        let trimmed = inside.trim();
        let (is_close, body) = trimmed
            .strip_prefix('/')
            .map_or((false, trimmed), |rest| (true, rest.trim_start()));
        let name_end = body
            .char_indices()
            .find(|(_, c)| !c.is_ascii_alphanumeric())
            .map_or(body.len(), |(ix, _)| ix);
        let name = body[..name_end].to_ascii_lowercase();
        match name.as_str() {
            "ruby" if is_close => ruby_depth = ruby_depth.saturating_sub(1),
            "ruby" => ruby_depth += 1,
            "rt" | "rp" if ruby_depth == 0 => {
                return Err(Violation::ContentModel {
                    violation: "<rt> or <rp> outside <ruby>",
                    snippet: first_occurrence_context_bytes(html, i, 80),
                });
            }
            _ => {}
        }
        i += gt + 1;
    }
    Ok(())
}

/// Tier K — every `<ruby>` that opens with `<rp>(</rp>` also closes
/// with the matching `<rp>)</rp>` before `</ruby>`.
///
/// Ruby elements emitted by the aozora-flavored-markdown renderer always carry both rp
/// parentheses; an asymmetric shape is a render bug. We scan every
/// `<ruby>` … `</ruby>` slice and enforce the pair.
///
/// # Errors
///
/// Returns [`Violation::MarkupIncomplete`] describing the missing half.
pub fn check_markup_completeness(html: &str) -> Result<(), Violation> {
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find("<ruby>") {
        let ruby_start = search_from + rel;
        let Some(close_rel) = html[ruby_start..].find("</ruby>") else {
            return Err(Violation::MarkupIncomplete {
                violation: "<ruby> without matching </ruby>",
                snippet: first_occurrence_context_bytes(html, ruby_start, 80),
            });
        };
        let ruby_end = ruby_start + close_rel;
        let body = &html[ruby_start..ruby_end];
        let open_paren = body.contains("<rp>(</rp>");
        let close_paren = body.contains("<rp>)</rp>");
        if open_paren != close_paren {
            return Err(Violation::MarkupIncomplete {
                violation: if open_paren {
                    "<ruby> carries `<rp>(</rp>` without `<rp>)</rp>`"
                } else {
                    "<ruby> carries `<rp>)</rp>` without `<rp>(</rp>`"
                },
                snippet: first_occurrence_context_bytes(html, ruby_start, 80),
            });
        }
        search_from = ruby_end + "</ruby>".len();
    }
    Ok(())
}

/// Aggregate runner: apply every predicate and collect all diagnostics.
///
/// Returns `Ok(())` when every predicate is satisfied; otherwise returns
/// the full list of violations. Property tests that want one red per
/// failing invariant should pattern-match on individual predicates;
/// fixture / corpus callers that only need a pass/fail should use this.
///
/// # Errors
///
/// Returns `Err(Vec<Violation>)` containing every violation found.
pub fn assert_invariants(html: &str) -> Result<(), Vec<Violation>> {
    type Predicate = fn(&str) -> Result<(), Violation>;
    let predicates: &[Predicate] = &[
        check_no_bare_bracket,
        check_no_sentinel_leak,
        check_heading_integrity,
        check_html_tag_balance,
        check_directive_wrapper_shape,
        check_no_xss_marker,
        check_css_class_contract,
        check_escape_invariants,
        check_content_model,
        check_markup_completeness,
    ];
    let violations: Vec<_> = predicates.iter().filter_map(|p| p(html).err()).collect();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

// ---------------------------------------------------------------------------
// Shared predicate helpers
// ---------------------------------------------------------------------------

/// Collect every token that appears inside a `class="..."` attribute
/// value anywhere in `html`. Used by the heading-integrity and
/// CSS-class-contract predicates; shared so the tokeniser stays
/// consistent across both.
///
/// **Tag-boundary aware.** A naive `find("class=\"")` would also
/// match `class=` substrings that appear inside body text (e.g. an
/// `<img alt="raw text with class=…">` whose alt attribute happens to
/// carry the four-byte `class=` sequence). The tokeniser walks every
/// `<` ... `>` tag body, respecting quoted attribute values so the
/// `>` inside a quote doesn't prematurely close the tag, and only
/// looks for `class="..."` inside that scope.
fn collect_class_tokens(html: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let tag_start = i + 1;
        let mut j = tag_start;
        let mut quote: Option<u8> = None;
        while j < bytes.len() {
            let b = bytes[j];
            match quote {
                Some(q) => {
                    if b == q {
                        quote = None;
                    }
                }
                None => {
                    if b == b'"' || b == b'\'' {
                        quote = Some(b);
                    } else if b == b'>' {
                        break;
                    }
                }
            }
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let inside = &html[tag_start..j];
        if let Some(rel) = inside.find("class=\"") {
            let after = &inside[rel + "class=\"".len()..];
            if let Some(close) = after.find('"') {
                let value = &after[..close];
                for tok in value.split_whitespace() {
                    out.insert(tok.to_owned());
                }
            }
        }
        i = j + 1;
    }
    out
}

/// Accept a class token when it is either directly in the library's
/// [`AOZORA_MD_CLASSES`](aozora_flavored_markdown::AOZORA_MD_CLASSES) or of
/// the form `<listed-base>-N` where `N` is a non-negative decimal integer.
fn is_recognised_class(class: &str) -> bool {
    if is_contract_class(class) {
        return true;
    }
    // Family-suffix variants: any `<base>-<suffix>` where `<base>` is
    // in the pinned list. Covers numeric modifiers
    // (`aozora-md-indent-2`, `aozora-md-container-indent-3`, `aozora-md-align-end-1`)
    // and slug modifiers (`aozora-md-section-break-choho`,
    // `aozora-md-bouten-goma`, `aozora-md-section-break-dan`, …) emitted by
    // the sibling renderer.
    if let Some(stem_end) = class.rfind('-') {
        let stem = &class[..stem_end];
        let suffix = &class[stem_end + 1..];
        if !suffix.is_empty() && is_contract_class(stem) {
            return true;
        }
    }
    false
}

/// Case-insensitive ASCII substring search. Both `needle` and the
/// scanned bytes are lower-cased before comparison; only ASCII chars
/// are folded so non-ASCII stays byte-comparable.
fn find_ascii_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    let haystack_bytes = haystack.as_bytes();
    let needle_lower: Vec<u8> = needle.bytes().map(|b| b.to_ascii_lowercase()).collect();
    if needle_lower.is_empty() || haystack_bytes.len() < needle_lower.len() {
        return None;
    }
    for i in 0..=haystack_bytes.len() - needle_lower.len() {
        let window = &haystack_bytes[i..i + needle_lower.len()];
        if window
            .iter()
            .zip(&needle_lower)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            return Some(i);
        }
    }
    None
}

/// Detect `on<event>=` attribute handlers inside tag bodies.
///
/// Walks `html` tracking whether the cursor is between `<` and `>`
/// (tag-body context). Only matches `on[a-z]+[ ]*=` in that context —
/// the same pattern in text content (`"use onerror= to hook errors"`
/// as prose) is harmless and must not fire.
fn find_event_handler_attribute(html: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut in_tag = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            _ if in_tag && i + 3 < bytes.len() => {
                let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'<';
                if prev_ok
                    && bytes[i].eq_ignore_ascii_case(&b'o')
                    && bytes[i + 1].eq_ignore_ascii_case(&b'n')
                    && bytes[i + 2].is_ascii_lowercase()
                {
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j].is_ascii_lowercase() {
                        j += 1;
                    }
                    while j < bytes.len() && bytes[j] == b' ' {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'=' {
                        return Some(i);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Detect `javascript:` URI scheme inside tag bodies. Same
/// tag-context rule as [`find_event_handler_attribute`] — the string
/// in plain prose is harmless, only the in-attribute occurrence is a
/// browser-executable XSS vector.
fn find_javascript_uri_in_tag(html: &str) -> Option<usize> {
    const NEEDLE: &[u8] = b"javascript:";
    let bytes = html.as_bytes();
    let mut in_tag = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            _ if in_tag && i + NEEDLE.len() <= bytes.len() => {
                let window = &bytes[i..i + NEEDLE.len()];
                if window
                    .iter()
                    .zip(NEEDLE)
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
                {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// HTML well-formedness validator (relocated from tests/common/mod.rs)
// ---------------------------------------------------------------------------

/// Kinds of structural violations [`check_well_formed`] reports.
///
/// `near` snippets surface ±48 characters around the offending byte
/// offset so the failure message is actionable without a full dump of
/// the rendered HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WellFormedError {
    UnclosedTag {
        tag: String,
        near: String,
    },
    ExtraClose {
        tag: String,
        near: String,
    },
    MisorderedClose {
        opened: String,
        closed: String,
        near: String,
    },
    MalformedTag {
        near: String,
    },
}

impl fmt::Display for WellFormedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedTag { tag, near } => {
                write!(f, "unclosed <{tag}> near {near:?}")
            }
            Self::ExtraClose { tag, near } => {
                write!(f, "extra </{tag}> near {near:?}")
            }
            Self::MisorderedClose {
                opened,
                closed,
                near,
            } => write!(
                f,
                "</{closed}> closes while <{opened}> is still open, near {near:?}"
            ),
            Self::MalformedTag { near } => {
                write!(f, "malformed tag near {near:?}")
            }
        }
    }
}

/// Return the list of well-formedness violations in `html`. Empty
/// vector means the document is balanced.
///
/// Runs in a single forward pass over the bytes; the open-tag stack
/// keeps O(depth) memory. Void-element recognition is a sorted
/// `&'static [&str]` scanned with `binary_search`.
#[must_use]
pub fn check_well_formed(html: &str) -> Vec<WellFormedError> {
    let mut errors = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(lt) = find_from(bytes, i, b'<') else {
            break;
        };
        let Some(gt) = find_from(bytes, lt + 1, b'>') else {
            errors.push(WellFormedError::MalformedTag {
                near: snippet(html, lt),
            });
            break;
        };
        let inside = &html[lt + 1..gt];
        match parse_tag(inside) {
            Some(Tag::Open(name)) => {
                if !is_void_element(&name) {
                    stack.push((name, lt));
                }
            }
            Some(Tag::Close(name)) => match stack.pop() {
                Some((top, _)) if top == name => {}
                Some((top, top_pos)) => {
                    errors.push(WellFormedError::MisorderedClose {
                        opened: top.clone(),
                        closed: name,
                        near: snippet(html, lt),
                    });
                    stack.push((top, top_pos));
                }
                None => errors.push(WellFormedError::ExtraClose {
                    tag: name,
                    near: snippet(html, lt),
                }),
            },
            Some(Tag::SelfClose | Tag::Doctype | Tag::Comment) => {}
            None => errors.push(WellFormedError::MalformedTag {
                near: snippet(html, lt),
            }),
        }
        i = gt + 1;
    }

    for (name, pos) in stack {
        errors.push(WellFormedError::UnclosedTag {
            tag: name,
            near: snippet(html, pos),
        });
    }
    errors
}

enum Tag {
    Open(String),
    Close(String),
    SelfClose,
    Doctype,
    Comment,
}

fn parse_tag(inside: &str) -> Option<Tag> {
    let trimmed = inside.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('!') {
        return Some(Tag::Doctype);
    }
    if trimmed.starts_with('?') {
        return Some(Tag::Comment);
    }
    let (is_close, body) = trimmed
        .strip_prefix('/')
        .map_or((false, trimmed), |rest| (true, rest.trim_start()));
    let (name, rest) = split_tag_name(body)?;
    if name.is_empty() {
        return None;
    }
    let self_closing = rest.trim_end().ends_with('/');
    if is_close {
        Some(Tag::Close(name))
    } else if self_closing {
        drop(name);
        Some(Tag::SelfClose)
    } else {
        Some(Tag::Open(name))
    }
}

fn split_tag_name(body: &str) -> Option<(String, &str)> {
    let end = body
        .char_indices()
        .find(|(_, c)| !is_tag_name_char(*c))
        .map_or(body.len(), |(i, _)| i);
    if end == 0 {
        return None;
    }
    let name = body[..end].to_ascii_lowercase();
    let rest = &body[end..];
    Some((name, rest))
}

const fn is_tag_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void_element(name: &str) -> bool {
    VOID_ELEMENTS.binary_search(&name).is_ok()
}

fn find_from(bytes: &[u8], start: usize, target: u8) -> Option<usize> {
    bytes
        .get(start..)
        .and_then(|slice| slice.iter().position(|&b| b == target))
        .map(|rel| rel + start)
}

fn snippet(html: &str, pos: usize) -> String {
    let lo = html
        .char_indices()
        .take_while(|(i, _)| i + 48 <= pos)
        .last()
        .map_or(0, |(i, _)| i);
    let hi = html
        .char_indices()
        .find(|(i, _)| *i >= pos + 48)
        .map_or(html.len(), |(i, _)| i);
    let lo = clamp_to_char_boundary(html, lo);
    let hi = clamp_to_char_boundary(html, hi);
    html.get(lo..hi).unwrap_or(html).to_owned()
}

fn clamp_to_char_boundary(s: &str, mut i: usize) -> usize {
    if i > s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // strip_directive_wrappers (existing)
    // -------------------------------------------------------------------

    #[test]
    fn strip_returns_text_outside_wrappers() {
        let html =
            r#"<p>hello <span class="aozora-md-directive" hidden>［＃改ページ］</span> world</p>"#;
        assert_eq!(strip_directive_wrappers(html), "<p>hello  world</p>");
    }

    #[test]
    fn strip_is_idempotent() {
        let html = r#"a <span class="aozora-md-directive" hidden>X</span> b"#;
        let once = strip_directive_wrappers(html);
        let twice = strip_directive_wrappers(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn strip_handles_malformed_open_without_close() {
        let html = r#"a <span class="aozora-md-directive" hidden>X b"#;
        let out = strip_directive_wrappers(html);
        assert!(out.contains("X b"));
    }

    #[test]
    fn first_occurrence_context_snaps_to_char_boundaries() {
        let text = "ああああ［＃改ページ］ええええ";
        let ctx = first_occurrence_context(text, "［＃", 4);
        assert!(ctx.contains("［＃"));
    }

    #[test]
    fn first_occurrence_context_reports_missing() {
        assert_eq!(
            first_occurrence_context("plain text", "［＃", 10),
            "<needle missing>"
        );
    }

    #[test]
    fn snap_helpers_are_monotonic() {
        let s = "abcあいう";
        assert_eq!(snap_left(s, 0), 0);
        assert_eq!(snap_right(s, s.len()), s.len());
        assert!(snap_left(s, s.len()) <= s.len());
    }

    #[test]
    fn assert_no_bare_bracket_passes_for_clean_input() {
        assert_no_bare_bracket("<p>plain paragraph</p>");
    }

    #[test]
    #[should_panic(expected = "Tier A")]
    fn assert_no_bare_bracket_panics_on_leak() {
        assert_no_bare_bracket("<p>prefix ［＃改ページ］ suffix</p>");
    }

    #[test]
    fn assert_no_bare_bracket_tolerates_wrapped_occurrences() {
        let html = r#"<p>prefix <span class="aozora-md-directive" hidden>［＃改ページ］</span> suffix</p>"#;
        assert_no_bare_bracket(html);
    }

    // -------------------------------------------------------------------
    // Invariant predicates — unit pinning
    //
    // Test names are prefixed `invariant_unit_` so `just invariants`
    // can filter just these out of the broader suite.
    // -------------------------------------------------------------------

    fn clean_html() -> &'static str {
        r#"<p>hello world</p><div class="aozora-md-container"><p>inside</p></div>"#
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_passes_on_clean_input() {
        check_no_bare_bracket(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_fires_on_leak() {
        let html = "<p>leak ［＃改ページ］ here</p>";
        let Err(Violation::BareBracket { total, .. }) = check_no_bare_bracket(html) else {
            panic!("expected BareBracket violation");
        };
        assert_eq!(total, 1);
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_tolerates_wrapper() {
        let html = r#"<span class="aozora-md-directive" hidden>［＃改ページ］</span>"#;
        check_no_bare_bracket(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_tolerates_code_block_content() {
        // A fenced block's body is the user's bytes verbatim, so notation
        // typed inside it is supposed to reach the output unwrapped.
        let html = "<pre><code>｜青梅《おうめ》\n［＃改ページ］\n</code></pre>";
        check_no_bare_bracket(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_tolerates_an_inline_code_span() {
        // An inline code span restores the literal context just as a fence
        // does — `render_to_string("`可哀想［＃「可哀想」に傍点］`")` emits
        // exactly this, and it is the pinned correct output.
        let html = "<p><code>可哀想［＃「可哀想」に傍点］</code> in a code span</p>";
        check_no_bare_bracket(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_still_fires_outside_a_code_block() {
        // The code carve-out must not blind the predicate to a leak that
        // merely shares a document with a code element.
        let html = "<pre><code>［＃改ページ］\n</code></pre><p>leak ［＃改丁］ here</p>";
        let Err(Violation::BareBracket { total, .. }) = check_no_bare_bracket(html) else {
            panic!("expected BareBracket violation");
        };
        assert_eq!(total, 1, "only the leak outside the code block counts");
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_still_fires_after_an_unclosed_code_tag() {
        // Tier A is the canary that must never go quiet: markup it cannot
        // delimit stays in scope rather than swallowing the rest of the
        // document (as the Tier I scrub deliberately does).
        let html = "<pre><code>x<p>［＃改ページ］</p>";
        check_no_bare_bracket(html).expect_err("unclosed <code must not silence Tier A");
    }

    #[test]
    fn invariant_unit_check_no_sentinel_leak_passes_on_clean_input() {
        check_no_sentinel_leak(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_sentinel_leak_fires_on_each_sentinel() {
        for c in ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
            let html = format!("x{c}y");
            let err = check_no_sentinel_leak(&html).expect_err("must leak");
            assert!(
                matches!(err, Violation::SentinelLeak { codepoint, .. } if codepoint == c),
                "expected SentinelLeak for {c:?}, got {err:?}",
            );
        }
    }

    #[test]
    fn invariant_unit_check_heading_integrity_passes_on_bouten_inside_heading() {
        let html = r#"<h1>本文<em class="aozora-md-bouten aozora-md-bouten-goma aozora-md-bouten-right">強</em></h1>"#;
        check_heading_integrity(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_heading_integrity_fires_on_indent_leak() {
        let html = r#"<h1><span class="aozora-md-indent aozora-md-indent-2" data-amount="2"></span>第一篇</h1>"#;
        let Err(Violation::HeadingContaminated {
            level,
            forbidden_class,
            ..
        }) = check_heading_integrity(html)
        else {
            panic!("expected HeadingContaminated");
        };
        assert_eq!(level, 1);
        assert_eq!(forbidden_class, "aozora-md-indent");
    }

    #[test]
    fn invariant_unit_check_heading_integrity_fires_on_annotation_leak() {
        let html = r#"<h2><span class="aozora-md-directive" hidden>［＃X］</span>第一篇</h2>"#;
        let Err(Violation::HeadingContaminated {
            level,
            forbidden_class,
            ..
        }) = check_heading_integrity(html)
        else {
            panic!("expected HeadingContaminated");
        };
        assert_eq!(level, 2);
        assert_eq!(forbidden_class, "aozora-md-directive");
    }

    #[test]
    fn invariant_unit_check_html_tag_balance_passes_on_clean_input() {
        check_html_tag_balance(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_html_tag_balance_fires_on_unclosed_div() {
        let html = "<p>x</p><div>y";
        let err = check_html_tag_balance(html).expect_err("must fire");
        assert!(matches!(err, Violation::UnbalancedTag(_)));
    }

    #[test]
    fn invariant_unit_check_directive_wrapper_shape_passes_on_well_formed() {
        let html = r#"a <span class="aozora-md-directive" hidden>X</span> b"#;
        check_directive_wrapper_shape(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_directive_wrapper_shape_fires_on_missing_hidden() {
        let html = r#"a <span class="aozora-md-directive">X</span> b"#;
        let err = check_directive_wrapper_shape(html).expect_err("must fire");
        assert!(matches!(err, Violation::DirectiveWrapper { .. }));
    }

    #[test]
    fn invariant_unit_check_directive_wrapper_shape_fires_on_unclosed() {
        let html = r#"a <span class="aozora-md-directive" hidden>X b"#;
        let err = check_directive_wrapper_shape(html).expect_err("must fire");
        assert!(matches!(err, Violation::DirectiveWrapper { .. }));
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_passes_on_clean_input() {
        check_no_xss_marker(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_fires_on_script_tag() {
        let err = check_no_xss_marker("<p><script>x</script></p>").expect_err("must fire");
        assert!(matches!(
            err,
            Violation::XssLeak {
                marker: "<script",
                ..
            }
        ));
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_fires_on_javascript_uri() {
        let err = check_no_xss_marker(r#"<a href="javascript:x">go</a>"#).expect_err("must fire");
        assert!(matches!(
            err,
            Violation::XssLeak {
                marker: "javascript:",
                ..
            }
        ));
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_fires_on_onerror_attr() {
        let err = check_no_xss_marker("<img src=x onerror=alert(1)>").expect_err("must fire");
        assert!(matches!(
            err,
            Violation::XssLeak {
                marker: "on<event>=",
                ..
            }
        ));
    }

    #[test]
    fn invariant_unit_check_css_class_contract_passes_on_known_classes() {
        let html = r#"<div class="aozora-md-container aozora-md-container-indent aozora-md-container-indent-2">x</div>"#;
        check_css_class_contract(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_css_class_contract_accepts_aozora_md_indent_numeric_suffix() {
        let html = r#"<span class="aozora-md-indent aozora-md-indent-3">x</span>"#;
        check_css_class_contract(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_css_class_contract_ignores_non_aozora_md_classes() {
        let html = r#"<pre class="language-rust">let x = 1;</pre>"#;
        check_css_class_contract(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_css_class_contract_fires_on_unknown_aozora_md_class() {
        let html = r#"<span class="aozora-md-mystery-variant">x</span>"#;
        let Err(Violation::UnknownCssClass { class, .. }) = check_css_class_contract(html) else {
            panic!("expected UnknownCssClass");
        };
        assert_eq!(class, "aozora-md-mystery-variant");
    }

    #[test]
    fn invariant_unit_check_escape_invariants_passes_on_single_escape() {
        let html = "<p>&lt;script&gt;</p>";
        check_escape_invariants(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_escape_invariants_fires_on_double_encoded() {
        let html = "<p>&amp;lt;oops&amp;gt;</p>";
        let err = check_escape_invariants(html).expect_err("must fire");
        assert!(matches!(err, Violation::DoubleEncodedEntity { .. }));
    }

    #[test]
    fn invariant_unit_check_content_model_passes_on_ruby_shape() {
        let html = "<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>";
        check_content_model(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_content_model_fires_on_orphan_rt() {
        let html = "<p><rt>orphan</rt></p>";
        let err = check_content_model(html).expect_err("must fire");
        assert!(matches!(err, Violation::ContentModel { .. }));
    }

    #[test]
    fn invariant_unit_check_markup_completeness_passes_on_symmetric_rp() {
        let html = "<ruby>x<rp>(</rp><rt>y</rt><rp>)</rp></ruby>";
        check_markup_completeness(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_markup_completeness_fires_on_missing_close_paren() {
        let html = "<ruby>x<rp>(</rp><rt>y</rt></ruby>";
        let err = check_markup_completeness(html).expect_err("must fire");
        assert!(matches!(err, Violation::MarkupIncomplete { .. }));
    }

    #[test]
    fn invariant_unit_assert_invariants_aggregates_clean_pass() {
        let html = r#"<ruby>青<rp>(</rp><rt>あ</rt><rp>)</rp></ruby><span class="aozora-md-combine-upright">20</span>"#;
        assert_invariants(html).unwrap();
    }

    #[test]
    fn invariant_unit_assert_invariants_collects_multiple_violations() {
        // Bare bracket + unknown class + missing rp in one sample.
        let html =
            r#"<ruby>x<rp>(</rp><rt>y</rt></ruby><span class="aozora-md-unknown">［＃X］</span>"#;
        let violations = assert_invariants(html).expect_err("must fire");
        assert!(!violations.is_empty());
    }

    // -------------------------------------------------------------------
    // Well-formedness validator smoke (inherited from tests/common/mod.rs)
    // -------------------------------------------------------------------

    #[test]
    fn invariant_unit_well_formed_accepts_balanced_doc() {
        assert!(check_well_formed("<p>x<em>y</em></p>").is_empty());
    }

    #[test]
    fn invariant_unit_well_formed_flags_unclosed() {
        let errs = check_well_formed("<p>x");
        assert!(
            errs.iter()
                .any(|e| matches!(e, WellFormedError::UnclosedTag { .. }))
        );
    }

    #[test]
    fn invariant_unit_well_formed_flags_extra_close() {
        let errs = check_well_formed("</p>");
        assert!(
            errs.iter()
                .any(|e| matches!(e, WellFormedError::ExtraClose { .. }))
        );
    }

    #[test]
    fn invariant_unit_well_formed_accepts_void_elements() {
        assert!(check_well_formed("<p>x<br>y<hr></p>").is_empty());
    }
}
