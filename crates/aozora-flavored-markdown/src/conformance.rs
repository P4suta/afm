// CommonMark 0.31.2 and GFM 0.29 conformance, against the spec fixtures
// under `spec/` (converted from the upstream sources by `xtask spec-refresh`).
//
// comrak claims 100% CommonMark compatibility; this crate wraps it unmodified,
// so 652/652 is the expectation. A drop means the wrapper — lexer pre-pass,
// option defaults, the HTML splice — perturbed upstream behaviour.
//
// These runners are the executable half of the README's compatibility claim,
// so what they render with is what that claim names and nothing adjacent:
// `Options::commonmark()` for the CommonMark suite, `Options::gfm()` for the
// GFM one. Both are public, so a reader can reach for the same constructor
// the README points at and get the behaviour measured here. `Options::new()`
// — the Aozora dialect — is deliberately NOT under test: it sets
// `hardbreaks`, which turns a soft break into a `<br>` and so cannot render
// the spec verbatim. That is a dialect decision, not a conformance gap, and
// the README now says which preset carries which promise.
//
// The one delta both runners add is raw-HTML passthrough, because the
// expected output in both fixtures contains raw HTML. `Options::with_raw_html`
// turns it on and it is `#[cfg(test)]`, which is why these runners live in
// `src/` instead of `tests/`: an integration test is a separate crate and
// could only reach that switch if it were public.
//
// One `assert_eq!` per example, rather than a divergence collector: the
// collector's own reporting is unreachable while the suite passes, and
// `pretty_assertions` already diffs the first mismatch better than a summary
// of five would.

use std::collections::BTreeSet;

use pretty_assertions::assert_eq;
use serde::Deserialize;

use crate::{Options, render};

const COMMONMARK_FIXTURE: &str = include_str!("../../../spec/commonmark-0.31.2.json");
const GFM_FIXTURE: &str = include_str!("../../../spec/gfm-0.29-gfm.json");

// Attribute order and self-closing style on `<input type="checkbox">` differ
// between comrak's renderer and the GFM 0.29 expected output. The HTML is
// semantically identical and browsers render both the same. Listed explicitly
// so a renderer change that affects these cases still surfaces.
const KNOWN_COSMETIC_DIVERGENCES: &[u32] = &[279, 280];

// The GFM spec's untagged examples are inherited CommonMark 0.29 cases, and a
// handful of emphasis-disambiguation cases moved between 0.29 and 0.31.2 — GFM
// example 398 (`__foo, __bar__, baz__`) wants a flat `<strong>`, 0.31.2 a
// nested one. The CommonMark runner covers the authoritative semantics, so
// asking this one to also verify superseded ones would report false
// regressions.
const GFM_EXTENSION_TAGS: [&str; 5] = [
    "autolink",
    "disabled",
    "strikethrough",
    "table",
    "tagfilter",
];

#[derive(Debug, Deserialize)]
struct SpecExample {
    example: u32,
    section: String,
    markdown: String,
    html: String,
    // Absent throughout the CommonMark fixture, and present on the GFM
    // examples that declare an extension.
    #[serde(default)]
    extension: Option<String>,
}

fn load(fixture: &str) -> Vec<SpecExample> {
    serde_json::from_str(fixture).expect("spec fixture parses as JSON")
}

// A named function rather than a `let` inside the runner, so the pin at the
// bottom of this file can read the same configuration the suite runs with
// instead of a copy of it.
fn commonmark_options() -> Options {
    Options::commonmark().with_raw_html(true)
}

#[test]
fn commonmark_0_31_2_full_pass() {
    let examples = load(COMMONMARK_FIXTURE);
    let opts = commonmark_options();
    assert_eq!(
        examples.len(),
        652,
        "fixture example count must match the spec (re-run `just spec-refresh` if this drifts)"
    );

    for ex in &examples {
        assert_eq!(
            render(&ex.markdown, &opts).html,
            ex.html,
            "CommonMark 0.31.2 example {} (section {:?}) markdown {:?}",
            ex.example,
            ex.section,
            ex.markdown
        );
    }
}

// cmark-gfm's own runner enables only the extension the example declares.
// This does not mirror that: `Options::gfm()` carries all four at once, and a
// caller who reaches for it gets all four at once, so measuring one at a time
// would leave the interaction between them — the configuration the README
// actually names — untested. Every extension-tagged example therefore renders
// under the whole preset, which is the stronger claim of the two.
//
// `disabled` labels the task-list-items output (the `disabled` attribute on
// `<input type="checkbox">`), not a disabled example. `tagfilter` is the one
// tag `gfm()` does not cover: the filter only bites while raw HTML is passing
// through, which no public constructor can arrange, so it stays a switch the
// runner adds rather than surface a caller can ask for.
fn options_for(extension: &str) -> Options {
    let opts = Options::gfm().with_raw_html(true);
    match extension {
        "autolink" | "strikethrough" | "table" | "disabled" => opts,
        "tagfilter" => opts.with_tagfilter(true),
        other => panic!("unknown GFM extension tag in fixture: {other}"),
    }
}

#[test]
fn gfm_0_29_extension_pass() {
    let all = load(GFM_FIXTURE);
    let tagged: Vec<(&SpecExample, &str)> = all
        .iter()
        .filter_map(|ex| ex.extension.as_deref().map(|tag| (ex, tag)))
        .collect();
    // Measured 2026-04-23 against the GFM 0.29 spec: 24 examples carry an
    // explicit extension tag. Floor at 20 to catch regressions without being
    // brittle against a minor spec refresh.
    assert!(
        tagged.len() >= 20,
        "GFM fixture should have at least 20 extension-tagged examples, got {}",
        tagged.len()
    );

    for (ex, tag) in tagged {
        if KNOWN_COSMETIC_DIVERGENCES.contains(&ex.example) {
            continue;
        }
        assert_eq!(
            render(&ex.markdown, &options_for(tag)).html,
            ex.html,
            "GFM 0.29 example {} (section {:?}, extension {tag:?}) markdown {:?}",
            ex.example,
            ex.section,
            ex.markdown
        );
    }
}

#[test]
fn gfm_extension_tags_are_exhaustive() {
    // Sanity: every tag the fixture carries is one `options_for` handles.
    let tags: BTreeSet<String> = load(GFM_FIXTURE)
        .iter()
        .filter_map(|e| e.extension.clone())
        .collect();
    let known: BTreeSet<String> = GFM_EXTENSION_TAGS.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        tags, known,
        "GFM fixture contains a tag this runner does not handle; update `options_for`"
    );
}

#[test]
fn each_runner_measures_the_public_preset_the_readme_names() {
    // A green suite says the examples passed. It does not say WHICH
    // configuration passed them, and that is the whole of what the README
    // sends a reader here to check. Until this test the GFM runner built on
    // `Options::commonmark()` and switched on the single extension each
    // example declared: green, and proof of a preset no caller can name —
    // the README's sentence about `Options::gfm()` would have been untrue
    // with every gate in the repository still passing. Narrowing it back
    // that way now fails here.
    assert_eq!(
        commonmark_options().with_raw_html(false),
        Options::commonmark(),
        "the CommonMark runner must measure `Options::commonmark()` itself"
    );
    for tag in GFM_EXTENSION_TAGS {
        let opts = options_for(tag);
        assert_eq!(
            opts.clone().with_raw_html(false).with_tagfilter(false),
            Options::gfm(),
            "the GFM runner must measure `Options::gfm()` itself, and it does not for {tag:?}"
        );
        // Both deltas are documented above `options_for`, and only one of
        // them is per-example. A tag that stopped passing raw HTML through
        // would silently stop rendering what the fixture expects; a tag that
        // started switching the filter on would be measuring cmark-gfm's
        // tagfilter configuration under the name of the whole preset.
        assert_ne!(
            opts.clone().with_raw_html(false),
            opts,
            "the fixture's expected output contains raw HTML, so {tag:?} must pass it through"
        );
        assert_eq!(
            opts.clone().with_tagfilter(false) != opts,
            tag == "tagfilter",
            "only the tagfilter examples may switch the GFM tag filter on, not {tag:?}"
        );
    }
}
