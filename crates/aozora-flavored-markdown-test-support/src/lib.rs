//! Test predicates and invariant helpers for the aozora-flavored-markdown
//! integration test suite.
//!
//! Each `check_*` predicate codifies one invariant — a lettered HTML tier, or
//! a numbered one the canonicaliser owes — and returns `Result<(), Violation>` so
//! unit tests, property tests, the corpus sweep, and fuzz harnesses compose
//! them on equal footing. [`assert_invariants`] runs the always-on HTML ones.
//!
//! A separate crate (an `aozora-flavored-markdown` dev-dependency) so the
//! predicates and their `proptest` dependency stay out of the production
//! crate's type-check / lint / `cargo doc` / coverage surface.
//!
//! **Tiers H and L have no predicate here, and cannot.** Each bug shape is
//! byte-identical to a legitimate construct, so a shape-only predicate would
//! also fire on valid output: a wrongly promoted `<h2>prose</h2>` reads
//! exactly like a setext heading, and an empty promoted `<hN></hN>` exactly
//! like CommonMark's rendering of `##`. Both are pinned instead where the
//! source — and therefore the intent — is known, in
//! `aozora-flavored-markdown/tests/heading_promotion.rs`.

#![forbid(unsafe_code)]

// The repository's landing README, compiled as a doctest by `just test-doc`.
// It lives here because the crate whose API it demonstrates cannot hold it:
// an `include_str!` reaching outside a package is not carried by
// `cargo publish`, so the library's own include points at its own README now
// (DEV-225). This crate is `publish = false` and has a lib target, which makes
// it the one place in the workspace that can reach the repository root and
// still be a doctest.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct RootReadmeDoctests;

pub mod config;
pub mod generators;

use aozora_flavored_markdown::{classes, sentinels};
use core::error::Error;
use core::fmt;
use std::borrow::Cow;
use std::collections::HashSet;

/// Marks a heading the notation asked for, as opposed to one this crate
/// composed out of parts.
const AOZORA_MD_HEADING: &str = "aozora-md-heading";

// ---------------------------------------------------------------------------
// Rendered-HTML post-processing
// ---------------------------------------------------------------------------

const AOZORA_MD_DIRECTIVE_OPEN: &str = r#"<span class="aozora-md-directive" hidden>"#;
const AOZORA_MD_DIRECTIVE_CLOSE: &str = "</span>";

/// Remove `<span class="aozora-md-directive" hidden>…</span>` wrappers.
/// Idempotent — Tier E leans on that to validate wrapper shape.
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

/// [`check_no_bare_bracket`] for tests that want a panic rather than a
/// `Result`. The tier keeps exactly one definition, `<code>` exception and
/// all.
///
/// # Panics
///
/// On any bare `［＃`, printing the [`Violation::BareBracket`] diagnostic
/// plus the offending HTML.
pub fn assert_no_bare_bracket(html: &str) {
    if let Err(violation) = check_no_bare_bracket(html) {
        panic!("{violation}\n  full html = {html:?}");
    }
}

/// A `±window` snippet around the first `needle`, snapped to UTF-8
/// boundaries so the excerpt is always losslessly printable.
#[must_use]
pub fn first_occurrence_context(haystack: &str, needle: &str, window: usize) -> String {
    let Some(at) = haystack.find(needle) else {
        return "<needle missing>".to_owned();
    };
    context_window(haystack, at, needle.len(), window)
}

/// [`first_occurrence_context`] for predicates whose offending location is
/// structural rather than a substring match (tag balance, heading
/// contamination).
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

/// Round down to the nearest UTF-8 character boundary.
#[must_use]
pub const fn snap_left(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round up to the nearest UTF-8 character boundary.
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

/// One variant per predicate.
///
/// That lets [`assert_invariants`] route diagnostics without losing
/// structure. Snippets stay ≤ ±80 bytes so proptest shrinking does not hold
/// large values in flight during long searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// Tier A — a bare `［＃` leaked outside any `aozora-md-directive` wrapper.
    BareBracket {
        /// Byte offset of the first leak, into the stripped HTML.
        first_offset: usize,
        /// Context around that offset.
        snippet: String,
        /// How many leaked in all, so a shrunk case still reports the scale.
        total: usize,
    },
    /// Tier B — a PUA sentinel (U+E000–U+E004) reached the rendered HTML.
    SentinelLeak {
        /// Which sentinel from `sentinels::ALL` survived.
        codepoint: char,
        /// Byte offset of the first occurrence, into the HTML.
        first_offset: usize,
        /// Context around that offset.
        snippet: String,
    },
    /// Tier C — a heading (`<h1>`–`<h6>`) body contains a forbidden class.
    HeadingContaminated {
        /// Which heading level, 1 through 6.
        level: u8,
        /// The class token that must not appear in a heading body.
        forbidden_class: String,
        /// Context around the offending heading.
        snippet: String,
    },
    /// Tier D — a tag-balance violation from [`check_well_formed`].
    UnbalancedTag(
        /// The first imbalance the scan found.
        WellFormedError,
    ),
    /// Tier E — the `aozora-md-directive` wrapper shape is malformed.
    DirectiveWrapper {
        /// Which wrapper rule was broken.
        violation: &'static str,
        /// Context around the offending wrapper.
        snippet: String,
    },
    /// Tier F — an XSS marker leaked into the HTML.
    XssLeak {
        /// The marker the fixture planted, found unescaped.
        marker: &'static str,
        /// Byte offset of the first occurrence, into the HTML.
        first_offset: usize,
        /// Context around that offset.
        snippet: String,
    },
    /// Tier G — an `aozora-md-*` class token the library's
    /// [`classes::is_known`] does not recognise.
    UnknownCssClass {
        /// The unrecognised class token.
        class: String,
        /// Context around the element carrying it.
        snippet: String,
    },
    /// Tier I — a double-encoded HTML entity (e.g. `&amp;lt;`) slipped in.
    DoubleEncodedEntity {
        /// Context around the double-encoded entity.
        snippet: String,
    },
    /// Tier J — HTML content-model violation (orphan `<rt>`, `<rp>`, …).
    ContentModel {
        /// Which content-model rule was broken.
        violation: &'static str,
        /// Context around the offending element.
        snippet: String,
    },
    /// Tier K — `<ruby>` element missing its `<rp>(</rp>` ↔ `<rp>)</rp>` pair.
    MarkupIncomplete {
        /// Which half of the pair is missing.
        violation: &'static str,
        /// Context around the incomplete `<ruby>`.
        snippet: String,
    },
    /// I5 — `canonicalize` rewrote the interior of a fenced code block.
    FenceRewritten {
        /// The fence body as it was written, which had to survive verbatim.
        interior: String,
        /// Context around the rewritten fence.
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
                "Tier G: unknown CSS class `{class}` (classes::is_known says no): {snippet}",
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
            Self::FenceRewritten { interior, snippet } => write!(
                f,
                "I5: fenced code interior {interior:?} did not survive canonicalize: {snippet}",
            ),
        }
    }
}

impl Error for Violation {}

// ---------------------------------------------------------------------------
// Predicates — one per tier
// ---------------------------------------------------------------------------

/// Tier A — no bare `［＃` outside `aozora-md-directive` wrappers.
///
/// **`<code>` regions are excepted**, fenced and inline alike: a code body
/// is the user's bytes verbatim (CommonMark §6.1), so notation typed inside
/// one *must* surface unwrapped — that is the renderer restoring a literal
/// context, not leaking an unparsed construct. The exception stops short of
/// a blanket skip: an unclosed `<code` keeps the rest of the document in
/// scope, so a leak following malformed markup still fires.
///
/// # Errors
///
/// [`Violation::BareBracket`] when a bare `［＃` survives the strip.
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

/// Tier B — rendered HTML carries no codepoint from [`sentinels::ALL`].
///
/// **Not gated on a clean parse.** An author who types U+E001 does not get
/// one back — the parser reports it *and* overwrites it with U+FFFD, so a
/// construct sentinel in the output was substituted and never resolved: a
/// bug on any input, most of all on one that also produced diagnostics.
/// The mask is the member that argument misses — masking bails out on a
/// source already carrying one, so `src` tells a leak from the author's
/// own byte.
///
/// # Errors
///
/// [`Violation::SentinelLeak`] naming the offending codepoint.
pub fn check_no_sentinel_leak(src: &str, html: &str) -> Result<(), Violation> {
    // Single source of truth: the substitution is the library's, so the
    // set it publishes is what a leak is measured against. A sentinel added
    // there flows in here automatically instead of silently going unchecked
    // against a hardcoded U+E001..U+E004 copy.
    for &c in &sentinels::ALL {
        if c == sentinels::MASK && src.contains(c) {
            continue;
        }
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

/// Tier C — `<h1>`–`<h6>` bodies carry no indent marker or raw-directive
/// wrapper. Other Aozora markup (bouten, gaiji, tcy, kaeriten) is fine.
///
/// **A heading the parser rendered whole is exempt.**
/// `［＃中見出し］…［＃中見出し終わり］` arrives as one fragment whose body is
/// whatever the parser put there — including the hidden wrapper an editor's
/// note renders to. That is not this crate composing a heading out of parts,
/// which is what the tier is written against; it is held to the parser's own
/// output by `tests/aozora_parity.rs` instead. `aozora-md-heading` is how
/// such a heading is told apart from a markdown one.
///
/// # Errors
///
/// [`Violation::HeadingContaminated`] on the first offending heading.
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

/// Tier D — every open tag has a matching close tag (void elements
/// exempted).
///
/// # Errors
///
/// [`Violation::UnbalancedTag`] carrying the first [`WellFormedError`].
pub fn check_html_tag_balance(html: &str) -> Result<(), Violation> {
    let errors = check_well_formed(html);
    if let Some(first) = errors.into_iter().next() {
        return Err(Violation::UnbalancedTag(first));
    }
    Ok(())
}

/// Tier E — every `<span class="aozora-md-directive" hidden>` closes, never
/// nests, and carries `hidden`.
///
/// # Errors
///
/// [`Violation::DirectiveWrapper`] naming the specific shape violation.
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

/// Tier F — no XSS marker reaches the HTML as an *executable* construct.
///
/// * `<script` needs no tag-context test: text content always escapes `<` to
///   `&lt;` and comrak's default suppresses raw-HTML passthrough, so the
///   literal can only appear if we emitted the tag ourselves.
/// * `javascript:` and `on<event>=` are required to sit inside a tag body,
///   the only position a browser acts on. Both read as harmless prose
///   elsewhere — a tutorial discussing JS URIs, `onerror=alert(1)` inside a
///   hidden directive wrapper.
///
/// # Errors
///
/// [`Violation::XssLeak`] naming the detected marker.
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
/// Recognised is [`classes::is_known`] and nothing looser: a family rule of
/// this checker's own is how a library predicate that rejected
/// `aozora-md-indent-2` stayed invisible to every gate calling this.
///
/// `<pre><code>` regions are stripped first: a user-supplied info string
/// surfaces as `class="language-X"` for arbitrary `X`, and that attribute is
/// the user's, not ours.
///
/// # Errors
///
/// [`Violation::UnknownCssClass`] on the first unrecognised class.
pub fn check_css_class_contract(html: &str) -> Result<(), Violation> {
    let scope = strip_pre_code_blocks(html);
    let tokens = collect_class_tokens(&scope);
    for token in &tokens {
        if !token.starts_with("aozora-md-") {
            continue;
        }
        if classes::is_known(token) {
            continue;
        }
        return Err(Violation::UnknownCssClass {
            class: token.clone(),
            snippet: first_occurrence_context(&scope, token, 80),
        });
    }
    Ok(())
}

/// Tier I — no double-encoded HTML entities (`&amp;lt;` and friends, each
/// meaning the escape pass ran twice).
///
/// **Code blocks are excluded**: a `<pre><code>` body holds the user's bytes
/// verbatim, so one that carried `&amp;` MUST surface as `&amp;amp;` once
/// escaped. That is the spec, not a double-encode bug.
///
/// # Errors
///
/// [`Violation::DoubleEncodedEntity`] at the first offender.
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

/// I5 — `canonicalize` reproduces every fenced code interior byte for byte.
///
/// # Errors
///
/// [`Violation::FenceRewritten`] on the first one the lexer rewrote instead.
pub fn check_fence_fidelity(src: &str, out: &str) -> Result<(), Violation> {
    for interior in fence_interiors(src) {
        if !out.contains(interior) {
            // Anchored on whichever marker survived, so the excerpt lands on
            // the offending fence rather than on the head of the document.
            let anchor = if out.contains("```") { "```" } else { "~~~" };
            return Err(Violation::FenceRewritten {
                interior: interior.to_owned(),
                snippet: first_occurrence_context(out, anchor, 80),
            });
        }
    }
    Ok(())
}

// Unconditional: whatever a fence encloses — CRLF, a run of blank lines, a
// decorative rule row, a PUA codepoint — is the author's byte and comes back
// as written. The carve-outs this once had were all line structure, which a
// character mask cannot reach and lifting the region out whole does.
fn fence_interiors(src: &str) -> Vec<&str> {
    let mut interiors = Vec::new();
    let mut open: Option<(u8, usize, usize)> = None;
    let mut pos = 0;
    for line in src.split_inclusive('\n') {
        let end = pos + line.len();
        match open {
            // A raw-HTML block swallows the lines after it for a span only a
            // block parser can measure, so a fence read past one would be
            // prose. Stop reading rather than guess where the block ends.
            None if line.trim_start().starts_with('<') => return interiors,
            None => open = fence_open(line).map(|(marker, width)| (marker, width, end)),
            Some((marker, width, start)) if fence_close(line, marker, width) => {
                if start < pos {
                    interiors.push(&src[start..pos]);
                }
                open = None;
            }
            // An unterminated fence is dropped rather than run to EOF: its
            // tail is where the trailing-newline trim lands.
            Some(_) => {}
        }
        pos = end;
    }
    interiors
}

// Deliberately a second implementation of the fence rules rather than a call
// into the library: a checker sharing the code under test cannot fail when
// that code is wrong. Up to three spaces of indent, three or more markers,
// tabs excluded. Column-anchored, so a fence behind a container prefix is
// simply not read here — deciding one needs the block parser this is not, and
// under-reading costs a case rather than inventing one.
fn fence_open(line: &str) -> Option<(u8, usize)> {
    let stripped = trim_fence_indent(line);
    let bytes = stripped.as_bytes();
    let &first = bytes.first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let width = bytes.iter().take_while(|&&b| b == first).count();
    // CommonMark §4.5: a backtick fence's info string may hold no backtick, so
    // a line like ```` ```a`b ```` opens nothing and its "interior" is prose.
    if first == b'`' && bytes[width..].contains(&b'`') {
        return None;
    }
    (width >= 3).then_some((first, width))
}

fn fence_close(line: &str, marker: u8, width: usize) -> bool {
    let bytes = trim_fence_indent(line).as_bytes();
    let run = bytes.iter().take_while(|&&b| b == marker).count();
    run >= width
        && bytes[run..]
            .iter()
            .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

fn trim_fence_indent(line: &str) -> &str {
    let consumed = line.bytes().take(3).take_while(|&b| b == b' ').count();
    &line[consumed..]
}

/// Every always-on invariant, with a panic message shaped so a libFuzzer
/// crash artifact reads as "tier + source + html" without manual triage.
///
/// Tier A is deliberately left out: a bare `［＃` is legitimate output for a
/// source whose bracket pairing is malformed, so its precondition belongs to
/// the caller. Tier I is gated on [`source_contains_html_entity_literal`],
/// and [`check_no_sentinel_leak`] reads `src` for its own carve-out. Every
/// other tier here holds on arbitrary bytes.
///
/// # Panics
///
/// On the first invariant violation.
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
    if let Err(e) = check_no_sentinel_leak(src, html) {
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

/// True iff `src` would let comrak's escape pass legitimately surface a
/// `&amp;{lt,gt,amp,…};`. Callers gate Tier I on the negation.
///
/// Two spec-correct causes of an apparent double-encode:
///
/// 1. an entity literal in the source — the escape pass turns its leading
///    `&` into `&amp;` and preserves the rest.
/// 2. a PUA sentinel in the source. The lexer flags it, but the splicer
///    still walks the resulting Text nodes; the sentinel splits the text
///    around itself and the leading half can collapse to a `&` adjacent to a
///    literal `amp;`, which comrak escapes.
///
/// Deliberately **permissive**: a missed double-escape beats wedging fuzz on
/// comrak-correct output. Tier I targets our own regressions, not a forensic
/// accounting of every entity.
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
/// Malformed markup has no right answer, so each caller picks its own error.
#[derive(Clone, Copy)]
enum Unclosed {
    /// Rather miss a violation than report one against undelimitable markup.
    Swallow,
    /// Rather report.
    Retain,
}

/// Substring scanning, not parsing: these predicates read renderer output,
/// where the tags of interest are emitted in one fixed shape.
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

/// An unclosed block swallows the remainder, so a stray `&amp;amp;` behind
/// it cannot false-positive Tier I.
fn strip_pre_code_blocks(html: &str) -> Cow<'_, str> {
    strip_regions(html, "<pre><code", "</code></pre>", Unclosed::Swallow)
}

/// Fenced and inline code alike. Unlike [`strip_pre_code_blocks`] an
/// unclosed `<code` retains the remainder — Tier A is the canary that must
/// not go quiet.
fn strip_code_regions(html: &str) -> Cow<'_, str> {
    strip_regions(html, "<code", "</code>", Unclosed::Retain)
}

/// Tier J — every `<rt>` and `<rp>` sits inside an open `<ruby>`. A stray
/// one means a fragment was emitted without its containing element, e.g. a
/// post-process bug that detached the base.
///
/// # Errors
///
/// [`Violation::ContentModel`] naming the orphaned element.
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

/// Tier K — every `<ruby>` that opens with `<rp>(</rp>` also closes with
/// `<rp>)</rp>`. The renderer always emits both, so an asymmetric shape is a
/// render bug.
///
/// # Errors
///
/// [`Violation::MarkupIncomplete`] describing the missing half.
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

/// Every predicate, with all diagnostics collected. For fixture and corpus
/// callers that only need pass/fail; property tests wanting one red per
/// failing invariant should call the predicates individually.
///
/// # Errors
///
/// Every violation found.
pub fn assert_invariants(src: &str, html: &str) -> Result<(), Vec<Violation>> {
    type Predicate = fn(&str) -> Result<(), Violation>;
    let predicates: &[Predicate] = &[
        check_no_bare_bracket,
        check_heading_integrity,
        check_html_tag_balance,
        check_directive_wrapper_shape,
        check_no_xss_marker,
        check_css_class_contract,
        check_escape_invariants,
        check_content_model,
        check_markup_completeness,
    ];
    let violations: Vec<_> = check_no_sentinel_leak(src, html)
        .err()
        .into_iter()
        .chain(predicates.iter().filter_map(|p| p(html).err()))
        .collect();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

// ---------------------------------------------------------------------------
// Shared predicate helpers
// ---------------------------------------------------------------------------

/// **Tag-boundary aware**, because a naive `find("class=\"")` also matches
/// body text — an `<img alt="…class=…">` carries the sequence harmlessly.
/// Quoted attribute values are respected so a `>` inside a quote does not
/// close the tag early.
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

/// Only ASCII is folded, so non-ASCII stays byte-comparable.
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

/// Matches `on[a-z]+ *=` only between `<` and `>`; the same pattern in prose
/// ("use onerror= to hook errors") is harmless and must not fire.
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

/// `near` snippets surface ±48 characters around the offending offset, so a
/// failure message is actionable without a full dump of the rendered HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WellFormedError {
    /// An element opened and the document ended with it still open.
    UnclosedTag {
        /// Name of the tag left open.
        tag: String,
        /// Context around where it opened.
        near: String,
    },
    /// A closing tag arrived with nothing open to close.
    ExtraClose {
        /// Name of the tag being closed.
        tag: String,
        /// Context around the stray close.
        near: String,
    },
    /// Elements closed out of order, so the nesting crosses over.
    MisorderedClose {
        /// The innermost tag still open.
        opened: String,
        /// The tag that tried to close over it.
        closed: String,
        /// Context around the crossing.
        near: String,
    },
    /// The scanner met a `<` it could not read as a tag at all.
    MalformedTag {
        /// Context around the unreadable markup.
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

/// Empty means balanced. Single forward pass; the open-tag stack costs
/// O(depth).
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
        // does — `to_html("`可哀想［＃「可哀想」に傍点］`")` emits
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
        check_no_sentinel_leak("clean source", clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_sentinel_leak_fires_on_each_sentinel() {
        for c in sentinels::ALL {
            let html = format!("x{c}y");
            let err = check_no_sentinel_leak("plain source", &html).expect_err("must leak");
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
    fn invariant_unit_check_css_class_contract_fires_on_an_unlisted_slug_on_a_listed_stem() {
        // The slack this checker used to hold: `<listed stem>-<anything>`
        // passed, so Tier G could not tell a real slug family member
        // (`aozora-md-bouten-goma`, which the parser lists in full) from a
        // token no renderer emits. The parser's list carries every slug
        // verbatim and only the numeric variants by stem, so the numeric rule
        // in `classes::is_known` is the whole of what a suffix may mean.
        let html = r#"<em class="aozora-md-bouten aozora-md-bouten-zzq">x</em>"#;
        let Err(Violation::UnknownCssClass { class, .. }) = check_css_class_contract(html) else {
            panic!("expected UnknownCssClass for a slug the parser does not publish");
        };
        assert_eq!(class, "aozora-md-bouten-zzq");
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
    fn invariant_unit_check_fence_fidelity_passes_when_interior_survives() {
        let src = "```\n｜青梅《おうめ》\n```\n";
        check_fence_fidelity(src, src).unwrap();
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_fires_on_a_canonicalised_interior() {
        // Exactly what an unmasked `canonicalize` used to return.
        let src = "```\n｜青梅《おうめ》\n```\n";
        let err = check_fence_fidelity(src, "```\n青梅《おうめ》\n```\n").expect_err("must fire");
        assert!(matches!(err, Violation::FenceRewritten { .. }));
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_ignores_prose_outside_the_fence() {
        // The one rewrite `canonicalize` is *supposed* to make.
        let src = "｜青梅《おうめ》\n\n```\n｜奥多摩《おくたま》\n```\n";
        let out = "青梅《おうめ》\n\n```\n｜奥多摩《おくたま》\n```\n";
        check_fence_fidelity(src, out).unwrap();
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_skips_an_unterminated_fence() {
        let src = "```\n｜青梅《おうめ》\n";
        check_fence_fidelity(src, "```\n青梅《おうめ》\n").unwrap();
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_reads_what_it_used_to_carve_out() {
        // CRLF, a 3+ newline run, a PUA codepoint and a decorative rule row
        // were each excused while the fence interior was protected character
        // by character. Every one of them is line structure, and every one of
        // them is now the author's byte like any other.
        for src in [
            "```\r\n｜青梅《おうめ》\r\n```\r\n",
            "```\n｜青梅《おうめ》\n\n\nx\n```\n",
            "\u{E000}\n```\n｜青梅《おうめ》\n```\n",
            "```\n｜青梅《おうめ》\n------------\n```\n",
        ] {
            let err = check_fence_fidelity(src, "").expect_err("must fire");
            assert!(matches!(err, Violation::FenceRewritten { .. }), "{src:?}");
        }
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_reads_every_fence_in_the_document() {
        let src = "```\n------------\n```\n\n```\n｜青梅《おうめ》\n```\n";
        let err = check_fence_fidelity(src, "").expect_err("must fire");
        assert!(matches!(err, Violation::FenceRewritten { .. }));
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_reads_no_fence_inside_a_raw_html_block() {
        // Read past a raw-HTML line and the scanner pairs a marker the block
        // swallowed with a real one further down, calling the prose between
        // them an interior — prose `canonicalize` is *supposed* to canonicalise,
        // so a correct output would read as a violation.
        let src = "<div>\n```\n</div>\n\n｜青梅《おうめ》\n\n```\n";
        let out = "<div>\n```\n</div>\n\n青梅《おうめ》\n\n```\n";
        check_fence_fidelity(src, out).unwrap();
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_reads_no_fence_from_a_backticked_info_string() {
        // CommonMark §4.5: a backtick fence's info string may hold no
        // backtick, so this line opens nothing and what follows is prose.
        let src = "```a`b\n｜青梅《おうめ》\n```\n";
        let out = "```a`b\n青梅《おうめ》\n```\n";
        check_fence_fidelity(src, out).unwrap();
    }

    #[test]
    fn invariant_unit_check_fence_fidelity_ignores_a_container_nested_fence() {
        // Not an excuse for `canonicalize`, which does hold it byte for byte: a
        // column-anchored scanner cannot tell a fence behind a blockquote
        // marker from a lazy continuation, so it reads neither.
        let src = "> ```\n> ｜青梅《おうめ》\n> ```\n";
        check_fence_fidelity(src, "> ```\n> 青梅《おうめ》\n> ```\n").unwrap();
    }

    #[test]
    fn invariant_unit_assert_invariants_aggregates_clean_pass() {
        let html = r#"<ruby>青<rp>(</rp><rt>あ</rt><rp>)</rp></ruby><span class="aozora-md-combine-upright">20</span>"#;
        assert_invariants("青《あ》20", html).unwrap();
    }

    #[test]
    fn invariant_unit_assert_invariants_collects_multiple_violations() {
        // Bare bracket + unknown class + missing rp in one sample.
        let html =
            r#"<ruby>x<rp>(</rp><rt>y</rt></ruby><span class="aozora-md-unknown">［＃X］</span>"#;
        let violations = assert_invariants("x《y》［＃X］", html).expect_err("must fire");
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
