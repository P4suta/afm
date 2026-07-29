// Every flag, sub-command and enumerated value the binary accepts, and
// nothing else. clap's derive is the single description of that surface:
// `--help`, `completions` and the man page are all rendered from these types,
// so a flag described anywhere else would be a second copy that can drift.

use std::path::PathBuf;

use clap::{ArgAction, ArgGroup, Parser, Subcommand, ValueEnum};

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
        aozora-flavored-markdown fmt --check input.md\n  \
        aozora-flavored-markdown completions zsh > _aozora-flavored-markdown",
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,

    /// Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files.
    #[arg(long, global = true, value_enum, default_value_t = InputEncoding::Utf8)]
    pub(crate) encoding: InputEncoding,

    /// Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through.
    #[arg(long, global = true)]
    pub(crate) strict: bool,

    /// When to colorize diagnostics: auto (TTY-aware), always, or never.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub(crate) color: ColorChoice,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) verbose: u8,

    /// Decrease log verbosity (-q errors only). `RUST_LOG` overrides.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) quiet: u8,

    /// Diagnostic output format: human-readable lines, or stable JSON for tooling.
    #[arg(long, global = true, value_enum, default_value_t = DiagFormat::Human)]
    pub(crate) format: DiagFormat,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
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
    /// Canonicalize source and check, display, or write the result.
    #[command(group(
        ArgGroup::new("fmt-mode")
            .required(true)
            .args(["check", "diff", "write"])
    ))]
    Fmt {
        /// Path to the source. Use `-` for stdin with `--check` or `--diff`.
        input: PathBuf,

        /// Exit 1 when the source is not canonical; write nothing.
        #[arg(long, group = "fmt-mode")]
        check: bool,

        /// Print a unified diff; exit 1 when the source is not canonical.
        #[arg(long, group = "fmt-mode")]
        diff: bool,

        /// Replace the input file with canonical source.
        #[arg(long, group = "fmt-mode")]
        write: bool,
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
pub(crate) enum InputEncoding {
    Utf8,
    Sjis,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub(crate) enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
pub(crate) enum DiagFormat {
    /// Graphical diagnostics (severity, code, message, source snippet) for humans.
    #[default]
    Human,
    /// A stable `aozora-md.diagnostics.v1` JSON envelope for tooling.
    Json,
}
