//! `render` against every example in the CommonMark and GFM specs, through
//! the dialect the README names.
//!
//! The other half of the claim `serialize_commonmark_identity.rs` opened.
//! That file asks what `canonicalize` does to a pure-CommonMark source; this
//! one asks what `render` does to it, which is the question the README's
//! headline sentence is actually about.
//!
//! Nothing held that half. The conformance runners in `src/conformance.rs`
//! prove `Options::commonmark()` and `Options::gfm()` verbatim against the
//! two corpora — but neither of those is the configuration `to_html` uses,
//! and neither is what a caller gets from `Options::default()`. The dialect
//! adds three things to `gfm()`: hardbreaks, CJK-friendly emphasis, and the
//! 青空文庫 pre-pass. The README now says which of them costs the strict
//! reading (hardbreaks, deliberately) and points here for the rest. So the
//! rest is measured, over the same 3 972 documents the serialize half sweeps:
//!
//! * [`nothing_but_the_aozora_pass_stands_between_the_dialect_and_gfm`] is
//!   the control. With the notation pass off, the dialect must render every
//!   document exactly as `gfm()` does — no exceptions, so a divergence below
//!   is attributable to the pass rather than merely noticed.
//! * [`the_dialect_renders_every_spec_example_as_gfm_does_once_hardbreaks_is_off`]
//!   is the claim itself, and it is not clean: four documents survive as a
//!   defect, pinned rather than excused.
//! * [`hardbreaks_is_why_the_headline_claim_needs_a_condition`] pins why the
//!   unconditional sentence this replaced was false.
//! * [`the_rule_row_protection_covers_canonicalize_and_leaves_render_open`]
//!   states the surviving defect precisely, on the same matrix the serialize
//!   half already runs green.

use aozora_flavored_markdown::{Options, canonicalize, render, to_html};
use pretty_assertions::assert_eq;
use serde::Deserialize;

const COMMONMARK: &str = include_str!("../../../spec/commonmark-0.31.2.json");
const GFM: &str = include_str!("../../../spec/gfm-0.29-gfm.json");

/// Measured 2026-07-28 over all 3 972 documents. Every one of them is
/// CommonMark example 83 and its GFM twin 53 —
/// `"Foo\n-------------------------\n\nFoo\n=\n"` — in its bare and in its
/// list-item shape. The 25-character setext underline is long enough for the
/// sibling parser to read as a decorative rule and push onto a stanza of its
/// own, so `<h2>Foo</h2>` comes out as `<p>Foo</p><hr />`.
///
/// A surviving defect rather than a tolerance: the right value is 0. It is
/// pinned as a count so that fixing it fails here — with the message telling
/// the reader what to write instead — rather than passing silently.
/// [`the_rule_row_protection_covers_canonicalize_and_leaves_render_open`]
/// says what is broken and where the protection that should cover it lives.
const EXPECTED_RENDER_DIVERGENCES: usize = 4;

/// Measured 2026-07-28 over the 270 cells of the matrix below: 52 of them
/// render differently under the dialect than under `gfm()`, every one at a
/// width of ten or thirty-five. Widths 1, 3 and 9 are the control group —
/// below whatever threshold the sibling parser applies, and none of them
/// moves — which is what makes this a measurement of one rewrite rather than
/// of the pre-pass in general.
///
/// The right value is 0, for the same defect as above.
const RULE_ROWS_RENDER_CORRUPTS: usize = 52;

/// A `@` stands for the row under test. Copied from
/// `serialize_commonmark_identity.rs` deliberately: the point of this file is
/// that the same cells behave differently on the other side of the crate, and
/// a shared helper crate would let one list drift into covering only the half
/// that passes.
const BLOCK_CONTEXTS: &[&str] = &[
    "@\n",
    "aaa\n@\n",
    "aaa\n\n@\n",
    "aaa\n@\nbbb\n",
    "- aaa\n@\n",
    "- aaa\n  @\n",
    "1. aaa\n   @\n",
    "> aaa\n@\n",
    "> aaa\n> @\n",
    "> @\n",
    "- @\n",
    "| a |\n| - |\n| b |\n@\n",
    "aaa\n    @\n",
    "    aaa\n    @\n",
    "# h\n@\n",
    "[a]: /url\n@\n",
    "<div>\n@\n</div>\n",
    "```\n@\n```\n",
];

/// The same widths the serialize half uses, for the same reason: both sides
/// of each grammar's threshold, with none of the sibling parser's own
/// constants named here.
const RULE_WIDTHS: &[usize] = &[1, 3, 9, 10, 35];

#[derive(Debug, Deserialize)]
struct SpecExample {
    example: u32,
    section: String,
    markdown: String,
}

fn load(fixture: &str) -> Vec<SpecExample> {
    serde_json::from_str(fixture).expect("spec fixture parses as JSON")
}

/// The dialect with the one knob the README tells a reader to turn off. This
/// is the configuration the strict reading is claimed for, spelled the way
/// the README spells it.
fn strict_dialect() -> Options {
    Options::default().with_hardbreaks(false)
}

/// Every line prefixed with `> `, a blank one with a bare `>` so the quote
/// does not end.
fn inside_a_blockquote(src: &str) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    for line in src.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        out.push_str(if body.is_empty() { ">" } else { "> " });
        out.push_str(body);
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// The same, as one list item.
fn inside_a_list_item(src: &str) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    for (index, line) in src.split_inclusive('\n').enumerate() {
        let body = line.trim_end_matches('\n');
        if !body.is_empty() {
            out.push_str(if index == 0 { "- " } else { "  " });
            out.push_str(body);
        }
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Every spec example in every container shape, each labelled with where it
/// came from. The same 1 324 × 3 documents the serialize half sweeps, so the
/// two halves of the claim are measured over one corpus rather than two.
fn every_spec_document() -> Vec<(String, String)> {
    let commonmark = load(COMMONMARK);
    let gfm = load(GFM);
    assert_eq!(commonmark.len(), 652, "re-run `just spec-refresh`");
    assert_eq!(gfm.len(), 672, "re-run `just spec-refresh`");

    let mut documents = Vec::with_capacity((commonmark.len() + gfm.len()) * 3);
    for (corpus, examples) in [("CommonMark", &commonmark), ("GFM", &gfm)] {
        for example in examples {
            for (shape, doc) in [
                ("bare", example.markdown.clone()),
                ("in a blockquote", inside_a_blockquote(&example.markdown)),
                ("in a list item", inside_a_list_item(&example.markdown)),
            ] {
                let where_ = format!(
                    "{corpus} example {} ({}), {shape}",
                    example.example, example.section
                );
                documents.push((where_, doc));
            }
        }
    }
    documents
}

/// A line that is a run of one rule character and nothing else, at any
/// width. Deliberately not a threshold: the threshold is the sibling
/// parser's, and naming it here would pin this crate to a number it does not
/// own.
fn has_a_rule_row(src: &str) -> bool {
    src.lines().any(|line| {
        let row = line.trim();
        let mut bytes = row.bytes();
        let Some(first) = bytes.next() else {
            return false;
        };
        matches!(first, b'-' | b'=' | b'_') && bytes.all(|byte| byte == first)
    })
}

#[test]
fn nothing_but_the_aozora_pass_stands_between_the_dialect_and_gfm() {
    // The dialect is not `gfm()` plus the notation pass: it also carries
    // `cjk_friendly_emphasis`, a knob that edits CommonMark's own flanking
    // rules rather than adding notation on top of them. `options_surface_contract`
    // sweeps the knob space for XSS and for each knob being load-bearing, on
    // generated fragments — never against the corpus the compatibility claim
    // is made about, so no gate had ever asked what that one costs a spec
    // example. It costs nothing, on all 3 972, which is what lets the next
    // test attribute its divergences to the pass.
    let control = strict_dialect().with_aozora(false);
    let gfm = Options::gfm();
    for (where_, doc) in every_spec_document() {
        assert_eq!(
            render(&doc, &control).html,
            render(&doc, &gfm).html,
            "{where_} moved with the aozora pass already off\n  src = {doc:?}"
        );
    }
}

#[test]
fn the_dialect_renders_every_spec_example_as_gfm_does_once_hardbreaks_is_off() {
    let dialect = strict_dialect();
    let gfm = Options::gfm();

    let mut divergences = Vec::new();
    for (where_, doc) in every_spec_document() {
        if render(&doc, &dialect).html == render(&doc, &gfm).html {
            continue;
        }
        // Two things are demanded of a divergence before it may be counted
        // rather than failed. It has to carry a rule row — necessary and not
        // sufficient, which is why the count below is what has the teeth —
        // and `canonicalize` has to reproduce the very same document
        // verbatim, which is the statement that the protection exists and
        // that this path is simply outside it.
        assert!(
            has_a_rule_row(&doc),
            "{where_} diverged with no rule row in it — a failure mode this file has not seen\n  \
             src = {doc:?}"
        );
        assert_eq!(
            canonicalize(&doc).as_deref(),
            Ok(doc.as_str()),
            "{where_} is not a document `canonicalize` protects either"
        );
        divergences.push(where_);
    }
    assert_eq!(
        divergences.len(),
        EXPECTED_RENDER_DIVERGENCES,
        "the set of spec examples the dialect does not render as GFM does moved. If it shrank, \
         the defect is being fixed — lower EXPECTED_RENDER_DIVERGENCES, and delete it at 0:\n  {}",
        divergences.join("\n  ")
    );
}

#[test]
fn hardbreaks_is_why_the_headline_claim_needs_a_condition() {
    // The sentence this file is the proof of used to read "strict superset:
    // pure CommonMark input renders identically", unconditionally. It was
    // false the day it was written, for a reason with nothing to do with
    // 青空文庫 notation: the shipped dialect turns every soft break into a
    // `<br>`. Both outputs are pinned side by side so the unconditional
    // sentence cannot come back with every gate green.
    let soft_break = "aaa\nbbb\n";
    assert_eq!(
        render(soft_break, &Options::default()).html,
        "<p>aaa<br />\nbbb</p>\n",
        "the shipped dialect must still hard-break — that is what makes it a dialect"
    );
    assert_eq!(
        render(soft_break, &Options::gfm()).html,
        "<p>aaa\nbbb</p>\n",
        "GFM must not, which is the whole of the difference"
    );

    // And it is not a corner of the corpus. Measured 2026-07-28: 411 of the
    // 3 972 documents. Asserted as a floor because the exact number is a
    // property of the spec's line breaks rather than of this crate — but a
    // floor this high is only reachable while hardbreaks is genuinely on.
    let dialect = Options::default();
    let strict = strict_dialect();
    let moved = every_spec_document()
        .iter()
        .filter(|(_, doc)| render(doc, &dialect).html != render(doc, &strict).html)
        .count();
    assert!(
        moved >= 300,
        "hardbreaks moved only {moved} of the corpus's documents; the condition on the README's \
         claim is about a difference that has stopped existing"
    );
}

#[test]
fn the_rule_row_protection_covers_canonicalize_and_leaves_render_open() {
    // THE SURVIVING DEFECT, stated where it can be measured.
    //
    // `crate::verbatim_regions` holds a rule row out of the reach of the
    // CommonMark-blind sibling parser, which would otherwise push one onto a
    // stanza of its own and split whichever block CommonMark had given the
    // bytes to. `serialize_commonmark_identity`'s
    // `a_rule_row_stays_in_the_block_that_owns_it` runs exactly the matrix
    // below and is green.
    //
    // It is green for `canonicalize` alone. `verbatim_regions` has one
    // caller, `canonicalise_pass`; `render` masks with `code_block_mask`
    // instead, which hides 青空文庫 triggers inside fences and knows nothing
    // about rule rows. So the protection covers the half of the crate that
    // owes a caller its source back and not the half that every caller
    // actually uses — and the assertion inside the loop is that statement:
    // every cell `render` corrupts is one `canonicalize` reproduces verbatim.
    let dialect = strict_dialect();
    let gfm = Options::gfm();

    let mut corrupted = 0usize;
    let mut cells = 0usize;
    for rule in ['-', '=', '_'] {
        for &width in RULE_WIDTHS {
            let row = String::from(rule).repeat(width);
            for context in BLOCK_CONTEXTS {
                let src = context.replace('@', &row);
                cells += 1;
                if render(&src, &dialect).html == render(&src, &gfm).html {
                    continue;
                }
                assert_eq!(
                    canonicalize(&src).as_deref(),
                    Ok(src.as_str()),
                    "a rule row `canonicalize` does not protect either, in {context:?}"
                );
                corrupted += 1;
            }
        }
    }
    assert_eq!(
        cells,
        3 * RULE_WIDTHS.len() * BLOCK_CONTEXTS.len(),
        "the matrix stopped covering what it enumerates"
    );
    assert_eq!(
        corrupted, RULE_ROWS_RENDER_CORRUPTS,
        "the render-side rule-row corruption moved. If it shrank, the defect is being fixed — \
         lower RULE_ROWS_RENDER_CORRUPTS, and delete this test at 0"
    );

    // The shape, spelled out, because a count says nothing about severity:
    // a setext H2 whose underline is ten characters long stops being a
    // heading, through the crate's own one-liner entry point.
    assert_eq!(
        to_html("Foo\n----------\n"),
        "<p>Foo</p>\n<hr />\n",
        "pinning the defect: a long setext underline splits the heading it belongs to"
    );
    assert_eq!(
        render("Foo\n----------\n", &gfm).html,
        "<h2>Foo</h2>\n",
        "what CommonMark says the same source means"
    );
    assert_eq!(
        render("Foo\n---\n", &dialect).html,
        "<h2>Foo</h2>\n",
        "a short underline is below the sibling's threshold and survives"
    );
}
