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
//!   is the claim itself, and it is clean.
//! * [`hardbreaks_is_why_the_headline_claim_needs_a_condition`] pins why the
//!   unconditional sentence this replaced was false.
//! * [`a_rule_row_renders_in_the_block_that_owns_it`] holds the matrix the
//!   serialize half already runs green, now that `render` runs it green too.
//! * [`every_reserved_codepoint_is_neutralised_on_the_render_path`] holds the
//!   *other* matrix the serialize half runs — the one whose two halves answer
//!   differently on purpose.

use aozora_flavored_markdown::{Options, render, sentinels, to_html};
use pretty_assertions::assert_eq;
use serde::Deserialize;

const COMMONMARK: &str = include_str!("../../../spec/commonmark-0.31.2.json");
const GFM: &str = include_str!("../../../spec/gfm-0.29-gfm.json");

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
    // No tolerance and no pinned count. Both were here until the rule-row
    // protection reached this half of the crate: four documents — CommonMark
    // example 83 and its GFM twin 53, bare and in a list item — used to come
    // out as `<p>Foo</p><hr />` where the spec says `<h2>Foo</h2>`, because
    // the sibling parser pushed the 25-character setext underline onto a
    // stanza of its own and comrak read the split form.
    let dialect = strict_dialect();
    let gfm = Options::gfm();
    for (where_, doc) in every_spec_document() {
        assert_eq!(
            render(&doc, &dialect).html,
            render(&doc, &gfm).html,
            "{where_} does not render as GFM does\n  src = {doc:?}"
        );
    }
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
fn a_rule_row_renders_in_the_block_that_owns_it() {
    // `crate::verbatim_regions` holds a rule row out of the reach of the
    // CommonMark-blind sibling parser, which would otherwise push one onto a
    // stanza of its own and split whichever block CommonMark had given the
    // bytes to. `serialize_commonmark_identity`'s
    // `a_rule_row_stays_in_the_block_that_owns_it` runs exactly the matrix
    // below for `canonicalize`; this is the same matrix for `render`.
    //
    // It used to be green for `canonicalize` alone, at 52 of these 270 cells
    // corrupted, every one of them at a width of ten or thirty-five: the
    // module had one caller, `canonicalise_pass`, and `render` masked with
    // `code_block_mask`, which knows only about fences. Widths 1, 3 and 9 are
    // still the control group — below whatever threshold the sibling parser
    // applies, and they never moved even while the defect stood.
    let dialect = strict_dialect();
    let gfm = Options::gfm();

    let mut cells = 0usize;
    for rule in ['-', '=', '_'] {
        for &width in RULE_WIDTHS {
            let row = String::from(rule).repeat(width);
            for context in BLOCK_CONTEXTS {
                let src = context.replace('@', &row);
                cells += 1;
                assert_eq!(
                    render(&src, &dialect).html,
                    render(&src, &gfm).html,
                    "a rule row left the block CommonMark gave it to, in {context:?}"
                );
            }
        }
    }
    assert_eq!(
        cells,
        3 * RULE_WIDTHS.len() * BLOCK_CONTEXTS.len(),
        "the matrix stopped covering what it enumerates"
    );

    // The shape, spelled out, because a matrix says nothing about severity:
    // a setext H2 whose underline is ten characters long stayed a paragraph
    // and a `<hr>`, through the crate's own one-liner entry point.
    assert_eq!(
        to_html("Foo\n----------\n"),
        "<h2>Foo</h2>\n",
        "a long setext underline underlines the heading it belongs to"
    );
    assert_eq!(
        render("Foo\n----------\n", &gfm).html,
        "<h2>Foo</h2>\n",
        "what CommonMark says the same source means"
    );
    assert_eq!(
        render("Foo\n---\n", &dialect).html,
        "<h2>Foo</h2>\n",
        "a short underline never crossed the sibling's threshold and still works"
    );
}

#[test]
fn a_source_that_carries_the_substitution_gets_no_rule_row_protection() {
    // THE SURVIVING DEFECT of the fix above, stated where it can be measured.
    //
    // `verbatim_regions::hide_rule_rows` substitutes one byte for one so the
    // parser's offsets keep addressing the caller's own text, and the bytes it
    // substitutes are U+0001..U+0003. A source that already carries one of the
    // three would make the reveal claim a byte the author wrote, so the
    // protection stands down for the whole document and the rule row goes back
    // to leaving the block CommonMark gave it to. Same shape and same reason as
    // `code_block_mask` standing down on a source-typed U+E000; unlike that
    // one, this one is not a documented contract at the public surface, because
    // a C0 control is not a codepoint this crate reserves — it is ordinary (if
    // strange) CommonMark text.
    //
    // Asserted as an inequality, in the idiom `serialize_commonmark_identity`
    // uses for the doubled BOM: a defect named by a green test is a defect that
    // cannot be rediscovered as a surprise.
    let dialect = strict_dialect();
    let gfm = Options::gfm();
    for hidden in ['\u{1}', '\u{2}', '\u{3}'] {
        let src = format!("{hidden}\nFoo\n----------\n");
        assert_ne!(
            render(&src, &dialect).html,
            render(&src, &gfm).html,
            "U+{:04X} in the source no longer switches the protection off — if the mechanism \
             stopped needing to stand down, delete this test rather than weaken it",
            hidden as u32
        );
        assert_eq!(
            render(&src, &dialect).html,
            format!("<p>{hidden}\nFoo</p>\n<hr />\n"),
            "and what it costs is exactly the pre-fix reading of the row"
        );
    }
    // A neighbouring control character is not part of the substitution, so it
    // costs nothing: the bail-out is keyed to the three bytes the reveal would
    // claim rather than to C0 in general.
    let src = "\u{4}\nFoo\n----------\n";
    assert_eq!(
        render(src, &dialect).html,
        render(src, &gfm).html,
        "the carve-out widened past the three bytes it is about"
    );
}

#[test]
fn every_reserved_codepoint_is_neutralised_on_the_render_path() {
    // The render-side mirror of `serialize_commonmark_identity`'s
    // `every_reserved_codepoint_in_the_source_comes_back_as_written`, over the
    // same five codepoints and the same 18 contexts.
    //
    // It is deliberately NOT the same assertion. `canonicalize` owes its caller
    // the source it was handed, so it preserves all five; `render` owes nobody
    // an offset into a text with a substituted sentinel in it, and the sibling
    // parser neutralises the four it lexes with. The two halves answering
    // differently is the whole of DEV-235, and the decision taken there is
    // "destroyed by design" — recorded on `sentinels` at the public surface and
    // in both READMEs. This is the test that makes it un-silent.
    //
    // Every cell is stated exactly rather than counted: what comes out is what
    // GFM renders with the codepoint rewritten to U+FFFD, which for `MASK` is
    // the identity because masking stands down on such a source.
    let dialect = strict_dialect();
    let gfm = Options::gfm();

    let mut destroyed = 0usize;
    let mut cells = 0usize;
    for reserved in sentinels::ALL {
        for context in BLOCK_CONTEXTS {
            let src = context.replace('@', &format!("a{reserved}b"));
            let commonmark = render(&src, &gfm).html;
            let expected = if reserved == sentinels::MASK {
                commonmark.clone()
            } else {
                commonmark.replace(reserved, "\u{FFFD}")
            };
            let out = render(&src, &dialect).html;
            assert_eq!(
                out, expected,
                "reserved U+{:04X} did not reach the output as `sentinels` promises, in {context:?}",
                reserved as u32
            );
            cells += 1;
            if out != commonmark {
                destroyed += 1;
            }
        }
    }
    assert_eq!(
        cells,
        sentinels::ALL.len() * BLOCK_CONTEXTS.len(),
        "the matrix stopped covering what it enumerates"
    );

    // The 68-of-90 figure DEV-235 asked to have reproduced, derived rather than
    // measured: 90 cells, less the 18 where `MASK` comes back as written, less
    // the one context per destroyed codepoint where the difference cannot show
    // — a raw HTML block, which comrak collapses to `<!-- raw HTML omitted -->`
    // whatever is inside it.
    assert_eq!(
        destroyed,
        BLOCK_CONTEXTS.len() * (sentinels::ALL.len() - 1) - (sentinels::ALL.len() - 1),
        "the render-side reserved-codepoint contract moved"
    );
    assert_eq!(destroyed, 68, "…and 68 is what that arithmetic comes to");
    let raw_html = "<div>\na\u{E001}b\n</div>\n";
    assert_eq!(
        render(raw_html, &dialect).html,
        render(raw_html, &gfm).html,
        "the four cells that agree do so because neither dialect emits the byte at all"
    );

    // The shape, spelled out at the one-liner entry point, for both answers.
    assert_eq!(
        to_html("a\u{E001}b"),
        "<p>a\u{FFFD}b</p>\n",
        "an author-typed sentinel is destroyed, and `sentinels` says so"
    );
    assert_eq!(
        to_html("a\u{E000}b"),
        "<p>a\u{E000}b</p>\n",
        "the mask is the one that comes back, masking having stood down"
    );
}
