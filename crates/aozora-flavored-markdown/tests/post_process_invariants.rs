//! End-to-end HTML invariants for `aozora_flavored_markdown::render_to_string`.
//!
//! The splicer (`ast_splice`) replaces the lexer's sentinels in the comrak
//! AST before `comrak::format_html`. The invariants asserted here are
//! expressed against the final rendered HTML:
//!
//! - **Tier-A invariant**: no bare `［＃` ever leaks into rendered HTML
//!   outside an `aozora-md-directive` wrapper, for arbitrary inputs.
//! - **Sentinel consumption**: no PUA sentinel character (U+E001 ..
//!   U+E004) survives in the output for inputs the lexer reports as
//!   well-formed.
//! - **Count invariant**: an input with *N* explicit-delimiter rubies
//!   renders into HTML with at least *N* `<ruby>` tags. *N* unknown
//!   `［＃…］` annotations render into at least *N* `aozora-md-directive`
//!   wrappers.
//! - **Document order**: Aozora-derived markup appears in the output
//!   in the same order as the corresponding constructs appear in
//!   source.
//! - **Determinism**: `render_to_string(x)` produces identical output
//!   on independent invocations.
//! - **No panic on malformed shapes**: proptest-driven random
//!   combinations of Aozora triggers and plain text never crash.
//!
//! The tests here deliberately overlap with `property_html_shape` and
//! the lexer's own validate phase — defence-in-depth is the point.

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown::{Options, render as render_with_diag, sentinels};
use aozora_flavored_markdown_test_support::check_no_bare_bracket;
use aozora_flavored_markdown_test_support::generators::aozora_fragment;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Sentinel consumption — unit-level smoke
// ---------------------------------------------------------------------------

#[test]
fn render_consumes_every_inline_sentinel_on_simple_ruby() {
    let html = render_to_string("｜青梅《おうめ》へ");
    assert!(
        !html.contains(sentinels::INLINE),
        "sentinels::INLINE leaked into HTML: {html:?}"
    );
}

#[test]
fn render_consumes_every_block_sentinel_on_page_break() {
    let html = render_to_string("前\n\n［＃改ページ］\n\n後");
    for s in [
        sentinels::BLOCK_LEAF,
        sentinels::BLOCK_OPEN,
        sentinels::BLOCK_CLOSE,
    ] {
        assert!(!html.contains(s), "block sentinel {s:?} leaked: {html:?}");
    }
}

#[test]
fn render_consumes_sentinels_for_mixed_inline_and_block() {
    let html = render_to_string("｜漢字《かんじ》の話。\n\n［＃改ページ］\n\n［＃ほげ］まとめ");
    for s in [
        sentinels::INLINE,
        sentinels::BLOCK_LEAF,
        sentinels::BLOCK_OPEN,
        sentinels::BLOCK_CLOSE,
    ] {
        assert!(!html.contains(s), "sentinel {s:?} leaked: {html:?}");
    }
}

// ---------------------------------------------------------------------------
// Count invariants
// ---------------------------------------------------------------------------

#[test]
fn explicit_ruby_count_matches_input_count() {
    let html = render_to_string("｜青梅《おうめ》と｜鶴見《つるみ》、｜立川《たちかわ》");
    let ruby_count = html.matches("<ruby>").count();
    assert_eq!(
        ruby_count, 3,
        "3 explicit rubies must yield ≥ 3 <ruby> tags, got {ruby_count}: {html}"
    );
}

#[test]
fn unknown_annotation_count_matches_input_count() {
    let html = render_to_string("［＃ほげ］と［＃ふが］と［＃ぴよ］");
    let annotation_count = html.matches("aozora-md-directive").count();
    assert!(
        annotation_count >= 3,
        "3 unknown ［＃…］ must yield ≥ 3 aozora-md-directive wrappers, got {annotation_count}: {html}"
    );
}

// ---------------------------------------------------------------------------
// Document order
// ---------------------------------------------------------------------------

#[test]
fn aozora_constructs_render_in_source_order() {
    let html = render_to_string("｜一《いち》と｜二《に》と［＃ほげ］と｜三《さん》");
    // Find every "ruby" / "annotation" landmark and confirm the
    // sequence is Ruby, Ruby, Annotation, Ruby.
    let order = order_of_landmarks(&html, &["<ruby>", "aozora-md-directive"]);
    assert_eq!(
        order,
        vec!["<ruby>", "<ruby>", "aozora-md-directive", "<ruby>"],
        "Aozora landmarks must appear in source order; got {order:?}\nhtml: {html}"
    );
}

fn order_of_landmarks(html: &str, needles: &[&'static str]) -> Vec<&'static str> {
    let mut hits: Vec<(usize, &'static str)> = Vec::new();
    for &needle in needles {
        let mut search_from = 0;
        while let Some(rel) = html[search_from..].find(needle) {
            hits.push((search_from + rel, needle));
            search_from += rel + needle.len();
        }
    }
    hits.sort_by_key(|&(pos, _)| pos);
    hits.into_iter().map(|(_, n)| n).collect()
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn render_is_deterministic() {
    let src = "｜青梅《おうめ》前［＃改ページ］後\n［＃ほげ］続き";
    assert_eq!(render_to_string(src), render_to_string(src));
}

// ---------------------------------------------------------------------------
// Tier-A invariant
// ---------------------------------------------------------------------------

fn tier_a_holds(html: &str) -> bool {
    check_no_bare_bracket(html).is_ok()
}

#[test]
fn tier_a_holds_for_every_static_fixture() {
    let fixtures = [
        "plain text no annotations",
        "｜青梅《おうめ》",
        "｜漢字《かんじ》の話",
        "前［＃改ページ］後",
        "［＃改丁］\n",
        "前［＃ほげふが］後",
        "前［＃０字下げ］後",
        "語※［＃「木＋吶のつくり」、第3水準1-85-54］で",
        "前［＃地付き］末尾",
        "冒頭でXが先行する。X［＃「X」に傍点］の強調。",
        "前［＃ここから２字下げ］本文［＃ここで字下げ終わり］後",
    ];
    for src in fixtures {
        let html = render_to_string(src);
        assert!(
            tier_a_holds(&html),
            "Tier-A leaked ［＃ for input {src:?}, html = {html:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

fn lexer_is_well_formed(src: &str) -> bool {
    render_with_diag(src, &Options::default())
        .diagnostics
        .is_empty()
}

proptest! {
    /// Arbitrary combinations of Aozora triggers must:
    ///
    /// 1. Not panic `render_to_string` — the pipeline must be total.
    /// 2. Not leak any sentinel character into the rendered HTML for
    ///    well-formed inputs.
    /// 3. For well-formed inputs (matched brackets), not leak `［＃`
    ///    into rendered HTML (Tier-A canary).
    #[test]
    fn render_survives_arbitrary_aozora_shaped_input(src in aozora_fragment(16)) {
        let html = render_to_string(&src);
        if lexer_is_well_formed(&src) {
            for s in [sentinels::INLINE, sentinels::BLOCK_LEAF, sentinels::BLOCK_OPEN, sentinels::BLOCK_CLOSE] {
                prop_assert!(
                    !html.contains(s),
                    "sentinel {:?} leaked for src {:?}, html {:?}",
                    s, src, html,
                );
            }
            prop_assert!(
                tier_a_holds(&html),
                "Tier-A leaked for src {:?}, html {:?}",
                src, html,
            );
        }
    }

    /// Determinism: two independent invocations produce identical HTML.
    #[test]
    fn render_determinism(src in aozora_fragment(16)) {
        let a = render_to_string(&src);
        let b = render_to_string(&src);
        prop_assert_eq!(a, b);
    }
}

// ---------------------------------------------------------------------------
// Malformed-input boundary conditions
// ---------------------------------------------------------------------------

#[test]
fn malformed_unclosed_bracket_does_not_panic() {
    drop(render_to_string("前［＃"));
    drop(render_to_string("［＃ほげ"));
    let a = render_to_string("［＃");
    let b = render_to_string("［＃");
    assert_eq!(a, b);
}

#[test]
fn malformed_unclosed_ruby_does_not_panic() {
    drop(render_to_string("｜青梅《"));
    drop(render_to_string("《》"));
    drop(render_to_string("｜"));
    drop(render_to_string("※"));
}

#[test]
fn malformed_stray_close_bracket_does_not_panic() {
    drop(render_to_string("stray］text"));
    drop(render_to_string("》trailing"));
}

// ---------------------------------------------------------------------------
// PUA collision
// ---------------------------------------------------------------------------

#[test]
fn source_containing_pua_characters_does_not_panic() {
    let html = render_to_string("before\u{E001}after");
    assert!(!html.is_empty(), "render must produce some output");
    let a = render_to_string("before\u{E001}after");
    let b = render_to_string("before\u{E001}after");
    assert_eq!(a, b);
}
