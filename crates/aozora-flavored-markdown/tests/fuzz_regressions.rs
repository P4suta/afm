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
//! so a new file is picked up automatically. Every registered fuzz
//! target owns a directory here, empty or not (`.gitkeep` is what keeps
//! an empty one in git), and a missing one is a hard failure: the walk
//! used to answer "no such directory" with an empty list, and an empty
//! list is a pass — so three of the four tests below replayed nothing
//! at all and said so in green for as long as their directories did not
//! exist. The discovery walk returns artifacts in sorted order so
//! failure messages stay stable across machines and `nextest` runs.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::str;

use aozora::decode_sjis;
use aozora_flavored_markdown::{
    Options, RenderedBlocks, canonicalize, diagnose, render, render_blocks, sentinels,
};
use aozora_flavored_markdown_test_support::{
    assert_html_invariants, check_fence_fidelity, check_no_sentinel_leak,
};

#[test]
fn parse_render_regressions_replay_cleanly() {
    replay_each(
        "parse_render",
        |src| {
            let rendered = render(src, &Options::default());
            assert_html_invariants(src, &rendered.html);
            // The corpus is inputs libFuzzer found by looking for trouble, so
            // it is the sharpest set to ask the `check`-vs-`render` question
            // of: `diagnose` reaches the same diagnostics without rendering,
            // and an artifact promoted here is an input that once broke
            // something on the way.
            assert_eq!(
                diagnose(src, &Options::default()),
                rendered.diagnostics,
                "`diagnose` and `render` disagree about {src:?}"
            );
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
            let RenderedBlocks {
                blocks,
                diagnostics,
                ..
            } = render_blocks(src, &Options::default());
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
            // The streaming path builds its own construct table, so this is a
            // second producer of the diagnostics `diagnose` claims to be the
            // one answer for.
            assert_eq!(
                diagnose(src, &Options::default()),
                diagnostics,
                "`diagnose` and `render_blocks` disagree about {src:?}"
            );
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
    // No regressions captured yet is the steady state of a healthy target, so
    // an EMPTY directory is a pass. A MISSING one is not: the two used to be
    // the same answer, which is how this suite reported success over targets
    // it had never read a byte for.
    for path in collect_artifacts(&dir) {
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
    let entries = fs::read_dir(dir).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e}\n\
             Every registered fuzz target owns a directory under \
             tests/fuzz_regressions/, empty or not — `.gitkeep` is what tracks \
             an empty one. A missing directory is not an empty one: this walk \
             used to return no artifacts for it, and no artifacts is a pass.",
            dir.display()
        )
    });
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            // Skip companion `.expect.txt` / `.md` files — they're
            // archaeology, not test inputs — and every dotfile, which is what
            // `.gitkeep` is and what a stray `.DS_Store` would otherwise be
            // replayed as. A promoted artifact is named for its hash and
            // starts with `crash-` / `leak-` / `oom-`.
            let hidden = path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with('.'));
            path.is_file()
                && !hidden
                && path
                    .extension()
                    .is_none_or(|ext| ext != "txt" && ext != "md")
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_missing_directory_fails_the_walk_and_an_empty_one_does_not() {
    // The two answers this walk used to give as one. Every `#[test]` above
    // asserts nothing at all when `collect_artifacts` comes back empty — that
    // is what "no regressions pinned yet" looks like and it is a legitimate
    // pass — so a walk that answered "no such directory" with an empty list
    // handed three of those four tests a permanent, silent, green vacuum.
    //
    // Stated here rather than left to the four directories existing, because
    // those are a fact about the tree today and this is the property the suite
    // rests on: restore the `unwrap_or_default()` and every directory is still
    // in place, every test above still passes, and the suite is one `mv` away
    // from reporting success over nothing again with nothing to say so.
    let missing = regression_dir("a_target_no_bin_table_declares");
    assert!(
        !missing.exists(),
        "{} exists, so this test is measuring the wrong thing",
        missing.display()
    );
    let walked = panic::catch_unwind(|| collect_artifacts(&missing));
    assert!(
        walked.is_err(),
        "collect_artifacts returned {:?} for a directory that does not exist. An empty list is \
         how a healthy target with nothing pinned reads, so the two states have to be told \
         apart here — there is nowhere further up that can.",
        walked.unwrap_or_default()
    );

    // And the half that must NOT fail, or the suite would demand a promoted
    // crash per target and the first one would be manufactured to satisfy it.
    let empty = env::temp_dir().join(format!("aozora-md-empty-regressions-{}", process::id()));
    fs::create_dir_all(&empty).unwrap_or_else(|e| panic!("creating {}: {e}", empty.display()));
    let walked = collect_artifacts(&empty);
    drop(fs::remove_dir_all(&empty));
    assert!(
        walked.is_empty(),
        "an empty regression directory yielded {walked:?}"
    );
}
