// Everything the binary does once clap has spoken: pick the sub-command,
// read the input, run the pipeline, and turn the outcome into an exit code.
// `main.rs` is a shim over `run`, so the behaviour of the CLI is read here.

use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use aozora::decode_sjis;
use aozora_flavored_markdown::{Diagnostic, Options, diagnose, render};
use clap::{CommandFactory, Parser};
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::args::{Cli, ColorChoice, Command, DiagFormat, InputEncoding};
use crate::output::{DiagStream, Input, OutputSink, emit_diagnostics};

pub(crate) fn run() -> ExitCode {
    match dispatch(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    init_tracing(cli.verbose, cli.quiet);
    install_diagnostic_hook(resolve_color(cli.color))?;

    let args = match cli.command {
        Command::Render { input, output } => PipelineArgs {
            input,
            encoding: cli.encoding,
            strict: cli.strict,
            output: Some(OutputSink::from_arg(output)),
            format: cli.format,
        },
        Command::Check { input } => PipelineArgs {
            input,
            encoding: cli.encoding,
            strict: cli.strict,
            output: None,
            format: cli.format,
        },
        Command::Completions { shell } => return Ok(generate_completions(shell)),
        Command::Man => return render_man(),
    };
    run_pipeline(&args)
}

/// A struct rather than positional args, so the shared pipeline stays under
/// clippy's argument-count and bool-parameter limits as flags land.
#[derive(Debug)]
struct PipelineArgs {
    input: PathBuf,
    encoding: InputEncoding,
    strict: bool,
    /// `Some` renders into that sink; `None` is `check`, which never renders.
    output: Option<OutputSink>,
    format: DiagFormat,
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

/// Shared by `render` and `check`, which differ in whether the source is
/// rendered at all. Exit code 2 when `--strict` promotes a diagnostic.
fn run_pipeline(args: &PipelineArgs) -> Result<ExitCode> {
    let source = read_input(&args.input, args.encoding)?;
    let options = Options::default();
    let (html, diagnostics) = analyze(&source, &options, args.output.is_some());

    // JSON diagnostics for `check` go to stdout (pipe into `jq`); for `render`
    // they go to stderr so stdout stays pure HTML. Human format always stderr.
    let stream = if args.output.is_none() && matches!(args.format, DiagFormat::Json) {
        DiagStream::Stdout
    } else {
        DiagStream::Stderr
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
    emit_diagnostics(&diagnostics, input, args.format, stream);

    if args.strict && !diagnostics.is_empty() {
        // In JSON mode the envelope (and exit code 2) carry the failure; a
        // free-form line would corrupt a stdout JSON stream.
        if matches!(args.format, DiagFormat::Human) {
            eprintln!(
                "lexer が {} 件の診断を報告しました (--strict)",
                diagnostics.len()
            );
        }
        return Ok(ExitCode::from(2));
    }

    if let Some((sink, html)) = args.output.as_ref().zip(html) {
        sink.write_html(&html)?;
    }
    Ok(ExitCode::SUCCESS)
}

// The one place `render` and `check` differ, and the whole of the difference.
// `check` stops at the lexer, which is where every diagnostic comes from:
// `diagnose` reports exactly what `render` would have, so the two agree on
// every exit code without `check` paying for the comrak parse, the AST splice
// and the HTML formatting it then throws away. Named rather than inlined so
// "check renders nothing" is a value a test can hold, instead of a branch
// observable only through what does not appear on stdout.
fn analyze(source: &str, options: &Options, emit_html: bool) -> (Option<String>, Vec<Diagnostic>) {
    if emit_html {
        let rendered = render(source, options);
        (Some(rendered.html), rendered.diagnostics)
    } else {
        (None, diagnose(source, options))
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

#[cfg(test)]
mod tests {
    use super::*;

    // What a `check` is pointed at, taken to include the shapes where the two
    // modes have the most room to disagree: a construct the lexer objects to,
    // one it accepts, an annotation it does not know, and a fenced block whose
    // triggers are masked out from under it before either mode looks.
    const SOURCES: &[&str] = &[
        "",
        "clean input",
        "orphan》close",
        "｜青梅《おうめ》に行った",
        "［＃未知の注記］",
        "```\n｜青梅《おうめ》\n```\n",
        "# 見出し\n\n本文\n\n［＃改ページ］\n\n- a\n- b\n",
        "第一行\norphan》close\n《another",
    ];

    #[test]
    fn check_mode_renders_nothing() {
        // The acceptance criterion of DEV-216, as a value: `check` documented
        // itself as parsing "without rendering" while calling `render`
        // unconditionally, and no gate could see it because the HTML was
        // computed and then dropped on the floor — which looks identical from
        // outside the process.
        let options = Options::default();
        for source in SOURCES {
            let (html, _) = analyze(source, &options, false);
            assert!(
                html.is_none(),
                "`check` must not render {source:?}, got {html:?}"
            );
            let (html, _) = analyze(source, &options, true);
            assert!(
                html.is_some(),
                "`render` must still render {source:?}, got nothing"
            );
        }
    }

    #[test]
    fn the_two_modes_report_the_same_diagnostics() {
        // What makes skipping the render safe, and the thing that can now rot
        // silently: `check` and `render` reach their diagnostics down two
        // different code paths, so a diagnostic raised after the lexer would
        // reach one and not the other — and `check --strict`, the CI shape,
        // is the one that would stop seeing it.
        let options = Options::default();
        for source in SOURCES {
            let (_, checked) = analyze(source, &options, false);
            let (_, rendered) = analyze(source, &options, true);
            assert_eq!(
                checked, rendered,
                "`check` and `render` disagree about {source:?}"
            );
        }
    }

    #[test]
    fn the_verbosity_ladder_climbs_and_falls_from_warn() {
        for (verbose, quiet, expected) in [
            (0, 0, "warn"),
            (1, 0, "info"),
            (2, 0, "debug"),
            (3, 0, "trace"),
            (9, 0, "trace"),
            (0, 1, "error"),
            (0, 9, "error"),
            // `-v` and `-q` are counters, not switches: they cancel.
            (1, 1, "warn"),
            (3, 1, "debug"),
        ] {
            assert_eq!(
                verbosity_level(verbose, quiet),
                expected,
                "-v x{verbose} -q x{quiet} must select {expected}"
            );
        }
    }
}
