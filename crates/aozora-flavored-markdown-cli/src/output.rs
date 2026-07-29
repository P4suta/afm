// The two things this binary writes: the rendered HTML, and the diagnostics
// — graphically through miette, or as the `aozora-md.diagnostics.v1` envelope
// ADR-0012 pins. Which stream each goes to is decided in `app`, because it
// depends on which sub-command owns stdout.

use std::fs;
use std::path::{Path, PathBuf};

use aozora_flavored_markdown::{ByteSpan, Diagnostic, DiagnosticSource, Severity};
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::args::DiagFormat;

/// `render` owns stdout (the HTML), so its JSON diagnostics go to stderr;
/// `check` has no stdout payload, so its JSON goes where `jq` can reach it.
/// Human format always uses stderr.
#[derive(Copy, Clone, Debug)]
pub(crate) enum DiagStream {
    Stdout,
    Stderr,
}

impl DiagStream {
    fn write_line(self, line: &str) {
        match self {
            Self::Stdout => println!("{line}"),
            Self::Stderr => eprintln!("{line}"),
        }
    }
}

/// Resolved from `--output`; `None` and `-` both mean stdout.
#[derive(Debug)]
pub(crate) enum OutputSink {
    Stdout,
    File(PathBuf),
}

impl OutputSink {
    pub(crate) fn from_arg(output: Option<PathBuf>) -> Self {
        match output {
            Some(path) if path != Path::new("-") => Self::File(path),
            _ => Self::Stdout,
        }
    }

    /// Emit rendered HTML to stdout or a file, with a trailing newline either way.
    pub(crate) fn write_html(&self, html: &str) -> Result<()> {
        match self {
            Self::Stdout => {
                println!("{html}");
                Ok(())
            }
            Self::File(path) => fs::write(path, format!("{html}\n"))
                .into_diagnostic()
                .wrap_err_with(|| format!("出力ファイルを書けません: {}", path.display())),
        }
    }
}

/// A 1-based (line, character-column) pair, resolved against one scan.
// The scan the pair used to cost was per diagnostic, so labelling a report of
// `d` diagnostics over an `n`-byte source was O(n·d) — quadratic on exactly
// the input a `--format json` consumer reaches for, a long document with a
// lot to say about it. One pass records where each line begins and every
// lookup after that is a binary search over that table.
#[derive(Debug)]
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0usize];
        starts.extend(source.match_indices('\n').map(|(idx, _)| idx + 1));
        Self { starts }
    }

    fn locate(&self, source: &str, offset: u32) -> (u32, u32) {
        let offset = (offset as usize).min(source.len());
        // `starts[0]` is 0, so the count of line starts at or before `offset`
        // IS its 1-based line number.
        let line = self.starts.partition_point(|&start| start <= offset);
        let bol = self.starts.get(line - 1).copied().unwrap_or(0);
        // Characters, not bytes, counted from the line start — which is
        // always a boundary, so the slice cannot panic. A span landing INSIDE
        // a character counts the character it landed in, which is where the
        // scan this replaced put it: slicing at `offset` would panic and
        // declining to count would silently report column 1.
        let column = source[bol..]
            .char_indices()
            .take_while(|(idx, _)| bol + idx < offset)
            .count()
            + 1;
        (clamp_u32(line), clamp_u32(column))
    }
}

fn clamp_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The stable JSON contract for tooling (ADR-0012). Additive-only within
/// `v1`; a breaking change bumps the `schema` discriminant.
#[derive(Debug, serde::Serialize)]
struct DiagnosticReport {
    schema: &'static str,
    diagnostics: Vec<DiagnosticJson>,
}

#[derive(Debug, serde::Serialize)]
struct DiagnosticJson {
    code: String,
    severity: Severity,
    source: DiagnosticSource,
    /// **Not** part of the stability contract.
    message: String,
    span: ByteSpan,
    line: u32,
    /// 1-based, and a *character* column.
    column: u32,
}

impl DiagnosticReport {
    const SCHEMA: &'static str = "aozora-md.diagnostics.v1";

    fn build(diagnostics: &[Diagnostic], source: &str) -> Self {
        let lines = LineIndex::new(source);
        let diagnostics = diagnostics
            .iter()
            .map(|d| {
                let (line, column) = lines.locate(source, d.span().start);
                DiagnosticJson {
                    code: d.code().to_owned(),
                    severity: d.severity(),
                    source: d.source(),
                    message: d.message().to_owned(),
                    span: d.span(),
                    line,
                    column,
                }
            })
            .collect();
        Self {
            schema: Self::SCHEMA,
            diagnostics,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct Input<'a> {
    /// `<stdin>` or the file path, for labelling diagnostics.
    pub(crate) name: &'a str,
    pub(crate) text: &'a str,
}

/// Build a report only through the library's source-bound adapter, which
/// validates the range before miette can draw it.
fn report_of(d: &Diagnostic, input: Input<'_>) -> miette::Report {
    match d.bind_source(input.name, input.text) {
        Ok(bound) => miette::Report::new(bound),
        Err(error) => miette::miette!(
            "診断 {} を入力 {} に関連付けられません: {error}",
            d.code(),
            input.name
        ),
    }
}

/// Human format prints nothing on clean input; JSON always prints the
/// envelope, empty array included, so tooling can rely on parseable output.
pub(crate) fn emit_diagnostics(
    diagnostics: &[Diagnostic],
    input: Input<'_>,
    format: DiagFormat,
    stream: DiagStream,
) {
    match format {
        DiagFormat::Human => {
            for d in diagnostics {
                let report = report_of(d, input);
                // miette's renderer ends each report with a newline; `write_line`
                // adds one too, so trim to avoid a blank line between diagnostics.
                stream.write_line(format!("{report:?}").trim_end());
            }
        }
        DiagFormat::Json => {
            let report = DiagnosticReport::build(diagnostics, input.text);
            match serde_json::to_string(&report) {
                Ok(json) => stream.write_line(&json),
                Err(e) => eprintln!("診断を JSON 化できません: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use aozora_flavored_markdown::{Options, render};

    use super::*;

    // Sources picked so that the offsets inside them land everywhere a span
    // can: on a line start, on the `\n` itself, INSIDE a multi-byte
    // character, and past the end. The mapping is then asserted at every one
    // of those offsets, which is the part `json_line_col_is_one_based` — one
    // input, one diagnostic — cannot reach.
    const SOURCES: &[&str] = &[
        "",
        "\n",
        "\n\n\n",
        "abc",
        "abc\n",
        "a\nbb\nccc",
        "a\r\nb",
        "あいう\nえお",
        "🍣x\n🍣",
        "first line\norphan》close",
    ];

    // The per-diagnostic scan `LineIndex` replaced, kept here as the oracle.
    // It is the definition of the pair `aozora-md.diagnostics.v1` has
    // published since the envelope existed; an index is only a cheaper way to
    // compute the same answer, so "cheaper" has to be all that changed.
    fn linear_scan(source: &str, offset: u32) -> (u32, u32) {
        let offset = offset as usize;
        let mut line = 1u32;
        let mut column = 1u32;
        for (idx, ch) in source.char_indices() {
            if idx >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        (line, column)
    }

    #[test]
    fn the_line_index_is_the_linear_scan_at_every_offset() {
        for source in SOURCES {
            let index = LineIndex::new(source);
            // Three bytes past the end as well: an out-of-range span is
            // clamped rather than refused, so the oracle is asked the same
            // out-of-range question.
            for offset in 0..=(source.len() + 3) {
                let offset = u32::try_from(offset).expect("a test source is small");
                assert_eq!(
                    index.locate(source, offset),
                    linear_scan(source, offset),
                    "line/column disagree at offset {offset} of {source:?}"
                );
            }
        }
    }

    #[test]
    fn a_coordinate_past_u32_saturates_rather_than_wrapping() {
        // Only a source larger than `u32::MAX` reaches this, and the library
        // refuses one before the CLI is ever asked to label it — so the
        // saturation is asserted on the arithmetic itself.
        assert_eq!(clamp_u32(usize::MAX), u32::MAX, "the clamp must saturate");
        assert_eq!(clamp_u32(7), 7, "an in-range count must pass through");
    }

    #[test]
    fn a_report_carries_the_source_it_can_be_sliced_by_and_no_other() {
        const SRC: &str = "first line\norphan》close";
        let diagnostic = render(SRC, &Options::default())
            .diagnostics
            .into_iter()
            .next()
            .expect("the canary source must raise a diagnostic");

        let labelled = format!(
            "{:?}",
            report_of(
                &diagnostic,
                Input {
                    name: "in.md",
                    text: SRC,
                },
            )
        );
        assert!(
            labelled.contains("orphan"),
            "an in-bounds span must be labelled with the line it points at, got {labelled:?}"
        );

        // The same diagnostic against a text its span runs off the end of.
        // miette indexes the source it is handed, so attaching this one would
        // be an out-of-range read of the host's string.
        let withheld = format!(
            "{:?}",
            report_of(
                &diagnostic,
                Input {
                    name: "in.md",
                    text: "",
                },
            )
        );
        assert!(
            !withheld.contains("orphan"),
            "a span the text cannot be sliced by must go unlabelled, got {withheld:?}"
        );
    }
}
