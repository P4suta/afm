//! The one serde-friendly shape both upstream lexer diagnostics and this
//! crate's own host-level ones flatten into — what the CLI's
//! `aozora-md.diagnostics.v1` envelope and the wasm bridge serialise.
//!
//! Owning the type rather than re-exporting `aozora::Diagnostic` decouples
//! this crate's API from the parser's `SemVer`, as the IR enums and
//! `sentinels` do. Upstream maps in through a crate-private constructor —
//! a public `From` would put the parser's type back in this one's surface.

use core::error::Error as StdError;
use core::fmt;
#[cfg(feature = "miette")]
use core::iter;
use core::ops::Range;
use std::borrow::Cow;

/// Serialises to `"error"` / `"warning"` / `"note"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
// As `Severity`: an origin added upstream must not break external `match`es.
#[non_exhaustive]
pub enum DiagnosticSource {
    /// Traces back to the user-provided source text.
    Source,
    /// A pipeline invariant failed — a library bug.
    Internal,
}

/// Byte-offset range into the source text, end-exclusive.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
// Deliberately NOT `#[non_exhaustive]`, unlike every other public struct
// here: a span is geometrically closed at start + end, so sealing it would
// only cost every consumer literal construction and functional record update
// for a field set that cannot grow. `lsp_types::Position`,
// `miette::SourceSpan` and `proc_macro2::LineColumn` all make the same call.
pub struct ByteSpan {
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

impl ByteSpan {
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

impl From<ByteSpan> for Range<usize> {
    /// Slices the source the span was measured against: `&source[span.into()]`.
    fn from(span: ByteSpan) -> Self {
        span.start as usize..span.end as usize
    }
}

/// A non-fatal observation about a render; `Display` is the message alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub struct Diagnostic {
    // Private, and read through the accessors below. These names are still the
    // wire format — the `aozora-md.diagnostics.v1` envelope and the `.d.ts`
    // both come off this derive — so what moves here is the representation,
    // not the format: every `code` this crate builds is a `&'static str`, and
    // one read back off the wire has to own its bytes.
    severity: Severity,
    source: DiagnosticSource,
    code: Cow<'static, str>,
    message: String,
    span: ByteSpan,
}

// Three accessors below share a name with a method of a trait this type
// implements: `Error::source`, and miette's `code` / `severity`. Those exist to
// be *rendered* — they answer in `Option<Box<dyn Display>>` and in miette's own
// severity scale — and an inherent method wins name resolution, so a call site
// gets the typed answer and has nothing to disambiguate. Renaming them would
// put the accessors out of step with the field names, which are the
// `aozora-md.diagnostics.v1` wire names (ADR-0012), and with the sibling
// parser's `Diagnostic`, which resolves the same collision the same way.
// `expect` rather than `allow`: this fails the build if a trait impl leaves.
#[expect(
    clippy::same_name_method,
    reason = "the inherent accessors return this crate's own types; the same-named trait methods exist for miette's renderer and are never called by name"
)]
impl Diagnostic {
    /// How strictly a host should treat this.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Whether it blames the user's text or this library.
    #[must_use]
    pub const fn source(&self) -> DiagnosticSource {
        self.source
    }

    /// Stable identifier: `aozora::lex::…` upstream, `aozora-md::…` here.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// **Not** part of the stability contract.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Byte range in the source text this crate was handed.
    #[must_use]
    pub const fn span(&self) -> ByteSpan {
        self.span
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

// Nothing here wraps a lower-level failure, so the cause chain stays empty.
// The origin axis a reader might look for on `Error::source` is
// `Diagnostic::source`, which answers a different question.
impl StdError for Diagnostic {}

/// A diagnostic bound to the exact source text its byte span addresses.
///
/// Construct this through [`Diagnostic::bind_source`]. Unlike a bare
/// [`Diagnostic`], this is a `miette::Diagnostic`: the source and its
/// validated byte range cannot be supplied in separate steps.
#[cfg(feature = "miette")]
#[derive(Debug, Clone)]
pub struct SourceBoundDiagnostic {
    diagnostic: Diagnostic,
    source: miette::NamedSource<String>,
}

/// Why a [`Diagnostic`] could not be bound to a source.
#[cfg(feature = "miette")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum DiagnosticBindError {
    /// A source diagnostic named bytes outside the supplied UTF-8 source, in
    /// reverse order, or through the middle of a scalar value.
    #[error(
        "diagnostic byte span {span_start}..{span_end} does not address the supplied {source_len}-byte UTF-8 source"
    )]
    InvalidByteSpan {
        /// Inclusive diagnostic start byte.
        span_start: u32,
        /// Exclusive diagnostic end byte.
        span_end: u32,
        /// Length of the supplied source, in bytes.
        source_len: usize,
    },
}

#[cfg(feature = "miette")]
impl SourceBoundDiagnostic {
    /// The diagnostic whose source was bound.
    #[must_use]
    pub const fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    /// Display name attached to the source.
    #[must_use]
    pub fn source_name(&self) -> &str {
        self.source.name()
    }

    /// Exact source text the diagnostic addresses.
    #[must_use]
    pub fn source_text(&self) -> &str {
        self.source.inner()
    }
}

#[cfg(feature = "miette")]
impl fmt::Display for SourceBoundDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.diagnostic.fmt(f)
    }
}

#[cfg(feature = "miette")]
impl StdError for SourceBoundDiagnostic {}

#[cfg(feature = "miette")]
impl miette::Diagnostic for SourceBoundDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(&self.diagnostic.code))
    }

    fn severity(&self) -> Option<miette::Severity> {
        // miette's three levels are Advice / Warning / Error — there is no
        // `Note`, so the quietest one takes it.
        Some(match self.diagnostic.severity {
            Severity::Error => miette::Severity::Error,
            Severity::Warning => miette::Severity::Warning,
            Severity::Note => miette::Severity::Advice,
        })
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        Some(&self.source)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        // An `Internal` diagnostic blames this library rather than a range of
        // the caller's text, and the `0..0` a document-scoped one carries
        // points at nothing — both render as header plus message.
        if self.diagnostic.source != DiagnosticSource::Source || self.diagnostic.span.is_empty() {
            return None;
        }
        Some(Box::new(iter::once(miette::LabeledSpan::new(
            None,
            self.diagnostic.span.start as usize,
            self.diagnostic.span.len() as usize,
        ))))
    }
}

#[cfg(feature = "miette")]
impl Diagnostic {
    /// Bind this diagnostic to the exact source text it was produced from.
    ///
    /// Non-empty source spans are checked for ordering, bounds, and UTF-8
    /// scalar boundaries before a miette report can be constructed.
    pub fn bind_source(
        &self,
        name: impl AsRef<str>,
        source: impl Into<String>,
    ) -> Result<SourceBoundDiagnostic, DiagnosticBindError> {
        let source = source.into();
        let start = self.span.start as usize;
        let end = self.span.end as usize;
        let valid = self.source != DiagnosticSource::Source
            || (start <= end
                && end <= source.len()
                && source.is_char_boundary(start)
                && source.is_char_boundary(end));
        if !valid {
            return Err(DiagnosticBindError::InvalidByteSpan {
                span_start: self.span.start,
                span_end: self.span.end,
                source_len: source.len(),
            });
        }
        Ok(SourceBoundDiagnostic {
            diagnostic: self.clone(),
            source: miette::NamedSource::new(name, source),
        })
    }
}

impl Diagnostic {
    /// Raised by the public entry points, before the core lexer would
    /// assert on the same boundary and abort.
    #[must_use]
    pub(crate) fn source_too_large(bytes: usize) -> Self {
        Self {
            severity: Severity::Error,
            source: DiagnosticSource::Source,
            code: Cow::Borrowed("aozora-md::source_too_large"),
            message: format!(
                "source is {bytes} bytes, over the {} byte (u32 span) limit; nothing was rendered",
                u32::MAX
            ),
            span: ByteSpan::new(0, 0),
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
            code: Cow::Borrowed("aozora-md::constructs_unresolved"),
            message: format!(
                "{count} 青空文庫 construct(s) could not be located in the source \
                 and were left out of the output"
            ),
            span: ByteSpan::new(0, 0),
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
            code: Cow::Borrowed(d.code()),
            message: d.to_string(),
            span: ByteSpan::new(span.start, span.end),
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

// The mapping below used to be the CLI's, in a `CliDiagnostic` under
// `main.rs` — a file the coverage floor excludes and that no test could reach
// except through miette's rendered text. Two of its branches were observed by
// nothing: the quietest level, and the decision not to claim a caret. Both
// are library code now, so both are checked here.
#[cfg(all(test, feature = "miette"))]
mod miette_impl {
    use miette::{Diagnostic as MietteDiagnostic, LabeledSpan, Severity as ReportLevel};

    use super::{ByteSpan, Cow, Diagnostic, DiagnosticSource, Severity};

    // The two real constructors are both document-scoped, so the
    // origin/level/span square is reached by literal — which inside the
    // defining crate is what `#[non_exhaustive]` still allows.
    fn probe(severity: Severity, source: DiagnosticSource, span: ByteSpan) -> Diagnostic {
        Diagnostic {
            severity,
            source,
            code: Cow::Borrowed("aozora-md::probe"),
            message: "probe".to_owned(),
            span,
        }
    }

    #[test]
    fn the_three_levels_land_on_three_distinct_miette_levels_in_the_same_order() {
        let mapped: Vec<Option<ReportLevel>> = [Severity::Error, Severity::Warning, Severity::Note]
            .into_iter()
            .map(|level| {
                let bound = probe(level, DiagnosticSource::Source, ByteSpan::new(0, 0))
                    .bind_source("probe.md", "")
                    .expect("empty span binds");
                MietteDiagnostic::severity(&bound)
            })
            .collect();
        assert_eq!(
            mapped,
            vec![
                Some(ReportLevel::Error),
                Some(ReportLevel::Warning),
                Some(ReportLevel::Advice),
            ],
            "miette has no `Note`, so the quietest level takes `Advice` — and no two of ours may \
             share one, or a note prints as a failure. The impl this replaced reached anything it \
             had not named through a `_` arm, which mapped it to `Error`"
        );
    }

    #[test]
    fn the_report_header_carries_the_stable_code_verbatim() {
        let d = probe(
            Severity::Error,
            DiagnosticSource::Source,
            ByteSpan::new(0, 0),
        );
        let bound = d.bind_source("probe.md", "").expect("empty span binds");
        let header = MietteDiagnostic::code(&bound).map(|code| code.to_string());
        assert_eq!(
            header.as_deref(),
            Some(d.code()),
            "the header must carry the code the wire carries, not a rendering of the `Cow` \
             that holds it"
        );
    }

    #[test]
    fn a_source_span_becomes_one_caret_over_the_bytes_it_names() {
        let d = probe(
            Severity::Warning,
            DiagnosticSource::Source,
            ByteSpan::new(4, 11),
        );
        let bound = d
            .bind_source("probe.md", "01234567890")
            .expect("range binds");
        let labels: Vec<LabeledSpan> = MietteDiagnostic::labels(&bound)
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(
            labels.len(),
            1,
            "an in-range source span must produce exactly one caret: {labels:?}"
        );
        assert_eq!(
            (labels[0].offset(), labels[0].len()),
            (4, 7),
            "the caret must cover the byte range the diagnostic names, end-exclusive"
        );
    }

    #[test]
    fn a_diagnostic_with_nothing_to_point_at_claims_no_caret() {
        // A caret is a claim about the caller's text. `Internal` blames this
        // library instead, and the `0..0` a document-scoped diagnostic
        // carries points at nothing. A reversed pair is rejected by
        // `bind_source` before it can reach label rendering.
        let cases = [
            probe(
                Severity::Error,
                DiagnosticSource::Internal,
                ByteSpan::new(4, 11),
            ),
            Diagnostic::source_too_large(5_000_000_000),
            Diagnostic::constructs_unresolved(3),
        ];
        for d in &cases {
            let bound = d
                .bind_source("probe.md", "01234567890")
                .expect("non-source and empty spans bind");
            assert!(
                MietteDiagnostic::labels(&bound).is_none(),
                "must claim no caret: {d:?}"
            );
        }
    }

    #[test]
    fn an_invalid_or_non_boundary_source_span_cannot_be_reported() {
        for span in [
            ByteSpan::new(9, 4),
            ByteSpan::new(0, 99),
            ByteSpan::new(1, 2),
        ] {
            let d = probe(Severity::Error, DiagnosticSource::Source, span);
            assert!(
                d.bind_source("probe.md", "🍣").is_err(),
                "{span:?} must not bind to a four-byte scalar"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_too_large_is_an_error_carrying_the_byte_counts() {
        let d = Diagnostic::source_too_large(5_000_000_000);
        assert_eq!(d.severity(), Severity::Error);
        assert_eq!(d.source(), DiagnosticSource::Source);
        assert_eq!(d.code(), "aozora-md::source_too_large");
        assert_eq!(d.span(), ByteSpan { start: 0, end: 0 });
        assert!(d.message().contains("5000000000"), "got: {d}");
        assert!(d.message().contains(&u32::MAX.to_string()), "got: {d}");
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
