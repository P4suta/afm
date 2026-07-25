//! `aozora-flavored-markdown` — command-line interface.
//!
//! Sub-commands:
//!   - `aozora-flavored-markdown render <input>`: render an aozora-flavored-markdown (.md) file to HTML on stdout.
//!   - `aozora-flavored-markdown check <input>`:  parse and surface diagnostics only; no rendering.
//!
//! `<input>` is a file path, or `-` to read from standard input. Input bytes may
//! be UTF-8 (default) or Shift_JIS (with `--encoding sjis`) to read original
//! Aozora Bunko .txt distributions without pre-conversion.

#![forbid(unsafe_code)]

use std::fmt::Display;
use std::io::{self, IsTerminal, Read};
use std::iter;
use std::path::{Path, PathBuf};
use std::{env, fs, process::ExitCode};

use aozora::decode_sjis;
use aozora_flavored_markdown::{Diagnostic, DiagnosticSource, Options, Severity, Span, render};
use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum};
use miette::{IntoDiagnostic, Result, WrapErr};

#[derive(Parser, Debug)]
#[command(
    name = "aozora-flavored-markdown",
    version,
    about = "aozora-flavored-markdown CLI",
    long_about = None,
    after_long_help = "EXAMPLES:\n  \
        aozora-flavored-markdown render input.md > out.html\n  \
        aozora-flavored-markdown render input.md -o out.html\n  \
        cat input.md | aozora-flavored-markdown render -\n  \
        aozora-flavored-markdown check --strict --format json input.md\n  \
        aozora-flavored-markdown completions zsh > _aozora-flavored-markdown",
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files.
    #[arg(long, global = true, value_enum, default_value_t = InputEncoding::Utf8)]
    encoding: InputEncoding,

    /// Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through.
    #[arg(long, global = true)]
    strict: bool,

    /// When to colorize diagnostics: auto (TTY-aware), always, or never.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    /// Decrease log verbosity (-q errors only). `RUST_LOG` overrides.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    quiet: u8,

    /// Diagnostic output format: human-readable lines, or stable JSON for tooling.
    #[arg(long, global = true, value_enum, default_value_t = DiagFormat::Human)]
    format: DiagFormat,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render the input to HTML on stdout.
    Render {
        /// Path to the aozora-flavored-markdown source. Use `-` for stdin.
        input: PathBuf,

        /// Write HTML here instead of stdout. Use `-` for stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Parse the input and report diagnostics without rendering.
    Check {
        /// Path to the aozora-flavored-markdown source. Use `-` for stdin.
        input: PathBuf,
    },
    /// Generate a shell completion script on stdout.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Render the man page (roff) on stdout. Hidden; used by packaging.
    #[command(hide = true, name = "_man")]
    Man,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum InputEncoding {
    Utf8,
    Sjis,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
enum DiagFormat {
    /// Graphical diagnostics (severity, code, message, source snippet) for humans.
    #[default]
    Human,
    /// A stable `aozora-md.diagnostics.v1` JSON envelope for tooling.
    Json,
}

/// Where a stream of diagnostics is written. `render` owns stdout (HTML), so
/// its JSON diagnostics go to stderr; `check` has no stdout payload, so its
/// JSON goes to stdout where `jq` can reach it. Human-format always uses stderr.
#[derive(Copy, Clone, Debug)]
enum DiagStream {
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

/// Where rendered HTML goes. Resolved from `--output`; `None` and `-` both
/// mean stdout.
#[derive(Debug)]
enum OutputSink {
    Stdout,
    File(PathBuf),
}

impl OutputSink {
    fn from_arg(output: Option<PathBuf>) -> Self {
        match output {
            Some(path) if path != Path::new("-") => Self::File(path),
            _ => Self::Stdout,
        }
    }
}

/// Resolved inputs for one render/check pass. Carrying these in a struct
/// (rather than a fistful of positional args) keeps the shared pipeline under
/// clippy's argument-count and bool-parameter limits as more flags land.
#[derive(Debug)]
struct PipelineArgs {
    input: PathBuf,
    encoding: InputEncoding,
    strict: bool,
    /// `render` prints HTML on success; `check` parses only.
    emit_html: bool,
    output: OutputSink,
    format: DiagFormat,
}

/// The `aozora-md.diagnostics.v1` envelope — the stable JSON contract for tooling.
/// See ADR-0012. Fields are additive-only within `v1`; a breaking change bumps
/// the `schema` discriminant.
#[derive(Debug, serde::Serialize)]
struct DiagnosticReport {
    schema: &'static str,
    diagnostics: Vec<DiagnosticJson>,
}

#[derive(Debug, serde::Serialize)]
struct DiagnosticJson {
    /// Stable machine-readable code string.
    code: &'static str,
    /// `error` / `warning` / `note` (serialised from [`Severity`]).
    severity: Severity,
    /// `source` (user input) / `internal` (pipeline bug).
    source: DiagnosticSource,
    /// Human-readable message. Not part of the stability contract.
    message: String,
    /// Byte-offset span into the (decoded) source.
    span: Span,
    /// 1-based line of `span.start`.
    line: u32,
    /// 1-based character column of `span.start`.
    column: u32,
}

impl DiagnosticReport {
    const SCHEMA: &'static str = "aozora-md.diagnostics.v1";

    fn build(diagnostics: &[Diagnostic], source: &str) -> Self {
        let diagnostics = diagnostics
            .iter()
            .map(|d| {
                let (line, column) = byte_offset_to_line_col(source, d.span.start);
                DiagnosticJson {
                    code: d.code,
                    severity: d.severity,
                    source: d.source,
                    message: d.message.clone(),
                    span: d.span,
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

/// CLI-local adapter that renders an aozora-flavored-markdown [`Diagnostic`] through miette's
/// graphical handler. The orphan rule forbids `impl miette::Diagnostic` on the
/// foreign aozora-flavored-markdown type directly, so we carry the data miette needs here.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct CliDiagnostic {
    code: &'static str,
    severity: Severity,
    message: String,
    /// The labelled source. `None` for `internal` diagnostics or when the span
    /// is degenerate / out of bounds — those render as a header + message with
    /// no snippet (and avoid materialising a huge source, e.g. `source_too_large`).
    source_code: Option<miette::NamedSource<String>>,
    /// `(start, len)` byte range of the caret, present iff `source_code` is.
    label: Option<(usize, usize)>,
}

impl CliDiagnostic {
    fn new(d: &Diagnostic, source: &str, name: &str) -> Self {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        // Only attach a snippet for in-bounds, non-degenerate user-source spans.
        let snippet_ok =
            matches!(d.source, DiagnosticSource::Source) && end > start && end <= source.len();
        let (source_code, label) = if snippet_ok {
            (
                Some(miette::NamedSource::new(name, source.to_owned())),
                Some((start, end - start)),
            )
        } else {
            (None, None)
        };
        Self {
            code: d.code,
            severity: d.severity,
            message: d.message.clone(),
            source_code,
            label,
        }
    }
}

impl miette::Diagnostic for CliDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn Display + 'a>> {
        Some(Box::new(self.code))
    }

    fn severity(&self) -> Option<miette::Severity> {
        // miette has no `Note`; its three levels are Advice / Warning / Error.
        Some(match self.severity {
            Severity::Error => miette::Severity::Error,
            Severity::Warning => miette::Severity::Warning,
            Severity::Note => miette::Severity::Advice,
        })
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.source_code
            .as_ref()
            .map(|s| -> &dyn miette::SourceCode { s })
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        let (start, len) = self.label?;
        let span = miette::LabeledSpan::new(None, start, len);
        Some(Box::new(iter::once(span)))
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    init_tracing(cli.verbose, cli.quiet);
    install_diagnostic_hook(resolve_color(cli.color))?;

    let args = match cli.command {
        Command::Render { input, output } => PipelineArgs {
            input,
            encoding: cli.encoding,
            strict: cli.strict,
            emit_html: true,
            output: OutputSink::from_arg(output),
            format: cli.format,
        },
        Command::Check { input } => PipelineArgs {
            input,
            encoding: cli.encoding,
            strict: cli.strict,
            emit_html: false,
            output: OutputSink::Stdout,
            format: cli.format,
        },
        Command::Completions { shell } => return Ok(generate_completions(shell)),
        Command::Man => return render_man(),
    };
    run_pipeline(&args)
}

/// Write a shell completion script for `shell` to stdout. The script is
/// generated from the canonical `Cli` definition, so it never drifts.
fn generate_completions(shell: clap_complete::Shell) -> ExitCode {
    let mut cmd = Cli::command();
    clap_complete::generate(
        shell,
        &mut cmd,
        "aozora-flavored-markdown",
        &mut io::stdout(),
    );
    ExitCode::SUCCESS
}

/// Write the roff man page to stdout. Driven by the canonical `Cli` so the
/// packaging step (`cargo xtask gen-man`) renders from a single source.
fn render_man() -> Result<ExitCode> {
    clap_mangen::Man::new(Cli::command())
        .render(&mut io::stdout())
        .into_diagnostic()
        .wrap_err("man ページを生成できません")?;
    Ok(ExitCode::SUCCESS)
}

/// Configure the tracing subscriber. An explicit `RUST_LOG` always wins;
/// otherwise the `-v`/`-q` count picks a default level.
fn init_tracing(verbose: u8, quiet: u8) {
    let filter = if env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else {
        tracing_subscriber::EnvFilter::new(verbosity_level(verbose, quiet))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

/// Map the net `-v`/`-q` count to a tracing level. Default (0) stays `warn`,
/// matching the historical behaviour.
fn verbosity_level(verbose: u8, quiet: u8) -> &'static str {
    match i16::from(verbose) - i16::from(quiet) {
        ..=-1 => "error",
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

/// Decide whether to colorize diagnostics. An explicit `--color always`/`never`
/// wins; under `auto` we honor `NO_COLOR`, then `CLICOLOR_FORCE`, then whether
/// stderr is a terminal.
fn resolve_color(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            if env::var_os("NO_COLOR").is_some() {
                false
            } else if env::var("CLICOLOR_FORCE").is_ok_and(|v| !v.is_empty() && v != "0") {
                true
            } else {
                io::stderr().is_terminal()
            }
        }
    }
}

/// Install the miette report hook so error reports honor the resolved color
/// choice instead of miette's own TTY auto-detection.
fn install_diagnostic_hook(color: bool) -> Result<()> {
    miette::set_hook(Box::new(move |_| {
        Box::new(miette::MietteHandlerOpts::new().color(color).build())
    }))
    .map_err(|e| miette::miette!("診断フォーマッタを初期化できません: {e}"))
}

/// Read → render → report. Shared by `render` and `check`; the only difference
/// is whether HTML reaches the output sink on success. Returns exit code 2 when
/// `--strict` promotes a lexer diagnostic to an error, otherwise 0.
fn run_pipeline(args: &PipelineArgs) -> Result<ExitCode> {
    let source = read_input(&args.input, args.encoding)?;
    let options = Options::default();
    let result = render(&source, &options);

    // JSON diagnostics for `check` go to stdout (pipe into `jq`); for `render`
    // they go to stderr so stdout stays pure HTML. Human format always stderr.
    let stream = match args.format {
        DiagFormat::Json if !args.emit_html => DiagStream::Stdout,
        _ => DiagStream::Stderr,
    };
    let name = if args.input == Path::new("-") {
        "<stdin>".to_owned()
    } else {
        args.input.display().to_string()
    };
    let input = Input {
        name: &name,
        text: &source,
    };
    emit_diagnostics(&result.diagnostics, input, args.format, stream);

    if args.strict && !result.diagnostics.is_empty() {
        // In JSON mode the envelope (and exit code 2) carry the failure; a
        // free-form line would corrupt a stdout JSON stream.
        if matches!(args.format, DiagFormat::Human) {
            eprintln!(
                "lexer が {} 件の診断を報告しました (--strict)",
                result.diagnostics.len()
            );
        }
        return Ok(ExitCode::from(2));
    }

    if args.emit_html {
        write_html(&args.output, &result.html)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Map a byte offset into `source` to a 1-based (line, character-column) pair.
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

/// Emit rendered HTML to stdout or a file, with a trailing newline either way.
fn write_html(sink: &OutputSink, html: &str) -> Result<()> {
    match sink {
        OutputSink::Stdout => {
            println!("{html}");
            Ok(())
        }
        OutputSink::File(path) => fs::write(path, format!("{html}\n"))
            .into_diagnostic()
            .wrap_err_with(|| format!("出力ファイルを書けません: {}", path.display())),
    }
}

/// Read the input as raw bytes — from a file path, or from standard input when
/// `input` is `-`. Encoding-agnostic; `read_input` performs the decode.
fn read_bytes(input: &Path) -> Result<Vec<u8>> {
    if input == Path::new("-") {
        let mut buf = Vec::new();
        io::stdin()
            .lock()
            .read_to_end(&mut buf)
            .into_diagnostic()
            .wrap_err("標準入力を読めません")?;
        Ok(buf)
    } else {
        fs::read(input)
            .into_diagnostic()
            .wrap_err_with(|| format!("入力ファイルを読めません: {}", input.display()))
    }
}

fn read_input(input: &Path, encoding: InputEncoding) -> Result<String> {
    let bytes = read_bytes(input)?;
    match encoding {
        InputEncoding::Utf8 => String::from_utf8(bytes)
            .into_diagnostic()
            .wrap_err("UTF-8 としてデコードできません — --encoding sjis を試してください"),
        InputEncoding::Sjis => decode_sjis(&bytes).map_err(Into::into),
    }
}

/// The decoded source plus a display name, used to label diagnostics.
#[derive(Copy, Clone, Debug)]
struct Input<'a> {
    /// Display name for the source (`<stdin>` or the file path).
    name: &'a str,
    /// The decoded source text.
    text: &'a str,
}

/// Emit diagnostics in the requested format on the chosen stream.
///
/// Human format renders each diagnostic graphically via miette (severity, code,
/// message, and a source snippet with a caret), honoring the resolved `--color`
/// choice; nothing is printed when there are none. JSON format always prints the
/// stable `aozora-md.diagnostics.v1` envelope (an empty array on clean input) so
/// tooling can rely on parseable output. Either way the stable `aozora::…` codes
/// let language servers and CI gates key on identifiers rather than free-form
/// messages.
fn emit_diagnostics(
    diagnostics: &[Diagnostic],
    input: Input<'_>,
    format: DiagFormat,
    stream: DiagStream,
) {
    match format {
        DiagFormat::Human => {
            for d in diagnostics {
                let report = miette::Report::new(CliDiagnostic::new(d, input.text, input.name));
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
