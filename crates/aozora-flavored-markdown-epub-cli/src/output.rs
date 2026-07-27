// The diagnostic reporting the library leaves to whoever owns the terminal:
// graphically through miette, or as the `aozora-md.diagnostics.v1` envelope
// ADR-0012 pins. The EPUB itself goes to a file, so JSON can have stdout.

use aozora_flavored_markdown::{Diagnostic, DiagnosticSource, Severity, Span};
use aozora_flavored_markdown_epub::{BuildReport, ChapterReport};

use crate::args::DiagFormat;

// The envelope ADR-0012 pins, with the `path` a book-shaped run needs —
// additive within `v1`, which is what that ADR allows.
#[derive(Debug, serde::Serialize)]
struct DiagnosticReport {
    schema: &'static str,
    diagnostics: Vec<DiagnosticJson>,
}

#[derive(Debug, serde::Serialize)]
struct DiagnosticJson {
    // The chapter file, as discovered.
    path: String,
    code: String,
    severity: Severity,
    source: DiagnosticSource,
    // **Not** part of the stability contract.
    message: String,
    span: Span,
    line: u32,
    // 1-based, and a *character* column.
    column: u32,
}

impl DiagnosticReport {
    const SCHEMA: &'static str = "aozora-md.diagnostics.v1";

    fn build(report: &BuildReport) -> Self {
        let diagnostics = report
            .chapters
            .iter()
            .flat_map(|chapter| {
                // One index per chapter, reused by every diagnostic in it.
                let lines = LineIndex::new(&chapter.text);
                chapter.diagnostics.iter().map(move |d| {
                    let (line, column) = lines.locate(&chapter.text, d.span().start);
                    DiagnosticJson {
                        path: chapter.path.display().to_string(),
                        code: d.code().to_owned(),
                        severity: d.severity(),
                        source: d.source(),
                        message: d.message().to_owned(),
                        span: d.span(),
                        line,
                        column,
                    }
                })
            })
            .collect();
        Self {
            schema: Self::SCHEMA,
            diagnostics,
        }
    }
}

// A 1-based (line, character-column) pair, resolved against one scan of the
// chapter. The scan it replaces ran per diagnostic, so a book of long
// chapters with a lot to say about them cost O(n·d) just to label the report.
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

// The library renders the header and the caret; the chapter text the caret
// points into is attached here, and withheld for a span it cannot be sliced
// by.
fn report_of(d: &Diagnostic, chapter: &ChapterReport) -> miette::Report {
    let report = miette::Report::new(d.clone());
    let (start, end) = (d.span().start as usize, d.span().end as usize);
    let in_bounds =
        matches!(d.source(), DiagnosticSource::Source) && end > start && end <= chapter.text.len();
    if in_bounds {
        report.with_source_code(miette::NamedSource::new(
            chapter.path.display().to_string(),
            chapter.text.clone(),
        ))
    } else {
        report
    }
}

// Human format prints nothing for a clean book; JSON always prints the
// envelope, empty array included, so tooling can rely on parseable output.
// The EPUB goes to a file rather than to stdout, so JSON can have stdout.
pub(crate) fn emit_diagnostics(report: &BuildReport, format: DiagFormat) {
    match format {
        DiagFormat::Human => {
            for chapter in &report.chapters {
                for d in &chapter.diagnostics {
                    // miette ends each report with a newline and `eprintln!`
                    // adds one too, so trim to avoid a blank line between them.
                    let rendered = format!("{:?}", report_of(d, chapter));
                    eprintln!("{}", rendered.trim_end());
                }
            }
        }
        DiagFormat::Json => match serde_json::to_string(&DiagnosticReport::build(report)) {
            Ok(json) => println!("{json}"),
            Err(e) => eprintln!("診断を JSON 化できません: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Chapter texts picked so that the offsets inside them land everywhere a
    // span can: on a line start, on the `\n` itself, INSIDE a multi-byte
    // character, and past the end. The mapping is asserted at every one of
    // those offsets, which is the part `json_writes_the_envelope_on_stdout` —
    // one book, one diagnostic — cannot reach.
    //
    // This file is a copy of the CLI's `output.rs` index, so it gets a copy
    // of the rule: a fix applied to one of the two and not the other is
    // exactly what a shared behavioural test would fail to notice.
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
        // Only a chapter larger than `u32::MAX` reaches this, and the
        // renderer refuses one before the CLI is ever asked to label it — so
        // the saturation is asserted on the arithmetic itself.
        assert_eq!(clamp_u32(usize::MAX), u32::MAX, "the clamp must saturate");
        assert_eq!(clamp_u32(7), 7, "an in-range count must pass through");
    }
}
