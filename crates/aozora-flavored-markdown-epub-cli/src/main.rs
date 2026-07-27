//! `aozora-flavored-markdown-epub` CLI — thin clap wrapper over the
//! `aozora_flavored_markdown_epub` library, plus the diagnostic reporting
//! that library leaves to whoever owns the terminal.

#![forbid(unsafe_code)]

use std::process::ExitCode;

mod app;
mod args;
mod output;

// A shim, deliberately: `_COV_IGNORE`'s `/main\.rs$` drops whatever lives in
// this file from the coverage denominator, so the less that lives here the
// less that exclusion hides.
fn main() -> ExitCode {
    app::run()
}
