//! `aozora-flavored-markdown` — command-line interface. `--help` lists the
//! sub-commands.
//!
//! Input is a file path or `-` for stdin, decoded as UTF-8 or — with
//! `--encoding sjis` — Shift_JIS, so original Aozora Bunko `.txt`
//! distributions need no pre-conversion.

#![forbid(unsafe_code)]

use std::process::ExitCode;

mod app;
mod args;
mod output;

// A shim, deliberately: `_COV_IGNORE`'s `/main\.rs$` drops whatever lives in
// this file from the coverage denominator, and it used to hold the whole CLI.
// Keeping it to the entry point is what makes that exclusion mean "the two
// lines cargo calls" rather than "the binary".
fn main() -> ExitCode {
    app::run()
}
