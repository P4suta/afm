//! The `Options` surface, checked as a *space* rather than as a default.
//!
//! Every other invariant sweep in this workspace — `property_html_shape`,
//! `property_gfm_aozora_mix`, all three fuzz targets, `fuzz_regressions` —
//! renders with `Options::default()` and nothing else. The whole non-default
//! configuration space was therefore invariant-free, which is how two
//! constructors that turned on comrak's raw-HTML passthrough
//! (`render.unsafe`) sat on the public surface behind nothing but
//! `#[doc(hidden)]`. This file holds the space itself:
//!
//! * the surface is **enumerated from the source**, so a knob added later is
//!   swept without anyone remembering to add it (and a knob deleted here
//!   fails rather than silently narrowing the sweep);
//! * every configuration reachable from that surface is fed an XSS payload
//!   corpus through all three entry points;
//! * every knob must be *load-bearing* end to end — a `with_*` that sets a
//!   field nothing reads would pass the value-level builder test in
//!   `src/lib.rs` and fail here.
//!
//! An integration test compiles as its own crate, so "reachable" here means
//! exactly what it means for a consumer from crates.io.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use aozora_flavored_markdown::{
    Diagnostic, Options, RenderedBlocks, canonicalize, diagnose, render, render_blocks,
    render_to_ir,
};
use aozora_flavored_markdown_test_support::config;
use aozora_flavored_markdown_test_support::generators::{
    aozora_fragment, commonmark_adversarial, pathological_aozora,
};
use aozora_flavored_markdown_test_support::{assert_html_invariants, check_no_xss_marker};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// the surface, as a table
// ---------------------------------------------------------------------------

/// A named public constructor of [`Options`].
type Constructor = (&'static str, fn() -> Options);

/// Every public constructor of [`Options`].
const CONSTRUCTORS: &[Constructor] = &[
    ("new", Options::new),
    ("commonmark", Options::commonmark),
    ("gfm", Options::gfm),
];

/// One public `with_*` knob, with a source that renders differently
/// depending on it. `marker` appears in the rendered HTML **iff** the knob
/// is on, which is what makes the knob load-bearing rather than decorative;
/// `default_on` is then readable straight off the same probe, so the shipped
/// dialect is a column of this table rather than folklore.
///
/// `wire` is the name the same knob answers to as JSON — the second door onto
/// this space, opened when `Options` became `Deserialize`.
struct Knob {
    name: &'static str,
    wire: &'static str,
    set: fn(Options, bool) -> Options,
    probe: &'static str,
    marker: &'static str,
    default_on: bool,
}

const KNOBS: &[Knob] = &[
    Knob {
        name: "with_aozora",
        wire: "aozora",
        set: |o, on| o.with_aozora(on),
        probe: "｜青梅《おうめ》",
        marker: "<ruby>",
        default_on: true,
    },
    Knob {
        name: "with_hardbreaks",
        wire: "hardbreaks",
        set: |o, on| o.with_hardbreaks(on),
        probe: "a\nb",
        marker: "<br />",
        default_on: true,
    },
    Knob {
        name: "with_smart_punctuation",
        wire: "smartPunctuation",
        set: |o, on| o.with_smart_punctuation(on),
        probe: "\"quoted\"",
        marker: "\u{201c}",
        default_on: false,
    },
    Knob {
        // The one knob whose default this commit flipped: the Aozora dialect
        // now emphasises across CJK punctuation, which CommonMark's flanking
        // rules refuse. `**「強調」**です` is the minimal witness — the
        // closing run is preceded by `」` (punctuation) and followed by `で`
        // (neither whitespace nor punctuation), so vanilla CommonMark denies
        // it right-flanking status and emits the asterisks literally.
        name: "with_cjk_friendly_emphasis",
        wire: "cjkFriendlyEmphasis",
        set: |o, on| o.with_cjk_friendly_emphasis(on),
        probe: "これは**「強調」**です",
        marker: "<strong>",
        default_on: true,
    },
    Knob {
        name: "with_source_line_anchors",
        wire: "sourceLineAnchors",
        set: |o, on| o.with_source_line_anchors(on),
        probe: "para",
        marker: "data-aozora-md-source-line",
        default_on: false,
    },
    Knob {
        name: "with_tables",
        wire: "tables",
        set: |o, on| o.with_tables(on),
        probe: "| a |\n| - |\n| b |\n",
        marker: "<table>",
        default_on: true,
    },
    Knob {
        name: "with_strikethrough",
        wire: "strikethrough",
        set: |o, on| o.with_strikethrough(on),
        probe: "~~x~~",
        marker: "<del>",
        default_on: true,
    },
    Knob {
        name: "with_autolinks",
        wire: "autolinks",
        set: |o, on| o.with_autolinks(on),
        probe: "see https://example.com/ ok",
        marker: "<a href=",
        default_on: true,
    },
    Knob {
        name: "with_task_lists",
        wire: "taskLists",
        set: |o, on| o.with_task_lists(on),
        probe: "- [ ] todo\n",
        marker: "checkbox",
        default_on: true,
    },
];

/// Names that were public before comrak was hidden and must never come back:
/// two raw-HTML constructors, the comrak escape hatches that made
/// `#[doc(hidden)]` on them meaningless, and the getters `Debug + Eq`
/// replaced.
const RETIRED: &[&str] = &[
    "commonmark_only",
    "gfm_only",
    "comrak",
    "comrak_mut",
    "aozora_enabled",
    "source_line_anchors",
    "with_aozora_enabled",
];

/// The `pub fn` names declared in an `impl Options` block, read off the
/// source. `pub(crate) fn` — the `#[cfg(test)]` spec constructors — is not a
/// public surface and does not match.
fn public_options_api() -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let src = fs::read_to_string(&path).expect("src/lib.rs must be readable");
    let mut names = BTreeSet::new();
    let mut inside = false;
    for line in src.lines() {
        if line == "impl Options {" {
            inside = true;
        } else if inside && line == "}" {
            inside = false;
        } else if inside && let Some(rest) = line.trim_start().strip_prefix("pub fn ") {
            names.insert(
                rest.chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect(),
            );
        }
    }
    assert!(
        !names.is_empty(),
        "no `impl Options` block found in {}; the reader must be retargeted, not deleted",
        path.display()
    );
    names
}

#[test]
fn the_swept_surface_is_the_whole_public_surface() {
    let declared = public_options_api();
    let swept: BTreeSet<String> = CONSTRUCTORS
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .chain(KNOBS.iter().map(|k| k.name.to_owned()))
        .collect();
    assert_eq!(
        declared, swept,
        "every public `Options` method must be swept by this file: adding one without a \
         row leaves a configuration nothing checks for raw HTML"
    );
}

#[test]
fn no_retired_options_method_has_come_back() {
    let declared = public_options_api();
    for name in RETIRED {
        assert!(
            !declared.contains(*name),
            "`Options::{name}` is public again; it either returns a comrak type the crate \
             does not re-export or turns on `render.unsafe`"
        );
    }
}

// ---------------------------------------------------------------------------
// the same surface, one workspace boundary away
// ---------------------------------------------------------------------------
//
// `crates/aozora-flavored-markdown/fuzz` declares its own `[workspace]`
// because libfuzzer-sys is nightly-only, so nothing in this crate's
// `--all-targets` build compiles it and no test here can import it. Its
// `options_space` target nevertheless carries a SECOND hand-written copy of
// the table above — one bit per knob, one slot per constructor — and that copy
// decides which configurations arbitrary bytes are ever rendered under.
//
// Over there the copy is held to `src/lib.rs` by two `const` assertions, and
// what those compare is a COUNT. A knob added upstream is caught; a row whose
// `name:` says one method while its own closure calls another is not, and
// neither is the bit order that `AOZORA_BIT` reads back to decide which inputs
// the target skips. Text is the only thing that crosses the workspace
// boundary, so the rows are read as text and compared by NAME here, in the
// suite `just test` runs, rather than by height in the nightly one.

/// Where the fuzz harnesses live, relative to this crate's manifest.
const FUZZ_TARGET_DIR: &str = "fuzz/fuzz_targets";

/// One row of an `Options` table in a fuzz harness: the file it sits in, the
/// name the row gives itself, and the method the row's own function calls.
struct FuzzRow {
    file: String,
    declared: String,
    called: String,
}

/// Every `Options` row every fuzz harness declares.
///
/// A row is `name: "…"` followed by the function that acts on it, which is the
/// shape rustfmt keeps both tables in. Reading both halves is the point: the
/// name is what a crash label and the bit-order assertion read, the call is
/// what actually runs, and nothing over there compares the two.
fn fuzz_option_rows() -> Vec<FuzzRow> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(FUZZ_TARGET_DIR);
    let mut harnesses: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    harnesses.sort();
    assert!(
        !harnesses.is_empty(),
        "{} holds no `.rs` harness; the reader is looking in the wrong place",
        dir.display()
    );

    let mut rows = Vec::new();
    for path in harnesses {
        let file = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_owned();
        let src =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let mut declared: Option<String> = None;
        for line in src.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("name: \"") {
                declared = rest.split('"').next().map(str::to_owned);
                continue;
            }
            let called = trimmed
                .strip_prefix("set: |o, on| o.")
                .and_then(|rest| rest.split('(').next())
                .or_else(|| {
                    trimmed
                        .strip_prefix("build: Options::")
                        .and_then(|rest| rest.split(',').next())
                });
            if let Some(called) = called {
                rows.push(FuzzRow {
                    file: file.clone(),
                    // `None` is a function with no name above it, which the
                    // count assertion below turns into a failure rather than
                    // into a row that quietly does not exist.
                    declared: declared.take().unwrap_or_default(),
                    called: called.trim().to_owned(),
                });
            }
        }
    }
    rows
}

#[test]
fn the_fuzzed_surface_is_the_whole_public_surface() {
    let declared_api = public_options_api();
    let rows = fuzz_option_rows();
    assert!(
        !rows.is_empty(),
        "no fuzz harness under {FUZZ_TARGET_DIR} declares an `Options` table, so every fuzz \
         target renders with `Options::default()` and nothing else — which is the state this \
         file's own header describes and the state `options_space` exists to end. Arbitrary \
         bytes would again reach exactly one of the {} configurations a consumer can build.",
        declared_api.len()
    );

    let mismatched: Vec<String> = rows
        .iter()
        .filter(|row| row.declared != row.called)
        .map(|row| {
            format!(
                "  {}: row `{}` calls `{}`",
                row.file, row.declared, row.called
            )
        })
        .collect();
    assert!(
        mismatched.is_empty(),
        "a fuzz harness's `Options` row names one method and calls another:\n{}\n\
         The name is what the crash label and the bit-order assertion read; the call is what \
         runs. The `const` assertions over there compare table HEIGHT, so the two halves of a \
         row can say different things and still build.",
        mismatched.join("\n")
    );

    let fuzzed: BTreeSet<String> = rows.iter().map(|row| row.declared.clone()).collect();
    assert_eq!(
        fuzzed, declared_api,
        "the public `Options` surface and the surface the fuzz harnesses sweep have come apart. \
         A knob missing there is an axis of the configuration space that arbitrary bytes never \
         reach, while the harness goes on reporting a full sweep of it."
    );
    assert_eq!(
        rows.len(),
        declared_api.len(),
        "{} `Options` row(s) across the fuzz harnesses for {} public method(s). Two rows for one \
         method leave another with none, and a function with no `name:` above it is a row this \
         reader cannot attribute.",
        rows.len(),
        declared_api.len()
    );
}

#[test]
fn nothing_but_impl_options_declares_a_method_at_impl_indentation() {
    // The premise the fuzz harness's own `const` scanner rests on, checked
    // here because it is a fact about THIS file that only breaks when someone
    // edits it. That scanner cannot parse: it counts `\n    pub fn ` across
    // the whole of `src/lib.rs` and asserts the total is its table's height,
    // so a `pub fn` added to any other `impl` block at the same indentation
    // fails the nightly `just fuzz-build` with a message about `impl Options`
    // — a red gate, pointing at the wrong file, for a change that broke
    // nothing. Stated here so the failure arrives in seconds and says what it
    // is.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let src = fs::read_to_string(&path).expect("src/lib.rs must be readable");
    let at_impl_indentation = src.matches("\n    pub fn ").count();
    assert_eq!(
        at_impl_indentation,
        public_options_api().len(),
        "{} declares {at_impl_indentation} `pub fn` at `impl` indentation and only {} of them \
         are `impl Options` methods. `fuzz/fuzz_targets/options_space.rs` counts the first \
         number and checks it against its own table of the second: give the new method more \
         than four spaces of indentation, move it out of this file, or teach that scanner the \
         block it belongs to.",
        path.display(),
        public_options_api().len()
    );
}

// ---------------------------------------------------------------------------
// the space, swept
// ---------------------------------------------------------------------------

/// Every configuration a consumer can reach: each constructor bare, each
/// constructor with each knob forced both ways, and — since `Options` became
/// `Deserialize` — every configuration the wire form reaches as well.
fn reachable_options() -> Vec<(String, Options)> {
    let mut out = Vec::new();
    for (ctor, build) in CONSTRUCTORS {
        out.push(((*ctor).to_owned(), build()));
        for knob in KNOBS {
            for on in [false, true] {
                out.push((
                    format!("{ctor}().{}({on})", knob.name),
                    (knob.set)(build(), on),
                ));
            }
        }
    }
    // "Reachable" is a claim about consumers, not about builders. A browser
    // host reaches this space by sending JSON, so the payload sweep below
    // has to travel that door too or the word narrows to mean "reachable by
    // the API this file happens to enumerate".
    #[cfg(feature = "serde")]
    out.extend(wire_reachable_options());
    out
}

/// Payloads whose whole purpose is to reach the browser as markup. Raw-HTML
/// passthrough is the only switch that would let any of them through, and
/// after this commit no public path sets it.
const XSS_PAYLOADS: &[&str] = &[
    "<script>alert(1)</script>",
    "<img src=x onerror=alert(1)>",
    "<a href=\"javascript:alert(1)\">x</a>",
    "<iframe src=\"data:text/html,<script>alert(1)</script>\"></iframe>",
    "<style>body{display:none}</style>",
    "<!-- <script>alert(1)</script> -->",
    "[link](javascript:alert(1))",
    "｜x《<script>alert(1)</script>》",
    "> <script>alert(1)</script>\n",
    "- <img src=x onerror=alert(1)>\n",
    "# <script>alert(1)</script>\n",
    "<script>alert(1)</script>\n\n［＃改ページ］\n\n<iframe></iframe>",
];

/// Markup that must never survive to the output, whatever the options.
const FORBIDDEN_MARKUP: &[&str] = &["<script", "<iframe", "<style", "onerror="];

fn assert_inert(label: &str, src: &str, html: &str) {
    check_no_xss_marker(html)
        .unwrap_or_else(|e| panic!("Tier F violated under {label} for src={src:?}: {e:?}"));
    for needle in FORBIDDEN_MARKUP {
        assert!(
            !html.contains(needle),
            "{needle} survived raw under {label} for src={src:?}: {html:?}"
        );
    }
}

#[test]
fn no_reachable_configuration_lets_raw_html_through() {
    for (label, opts) in reachable_options() {
        for src in XSS_PAYLOADS {
            assert_inert(&label, src, &render(src, &opts).html);
        }
    }
}

#[test]
fn no_reachable_configuration_lets_raw_html_through_the_ir_paths() {
    // The IR and per-block paths format the same AST through their own
    // call chains — the per-block one re-runs the mask restore per block —
    // so inertness is not inherited from the document path.
    for (label, opts) in reachable_options() {
        for src in XSS_PAYLOADS {
            assert_inert(&label, src, &render_to_ir(src, &opts).html);
            let RenderedBlocks { blocks, .. } = render_blocks(src, &opts);
            let joined: String = blocks.iter().map(|b| b.html.as_str()).collect();
            assert_inert(&label, src, &joined);
        }
    }
}

#[test]
fn the_canonicaliser_leaves_no_payload_more_dangerous_than_it_found_it() {
    // `canonicalize` takes no options, so it has no configuration space — but
    // it is the other public path a payload can travel, and its output is fed
    // back to `render` by every round-tripping host.
    for src in XSS_PAYLOADS {
        let canonical = canonicalize(src).expect("in-budget payload canonicalises");
        assert_inert(
            "canonicalize→render",
            src,
            &render(&canonical, &Options::new()).html,
        );
    }
}

// ---------------------------------------------------------------------------
// one document, one set of diagnostics
// ---------------------------------------------------------------------------

/// What every entry point says the lexer saw, for one source under one
/// configuration.
///
/// `diagnose` is the odd one out and the reason this rule exists: it is the
/// only entry point that reaches its answer *without* rendering, so it is the
/// only one whose answer can drift without a rendered document to contradict
/// it. The three renderers are not redundant either — `render_blocks` reads
/// its diagnostics off a `StreamingIrBuilder`'s own construct table, not off
/// the one the document path builds.
fn four_readings(src: &str, opts: &Options) -> [(&'static str, Vec<Diagnostic>); 4] {
    [
        ("diagnose", diagnose(src, opts)),
        ("render", render(src, opts).diagnostics),
        ("render_to_ir", render_to_ir(src, opts).diagnostics),
        ("render_blocks", render_blocks(src, opts).diagnostics),
    ]
}

fn assert_one_reading(label: &str, src: &str, opts: &Options) {
    let readings = four_readings(src, opts);
    let (_, expected) = &readings[0];
    for (name, actual) in &readings[1..] {
        assert_eq!(
            actual, expected,
            "`{name}` and `diagnose` disagree about {src:?} under {label}"
        );
    }
}

#[test]
fn no_reachable_configuration_makes_the_entry_points_disagree_about_a_source() {
    // The CLI's `check` sub-command stopped rendering: it asks `diagnose` and
    // exits 2 under `--strict` on what comes back, where it used to exit 2 on
    // what `render` came back with. That is only the same command if these
    // agree everywhere, and "everywhere" includes the configuration space —
    // `diagnose` has its own early return for `with_aozora(false)`, which is
    // a second place the two can part company.
    for (label, opts) in reachable_options() {
        for src in XSS_PAYLOADS {
            assert_one_reading(&label, src, &opts);
        }
        for src in DIAGNOSTIC_SOURCES.iter().chain(MASKED_SOURCES) {
            assert_one_reading(&label, src, &opts);
        }
    }
}

/// Sources that make the lexer say something, so the rule above is quantified
/// over non-empty diagnostics rather than agreeing four ways on nothing.
const DIAGNOSTIC_SOURCES: &[&str] = &[
    "orphan》close",
    "｜青梅《おうめ》と orphan》close",
    "第一行\n第二行》\n第三行",
];

/// Sources whose triggers are hidden from the lexer by the code-block mask.
/// Swept but not required to diagnose: masking is a step `diagnose` performs
/// for itself rather than inherits, so it is a place the readings could part
/// company — by one of them seeing a construct the other masked away.
const MASKED_SOURCES: &[&str] = &[
    "```\norphan》close\n```\n",
    "    orphan》close\n",
    "`｜青梅《おうめ》`",
];

#[test]
fn the_diagnostic_corpus_is_one_that_actually_diagnoses() {
    // A corpus that quietly stopped provoking the lexer would leave the rule
    // above passing on four empty vectors.
    for src in DIAGNOSTIC_SOURCES {
        assert!(
            !diagnose(src, &Options::default()).is_empty(),
            "{src:?} must still raise a diagnostic, or the corpus needs a new canary"
        );
    }
}

// ---------------------------------------------------------------------------
// every knob is load-bearing
// ---------------------------------------------------------------------------

#[test]
fn every_knob_changes_the_rendered_output_it_names() {
    // The unit test in `src/lib.rs` pins each builder to its *field*; this
    // pins the field to comrak and to the renderer. A knob wired to nothing
    // passes there and fails here.
    for knob in KNOBS {
        let on = render(knob.probe, &(knob.set)(Options::new(), true)).html;
        let off = render(knob.probe, &(knob.set)(Options::new(), false)).html;
        assert!(
            on.contains(knob.marker),
            "{}(true) must produce {:?} for {:?}, got {on:?}",
            knob.name,
            knob.marker,
            knob.probe
        );
        assert!(
            !off.contains(knob.marker),
            "{}(false) must suppress {:?} for {:?}, got {off:?}",
            knob.name,
            knob.marker,
            knob.probe
        );
    }
}

#[test]
fn the_default_dialect_is_the_one_every_knob_row_assumes() {
    // `Options::new()` is what `render(src, &Options::default())` means, so
    // each knob's default is a shipped rendering decision, not merely a
    // surface one. Read straight off the same probes: the marker is present
    // iff the knob defaults on.
    for knob in KNOBS {
        let html = render(knob.probe, &Options::new()).html;
        assert_eq!(
            html.contains(knob.marker),
            knob.default_on,
            "the default dialect changed for {}: {:?} rendered {html:?}",
            knob.name,
            knob.probe
        );
    }
}

#[test]
fn commonmark_is_the_dialect_the_spec_runners_measure_against() {
    // `Options::commonmark()` is the crate's own claim to be a CommonMark
    // superset, so every extension — the four GFM ones, the notation pass and
    // the CJK emphasis relaxation, all of which are *not* CommonMark — must
    // be off in it, and `gfm()` must differ from it in exactly the four.
    for knob in KNOBS {
        let html = render(knob.probe, &Options::commonmark()).html;
        assert!(
            !html.contains(knob.marker),
            "{} is on in `commonmark()`: {:?} rendered {html:?}",
            knob.name,
            knob.probe
        );
    }
    let rebuilt = Options::commonmark()
        .with_tables(true)
        .with_strikethrough(true)
        .with_autolinks(true)
        .with_task_lists(true);
    assert_eq!(
        rebuilt,
        Options::gfm(),
        "`gfm()` must be `commonmark()` plus exactly the four GFM extensions"
    );
}

// ---------------------------------------------------------------------------
// the second door: the same space, reached as JSON
// ---------------------------------------------------------------------------
//
// `Options` is `Deserialize`, so a browser host configures a render by
// sending an object rather than by calling a builder. That is a second
// surface, and it was opened without a reader: everything above enumerates
// `pub fn` names off the source, and a serde field name is not one.
//
// The gap that leaves is not hypothetical. The wasm bridge used to carry a
// hand-written shadow struct with two of the nine knobs on it, so seven were
// unreachable from JS — and nothing failed. The shadow deserialised fine,
// `tsc` type-checked against the two-property `.d.ts` the shadow generated,
// `just playground-build` was green, and this file never saw the wasm crate
// at all. Silence in every gate, for a public surface missing 78% of itself.
//
// So the rules below close the loop rather than adding an example: struct
// field → wire name → builder → TypeScript property, each link pinned to the
// next, so a knob cannot exist at one end and be missing at the other.

/// The fields `pub struct Options` declares in a *released* build, read off
/// the source.
///
/// The `#[cfg(test)]` pair is skipped deliberately: those fields are absent
/// from the shape a consumer links, so they are absent from its wire form
/// too. That the crate's own test build cannot reach them either — the
/// `#[serde(skip)]` that keeps `render.unsafe` off every spelling of the wire
/// — is pinned by a unit test in `src/lib.rs`, because that is the only build
/// in which the fields exist at all.
fn declared_option_fields() -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let src = fs::read_to_string(&path).expect("src/lib.rs must be readable");
    let mut names = BTreeSet::new();
    let mut inside = false;
    let mut behind_cfg_test = false;
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed == "pub struct Options {" {
            inside = true;
        } else if inside && trimmed == "}" {
            break;
        } else if inside && trimmed == "#[cfg(test)]" {
            behind_cfg_test = true;
        } else if inside && let Some((name, _)) = trimmed.split_once(": bool,") {
            if !behind_cfg_test {
                names.insert(name.to_owned());
            }
            behind_cfg_test = false;
        }
    }
    assert!(
        !names.is_empty(),
        "no `pub struct Options` fields found in {}; the reader must be retargeted, not deleted",
        path.display()
    );
    names
}

/// serde's `rename_all = "camelCase"`, applied here rather than taken on
/// trust — the point of the rule below is to compare two independent
/// spellings of the same knob set.
fn camel_case(snake: &str) -> String {
    let mut out = String::new();
    let mut capitalise = false;
    for ch in snake.chars() {
        if ch == '_' {
            capitalise = true;
        } else if capitalise {
            out.extend(ch.to_uppercase());
            capitalise = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[test]
fn the_wire_form_names_exactly_the_fields_the_struct_declares() {
    let expected: BTreeSet<String> = declared_option_fields()
        .iter()
        .map(|f| camel_case(f))
        .collect();
    let wired: BTreeSet<String> = KNOBS.iter().map(|k| k.wire.to_owned()).collect();
    assert_eq!(
        expected, wired,
        "a field the wire form does not name is a knob a browser host cannot set, and a wire \
         name no field backs is one it can set to nothing. Both read as working from JS"
    );
}

#[cfg(feature = "serde")]
fn decode(json: &str) -> Options {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("`{json}` must decode as Options: {e}"))
}

/// The configurations only the wire reaches: an object naming one knob — the
/// partial object `#[serde(default)]` exists for — and the two that name all
/// nine at once.
#[cfg(feature = "serde")]
fn wire_reachable_options() -> Vec<(String, Options)> {
    let mut out = Vec::new();
    for on in [false, true] {
        for knob in KNOBS {
            let json = format!("{{\"{}\": {on}}}", knob.wire);
            let opts = decode(&json);
            out.push((json, opts));
        }
        let fields: Vec<String> = KNOBS
            .iter()
            .map(|knob| format!("\"{}\": {on}", knob.wire))
            .collect();
        let json = format!("{{{}}}", fields.join(", "));
        let opts = decode(&json);
        out.push((json, opts));
    }
    out
}

#[cfg(feature = "serde")]
#[test]
fn every_knob_is_settable_over_the_wire() {
    // The load-bearing half, and the one the shadow struct failed: a knob
    // present on the type but absent from the wire is not a smaller API, it
    // is an API that accepts the setting and discards it.
    for knob in KNOBS {
        for on in [false, true] {
            let json = format!("{{\"{}\": {on}}}", knob.wire);
            assert_eq!(
                decode(&json),
                (knob.set)(Options::default(), on),
                "`{json}` must be `Options::default().{}({on})`",
                knob.name
            );
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn an_object_naming_every_knob_is_the_builder_chain_that_sets_them() {
    // Exhaustive rather than sampled — 2^9 is 512, and enumerating it costs
    // less than a proptest block. Two things ride on it that the one-knob
    // rule above cannot see: that no two wire names write the same field
    // (which would still pass singly, each shadowing the other's default),
    // and that an object naming all nine determines the value outright — so
    // the constructor it is compared against is immaterial.
    for bits in 0..(1u32 << KNOBS.len()) {
        let on = |index: usize| bits & (1u32 << index) != 0;
        let fields: Vec<String> = KNOBS
            .iter()
            .enumerate()
            .map(|(index, knob)| format!("\"{}\": {}", knob.wire, on(index)))
            .collect();
        let json = format!("{{{}}}", fields.join(", "));
        let decoded = decode(&json);
        for (ctor, build) in CONSTRUCTORS {
            let mut built = build();
            for (index, knob) in KNOBS.iter().enumerate() {
                built = (knob.set)(built, on(index));
            }
            assert_eq!(
                decoded, built,
                "`{json}` must equal the same chain built from `{ctor}()`"
            );
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn a_key_this_crate_does_not_know_is_ignored_rather_than_refused() {
    assert_eq!(
        decode("{}"),
        Options::default(),
        "an object naming nothing must be the shipped dialect, or `#[serde(default)]` is not \
         doing the job the `.d.ts`'s optional properties promise"
    );
    // Leniency is the shipped decision, and it has a sharp edge worth
    // stating in a test rather than in a comment nobody runs: the wasm
    // bridge used to spell the first knob `aozoraEnabled`, and a host still
    // sending that name is not told it has stopped working — it silently
    // gets the default. Refusing an unknown key instead is
    // `deny_unknown_fields`, which is a decision about the wire format and
    // not one a test makes; this pins what the format does today so that
    // decision is a visible edit rather than a drift.
    assert_eq!(
        decode(r#"{"aozoraEnabled": false}"#),
        Options::default(),
        "an unknown key must not change the configuration"
    );
}

/// The property names an emitted TypeScript interface declares, each with
/// whether it is optional.
#[cfg(feature = "tsify")]
fn interface_properties(decl: &str) -> BTreeSet<(String, bool)> {
    let body = decl
        .split_once("export interface")
        .expect("the declaration must be an interface")
        .1;
    let body = body
        .split_once('{')
        .expect("an interface has a body")
        .1
        .rsplit_once('}')
        .expect("an interface body closes")
        .0;
    body.split(';')
        .filter_map(|entry| {
            let name = entry.trim().split_once(':')?.0.trim();
            Some(
                name.strip_suffix('?')
                    .map_or_else(|| (name.to_owned(), false), |base| (base.to_owned(), true)),
            )
        })
        .collect()
}

#[cfg(feature = "tsify")]
#[test]
fn the_typescript_interface_offers_every_wire_knob_and_marks_it_optional() {
    // The last link, and the one no other gate can hold. `tsify` and `serde`
    // read the same attributes down different code paths, so the TypeScript
    // a host is typed against and the JSON serde will actually accept are
    // two derivations that agree by convention rather than by construction.
    // `tsc` cannot notice: a missing property is simply a narrower type, and
    // narrower type-checks.
    //
    // Optionality is half the claim. It is `#[serde(default)]` that makes a
    // partial object legal and the same attribute that makes `tsify` write
    // `?`, so dropping it would silently retype every knob as required —
    // and the resulting `.d.ts` would demand nine fields for a call that
    // still worked with none.
    let declared = interface_properties(<Options as tsify::Tsify>::DECL);
    assert!(
        !declared.is_empty(),
        "no properties parsed out of the `Options` declaration; the reader must be retargeted, \
         not deleted"
    );
    let expected: BTreeSet<(String, bool)> = KNOBS
        .iter()
        .map(|knob| (knob.wire.to_owned(), true))
        .collect();
    assert_eq!(
        declared, expected,
        "the TypeScript a browser host is typed against and the wire form serde accepts must be \
         one knob set, each property optional"
    );
}

// ---------------------------------------------------------------------------
// the space, under adversarial input
// ---------------------------------------------------------------------------

/// An arbitrary configuration a consumer can build: one constructor, then
/// every knob forced to a drawn value. Grown from the same table, so it
/// widens with the surface.
fn any_options() -> impl Strategy<Value = Options> {
    (
        0..CONSTRUCTORS.len(),
        prop::collection::vec(any::<bool>(), KNOBS.len()),
    )
        .prop_map(|(ctor, bits)| {
            let mut opts = (CONSTRUCTORS[ctor].1)();
            for (knob, on) in KNOBS.iter().zip(bits) {
                opts = (knob.set)(opts, on);
            }
            opts
        })
}

proptest! {
    #![proptest_config(config::default())]

    /// The XSS corpus is fixed, so it only covers payloads someone thought
    /// of. This crosses the option space with the adversarial generators
    /// `property_html_shape` already draws from — that file pins the same
    /// invariants under `Options::default()` alone, so this is the same
    /// property with the configuration quantified rather than fixed.
    #[test]
    fn arbitrary_configurations_hold_the_html_invariants(
        src in prop_oneof![
            aozora_fragment(12),
            pathological_aozora(6),
            commonmark_adversarial(),
        ],
        opts in any_options(),
    ) {
        let rendered = render(&src, &opts);
        assert_html_invariants(&src, &rendered.html);
        prop_assert_eq!(
            &render_to_ir(&src, &opts).html,
            &rendered.html,
            "the IR path must format the same document as the HTML path"
        );
        // Quantified here rather than only over the fixed corpus above: what
        // `check` now reports is `diagnose`'s answer, and an input nobody
        // wrote down is exactly where a second reading of the same source
        // would first differ.
        prop_assert_eq!(
            &diagnose(&src, &opts),
            &rendered.diagnostics,
            "`diagnose` must report what the render it skips would have"
        );
        // The per-block path restores the code-block mask a block at a time,
        // so it can leak where the document path does not.
        let RenderedBlocks { blocks, .. } = render_blocks(&src, &opts);
        let joined: String = blocks.iter().map(|b| b.html.as_str()).collect();
        assert_html_invariants(&src, &joined);
    }
}
