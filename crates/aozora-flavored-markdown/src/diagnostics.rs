//! aozora-flavored-markdown's own diagnostic surface.
//!
//! Two kinds of observation flow out of a render: lexer diagnostics from
//! the upstream `aozora` parser, and host-level ones that aozora-flavored-markdown raises before
//! the lexer ever runs (e.g. oversized input). This module is the single
//! serde-friendly shape both flatten into — the one type the CLI's
//! `aozora-md.diagnostics.v1` envelope and the wasm bridge serialise.
//!
//! Owning the public diagnostic type here (rather than re-exporting
//! `aozora`'s) keeps aozora-flavored-markdown's API decoupled from `aozora`'s `SemVer`, the same
//! way the IR enums and `sentinels` module shield consumers from upstream
//! churn. `aozora::Diagnostic` is mapped in via [`From`].

use serde::Serialize;

/// How strictly a host should treat a [`Diagnostic`]. Serialises to the
/// lowercase wire string (`"error"` / `"warning"` / `"note"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// Genuine error; the parse should be treated as suspect.
    Error,
    /// Recoverable observation; the parse continues and output is kept.
    Warning,
    /// Informational note; does not affect build / CI status.
    Note,
}

/// Origin axis of a [`Diagnostic`]: a user-input issue versus a
/// library-internal invariant violation. Serialises to `"source"` /
/// `"internal"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSource {
    /// The problem traces back to the user-provided source text.
    Source,
    /// A pipeline-internal invariant failed — indicates a library bug.
    Internal,
}

/// Byte-offset range into the source text, end-exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

/// A non-fatal observation about a render.
///
/// Carries the two routing axes ([`Severity`], [`DiagnosticSource`]), a
/// stable machine-readable [`code`](Self::code) (`aozora::lex::…` for
/// upstream lexer diagnostics, `aozora-md::…` for aozora-flavored-markdown host-level ones), a
/// human-readable `message`, and the byte [`Span`] it refers to. Construct
/// from an upstream diagnostic via [`From`]; consumers read the fields.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Diagnostic {
    /// Routing severity.
    pub severity: Severity,
    /// Whether the issue is user-source or library-internal.
    pub source: DiagnosticSource,
    /// Stable machine-readable identifier.
    pub code: &'static str,
    /// Human-readable message. Not part of the stability contract.
    pub message: String,
    /// Byte range in the source text this crate was handed.
    pub span: Span,
}

impl Diagnostic {
    /// aozora-flavored-markdown host-level diagnostic: the input exceeds the lexer's `u32`
    /// span budget (~4 GiB), so nothing was rendered. Raised by the public
    /// entry points before the core lexer is invoked.
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
            span: Span { start: 0, end: 0 },
        }
    }

    /// aozora-flavored-markdown host-level diagnostic: `count` constructs
    /// were recognised but their source text could not be located, so they
    /// were left out of the output.
    ///
    /// This crate renders a construct by handing its source run back to the
    /// parser (see `crate::fragment`). On a document the parser rewrites
    /// before lexing — a decorative rule gaining a blank line, an accent
    /// digraph combining — the ranges it reports address that rewritten
    /// text rather than the caller's, and a run that cannot be recovered
    /// from the caller's own source is dropped rather than guessed at.
    /// Document-scoped: the ranges that would locate the losses are the
    /// ones that could not be trusted in the first place.
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
            span: Span { start: 0, end: 0 },
        }
    }
}

impl From<&aozora::Diagnostic> for Diagnostic {
    fn from(d: &aozora::Diagnostic) -> Self {
        let span = d.span();
        Self {
            severity: d.severity().into(),
            source: d.source().into(),
            code: d.code(),
            message: d.to_string(),
            span: Span {
                start: span.start,
                end: span.end,
            },
        }
    }
}

impl From<aozora::Severity> for Severity {
    fn from(s: aozora::Severity) -> Self {
        // `aozora::Severity` is `#[non_exhaustive]`; a future variant maps
        // to the most conservative routing (`Error`).
        match s {
            aozora::Severity::Warning => Self::Warning,
            aozora::Severity::Note => Self::Note,
            _ => Self::Error,
        }
    }
}

impl From<aozora::DiagnosticSource> for DiagnosticSource {
    fn from(s: aozora::DiagnosticSource) -> Self {
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
        assert_eq!(Severity::from(aozora::Severity::Note), Severity::Note);
    }

    #[test]
    fn internal_source_maps_through_from_upstream() {
        assert_eq!(
            DiagnosticSource::from(aozora::DiagnosticSource::Internal),
            DiagnosticSource::Internal
        );
    }
}
