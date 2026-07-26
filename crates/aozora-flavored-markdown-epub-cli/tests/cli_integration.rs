//! End-to-end tests for the `aozora-flavored-markdown-epub` binary.
//!
//! There were none. The crate's only tests were `#[cfg(test)]` calls to `run`
//! in the same file, which is a different program from the one a user runs:
//! `main` maps the returned `ExitCode` onto the process, and *that* mapping is
//! the whole content of the `--strict` contract. The sibling CLI has had this
//! file since it grew exit codes; this one shipped a `--strict` flag, a JSON
//! envelope and an exit code 2 with nothing running the binary at all.
//!
//! `CARGO_BIN_EXE_…` is set by cargo for each `[[bin]]` target, so the binary
//! under test is the one this build just produced and no `assert_cmd`
//! dependency is needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str;

use serde_json::Value;
use tempfile::TempDir;

fn epub_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aozora-flavored-markdown-epub"))
}

const BOOK: &str = "title = \"Test Book\"\ncreator = \"Test Author\"\nlanguage = \"ja\"\n";

/// The canary the sibling CLI's suite uses, for the same reason: the balanced
/// stack always pairs `《` with `》`, so an orphan close is the one shape a
/// classifier rewrite cannot quietly stop reporting without a test noticing.
const DIAGNOSTIC_CHAPTER: &str = "first line\norphan》close\n";

const CLEAN_CHAPTER: &str = "# 第一章\n\n本文。\n";

/// A manuscript directory plus its `book.toml`, in a temp dir removed on drop.
fn fixture(chapters: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    fs::write(dir.path().join("book.toml"), BOOK).expect("write book.toml");
    let manuscript = dir.path().join("manuscript");
    fs::create_dir(&manuscript).expect("create manuscript dir");
    for (name, body) in chapters {
        fs::write(manuscript.join(name), body).expect("write chapter");
    }
    dir
}

/// Run `build` on a fixture, with whatever global flags precede it.
///
/// `NO_COLOR` is set because the human report goes through miette's graphical
/// handler, and this CLI has no `--color` of its own to pin it with.
fn build_in(dir: &Path, flags: &[&str]) -> (Output, PathBuf) {
    let out = dir.join("book.epub");
    let output = Command::new(epub_bin())
        .args(flags)
        .args([
            "build",
            "--input",
            dir.join("manuscript").to_str().expect("utf8 input"),
            "--metadata",
            dir.join("book.toml").to_str().expect("utf8 metadata"),
            "--output",
            out.to_str().expect("utf8 output"),
        ])
        .env("NO_COLOR", "1")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .expect("spawn the epub binary");
    (output, out)
}

fn stdout_of(out: &Output) -> &str {
    str::from_utf8(&out.stdout).expect("stdout must be UTF-8")
}

fn stderr_of(out: &Output) -> &str {
    str::from_utf8(&out.stderr).expect("stderr must be UTF-8")
}

fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("expected valid JSON, got {text:?}: {e}"))
}

// ---------------------------------------------------------------------------
// exit codes — 0 clean / 0 observed / 2 strict-with-diagnostics / 1 failure
// ---------------------------------------------------------------------------

#[test]
fn a_clean_book_exits_zero_and_reports_nothing() {
    let dir = fixture(&[("001.md", CLEAN_CHAPTER)]);
    let (out, epub) = build_in(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a clean build must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stderr_of(&out).is_empty(),
        "a clean book must print no diagnostics, got {:?}",
        stderr_of(&out)
    );
    assert!(epub.exists(), "the .epub must be written");
}

#[test]
fn a_chapter_diagnostic_without_strict_still_exits_zero() {
    // The default is the same verdict the library takes: a diagnostic is an
    // observation. It has to reach the terminal all the same — this is the
    // half that was dropped on the floor before the report existed.
    let dir = fixture(&[("001.md", DIAGNOSTIC_CHAPTER)]);
    let (out, epub) = build_in(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a diagnostic alone must not fail the run, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stderr_of(&out).contains("aozora::lex::unmatched_close"),
        "the stable diagnostic code must reach stderr, got {:?}",
        stderr_of(&out)
    );
    assert!(epub.exists(), "the .epub must still be written");
}

#[test]
fn strict_turns_a_chapter_diagnostic_into_exit_code_two() {
    // Two specifically, not merely non-zero: ADR-0012 gives the sibling CLI
    // 2 for a strict diagnostic and 1 for everything else, and a `--strict`
    // that exited 1 would be indistinguishable from a missing book.toml to
    // every CI script that reads the code.
    let dir = fixture(&[("001.md", DIAGNOSTIC_CHAPTER)]);
    let (out, _) = build_in(dir.path(), &["--strict"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "strict + diagnostic must exit 2, stderr = {:?}",
        stderr_of(&out)
    );
}

#[test]
fn strict_on_a_clean_book_exits_zero() {
    let dir = fixture(&[("001.md", CLEAN_CHAPTER)]);
    let (out, epub) = build_in(dir.path(), &["--strict"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "strict must not fail a book that raised nothing, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(epub.exists(), "the .epub must be written");
}

#[test]
fn strict_is_a_verdict_on_the_run_and_not_a_veto_on_the_output() {
    // This CLI differs from the sibling here: `--strict` suppresses the
    // sibling's HTML because stdout is where it goes, while packaging is the
    // phase *before* this one returns, so the file is on disk by the time
    // anything can object. Pinned rather than left to be rediscovered — a
    // later "make it consistent" would silently change what a CI job that
    // exits 2 has already produced.
    let dir = fixture(&[("001.md", DIAGNOSTIC_CHAPTER)]);
    let (out, epub) = build_in(dir.path(), &["--strict"]);
    assert_eq!(out.status.code(), Some(2), "the run still fails");
    assert!(
        epub.exists(),
        "the .epub must survive a strict failure, or the flag is a veto and its help text lies"
    );
}

#[test]
fn a_failure_that_is_not_a_diagnostic_exits_one() {
    let dir = fixture(&[("001.md", CLEAN_CHAPTER)]);
    fs::remove_file(dir.path().join("book.toml")).expect("remove book.toml");
    let (out, _) = build_in(dir.path(), &["--strict"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a missing book.toml is code 1, not the strict-diagnostic 2, stderr = {:?}",
        stderr_of(&out)
    );
}

// ---------------------------------------------------------------------------
// `--format json` — the `aozora-md.diagnostics.v1` envelope, per chapter
// ---------------------------------------------------------------------------

#[test]
fn json_writes_the_envelope_on_stdout() {
    let dir = fixture(&[("001.md", DIAGNOSTIC_CHAPTER)]);
    let (out, _) = build_in(dir.path(), &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "non-strict json exits 0");
    let json = parse_json(stdout_of(&out));
    assert_eq!(
        json["schema"], "aozora-md.diagnostics.v1",
        "the schema name is published, got {json}"
    );
    let diagnostics = json["diagnostics"]
        .as_array()
        .expect("`diagnostics` is an array");
    assert!(!diagnostics.is_empty(), "expected a diagnostic, got {json}");
    for field in [
        "path", "code", "severity", "source", "message", "line", "column",
    ] {
        assert!(
            !diagnostics[0][field].is_null(),
            "field {field} must be present, got {}",
            diagnostics[0]
        );
    }
    assert!(
        !diagnostics[0]["span"]["start"].is_null() && !diagnostics[0]["span"]["end"].is_null(),
        "span.start / span.end must be present, got {}",
        diagnostics[0]
    );
    assert_eq!(
        diagnostics[0]["code"], "aozora::lex::unmatched_close",
        "the canary's code is the published one, got {json}"
    );
    assert_eq!(
        diagnostics[0]["line"], 2,
        "the orphan is on the second line of the chapter, got {json}"
    );
}

#[test]
fn json_on_a_clean_book_is_an_empty_array_rather_than_no_output() {
    // Tooling parses stdout unconditionally; a clean run that printed nothing
    // would be a parse error at the other end.
    let dir = fixture(&[("001.md", CLEAN_CHAPTER)]);
    let (out, _) = build_in(dir.path(), &["--format", "json"]);
    let json = parse_json(stdout_of(&out));
    assert_eq!(json["schema"], "aozora-md.diagnostics.v1");
    assert!(
        json["diagnostics"].as_array().expect("array").is_empty(),
        "a clean book must yield an empty array, got {json}"
    );
}

#[test]
fn strict_json_keeps_stdout_pure_json() {
    // The strict summary line is Japanese prose; printing it on the JSON path
    // would corrupt the stream the exit code is telling the caller to read.
    let dir = fixture(&[("001.md", DIAGNOSTIC_CHAPTER)]);
    let (out, _) = build_in(dir.path(), &["--strict", "--format", "json"]);
    assert_eq!(out.status.code(), Some(2), "strict json still exits 2");
    let json = parse_json(stdout_of(&out));
    assert!(!json["diagnostics"].as_array().expect("array").is_empty());
}

#[test]
fn every_json_diagnostic_names_the_chapter_it_came_from() {
    // The envelope's one addition over the sibling's: a book has chapters, so
    // a diagnostic without a path is unattributable. Two chapters, one of them
    // clean, so "names the only file there was" cannot pass this.
    let dir = fixture(&[
        ("001-clean.md", CLEAN_CHAPTER),
        ("002-noisy.md", DIAGNOSTIC_CHAPTER),
    ]);
    let (out, _) = build_in(dir.path(), &["--format", "json"]);
    let json = parse_json(stdout_of(&out));
    let diagnostics = json["diagnostics"].as_array().expect("array");
    assert!(!diagnostics.is_empty(), "expected a diagnostic, got {json}");
    for diagnostic in diagnostics {
        let path = diagnostic["path"].as_str().expect("path is a string");
        assert!(
            path.ends_with("002-noisy.md"),
            "the diagnostic belongs to the noisy chapter, got {path}"
        );
    }
}

// ---------------------------------------------------------------------------
// the human report
// ---------------------------------------------------------------------------

#[test]
fn the_human_report_shows_the_offending_source_line() {
    // The caret is drawn into the text the report carries, which is the whole
    // reason `ChapterReport` holds the decoded chapter beside the spans. A
    // report without it degrades to a bare header, silently.
    let dir = fixture(&[("001.md", DIAGNOSTIC_CHAPTER)]);
    let (out, _) = build_in(dir.path(), &[]);
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("orphan"),
        "the graphical report must include the source snippet, got {stderr:?}"
    );
    assert!(
        stderr.contains("001.md"),
        "and name the chapter it came from, got {stderr:?}"
    );
    assert!(
        stdout_of(&out).is_empty(),
        "the human format must leave stdout alone, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn the_strict_summary_counts_diagnostics_rather_than_chapters() {
    // Two orphans in one chapter: a summary that counted `chapters` would say
    // 1 and read as correct.
    let dir = fixture(&[("001.md", "orphan》one\nand》two\n")]);
    let (out, _) = build_in(dir.path(), &["--strict"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr_of(&out).contains("2 件の診断"),
        "the strict line must report the diagnostic count, got {:?}",
        stderr_of(&out)
    );
}

// ---------------------------------------------------------------------------
// help / plumbing
// ---------------------------------------------------------------------------

#[test]
fn help_documents_strict_and_says_the_epub_is_still_written() {
    let out = Command::new(epub_bin())
        .arg("--help")
        .output()
        .expect("spawn the epub binary");
    assert!(out.status.success(), "--help must exit 0");
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("--strict"),
        "--help must list --strict, got {stdout:?}"
    );
    assert!(
        stdout.contains("--format"),
        "--help must list --format, got {stdout:?}"
    );
    assert!(
        stdout.contains("still written"),
        "the flag whose behaviour differs from the sibling CLI must say so in its help, \
         got {stdout:?}"
    );
}

#[test]
fn an_unknown_format_is_rejected_rather_than_defaulted() {
    let dir = fixture(&[("001.md", CLEAN_CHAPTER)]);
    let (out, _) = build_in(dir.path(), &["--format", "yaml"]);
    assert!(
        !out.status.success(),
        "an unsupported --format must be rejected, stdout = {:?}",
        stdout_of(&out)
    );
}
