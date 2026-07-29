//! `aozora-flavored-markdown-epub` CLI — thin clap wrapper over the
//! `aozora_flavored_markdown_epub` library, plus the diagnostic reporting
//! that library leaves to whoever owns the terminal.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the CLI renders requested output and diagnostics to its standard streams"
)]

use std::process::ExitCode;

mod app;
mod args;
mod output;

// A shim, deliberately: `main.rs` is no longer excused from the coverage
// denominator (`_COV_IGNORE`), so the entry point is measured like the rest of
// `src/` — the CLI integration tests reach these regions through the spawned
// binary.
fn main() -> ExitCode {
    app::run()
}
