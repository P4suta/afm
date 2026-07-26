// CommonMark 0.31.2 and GFM 0.29 conformance, against the spec fixtures
// under `spec/` (converted from the upstream sources by `xtask spec-refresh`).
//
// comrak claims 100% CommonMark compatibility; this crate wraps it unmodified,
// so 652/652 is the expectation. A drop means the wrapper — lexer pre-pass,
// option defaults, the HTML splice — perturbed upstream behaviour.
//
// Both suites need raw-HTML passthrough, because the spec's expected output
// contains raw HTML. `Options::spec_commonmark` is the only constructor that
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

#[test]
fn commonmark_0_31_2_full_pass() {
    let examples = load(COMMONMARK_FIXTURE);
    assert_eq!(
        examples.len(),
        652,
        "fixture example count must match the spec (re-run `just spec-refresh` if this drifts)"
    );

    let opts = Options::spec_commonmark();
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

// cmark-gfm's own runner enables only the extension the example declares, and
// this mirrors that. `disabled` labels the task-list-items output (the
// `disabled` attribute on `<input type="checkbox">`), not a disabled example.
fn options_for(extension: &str) -> Options {
    let opts = Options::spec_commonmark();
    match extension {
        "autolink" => opts.with_autolinks(true),
        "strikethrough" => opts.with_strikethrough(true),
        "table" => opts.with_tables(true),
        "tagfilter" => opts.with_tagfilter(true),
        "disabled" => opts.with_task_lists(true),
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
