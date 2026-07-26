//! `aozora-flavored-markdown` — command-line interface. `--help` lists the
//! sub-commands.
//!
//! Input is a file path or `-` for stdin, decoded as UTF-8 or — with
//! `--encoding sjis` — Shift_JIS, so original Aozora Bunko `.txt`
//! distributions need no pre-conversion.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal, Read};
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

/// `render` owns stdout (the HTML), so its JSON diagnostics go to stderr;
/// `check` has no stdout payload, so its JSON goes where `jq` can reach it.
/// Human format always uses stderr.
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

/// Resolved from `--output`; `None` and `-` both mean stdout.
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

/// A struct rather than positional args, so the shared pipeline stays under
/// clippy's argument-count and bool-parameter limits as flags land.
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
    span: Span,
    line: u32,
    /// 1-based, and a *character* column.
    column: u32,
}

impl DiagnosticReport {
    const SCHEMA: &'static str = "aozora-md.diagnostics.v1";

    fn build(diagnostics: &[Diagnostic], source: &str) -> Self {
        let diagnostics = diagnostics
            .iter()
            .map(|d| {
                let (line, column) = byte_offset_to_line_col(source, d.span().start);
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

/// Generated from the canonical `Cli` definition, so it cannot drift.
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

/// Also driven by `Cli`, so packaging renders from one source.
fn render_man() -> Result<ExitCode> {
    clap_mangen::Man::new(Cli::command())
        .render(&mut io::stdout())
        .into_diagnostic()
        .wrap_err("man ページを生成できません")?;
    Ok(ExitCode::SUCCESS)
}

/// An explicit `RUST_LOG` always wins over `-v` / `-q`.
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

/// The default (0) stays `warn`.
fn verbosity_level(verbose: u8, quiet: u8) -> &'static str {
    match i16::from(verbose) - i16::from(quiet) {
        ..=-1 => "error",
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

/// Under `auto`: `NO_COLOR`, then `CLICOLOR_FORCE`, then whether stderr is a
/// terminal.
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

/// So error reports honour the resolved colour choice rather than miette's
/// own TTY detection.
fn install_diagnostic_hook(color: bool) -> Result<()> {
    miette::set_hook(Box::new(move |_| {
        Box::new(miette::MietteHandlerOpts::new().color(color).build())
    }))
    .map_err(|e| miette::miette!("診断フォーマッタを初期化できません: {e}"))
}

/// Shared by `render` and `check`, which differ only in whether HTML reaches
/// the sink. Exit code 2 when `--strict` promotes a diagnostic to an error.
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

/// A 1-based (line, character-column) pair.
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

/// Bytes only; `read_input` performs the decode.
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

#[derive(Copy, Clone, Debug)]
struct Input<'a> {
    /// `<stdin>` or the file path, for labelling diagnostics.
    name: &'a str,
    text: &'a str,
}

/// The library renders the header and the caret; the text that caret points
/// into is the host's to supply, and is withheld for a span this input cannot
/// be sliced by — which also keeps `source_too_large` from copying its source.
fn report_of(d: &Diagnostic, input: Input<'_>) -> miette::Report {
    let report = miette::Report::new(d.clone());
    let (start, end) = (d.span().start as usize, d.span().end as usize);
    let in_bounds =
        matches!(d.source(), DiagnosticSource::Source) && end > start && end <= input.text.len();
    if in_bounds {
        report.with_source_code(miette::NamedSource::new(input.name, input.text.to_owned()))
    } else {
        report
    }
}

/// Human format prints nothing on clean input; JSON always prints the
/// envelope, empty array included, so tooling can rely on parseable output.
fn emit_diagnostics(
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
