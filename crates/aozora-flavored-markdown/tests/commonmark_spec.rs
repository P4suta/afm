//! CommonMark 0.31.2 spec conformance.
//!
//! Runs every example from `spec/commonmark-0.31.2.json` (vendored from the
//! upstream `commonmark-spec` repo, converted by `xtask spec-refresh`) and
//! asserts the rendered HTML is byte-identical to the spec's expected output.
//!
//! Drift expectations:
//! - comrak 0.52.0 claims "100% CommonMark compatibility" and passes all 652
//!   examples. aozora-flavored-markdown wraps it unmodified, so we expect
//!   652/652 here too.
//! - If this count drops, it means our wrapper (lex pre-pass, options
//!   default, post-process HTML splice) inadvertently mutated CommonMark
//!   behaviour — a regression that breaks the 100 % compat guarantee.

use aozora_flavored_markdown::{Options, render};
use pretty_assertions::assert_eq;
use serde::Deserialize;

const FIXTURE: &str = include_str!("../../../spec/commonmark-0.31.2.json");

#[derive(Debug, Deserialize)]
struct SpecExample {
    example: u32,
    section: String,
    markdown: String,
    html: String,
}

fn load() -> Vec<SpecExample> {
    serde_json::from_str(FIXTURE).expect("spec fixture parses as JSON")
}

#[test]
fn commonmark_0_31_2_full_pass() {
    let examples = load();
    assert_eq!(
        examples.len(),
        652,
        "fixture example count must match the spec (re-run `just spec-refresh` if this drifts)"
    );

    let opts = Options::commonmark_only();
    let mut failures: Vec<String> = Vec::new();

    for ex in &examples {
        let actual = render(&ex.markdown, &opts).html;
        if actual != ex.html {
            failures.push(format!(
                "example {} (section {:?}):\n  markdown: {:?}\n  expected: {:?}\n  actual:   {:?}",
                ex.example, ex.section, ex.markdown, ex.html, actual
            ));
            if failures.len() >= 5 {
                break;
            }
        }
    }

    assert!(
        failures.is_empty(),
        "CommonMark 0.31.2 conformance regressions (showing up to 5):\n\n{}",
        failures.join("\n\n"),
    );
}
