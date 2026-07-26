//! Permanent regression cases lifted from cargo-fuzz artifacts.
//!
//! Whenever `just fuzz-deep <target>` (or `fuzz-quick`) flags an
//! input, run the artifact through `just fuzz-triage <target>` to see
//! the panic message, fix the underlying issue, then call
//! `just fuzz-promote <target> <artifact>` to move the input into
//! `tests/fuzz_regressions/<target>/`. From that point on, every
//! `just test` run replays the fixed-up case — no nightly toolchain
//! required, no need to keep libFuzzer warm just to re-prove an old
//! crash stays fixed.
//!
//! ## Layout
//!
//! ```text
//! tests/fuzz_regressions/
//!   parse_render/
//!     <hash>             ── raw byte payload, fed verbatim to the target
//!     <hash>.expect.txt  ── (optional) the panic snippet that originally
//!                            justified the regression case, kept for human
//!                            archaeology; not parsed by the test runner
//!   serialize_round_trip/
//!     ...
//! ```
//!
//! The test discovers artifacts by reading the directory at run time,
//! so a new file is picked up automatically and a target with nothing
//! promoted yet needs no directory. The discovery walk returns
//! artifacts in sorted order so failure messages stay stable across
//! machines and `nextest` runs.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::str;

use aozora::decode_sjis;
use aozora_flavored_markdown::{
    Options, RenderedBlocks, canonicalize, render, render_blocks, sentinels,
};
use aozora_flavored_markdown_test_support::{
    assert_html_invariants, check_fence_fidelity, check_no_sentinel_leak,
};

#[test]
fn parse_render_regressions_replay_cleanly() {
    replay_each(
        "parse_render",
        |src| {
            let html = render(src, &Options::default()).html;
            assert_html_invariants(src, &html);
        },
        ReplayInput::Utf8,
    );
}

#[test]
fn render_blocks_regressions_replay_cleanly() {
    replay_each(
        "render_blocks",
        |src| {
            // Mirrors the `render_blocks` target: Tier B per chunk, the
            // rest on the concatenation, which is the only form that owes
            // tag balance when a container spans blocks.
            let RenderedBlocks { blocks, .. } = render_blocks(src, &Options::default());
            let mut joined = String::new();
            for block in &blocks {
                if let Err(e) = check_no_sentinel_leak(src, &block.html) {
                    panic!(
                        "sentinel leaked into one block: {e:?}\n  html = {:?}",
                        block.html
                    );
                }
                joined.push_str(&block.html);
            }
            assert_html_invariants(src, &joined);
        },
        ReplayInput::Utf8,
    );
}

#[test]
fn serialize_round_trip_regressions_replay_cleanly() {
    replay_each(
        "serialize_round_trip",
        |src| {
            // Mirrors the target: I3 (`canonicalize(canonicalize(x)) ==
            // canonicalize(x)`) plus I5, which is the half a fixed point
            // cannot see — a consistently wrong rewrite satisfies I3 — plus
            // I8, the half neither can see, since a reserved codepoint
            // overwritten with U+FFFD is consistent and sits outside a fence.
            // I9, totality: a promoted artifact is a bounded byte payload, so
            // neither error variant is reachable and an `Err` is a finding
            // rather than a case to skip. Skipping it would restore exactly
            // what the old `""` failure value bought — I3 satisfied by an
            // output that does not exist, I5 and I8 with nothing to compare.
            let first = canonicalize(src)
                .unwrap_or_else(|e| panic!("I9 (totality) violated for src={src:?}: {e}"));
            let second = canonicalize(&first)
                .unwrap_or_else(|e| panic!("I9 (totality) violated for a canonical form: {e}"));
            assert!(
                first == second,
                "I3 fixed-point broken for src={src:?}\n  first  = {first:?}\n  second = {second:?}"
            );
            if let Err(e) = check_fence_fidelity(src, &first) {
                panic!("I5 (fence fidelity) violated for src={src:?}: {e:?}");
            }
            for reserved in sentinels::ALL {
                assert_eq!(
                    src.matches(reserved).count(),
                    first.matches(reserved).count(),
                    "I8 (reserved codepoint fidelity) violated for U+{:04X}\n  src = {src:?}\n  out = {first:?}",
                    reserved as u32
                );
            }
        },
        ReplayInput::Utf8,
    );
}

#[test]
fn sjis_decode_regressions_replay_cleanly() {
    replay_each(
        "sjis_decode",
        |text| {
            let html = render(text, &Options::default()).html;
            assert_html_invariants(text, &html);
        },
        ReplayInput::Sjis,
    );
}

/// How `replay_each` should turn the raw artifact bytes into a `&str`
/// the assertion closure will be handed.
#[derive(Copy, Clone)]
enum ReplayInput {
    /// Decode as UTF-8; skip artifact on invalid UTF-8 (mirrors the
    /// `parse_render` / `serialize_round_trip` fuzz targets).
    Utf8,
    /// Decode via Shift_JIS; skip artifact on decode failure (mirrors
    /// the `sjis_decode` fuzz target).
    Sjis,
}

/// Walk every artifact under `tests/fuzz_regressions/<target>/` and
/// hand the decoded string to `assert_one`. Panics from the closure
/// are caught and re-raised with the artifact path prefix so a
/// failure points straight at the file on disk.
fn replay_each(target: &str, assert_one: impl Fn(&str), how: ReplayInput) {
    let dir = regression_dir(target);
    let artifacts = collect_artifacts(&dir);
    if artifacts.is_empty() {
        // No regressions captured yet — that's the steady-state of a
        // healthy target. Test stays green so we can tell missing
        // tests/fuzz_regressions/ apart from "no crashes recorded".
        return;
    }
    for path in artifacts {
        let path_display = path.display();
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read regression artifact {path_display}: {e}"));
        let owned: String;
        let src: &str = match how {
            ReplayInput::Utf8 => match str::from_utf8(&bytes) {
                Ok(s) => s,
                Err(_) => continue,
            },
            ReplayInput::Sjis => match decode_sjis(&bytes) {
                Ok(text) => {
                    owned = text;
                    &owned
                }
                Err(_) => continue,
            },
        };
        let label = path.display().to_string();
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| assert_one(src)));
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&'static str>()
                        .map(|s| (*s).to_owned())
                })
                .unwrap_or_else(|| "<non-string panic payload>".to_owned());
            panic!("regression artifact {label} still crashes:\n{message}\n  bytes = {bytes:?}");
        }
    }
}

fn regression_dir(target: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR resolves to `crates/aozora-flavored-markdown/`; the test
    // binary is invoked from anywhere under the workspace, so keep the
    // path manifest-relative for stability.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fuzz_regressions")
        .join(target)
}

fn collect_artifacts(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            // Skip companion `.expect.txt` / `.md` files — they're
            // archaeology, not test inputs.
            path.is_file()
                && path
                    .extension()
                    .is_none_or(|ext| ext != "txt" && ext != "md")
        })
        .collect();
    out.sort();
    out
}
