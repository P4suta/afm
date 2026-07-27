//! End-to-end integration tests for the `aozora-flavored-markdown` CLI binary.
//!
//! Uses `CARGO_BIN_EXE_aozora-flavored-markdown` (set by cargo for each `[[bin]]` target
//! during `cargo test`) so no `assert_cmd` dependency is pulled in.
//! Each test writes a temp file, invokes the binary with a specific
//! argument shape, and asserts on stdout / stderr / exit status.
//!
//! Coverage targets:
//!
//! - Default UTF-8 path: plain text, Aozora ruby, bracket annotations
//!   all render through the lexer + splice pipeline and reach stdout.
//! - Shift_JIS path (`--encoding sjis`): the same pipeline accepts
//!   legacy Aozora .txt byte streams without pre-conversion.
//! - `check` subcommand: no-op on valid input, diagnostic on invalid.
//! - Help / version plumbing (clap); CLI must advertise itself.
//! - Failure modes: missing file, unreadable path, SJIS decode errors
//!   — each exits non-zero with a Japanese error message.

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output, Stdio};
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Absolute path to the freshly-built `aozora-flavored-markdown` binary, provided by cargo
/// for every integration test in this crate.
fn aozora_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aozora-flavored-markdown"))
}

/// Write `contents` to a new unique file under `/tmp`. Returns the
/// path; the file is cleaned up on test-process exit (we don't fuss
/// with explicit cleanup — tmp files are small and the test directory
/// is ephemeral inside the Docker sandbox).
fn write_temp_bytes(contents: &[u8], suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backward — filesystem tests cannot proceed")
        .subsec_nanos();
    let pid = process::id();
    // Include both pid and nanos so multiple tests running in parallel
    // don't collide on the same path.
    let path = env::temp_dir().join(format!("aozora_cli_test_{pid}_{nanos}{suffix}"));
    let mut f = File::create(&path).expect("temp file create");
    f.write_all(contents).expect("temp file write");
    path
}

fn write_temp_utf8(contents: &str) -> PathBuf {
    write_temp_bytes(contents.as_bytes(), ".md")
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(aozora_bin())
        .args(args)
        .output()
        .expect("spawn aozora-flavored-markdown binary")
}

/// Like `run_cli` but feeds `stdin` to the child's standard input, so we can
/// exercise the `-` (read-from-stdin) input path. The piped `ChildStdin` is
/// dropped right after the write, closing the pipe so the child sees EOF; our
/// inputs are tiny, so writing before reading cannot deadlock the OS buffer.
fn run_cli_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(aozora_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora-flavored-markdown binary");
    child
        .stdin
        .take()
        .expect("child stdin is piped")
        .write_all(stdin)
        .expect("write child stdin");
    child
        .wait_with_output()
        .expect("wait for aozora-flavored-markdown binary")
}

/// Run `aozora-flavored-markdown` with a hermetic environment: the colour-related vars that
/// otherwise leak in from CI are cleared, then `envs` is applied. Lets the
/// `--color` / `NO_COLOR` / `CLICOLOR_FORCE` tests assert on a known baseline.
fn run_cli_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(aozora_bin());
    cmd.args(args)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("RUST_LOG");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("spawn aozora-flavored-markdown binary")
}

/// A unique, not-yet-created path under the temp dir — for `-o <file>` tests.
fn unique_temp_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backward")
        .subsec_nanos();
    env::temp_dir().join(format!("aozora_cli_out_{}_{nanos}{suffix}", process::id()))
}

fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("expected valid JSON, got {text:?}: {e}"))
}

fn stdout_of(out: &Output) -> &str {
    str::from_utf8(&out.stdout).expect("stdout must be UTF-8")
}

fn stderr_of(out: &Output) -> &str {
    str::from_utf8(&out.stderr).expect("stderr must be UTF-8")
}

// ---------------------------------------------------------------------------
// Help / version plumbing
// ---------------------------------------------------------------------------

#[test]
fn help_flag_succeeds_and_mentions_subcommands() {
    let out = run_cli(&["--help"]);
    assert!(
        out.status.success(),
        "--help must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("render"),
        "--help must list `render` subcommand, got {stdout:?}"
    );
    assert!(
        stdout.contains("check"),
        "--help must list `check` subcommand, got {stdout:?}"
    );
}

#[test]
fn version_flag_succeeds_and_emits_non_empty_output() {
    let out = run_cli(&["--version"]);
    assert!(out.status.success(), "--version must exit 0");
    assert!(
        !stdout_of(&out).trim().is_empty(),
        "--version must print a non-empty version string"
    );
}

#[test]
fn help_strict_text_matches_behavior() {
    // The `--strict` help once claimed it only fired on "unknown
    // annotation", but the flag promotes *any* lexer diagnostic. Guard
    // against the stale wording creeping back, and assert the corrected
    // text mentions diagnostics.
    let out = run_cli(&["--help"]);
    assert!(out.status.success(), "--help must exit 0");
    let stdout = stdout_of(&out);
    assert!(
        !stdout.contains("annotation"),
        "--strict help must not claim 'unknown annotation', got {stdout:?}"
    );
    assert!(
        stdout.contains("diagnostic"),
        "--strict help must describe diagnostics, got {stdout:?}"
    );
}

// ---------------------------------------------------------------------------
// stdin (`-`) input path
// ---------------------------------------------------------------------------

#[test]
fn render_reads_from_stdin_dash() {
    let out = run_cli_stdin(&["render", "-"], b"Hello, world.");
    assert!(
        out.status.success(),
        "render from stdin must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("<p>Hello, world.</p>"),
        "stdin body must render to <p>, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn render_stdin_sjis_decodes() {
    // 「青空文庫」 in Shift_JIS — same bytes as the file-based SJIS test,
    // proving `--encoding sjis` also applies to the stdin byte stream.
    let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
    let out = run_cli_stdin(&["--encoding", "sjis", "render", "-"], bytes);
    assert!(
        out.status.success(),
        "SJIS stdin render must succeed, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("青空文庫"),
        "decoded stdin text must reach the HTML output, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn check_reads_from_stdin_dash() {
    let out = run_cli_stdin(&["check", "-"], "｜青梅《おうめ》".as_bytes());
    assert!(
        out.status.success(),
        "check from stdin must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).is_empty(),
        "check must not emit on stdout, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn render_stdin_dash_undecodable_utf8_fails() {
    let out = run_cli_stdin(&["render", "-"], &[0x80, 0x81]);
    assert!(
        !out.status.success(),
        "invalid UTF-8 on stdin must exit non-zero"
    );
    assert!(
        stderr_of(&out).contains("UTF-8"),
        "error must mention UTF-8, got {:?}",
        stderr_of(&out)
    );
}

// ---------------------------------------------------------------------------
// `render` subcommand — UTF-8 path
// ---------------------------------------------------------------------------

#[test]
fn render_plain_utf8_to_html_on_stdout() {
    let path = write_temp_utf8("Hello, world.");
    let out = run_cli(&["render", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "render plain UTF-8 must succeed, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("<p>Hello, world.</p>"),
        "render output must wrap plain text in <p>, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn render_aozora_ruby_emits_ruby_tag() {
    let path = write_temp_utf8("｜青梅《おうめ》へ");
    let out = run_cli(&["render", path.to_str().unwrap()]);
    assert!(out.status.success(), "render must succeed");
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("<ruby>青梅"),
        "ruby recognition missing in CLI output, got {stdout:?}"
    );
    assert!(
        stdout.contains("<rt>おうめ"),
        "ruby reading missing, got {stdout:?}"
    );
}

#[test]
fn render_unknown_annotation_wraps_in_hidden_span() {
    let path = write_temp_utf8("前［＃ほげふが］後");
    let out = run_cli(&["render", path.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains(r#"<span class="aozora-md-directive" hidden>"#),
        "unknown ［＃...］ must wrap as aozora-md-directive span, got {stdout:?}"
    );
}

#[test]
fn render_page_break_emits_block_level_div() {
    let path = write_temp_utf8("前\n\n［＃改ページ］\n\n後");
    let out = run_cli(&["render", path.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains(r#"<div class="aozora-md-page-break"></div>"#),
        "page break div missing, got {stdout:?}"
    );
}

// ---------------------------------------------------------------------------
// `render` subcommand — Shift_JIS path
// ---------------------------------------------------------------------------

#[test]
fn render_sjis_file_with_explicit_flag() {
    // 「青空文庫」 in Shift_JIS.
    let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
    let path = write_temp_bytes(bytes, ".txt");
    let out = run_cli(&["--encoding", "sjis", "render", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "SJIS render must succeed, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("青空文庫"),
        "decoded text must reach the HTML output, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn sjis_decode_failure_exits_nonzero_with_japanese_message() {
    // Invalid lead byte (0xFF) — SJIS decode must bail.
    let path = write_temp_bytes(&[0xFF, 0xFF, 0xFF], ".txt");
    let out = run_cli(&["--encoding", "sjis", "render", path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "SJIS decode of invalid bytes must exit non-zero"
    );
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("Shift_JIS"),
        "error must mention Shift_JIS, got {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `check` subcommand
// ---------------------------------------------------------------------------

#[test]
fn check_succeeds_on_valid_utf8_input() {
    let path = write_temp_utf8("｜青梅《おうめ》");
    let out = run_cli(&["check", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "check must exit 0 on valid input, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).is_empty(),
        "check must not emit on stdout, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn check_fails_on_undecodable_utf8() {
    // A raw 0x80 byte is not valid UTF-8 — read_input rejects it when
    // encoding=utf8 (default).
    let path = write_temp_bytes(&[0x80, 0x81], ".md");
    let out = run_cli(&["check", path.to_str().unwrap()]);
    assert!(!out.status.success(), "invalid UTF-8 must exit non-zero");
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("UTF-8"),
        "error must mention UTF-8, got {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// Failure modes
// ---------------------------------------------------------------------------

#[test]
fn render_missing_file_exits_nonzero_with_japanese_message() {
    let path = Path::new("/tmp/this_path_does_not_exist_ever_aozora_md_cli_test");
    let out = run_cli(&["render", path.to_str().unwrap()]);
    assert!(!out.status.success(), "missing file must exit non-zero");
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("入力ファイル"),
        "error must be in Japanese and mention 入力ファイル, got {stderr:?}"
    );
}

#[test]
fn missing_subcommand_prints_usage_and_exits_nonzero() {
    let out = run_cli(&[]);
    assert!(
        !out.status.success(),
        "running with no args must fail (subcommand required)"
    );
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("Usage") || stderr.contains("USAGE") || stderr.contains("使い方"),
        "no-args error must include usage hint, got {stderr:?}"
    );
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    let out = run_cli(&["invalidsubcmd"]);
    assert!(
        !out.status.success(),
        "unknown subcommand must exit non-zero"
    );
}

// ---------------------------------------------------------------------------
// --strict + diagnostics flow
// ---------------------------------------------------------------------------

/// Craft an input the lexer *will* complain about. The Phase 2
/// balanced-stack walk flags an orphan `》` (ruby close with no
/// matching open) as `aozora-flavored-markdown::lex::unmatched_close` — stable across
/// classifier iterations because the trigger table always pairs
/// `《` with `》`. If a future rewrite elides this shape silently,
/// the strict-mode test fails and forces the author to pick a
/// replacement canary that actually fires.
const DIAGNOSTIC_INPUT: &str = "orphan》close";

#[test]
fn check_without_strict_passes_even_with_diagnostics() {
    // Plain `check` must remain a syntax sanity-check that succeeds
    // on well-formed UTF-8 regardless of diagnostic count.
    let path = write_temp_utf8("hello");
    let out = run_cli(&["check", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "plain check must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
}

#[test]
fn render_strict_without_diagnostics_succeeds() {
    // --strict must NOT fail when the lexer produces zero diagnostics.
    let path = write_temp_utf8("clean input");
    let out = run_cli(&["--strict", "render", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "strict render on clean input must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("<p>clean input</p>"),
        "render output must appear on stdout under strict mode, got {:?}",
        stdout_of(&out)
    );
}

/// When any lexer diagnostic fires under `--strict`, the binary
/// must:
///
/// * exit non-zero
/// * NOT print the HTML on stdout
/// * print a short Japanese error including the count
///
/// The diagnostic body itself lands on stderr via `emit_diagnostics`
/// so tooling can parse the `aozora-flavored-markdown::…` code.
#[test]
fn render_strict_with_lexer_diagnostic_fails_nonzero() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let baseline = run_cli(&["check", path.to_str().unwrap()]);
    // Sanity: non-strict check must succeed (prove the diagnostic is
    // non-fatal in the default mode).
    assert!(
        baseline.status.success(),
        "non-strict check must still exit 0 on diagnostic-heavy input"
    );
    let strict = run_cli(&["--strict", "render", path.to_str().unwrap()]);
    assert!(
        !strict.status.success(),
        "--strict must turn the lexer diagnostic into a hard error"
    );
    let stderr = stderr_of(&strict);
    assert!(
        stderr.contains("--strict") || stderr.contains("診断"),
        "strict failure message must reference --strict or 診断, got {stderr:?}"
    );
}

#[test]
fn check_strict_on_diagnostic_input_fails() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["--strict", "check", path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "check --strict with lexer diagnostic must exit non-zero"
    );
}

// ---------------------------------------------------------------------------
// Exit-code contract: 0 success / 1 generic error / 2 strict diagnostic
// ---------------------------------------------------------------------------

#[test]
fn strict_diagnostic_exits_with_code_two() {
    // `ref/cli.md` promises exit code 2 specifically for a strict-mode
    // diagnostic, distinct from generic failures (code 1).
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["--strict", "render", path.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "strict diagnostic must exit with code 2, stderr = {:?}",
        stderr_of(&out)
    );
}

#[test]
fn check_strict_diagnostic_exits_code_two() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["--strict", "check", path.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "check --strict diagnostic must exit with code 2, stderr = {:?}",
        stderr_of(&out)
    );
}

#[test]
fn generic_error_exits_with_code_one() {
    // A plain I/O failure must stay code 1 so it is distinguishable from
    // the strict-diagnostic code 2.
    let out = run_cli(&[
        "render",
        "/tmp/this_path_does_not_exist_ever_aozora_md_cli_test",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "generic I/O error must exit with code 1, stderr = {:?}",
        stderr_of(&out)
    );
}

#[test]
fn diagnostics_print_to_stderr_with_aozora_code() {
    // Even without --strict, diagnostics surface on stderr so tooling
    // (Language servers, CI grep) can react. The lexer lives in the
    // sibling `aozora` crate, so the diagnostic codes carry the
    // `aozora::…` prefix from upstream. Human output is now a miette
    // graphical block; `--color never` keeps it deterministic (no ANSI).
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["--color", "never", "check", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "check on diagnostic-heavy input must still exit 0 without --strict"
    );
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("aozora::lex::unmatched_close"),
        "stderr must carry the stable diagnostic code, got {stderr:?}"
    );
    assert!(
        !stderr.contains(ESC),
        "--color never must not emit ANSI, got {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `--output` / `-o`
// ---------------------------------------------------------------------------

#[test]
fn render_output_file_writes_html() {
    let src = write_temp_utf8("Hello, world.");
    let dst = unique_temp_path(".html");
    let out = run_cli(&["render", src.to_str().unwrap(), "-o", dst.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "render -o must succeed, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).is_empty(),
        "render -o <file> must not also print HTML to stdout, got {:?}",
        stdout_of(&out)
    );
    let written = fs::read_to_string(&dst).expect("output file must exist");
    assert!(
        written.contains("<p>Hello, world.</p>"),
        "output file must contain the rendered HTML, got {written:?}"
    );
}

#[test]
fn render_output_dash_is_stdout() {
    let src = write_temp_utf8("Hello, world.");
    let out = run_cli(&["render", src.to_str().unwrap(), "-o", "-"]);
    assert!(out.status.success(), "render -o - must succeed");
    assert!(
        stdout_of(&out).contains("<p>Hello, world.</p>"),
        "-o - must write HTML to stdout, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn render_output_to_unwritable_path_fails() {
    let src = write_temp_utf8("Hello, world.");
    // The parent directory does not exist, so the file cannot be created.
    let out = run_cli(&[
        "render",
        src.to_str().unwrap(),
        "-o",
        "/tmp/aozora_no_such_dir_xyz/out.html",
    ]);
    assert!(
        !out.status.success(),
        "unwritable -o path must exit non-zero"
    );
    assert!(
        stderr_of(&out).contains("出力ファイル"),
        "error must mention 出力ファイル, got {:?}",
        stderr_of(&out)
    );
}

// ---------------------------------------------------------------------------
// `--color` / NO_COLOR / CLICOLOR_FORCE
// ---------------------------------------------------------------------------

/// A non-existent input drives the miette error report, which is where the
/// colour choice is observable.
const MISSING_INPUT: &str = "/tmp/this_path_does_not_exist_ever_aozora_md_cli_color";

/// ANSI escape introducer (ESC).
const ESC: char = '\u{1b}';

#[test]
fn color_always_emits_ansi_even_when_piped() {
    let out = run_cli_env(&["--color", "always", "render", MISSING_INPUT], &[]);
    assert!(!out.status.success(), "missing file must still fail");
    assert!(
        stderr_of(&out).contains(ESC),
        "--color always must emit ANSI, got {:?}",
        stderr_of(&out)
    );
}

#[test]
fn color_never_suppresses_ansi() {
    let out = run_cli_env(&["--color", "never", "render", MISSING_INPUT], &[]);
    assert!(!out.status.success());
    assert!(
        !stderr_of(&out).contains(ESC),
        "--color never must not emit ANSI, got {:?}",
        stderr_of(&out)
    );
}

#[test]
fn no_color_env_disables_color() {
    let out = run_cli_env(&["render", MISSING_INPUT], &[("NO_COLOR", "1")]);
    assert!(!out.status.success());
    assert!(
        !stderr_of(&out).contains(ESC),
        "NO_COLOR must disable ANSI under the default auto mode, got {:?}",
        stderr_of(&out)
    );
}

#[test]
fn clicolor_force_enables_color() {
    let out = run_cli_env(&["render", MISSING_INPUT], &[("CLICOLOR_FORCE", "1")]);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains(ESC),
        "CLICOLOR_FORCE must enable ANSI even when stderr is piped, got {:?}",
        stderr_of(&out)
    );
}

// ---------------------------------------------------------------------------
// `-v` / `-q` verbosity plumbing
// ---------------------------------------------------------------------------

#[test]
fn verbose_and_quiet_flags_parse_and_run() {
    let src = write_temp_utf8("clean input");
    let path = src.to_str().unwrap();
    let cases: [&[&str]; 4] = [&["-v"], &["-vvv"], &["-q"], &["-qq"]];
    for flag in cases {
        let mut args: Vec<&str> = flag.to_vec();
        args.push("render");
        args.push(path);
        let out = run_cli(&args);
        assert!(
            out.status.success(),
            "{flag:?} render must exit 0, stderr = {:?}",
            stderr_of(&out)
        );
        assert!(
            stdout_of(&out).contains("<p>clean input</p>"),
            "{flag:?} must still render to stdout, got {:?}",
            stdout_of(&out)
        );
    }
}

// ---------------------------------------------------------------------------
// `--format json` machine-readable diagnostics (aozora-md.diagnostics.v1, ADR-0012)
// ---------------------------------------------------------------------------

#[test]
fn check_json_emits_valid_envelope_on_stdout() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["check", "--format", "json", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "non-strict check --format json must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    let v = parse_json(stdout_of(&out));
    assert_eq!(v["schema"], "aozora-md.diagnostics.v1");
    assert!(
        !v["diagnostics"]
            .as_array()
            .expect("diagnostics array")
            .is_empty(),
        "expected at least one diagnostic, got {v}"
    );
}

#[test]
fn check_json_clean_input_is_empty_array() {
    let path = write_temp_utf8("clean input");
    let out = run_cli(&["check", "--format", "json", path.to_str().unwrap()]);
    assert!(out.status.success());
    let v = parse_json(stdout_of(&out));
    assert_eq!(v["schema"], "aozora-md.diagnostics.v1");
    assert!(
        v["diagnostics"].as_array().expect("array").is_empty(),
        "clean input must yield an empty diagnostics array, got {v}"
    );
}

#[test]
fn check_json_schema_fields_present() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["check", "--format", "json", path.to_str().unwrap()]);
    let v = parse_json(stdout_of(&out));
    let d = &v["diagnostics"][0];
    for field in ["code", "severity", "source", "message", "line", "column"] {
        assert!(
            !d[field].is_null(),
            "field {field} must be present, got {d}"
        );
    }
    assert!(
        !d["span"]["start"].is_null() && !d["span"]["end"].is_null(),
        "span.start / span.end must be present, got {d}"
    );
    assert!(
        d["code"].as_str().unwrap().starts_with("aozora::"),
        "code must carry the aozora:: prefix, got {d}"
    );
    let severity = d["severity"].as_str().unwrap();
    assert!(
        matches!(severity, "error" | "warning" | "note"),
        "severity must be a stable wire string, got {severity}"
    );
}

#[test]
fn json_stable_codes() {
    // Pins the public contract: the canary input yields this exact code.
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["check", "--format", "json", path.to_str().unwrap()]);
    let v = parse_json(stdout_of(&out));
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "aozora::lex::unmatched_close", "got {v}");
    assert_eq!(d["severity"], "error");
    assert_eq!(d["source"], "source");
}

#[test]
fn render_json_diagnostics_go_to_stderr_html_to_stdout() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["render", "--format", "json", path.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(
        stdout_of(&out).contains('<'),
        "render stdout must stay HTML, got {:?}",
        stdout_of(&out)
    );
    let v = parse_json(stderr_of(&out));
    assert_eq!(v["schema"], "aozora-md.diagnostics.v1");
    assert!(!v["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn check_json_strict_stdout_is_pure_json() {
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&[
        "--strict",
        "check",
        "--format",
        "json",
        path.to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "strict json must still exit 2, stderr = {:?}",
        stderr_of(&out)
    );
    // No free-form Japanese line is allowed to corrupt the stdout JSON.
    let v = parse_json(stdout_of(&out));
    assert!(!v["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn json_line_col_is_one_based() {
    let path = write_temp_utf8("first line\norphan》close");
    let out = run_cli(&["check", "--format", "json", path.to_str().unwrap()]);
    let v = parse_json(stdout_of(&out));
    let d = &v["diagnostics"][0];
    assert_eq!(
        d["line"], 2,
        "diagnostic on the 2nd line must report line 2, got {v}"
    );
    // Pinned to the column, not merely to `>= 1`. That weaker assertion is
    // true of the constant 1, which is what a resolver that fails to count
    // returns — and the count is the whole of what this field is.
    assert_eq!(
        d["column"], 7,
        "the orphan is the 7th character of its line, got {v}"
    );
}

#[test]
fn json_column_counts_characters_and_not_bytes() {
    // The envelope publishes a *character* column, and every source this
    // tool exists for is CJK — where a byte column is not merely a different
    // number but a wrong one, three times over. Nothing pinned that: the one
    // line/column case above is ASCII, in which the two agree.
    let path = write_temp_utf8("序章\n日本語の》close");
    let out = run_cli(&["check", "--format", "json", path.to_str().unwrap()]);
    let v = parse_json(stdout_of(&out));
    let d = &v["diagnostics"][0];
    assert_eq!(d["line"], 2, "the orphan is on the second line, got {v}");
    assert_eq!(
        d["column"], 5,
        "the orphan is the 5th character of its line (the 13th byte), got {v}"
    );
}

#[test]
fn check_and_render_report_the_same_diagnostics() {
    // `check` stopped calling `render`: it asks the library for the
    // diagnostics alone. That is the same command only if the two paths
    // answer identically — and the shape that would hide a difference is
    // exactly the one that hid the old defect, since `check` prints no HTML
    // either way. Compared envelope to envelope, on stdout for one and
    // stderr for the other.
    let sources = [
        "clean input",
        "orphan》close",
        "｜青梅《おうめ》に行った",
        "```\n｜青梅《おうめ》\n```\n",
        "第一行\norphan》close\n《third",
        "# 見出し\n\n本文\n\n［＃改ページ］\n",
    ];
    for src in sources {
        let path = write_temp_utf8(src);
        let arg = path.to_str().unwrap();
        let checked = run_cli(&["check", "--format", "json", arg]);
        let rendered = run_cli(&["render", "--format", "json", arg]);
        assert_eq!(
            parse_json(stdout_of(&checked)),
            parse_json(stderr_of(&rendered)),
            "check and render must publish one envelope for {src:?}"
        );
        for stream in [stdout_of(&checked), stderr_of(&checked)] {
            for markup in ["<p", "<ruby", "<h1", "<div"] {
                assert!(
                    !stream.contains(markup),
                    "check must emit no HTML for {src:?}, found {markup} in {stream:?}"
                );
            }
        }
        // The verdict has to travel with the diagnostics, or `check --strict`
        // in CI passes a source `render --strict` would refuse.
        let checked = run_cli(&["--strict", "check", "--format", "json", arg]);
        let rendered = run_cli(&["--strict", "render", "--format", "json", arg]);
        assert_eq!(
            checked.status.code(),
            rendered.status.code(),
            "check --strict and render --strict must agree on {src:?}"
        );
    }
}

#[test]
fn human_format_is_graphical() {
    // The default human format renders miette's graphical diagnostic
    // (severity, code, message, source snippet) on stderr — not the old
    // `diagnostic [code]: message` line. JSON output is unaffected.
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["--color", "never", "check", path.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(
        stderr_of(&out).contains("aozora::lex::unmatched_close"),
        "graphical human output must still carry the code, got {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).is_empty(),
        "human check must keep stdout empty, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn human_diagnostic_snippet_shows_source_and_no_ansi_with_color_never() {
    // The graphical block must include a snippet of the offending source,
    // and `--color never` must keep it ANSI-free for deterministic capture.
    let path = write_temp_utf8(DIAGNOSTIC_INPUT);
    let out = run_cli(&["--color", "never", "check", path.to_str().unwrap()]);
    assert!(out.status.success());
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("orphan"),
        "graphical output must show the offending source line, got {stderr:?}"
    );
    assert!(
        !stderr.contains(ESC),
        "--color never must not emit ANSI, got {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `completions` / hidden `_man` / `--help` examples
// ---------------------------------------------------------------------------

#[test]
fn completions_emit_per_shell_markers() {
    // Each generator stamps a shell-specific marker we can key on without
    // pinning the (large, version-sensitive) full script.
    let cases = [
        ("bash", "_aozora-flavored-markdown()"),
        ("zsh", "#compdef aozora-flavored-markdown"),
        ("fish", "complete -c aozora-flavored-markdown"),
        ("powershell", "Register-ArgumentCompleter"),
        ("elvish", "edit:completion"),
    ];
    for (shell, marker) in cases {
        let out = run_cli(&["completions", shell]);
        assert!(
            out.status.success(),
            "completions {shell} must exit 0, stderr = {:?}",
            stderr_of(&out)
        );
        assert!(
            stdout_of(&out).contains(marker),
            "completions {shell} must contain {marker:?}"
        );
    }
}

#[test]
fn completions_unknown_shell_fails() {
    let out = run_cli(&["completions", "tcsh"]);
    assert!(
        !out.status.success(),
        "an unsupported shell must be rejected"
    );
}

#[test]
fn hidden_man_subcommand_renders_roff() {
    let out = run_cli(&["_man"]);
    assert!(
        out.status.success(),
        "_man must exit 0, stderr = {:?}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains(".TH aozora-flavored-markdown"),
        "_man must render a roff man page (.TH aozora-flavored-markdown), got {:.120}",
        stdout_of(&out)
    );
}

#[test]
fn man_subcommand_is_hidden_from_help() {
    let out = run_cli(&["--help"]);
    assert!(out.status.success());
    assert!(
        !stdout_of(&out).contains("_man"),
        "the _man helper must not appear in --help, got {:?}",
        stdout_of(&out)
    );
}

#[test]
fn help_shows_examples() {
    let out = run_cli(&["--help"]);
    assert!(out.status.success());
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("EXAMPLES"),
        "--help must show an EXAMPLES section, got {stdout:?}"
    );
    assert!(
        stdout.contains("aozora-flavored-markdown completions"),
        "--help examples must mention completions, got {stdout:?}"
    );
}
