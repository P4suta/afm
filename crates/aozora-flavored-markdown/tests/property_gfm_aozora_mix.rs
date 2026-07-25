//! Aozora × GFM interaction properties.
//!
//! `property_html_shape.rs` already runs every always-on shape
//! predicate against `aozora_fragment` and against
//! `prop_oneof![aozora_fragment, commonmark_adversarial]`. That covers
//! "the two grammars in the same document" but does *not* push them
//! into the same paragraph, the same table cell, or the same list
//! item. GFM features (tables, strikethrough, autolinks) split the
//! comrak AST in ways that the post-process splice has to thread
//! through; a regression in that thread shows up only when the two
//! grammars share a containing block.
//!
//! The generators here weave Aozora trigger glyphs *inside* GFM
//! constructs:
//!
//! * `gfm_aozora_paragraph` — strikethrough, autolink, ruby,
//!   tate-chu-yoko and a bracket annotation in the same paragraph.
//! * `gfm_table_with_aozora_cells` — a 2×2 GFM table whose cells
//!   each carry a small Aozora fragment.
//! * `gfm_list_with_aozora_items` — a three-item GFM list with
//!   Aozora content in the items, including a nested item.
//!
//! Each property asserts the always-on tier predicates — Tier B (no PUA
//! sentinel leak) included, because no input may legitimately leak one —
//! plus Tier A (no bare `［＃` leak), gated on a clean parse so unbalanced
//! inputs don't sabotage the test.

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown::{Options, render as render_to_diagnostics};
use aozora_flavored_markdown_test_support::config::default_config;
use aozora_flavored_markdown_test_support::generators::aozora_fragment;
use aozora_flavored_markdown_test_support::{
    assert_html_invariants, check_html_tag_balance, check_no_bare_bracket,
};
use proptest::prelude::*;

/// Whether the render raised any diagnostic for `src` — the gate for
/// Tier A, which only meaningfully asserts on well-formed input. Mirrors
/// the helper in `property_html_shape.rs`.
fn parse_is_well_formed(src: &str) -> bool {
    render_to_diagnostics(src, &Options::default())
        .diagnostics
        .is_empty()
}

fn assert_mix_invariants(src: &str) {
    let html = render_to_string(src);

    // Always-on predicates: sentinel leak, tag balance, content model,
    // markup completeness, escape invariants, etc.
    assert_html_invariants(src, &html);

    // Cross-cutting tag balance is bundled inside `assert_html_invariants`,
    // but call the dedicated checker so a regression in the table-cell
    // or list-item path produces a focused failure message.
    check_html_tag_balance(&html)
        .unwrap_or_else(|e| panic!("HTML tag balance violated for src={src:?}: {e}\n---\n{html}"));

    // Tier A — gated on a clean parse.
    if parse_is_well_formed(src) {
        check_no_bare_bracket(&html)
            .unwrap_or_else(|e| panic!("Tier A (bare ［＃) violated for src={src:?}: {e}"));
    }
}

// ----------------------------------------------------------------------
// Generators — local to this file because the shape (GFM container +
// Aozora payload) is too task-specific for the shared generators.
// ----------------------------------------------------------------------

/// Aozora payloads that are short enough to live inside a GFM
/// container without overwhelming the generator. Each draw is a small
/// `aozora_fragment` plus one of a few hand-picked Aozora atoms so
/// the shrinker has stable anchors to reduce toward.
fn small_aozora_payload() -> impl Strategy<Value = String> {
    prop_oneof![
        aozora_fragment(4),
        Just("｜青梅《おうめ》".to_owned()),
        Just("漢字《かんじ》".to_owned()),
        Just("text［＃改ページ］".to_owned()),
        Just("｜文字《もじ》、※［＃「あ」、第1水準1-1］".to_owned()),
        Just("　　いろは".to_owned()),
        Just("ABC".to_owned()),
    ]
}

/// A single paragraph weaving GFM inline features and Aozora notation.
/// The output is a free-standing paragraph (terminated with a blank
/// line) so the surrounding integration is unambiguously paragraph-level.
fn gfm_aozora_paragraph() -> impl Strategy<Value = String> {
    (
        small_aozora_payload(),
        small_aozora_payload(),
        prop_oneof![
            Just("https://example.com/path".to_owned()),
            Just("http://aozora.gr.jp/".to_owned()),
            Just("`inline code`".to_owned()),
        ],
    )
        .prop_map(|(left, right, mid)| format!("Visit {mid} between ~~{left}~~ and {right}.\n\n"))
}

/// A 2-column × 2-row GFM table whose cells each carry an Aozora
/// payload. Drives the splice path through `<table>` / `<thead>` /
/// `<tbody>` / `<tr>` / `<td>` boundaries.
fn gfm_table_with_aozora_cells() -> impl Strategy<Value = String> {
    (
        small_aozora_payload(),
        small_aozora_payload(),
        small_aozora_payload(),
        small_aozora_payload(),
    )
        .prop_map(|(a, b, c, d)| {
            // Sanitise pipe and newline chars from cell payloads — they
            // would otherwise terminate the cell or row at the GFM
            // table syntax level. The test is about Aozora-in-table,
            // not about GFM table escape rules.
            let scrub = |s: String| s.replace('|', "／").replace('\n', " ");
            format!(
                "| col-a | col-b |\n| --- | --- |\n| {} | {} |\n| {} | {} |\n\n",
                scrub(a),
                scrub(b),
                scrub(c),
                scrub(d),
            )
        })
}

/// A GFM list with Aozora content inside top-level items and one
/// nested item. Drives the splice path through `<ul>` / `<li>` /
/// nested `<ul>` boundaries.
fn gfm_list_with_aozora_items() -> impl Strategy<Value = String> {
    (
        small_aozora_payload(),
        small_aozora_payload(),
        small_aozora_payload(),
    )
        .prop_map(|(a, b, c)| {
            // Newlines in list-item payloads would terminate the item
            // — strip them. List items don't get pipe-injected so we
            // leave `|` alone.
            let scrub = |s: String| s.replace('\n', " ");
            format!(
                "- top-level: {}\n  - nested: {}\n- another: {}\n\n",
                scrub(a),
                scrub(b),
                scrub(c),
            )
        })
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn ruby_inside_strikethrough_paragraph_is_well_formed() {
    assert_mix_invariants("Visit https://example.com between ~~｜青梅《おうめ》~~ and end.\n\n");
}

#[test]
fn aozora_in_gfm_table_cells_is_well_formed() {
    assert_mix_invariants(
        "| col-a | col-b |\n\
         | --- | --- |\n\
         | ｜青梅《おうめ》 | 漢字《かんじ》 |\n\
         | ABC | text［＃改ページ］ |\n\n",
    );
}

#[test]
fn aozora_in_gfm_list_items_is_well_formed() {
    assert_mix_invariants(
        "- top: ｜青梅《おうめ》\n\
           - nested: 漢字《かんじ》\n\
         - another: text［＃改ページ］\n\n",
    );
}

proptest! {
    #![proptest_config(default_config())]

    /// Aozora notation embedded in a GFM strikethrough / autolink
    /// paragraph must satisfy every always-on tier predicate plus
    /// Tier A and B on well-formed input.
    #[test]
    fn gfm_aozora_paragraph_is_well_formed(src in gfm_aozora_paragraph()) {
        assert_mix_invariants(&src);
    }

    /// Aozora notation inside GFM table cells must thread cleanly
    /// through `<table>` / `<tr>` / `<td>` boundaries.
    #[test]
    fn gfm_table_with_aozora_cells_is_well_formed(src in gfm_table_with_aozora_cells()) {
        assert_mix_invariants(&src);
    }

    /// Aozora notation inside GFM list items (including a nested item)
    /// must thread cleanly through `<ul>` / `<li>` boundaries.
    #[test]
    fn gfm_list_with_aozora_items_is_well_formed(src in gfm_list_with_aozora_items()) {
        assert_mix_invariants(&src);
    }
}
