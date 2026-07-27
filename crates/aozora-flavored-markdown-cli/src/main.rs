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

// A shim, deliberately: this file used to hold the whole CLI, and `_COV_IGNORE`
// used to drop it from the coverage denominator for being a `main.rs`. That
// exclusion is gone now that there is nothing left here to hide — the CLI
// integration tests reach these regions through the spawned binary — so
// whatever grows back into this file is measured like the rest of `src/`.
fn main() -> ExitCode {
    app::run()
}
