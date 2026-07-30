#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this command-line example prints serialized output and input errors"
)]

//! Parse an aozora-flavored-markdown source and confirm `canonicalize ∘ parse ≡ id` on
//! the lexer-normalised input, demonstrated on a single file.
//!
//! Run:
//!
//!     cargo run --example canonicalize-round-trip -p aozora-flavored-markdown -- input.md

use std::env;
use std::fs;
use std::process::ExitCode;

use aozora_flavored_markdown::canonicalize;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: canonicalize-round-trip <path/to/input.md>");
        return ExitCode::from(2);
    };

    let input = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let canonical = match canonicalize(&input) {
        Ok(s) => s,
        // The error type says which of the two it was, so print it rather
        // than restating a guess about the input.
        Err(e) => {
            eprintln!("failed to canonicalize {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Mirror the lexer's sanitize phase: strip UTF-8 BOM, CRLF → LF.
    // Anything the lexer normalises cannot round-trip byte-for-byte to
    // the *original* input; the contract is fixed-point on the
    // *normalised* input, which is what I3 checks in the corpus sweep.
    let expected = input
        .strip_prefix('\u{feff}')
        .unwrap_or(&input)
        .replace("\r\n", "\n");

    if canonical == expected {
        println!("round-trip OK ({} bytes)", canonical.len());
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "round-trip drift: {} bytes in (post-sanitize) → {} bytes out",
            expected.len(),
            canonical.len(),
        );
        ExitCode::from(3)
    }
}
