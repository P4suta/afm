//! The one serde-friendly shape both upstream lexer diagnostics and this
//! crate's own host-level ones flatten into — what the CLI's
//! `aozora-md.diagnostics.v1` envelope and the wasm bridge serialise.
//!
//! Owning the type rather than re-exporting `aozora::Diagnostic` decouples
//! this crate's API from the parser's `SemVer`, as the IR enums and
//! `sentinels` do. Upstream maps in through a crate-private constructor —
//! a public `From` would put the parser's type back in this one's surface.

use core::ops::Range;

use serde::Serialize;

/// Serialises to `"error"` / `"warning"` / `"note"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
// A level added upstream arrives here as a new variant; `#[non_exhaustive]`
// keeps that additive for external `match`es, the same bargain the IR enums
// take (ADR-0013).
#[non_exhaustive]
pub enum Severity {
    /// The parse should be treated as suspect.
    Error,
    /// The parse continues and its output is kept.
    Warning,
    /// Does not affect build / CI status.
    Note,
}

/// Serialises to `"source"` / `"internal"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
// As `Severity`: an origin added upstream must not break external `match`es.
#[non_exhaustive]
pub enum DiagnosticSource {
    /// Traces back to the user-provided source text.
    Source,
    /// A pipeline invariant failed — a library bug.
    Internal,
}

/// Byte-offset range into the source text, end-exclusive.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
// Deliberately NOT `#[non_exhaustive]`, unlike every other public struct
// here: a span is geometrically closed at start + end, so sealing it would
// only cost every consumer literal construction and functional record update
// for a field set that cannot grow. `lsp_types::Position`,
// `miette::SourceSpan` and `proc_macro2::LineColumn` all make the same call.
pub struct Span {
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

impl Span {
    /// No ordering is imposed on the pair; a reversed one reads as empty.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Saturating, so a reversed span measures 0 rather than wrapping.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// True of the `0..0` a document-scoped diagnostic carries.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

impl From<Span> for Range<usize> {
    /// Slices the source the span was measured against: `&source[span.into()]`.
    fn from(span: Span) -> Self {
        span.start as usize..span.end as usize
    }
}

/// A non-fatal observation about a render.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Diagnostic {
    /// How strictly a host should treat this.
    pub severity: Severity,
    /// Whether it blames the user's text or this library.
    pub source: DiagnosticSource,
    /// Stable identifier: `aozora::lex::…` upstream, `aozora-md::…` here.
    pub code: &'static str,
    /// **Not** part of the stability contract.
    pub message: String,
    /// Byte range in the source text this crate was handed.
    pub span: Span,
}

impl Diagnostic {
    /// Raised by the public entry points, before the core lexer would
    /// assert on the same boundary and abort.
    #[must_use]
    pub(crate) fn source_too_large(bytes: usize) -> Self {
        Self {
            severity: Severity::Error,
            source: DiagnosticSource::Source,
            code: "aozora-md::source_too_large",
            message: format!(
                "source is {bytes} bytes, over the {} byte (u32 span) limit; nothing was rendered",
                u32::MAX
            ),
            span: Span::new(0, 0),
        }
    }

    /// `count` constructs were recognised but could not be located in the
    /// caller's own text, so they were dropped rather than guessed at — see
    /// `crate::fragment`. Document-scoped, because the ranges that would
    /// locate the losses are exactly the ones that could not be trusted.
    #[must_use]
    pub(crate) fn constructs_unresolved(count: usize) -> Self {
        Self {
            severity: Severity::Warning,
            source: DiagnosticSource::Source,
            code: "aozora-md::constructs_unresolved",
            message: format!(
                "{count} 青空文庫 construct(s) could not be located in the source \
                 and were left out of the output"
            ),
            span: Span::new(0, 0),
        }
    }
}

impl Diagnostic {
    /// Flatten one upstream lexer observation into this crate's shape.
    pub(crate) fn from_upstream(d: &aozora::Diagnostic) -> Self {
        let span = d.span();
        Self {
            severity: Severity::from_upstream(d.severity()),
            source: DiagnosticSource::from_upstream(d.source()),
            code: d.code(),
            message: d.to_string(),
            span: Span::new(span.start, span.end),
        }
    }
}

impl Severity {
    pub(crate) fn from_upstream(s: aozora::Severity) -> Self {
        // `aozora::Severity` is `#[non_exhaustive]`; a future variant maps
        // to the most conservative routing (`Error`).
        match s {
            aozora::Severity::Warning => Self::Warning,
            aozora::Severity::Note => Self::Note,
            _ => Self::Error,
        }
    }
}

impl DiagnosticSource {
    pub(crate) fn from_upstream(s: aozora::DiagnosticSource) -> Self {
        // `aozora::DiagnosticSource` is `#[non_exhaustive]`; an unknown
        // future variant is treated as a source-side issue.
        match s {
            aozora::DiagnosticSource::Internal => Self::Internal,
            _ => Self::Source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_too_large_is_an_error_carrying_the_byte_counts() {
        let d = Diagnostic::source_too_large(5_000_000_000);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.source, DiagnosticSource::Source);
        assert_eq!(d.code, "aozora-md::source_too_large");
        assert_eq!(d.span, Span { start: 0, end: 0 });
        assert!(d.message.contains("5000000000"), "got: {}", d.message);
        assert!(
            d.message.contains(&u32::MAX.to_string()),
            "got: {}",
            d.message
        );
    }

    #[test]
    fn note_severity_maps_through_from_upstream() {
        assert_eq!(
            Severity::from_upstream(aozora::Severity::Note),
            Severity::Note
        );
    }

    #[test]
    fn internal_source_maps_through_from_upstream() {
        assert_eq!(
            DiagnosticSource::from_upstream(aozora::DiagnosticSource::Internal),
            DiagnosticSource::Internal
        );
    }
}
