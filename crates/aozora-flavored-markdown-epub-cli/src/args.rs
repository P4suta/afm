// The whole command surface, and nothing else. clap's derive is its single
// description, so `--help` and this file cannot disagree.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "aozora-flavored-markdown-epub",
    version,
    about,
    propagate_version = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) cmd: Cmd,

    /// Exit 2 if any chapter raised a diagnostic. The EPUB is still written.
    #[arg(long, global = true)]
    pub(crate) strict: bool,

    /// Diagnostic output format: human-readable reports, or stable JSON for tooling.
    #[arg(long, global = true, value_enum, default_value_t = DiagFormat::Human)]
    pub(crate) format: DiagFormat,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Cmd {
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
pub(crate) enum DiagFormat {
    /// Graphical diagnostics (severity, code, message, source snippet) for humans.
    #[default]
    Human,
    /// A stable `aozora-md.diagnostics.v1` JSON envelope for tooling.
    Json,
}
