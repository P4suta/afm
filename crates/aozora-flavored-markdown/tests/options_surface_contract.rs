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
use std::fs;
use std::path::Path;

use aozora_flavored_markdown::{Options, render, render_blocks_to_ir, render_to_ir, serialize};
use aozora_flavored_markdown_test_support::config::default_config;
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
struct Knob {
    name: &'static str,
    set: fn(Options, bool) -> Options,
    probe: &'static str,
    marker: &'static str,
    default_on: bool,
}

const KNOBS: &[Knob] = &[
    Knob {
        name: "with_aozora",
        set: |o, on| o.with_aozora(on),
        probe: "｜青梅《おうめ》",
        marker: "<ruby>",
        default_on: true,
    },
    Knob {
        name: "with_hardbreaks",
        set: |o, on| o.with_hardbreaks(on),
        probe: "a\nb",
        marker: "<br />",
        default_on: true,
    },
    Knob {
        name: "with_smart_punctuation",
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
        set: |o, on| o.with_cjk_friendly_emphasis(on),
        probe: "これは**「強調」**です",
        marker: "<strong>",
        default_on: true,
    },
    Knob {
        name: "with_source_line_anchors",
        set: |o, on| o.with_source_line_anchors(on),
        probe: "para",
        marker: "data-aozora-md-source-line",
        default_on: false,
    },
    Knob {
        name: "with_tables",
        set: |o, on| o.with_tables(on),
        probe: "| a |\n| - |\n| b |\n",
        marker: "<table>",
        default_on: true,
    },
    Knob {
        name: "with_strikethrough",
        set: |o, on| o.with_strikethrough(on),
        probe: "~~x~~",
        marker: "<del>",
        default_on: true,
    },
    Knob {
        name: "with_autolinks",
        set: |o, on| o.with_autolinks(on),
        probe: "see https://example.com/ ok",
        marker: "<a href=",
        default_on: true,
    },
    Knob {
        name: "with_task_lists",
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
// the space, swept
// ---------------------------------------------------------------------------

/// Every configuration one call chain can reach: each constructor bare, and
/// each constructor with each knob forced both ways.
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
            let (blocks, _) = render_blocks_to_ir(src, &opts);
            let joined: String = blocks.iter().map(|b| b.html.as_str()).collect();
            assert_inert(&label, src, &joined);
        }
    }
}

#[test]
fn the_canonicaliser_leaves_no_payload_more_dangerous_than_it_found_it() {
    // `serialize` takes no options, so it has no configuration space — but it
    // is the other public path a payload can travel, and its output is fed
    // back to `render` by every round-tripping host.
    for src in XSS_PAYLOADS {
        let canonical = serialize(src);
        assert_inert(
            "serialize→render",
            src,
            &render(&canonical, &Options::new()).html,
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
    #![proptest_config(default_config())]

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
        // The per-block path restores the code-block mask a block at a time,
        // so it can leak where the document path does not.
        let (blocks, _) = render_blocks_to_ir(&src, &opts);
        let joined: String = blocks.iter().map(|b| b.html.as_str()).collect();
        assert_html_invariants(&src, &joined);
    }
}
