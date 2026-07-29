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
//!   canonicalize_round_trip/
//!     ...
//! ```
//!
//! The test discovers artifacts by reading the directory at run time,
//! so a new file is picked up automatically. Every registered fuzz
//! target owns a directory here, empty or not (`.gitkeep` is what keeps
//! an empty one in git), and a missing one is a hard failure: the walk
//! used to answer "no such directory" with an empty list, and an empty
//! list is a pass — so three of the four tests that existed then replayed
//! nothing at all and said so in green for as long as their directories
//! did not exist. The discovery walk returns artifacts in sorted order so
//! failure messages stay stable across machines and `nextest` runs.
//!
//! The last section of this file reads the other corpus — the committed seeds
//! a run STARTS from — through the same per-target shape table, because the
//! bytes of a seed and the bytes of an artifact mean something only under the
//! reader that takes them.

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::str;

use aozora::decode_sjis;
use aozora_flavored_markdown::{
    Options, RenderedBlocks, canonicalize, diagnose, render, render_blocks, render_to_ir, sentinels,
};
use aozora_flavored_markdown_test_support::{
    assert_html_invariants, check_fence_fidelity, check_no_sentinel_leak,
};
use encoding_rs::SHIFT_JIS;

#[test]
fn options_space_regressions_replay_cleanly() {
    replay_each("options_space", |src| {
        // The one target whose input is not only a source: its first two
        // bytes are the option mask, and `ReplayInput::MaskedUtf8` has
        // already taken them off. What is replayed here is therefore the
        // source under the three PUBLIC CONSTRUCTORS rather than under
        // the exact knob combination that found it — a deliberate,
        // narrower net, recorded rather than glossed. The knob axis is
        // swept by `options_surface_contract.rs` over its own corpus, and
        // the exact configuration is still in the artifact's first two
        // bytes, which is what `just fuzz-triage options_space` replays
        // by handing the bytes back to the target itself. Reconstructing
        // it here would mean a copy of the knob table with nothing
        // holding it to `src/lib.rs`; the fuzz target's copy fails to
        // compile when it drifts, and a third one would not.
        //
        // The dialect carve-out the target itself makes: with the aozora
        // lexer off there is no substitution pass, so a reserved
        // codepoint the author typed comes back verbatim and Tier B —
        // which reads one as "substituted and never resolved" — is being
        // asked about a pipeline that never ran. `Options::new()` is the
        // one constructor here with the dialect on.
        let authored_sentinel = src.chars().any(|c| sentinels::ALL.contains(&c));
        for options in [Options::new(), Options::commonmark(), Options::gfm()] {
            if authored_sentinel && options != Options::new() {
                continue;
            }
            let rendered = render(src, &options);
            assert_html_invariants(src, &rendered.html);

            let ir = render_to_ir(src, &options);
            assert_html_invariants(src, &ir.html);

            let RenderedBlocks {
                blocks,
                diagnostics: block_diagnostics,
                ..
            } = render_blocks(src, &options);
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

            let expected = diagnose(src, &options);
            for (name, actual) in [
                ("render", &rendered.diagnostics),
                ("render_to_ir", &ir.diagnostics),
                ("render_blocks", &block_diagnostics),
            ] {
                assert_eq!(
                    actual, &expected,
                    "`{name}` and `diagnose` disagree about {src:?}"
                );
            }
        }
    });
}

#[test]
fn parse_render_regressions_replay_cleanly() {
    replay_each("parse_render", |src| {
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
    });
}

#[test]
fn render_blocks_regressions_replay_cleanly() {
    replay_each("render_blocks", |src| {
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
    });
}

#[test]
fn canonicalize_round_trip_regressions_replay_cleanly() {
    replay_each("canonicalize_round_trip", |src| {
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
    });
}

#[test]
fn sjis_decode_regressions_replay_cleanly() {
    replay_each("sjis_decode", |text| {
        let html = render(text, &Options::default()).html;
        assert_html_invariants(text, &html);
    });
}

/// How a target reads its raw bytes: what `replay_each` turns an artifact
/// into, and equally what the fuzzer makes of a committed seed.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ReplayInput {
    /// Decode as UTF-8; skip artifact on invalid UTF-8 (mirrors the
    /// `parse_render` / `canonicalize_round_trip` fuzz targets).
    Utf8,
    /// Decode via Shift_JIS; skip artifact on decode failure (mirrors
    /// the `sjis_decode` fuzz target).
    Sjis,
    /// Drop the leading option-mask bytes, then decode the rest as UTF-8
    /// (mirrors the `options_space` fuzz target, whose input carries its
    /// configuration in front of its source).
    MaskedUtf8,
}

impl ReplayInput {
    /// Whether the text this shape yields is the text the seeder started
    /// from. Two of the three are transport: dropping a fixed-width prefix
    /// and reading UTF-8 both invert exactly. Shift_JIS does not — CP932
    /// holds no U+2014, so a document carrying one is either refused
    /// outright or comes back with the codepoint 0x815C decodes to.
    const fn is_lossless(self) -> bool {
        !matches!(self, Self::Sjis)
    }
}

/// How many bytes of an `options_space` input are the option mask. The same
/// number the target's own `MASK_BYTES` reads, and the only thing the two
/// files have to agree on — the format is two little-endian bytes precisely
/// so that agreement is a number rather than a decoder.
const OPTION_MASK_BYTES: usize = 2;

/// The shape each registered target reads, in one place because two things
/// read it: the replay above, and the seed corpus below. Held to the `[[bin]]`
/// tables by `every_seed_corpus_is_in_the_shape_the_target_reading_it_reads`,
/// so a new target lands here as an unanswered question rather than being
/// replayed as UTF-8 because that is the common case.
const INPUT_SHAPES: &[(&str, ReplayInput)] = &[
    ("canonicalize_round_trip", ReplayInput::Utf8),
    ("options_space", ReplayInput::MaskedUtf8),
    ("parse_render", ReplayInput::Utf8),
    ("render_blocks", ReplayInput::Utf8),
    ("sjis_decode", ReplayInput::Sjis),
];

/// The shape `target` reads. Panics rather than defaulting: a target whose
/// format nobody declared is one whose corpus and whose pinned crashes are
/// both being read as something they may not be.
fn shape_of(target: &str) -> ReplayInput {
    let Some(&(_, how)) = INPUT_SHAPES.iter().find(|&&(name, _)| name == target) else {
        panic!(
            "no input shape declared for the fuzz target `{target}`. Every target's raw bytes \
             mean something, and what they mean is per-target: add it to INPUT_SHAPES."
        )
    };
    how
}

/// The text `bytes` stand for under `how`, or `None` when the target itself
/// would drop them.
fn decode(how: ReplayInput, bytes: &[u8]) -> Option<String> {
    match how {
        ReplayInput::Utf8 => str::from_utf8(bytes).ok().map(str::to_owned),
        ReplayInput::Sjis => decode_sjis(bytes).ok(),
        ReplayInput::MaskedUtf8 => bytes
            .get(OPTION_MASK_BYTES..)
            .and_then(|rest| str::from_utf8(rest).ok())
            .map(str::to_owned),
    }
}

/// Walk every artifact under `tests/fuzz_regressions/<target>/` and
/// hand the decoded string to `assert_one`. Panics from the closure
/// are caught and re-raised with the artifact path prefix so a
/// failure points straight at the file on disk.
fn replay_each(target: &str, assert_one: impl Fn(&str)) {
    let how = shape_of(target);
    let dir = regression_dir(target);
    // No regressions captured yet is the steady state of a healthy target, so
    // an EMPTY directory is a pass. A MISSING one is not: the two used to be
    // the same answer, which is how this suite reported success over targets
    // it had never read a byte for.
    for path in collect_artifacts(&dir) {
        let path_display = path.display();
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read regression artifact {path_display}: {e}"));
        let Some(src) = decode(how, &bytes) else {
            continue;
        };
        let src = src.as_str();
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
    // Stated here rather than left to the directories existing, because
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

// ---------------------------------------------------------------------------
// the corpus every one of those targets starts from
// ---------------------------------------------------------------------------
//
// The section above replays what a fuzzer already found. This one asks whether
// the fuzzer can find anything: a corpus is bytes, and bytes only mean
// something under the reader that takes them. `just fuzz-seed` writes one seed
// per source document per target and prints a count, and a count is the same
// number whether the bytes are in the shape their target reads or not — a
// target that drops every seed it is handed starts from one empty byte string
// and reports 700 seeds on the way there.
//
// These product tests read each committed seed through the same input shape as
// its fuzz target. A count alone cannot tell whether the target can consume
// those bytes, especially for the Shift_JIS and option-mask targets.

/// Where `just fuzz-seed` installs the committed corpus, relative to this
/// crate's manifest.
const CORPUS_ROOT: &str = "fuzz/corpus";

/// What tells a committed seed from libFuzzer's own output. The two share a
/// directory; only the first is in git and only the first is this file's
/// business.
const SEED_PREFIX: &str = "seed-";

/// The seed source that carries this crate's own dialect — ruby, bouten,
/// tate-chu-yoko, the paired containers. The rest of the corpus is spec
/// examples, which are the input class comrak is already fuzzed on upstream.
const SEED_SOURCE: &str = "playground/examples";

/// The workspace root: two levels up from this crate's manifest, which is
/// where `just fuzz-seed` reads its documents from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("resolving the workspace root: {e}"))
}

/// The fuzz targets the crate registers. `fuzz/Cargo.toml` is the registry: a
/// `[[bin]]` there is what `cargo fuzz run <name>` resolves, and reading it is
/// what keeps this file from holding a second list of target names.
fn registered_targets() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("Cargo.toml");
    let manifest =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let mut out = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // `[package]` declares a `name` too, and it is not a target.
            in_bin = trimmed == "[[bin]]";
        } else if in_bin && let Some(rest) = trimmed.strip_prefix("name = \"") {
            out.extend(rest.split('"').next().map(str::to_owned));
        }
    }
    out.sort();
    out
}

/// The committed seeds under `target`'s corpus directory, sorted.
fn seed_files(target: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CORPUS_ROOT)
        .join(target);
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e}\n\
             Every registered fuzz target owns a corpus directory. `just fuzz-seed` writes it \
             and it is committed, because a fuzzer handed nothing starts from one empty byte \
             string and spends its budget rediscovering that markdown has headings.",
            dir.display()
        )
    });
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with(SEED_PREFIX))
        })
        .collect();
    out.sort();
    out
}

/// The dialect documents `just fuzz-seed` starts from, as (name, text).
fn seed_source_documents() -> Vec<(String, String)> {
    let dir = repo_root().join(SEED_SOURCE);
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned();
            (name, text)
        })
        .collect();
    out.sort();
    out
}

/// The decoded text produced by the one CP932 projection used by the seeder.
///
/// `encoding_rs` is both the executable contract in `xtask cp932-project` and
/// the expectation here. This keeps platform `iconv` aliases and handwritten
/// dash substitutions out of the corpus contract.
fn cp932_projection(text: &str) -> Option<String> {
    let (encoded, _, had_errors) = SHIFT_JIS.encode(text);
    (!had_errors).then(|| {
        decode_sjis(encoded.as_ref()).expect("encoding_rs emitted bytes the aozora decoder rejects")
    })
}

/// One target's committed corpus, decoded through the shape that target
/// reads.
struct Corpus {
    target: String,
    how: ReplayInput,
    texts: BTreeSet<String>,
}

/// Read `target`'s corpus back through its own reader, asking the two
/// questions a seed owes on the way: does the target read it at all, and — for
/// a shape with a fixed-width header — is that header the neutral value.
fn decoded_corpus(target: &str) -> Corpus {
    let how = shape_of(target);
    let seeds = seed_files(target);
    assert!(
        !seeds.is_empty(),
        "`{target}` has no `{SEED_PREFIX}*` file in its corpus, so every run of it starts from \
         an empty corpus"
    );

    let mut texts = BTreeSet::new();
    let mut unreadable = Vec::new();
    let mut preset = Vec::new();
    for path in &seeds {
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        // A seed carrying a configuration is a seed that decides where the
        // search starts. The mask is the first thing the fuzzer mutates, so
        // what a seed owes is the SHAPE and not a corner of the space — and a
        // seed written with no mask at all fails here rather than being
        // silently read from its third byte.
        if how == ReplayInput::MaskedUtf8
            && bytes.get(..OPTION_MASK_BYTES) != Some([0u8; OPTION_MASK_BYTES].as_slice())
        {
            preset.push(path.display().to_string());
        }
        match decode(how, &bytes) {
            Some(text) => {
                texts.insert(text);
            }
            None => unreadable.push(path.display().to_string()),
        }
    }

    assert!(
        preset.is_empty(),
        "{} of `{target}`'s {} seeds do not open on the neutral option mask, e.g. {:?}. Either \
         they were written without the {OPTION_MASK_BYTES}-byte prefix this target reads — in \
         which case each one loses its first {OPTION_MASK_BYTES} bytes to the mask and is a \
         different document from the one it was made from — or the seeder has started choosing \
         which corner of the option space every run begins in.",
        preset.len(),
        seeds.len(),
        preset.iter().take(3).collect::<Vec<_>>()
    );
    assert!(
        unreadable.is_empty(),
        "{} of `{target}`'s {} seeds are bytes it drops on sight, e.g. {:?}. A dropped seed is \
         one the fuzzer never runs and one `just fuzz-seed` counted anyway: the target returns \
         before its first assertion, and the corpus is that much smaller than it reports.",
        unreadable.len(),
        seeds.len(),
        unreadable.iter().take(3).collect::<Vec<_>>()
    );
    Corpus {
        target: target.to_owned(),
        how,
        texts,
    }
}

/// Every target is seeded from one set of documents, so decoding each corpus
/// through its own reader has to arrive back at that one set.
///
/// This is the statement that catches a per-target transform which is not the
/// inverse of the reader: the seed counts match, every seed decodes, and the
/// documents are a prefix, a suffix or a transcoding of the ones every other
/// target got.
fn one_document_set_behind_every_corpus(corpora: &[Corpus]) {
    let mut lossless = corpora.iter().filter(|corpus| corpus.how.is_lossless());
    let first = lossless
        .next()
        .unwrap_or_else(|| panic!("no registered target reads its input losslessly"));
    for corpus in lossless {
        let extra_here: Vec<&String> = corpus.texts.difference(&first.texts).collect();
        let absent_here: Vec<&String> = first.texts.difference(&corpus.texts).collect();
        assert!(
            extra_here.is_empty() && absent_here.is_empty(),
            "`{}` and `{}` are seeded from the same documents and do not decode to the same \
             ones: {} only `{}` has, {} only `{}` has, e.g. {:?}. Both readers are exact, so the \
             difference is in what was written — one of the two corpora is not the documents it \
             was made from.",
            corpus.target,
            first.target,
            extra_here.len(),
            corpus.target,
            absent_here.len(),
            first.target,
            extra_here
                .iter()
                .chain(&absent_here)
                .take(2)
                .map(|text| text.chars().take(60).collect::<String>())
                .collect::<Vec<_>>()
        );
    }

    // The lossy target is the exact representable projection of that same
    // source set. Both the generator and this expectation use encoding_rs.
    let reference: BTreeSet<String> = first
        .texts
        .iter()
        .filter_map(|text| cp932_projection(text))
        .collect();
    for corpus in corpora.iter().filter(|corpus| !corpus.how.is_lossless()) {
        let extra: Vec<&String> = corpus.texts.difference(&reference).collect();
        let missing: Vec<&String> = reference.difference(&corpus.texts).collect();
        assert!(
            extra.is_empty() && missing.is_empty(),
            "`{}` is not the exact encoding_rs CP932 projection: {} extra, {} missing, e.g. {:?}",
            corpus.target,
            extra.len(),
            missing.len(),
            extra
                .iter()
                .chain(&missing)
                .take(2)
                .map(|text| text.chars().take(60).collect::<String>())
                .collect::<Vec<_>>()
        );
    }
}

/// The content question, per target.
///
/// These dialect documents are the only 青空文庫 notation in a corpus dominated
/// by CommonMark and GFM examples, so every target must receive them.
fn every_corpus_carries_the_dialect(corpora: &[Corpus]) {
    let documents = seed_source_documents();
    assert!(
        documents.len() >= 5,
        "{SEED_SOURCE} came out holding {documents:?}; the reader is not finding the documents \
         `just fuzz-seed` copies"
    );
    for corpus in corpora {
        let fold = |text: &str| {
            if corpus.how.is_lossless() {
                Some(text.to_owned())
            } else {
                cp932_projection(text)
            }
        };
        let held: BTreeSet<String> = corpus.texts.iter().filter_map(|text| fold(text)).collect();
        let missing: Vec<&String> = documents
            .iter()
            .filter(|(_, text)| fold(text).is_some_and(|projected| !held.contains(&projected)))
            .map(|(name, _)| name)
            .collect();
        assert!(
            missing.is_empty(),
            "`{}`'s corpus does not carry {missing:?}. Those are the documents that put ruby, \
             bouten, tate-chu-yoko and the paired containers into the search — the layer this \
             crate exists for — and the {} spec examples that make up the rest carry none of \
             them.",
            corpus.target,
            corpus.texts.len().saturating_sub(documents.len())
        );
    }
}

#[test]
fn every_seed_corpus_is_in_the_shape_the_target_reading_it_reads() {
    let registered = registered_targets();
    assert!(
        registered.len() >= 3,
        "the fuzz manifest came out registering {registered:?}; the reader is not finding its \
         `[[bin]]` tables, so everything below passes by asking nothing"
    );
    let declared: Vec<String> = INPUT_SHAPES
        .iter()
        .map(|&(name, _)| name.to_owned())
        .collect();
    assert_eq!(
        declared, registered,
        "INPUT_SHAPES and the `[[bin]]` tables disagree about what the fuzz targets are. A \
         target with no declared shape has its corpus and its pinned crashes read as whatever \
         the default happens to be."
    );

    let corpora: Vec<Corpus> = registered
        .iter()
        .map(|target| decoded_corpus(target))
        .collect();
    one_document_set_behind_every_corpus(&corpora);
    every_corpus_carries_the_dialect(&corpora);
}
