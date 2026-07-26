//! `aozora-flavored-markdown-epub` CLI — thin clap wrapper over the
//! `aozora_flavored_markdown_epub` library, plus the diagnostic reporting
//! that library leaves to whoever owns the terminal.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use aozora_flavored_markdown::{Diagnostic, DiagnosticSource, Severity, Span};
use aozora_flavored_markdown_epub::{BuildOptions, BuildReport, ChapterReport, build};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "aozora-flavored-markdown-epub",
    version,
    about,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Exit 2 if any chapter raised a diagnostic. The EPUB is still written.
    #[arg(long, global = true)]
    strict: bool,

    /// Diagnostic output format: human-readable reports, or stable JSON for tooling.
    #[arg(long, global = true, value_enum, default_value_t = DiagFormat::Human)]
    format: DiagFormat,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Build an EPUB3 from a manuscript directory.
    Build {
        /// Input directory or single Aozora Flavored Markdown file.
        #[arg(long)]
        input: PathBuf,
        /// `book.toml` metadata path.
        #[arg(long)]
        metadata: PathBuf,
        /// Output `.epub` path.
        #[arg(long, short = 'o')]
        output: PathBuf,
    },
}

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
enum DiagFormat {
    /// Graphical diagnostics (severity, code, message, source snippet) for humans.
    #[default]
    Human,
    /// A stable `aozora-md.diagnostics.v1` JSON envelope for tooling.
    Json,
}

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
                chapter.diagnostics.iter().map(|d| {
                    let (line, column) = byte_offset_to_line_col(&chapter.text, d.span().start);
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

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

/// # Errors
///
/// Propagates whatever the library raises while building.
fn run(cli: Cli) -> miette::Result<ExitCode> {
    match cli.cmd {
        Cmd::Build {
            input,
            metadata,
            output,
        } => {
            let report = build(&BuildOptions::new(&input, &metadata, &output))?;
            emit_diagnostics(&report, cli.format);
            // Packaging is the phase before this one, so the file is already
            // on disk; a diagnostic is an observation, not a refusal. What
            // `--strict` decides is the verdict on the run, not the output.
            if cli.strict && !report.is_empty() {
                // In JSON mode the envelope and the exit code carry the
                // failure; a free-form line would corrupt a stdout stream.
                if matches!(cli.format, DiagFormat::Human) {
                    eprintln!(
                        "{} 件の診断を報告しました (--strict)",
                        report.diagnostic_count()
                    );
                }
                return Ok(ExitCode::from(2));
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

// A 1-based (line, character-column) pair.
fn byte_offset_to_line_col(source: &str, offset: u32) -> (u32, u32) {
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
fn emit_diagnostics(report: &BuildReport, format: DiagFormat) {
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
    use std::fs;
    use std::iter;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(iter::once("aozora-flavored-markdown-epub").chain(args.iter().copied()))
    }

    #[test]
    fn parses_build_with_all_paths() {
        let cli = parse(&[
            "build",
            "--input",
            "m",
            "--metadata",
            "b.toml",
            "--output",
            "o.epub",
        ])
        .expect("parses");
        match cli.cmd {
            Cmd::Build {
                input,
                metadata,
                output,
            } => {
                assert_eq!(input, PathBuf::from("m"));
                assert_eq!(metadata, PathBuf::from("b.toml"));
                assert_eq!(output, PathBuf::from("o.epub"));
            }
        }
    }

    #[test]
    fn output_has_a_short_flag() {
        let cli = parse(&[
            "build",
            "--input",
            "m",
            "--metadata",
            "b.toml",
            "-o",
            "o.epub",
        ])
        .expect("parses");
        match cli.cmd {
            Cmd::Build { output, .. } => assert_eq!(output, PathBuf::from("o.epub")),
        }
    }

    #[test]
    fn build_requires_output() {
        parse(&["build", "--input", "m", "--metadata", "b.toml"]).unwrap_err();
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        parse(&["frobnicate"]).unwrap_err();
    }

    #[test]
    fn run_builds_an_epub_from_a_manuscript_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manuscript = dir.path().join("manuscript");
        fs::create_dir(&manuscript).expect("mkdir");
        fs::write(manuscript.join("001-chapter.md"), "Hello").expect("write md");
        let metadata = dir.path().join("book.toml");
        fs::write(
            &metadata,
            "title = \"T\"\ncreator = \"A\"\nlanguage = \"ja\"\n",
        )
        .expect("write toml");
        let output = dir.path().join("out.epub");

        let cli = parse(&[
            "build",
            "--input",
            manuscript.to_str().expect("utf8 input"),
            "--metadata",
            metadata.to_str().expect("utf8 metadata"),
            "--output",
            output.to_str().expect("utf8 output"),
        ])
        .expect("parses");

        let _code = run(cli).expect("build succeeds");
        assert!(output.exists(), "the .epub output must be written");
    }

    #[test]
    fn run_errors_on_missing_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("only.md"), "x").expect("write md");
        let missing = dir.path().join("does-not-exist.toml");
        let output = dir.path().join("out.epub");

        let cli = parse(&[
            "build",
            "--input",
            dir.path().join("only.md").to_str().expect("utf8 input"),
            "--metadata",
            missing.to_str().expect("utf8 metadata"),
            "--output",
            output.to_str().expect("utf8 output"),
        ])
        .expect("parses");

        run(cli).unwrap_err();
    }
}
