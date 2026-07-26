//! Property test — "must-never-be" invariants for rendered HTML shape.
//!
//! Runs every tier-A/B/D/E/G/H/I/J/K/L predicate from
//! [`aozora_flavored_markdown_test_support`] against adversarial random input drawn
//! from three stratified generators:
//!
//! * [`aozora_fragment`] — balanced and unbalanced mixes of Aozora
//!   triggers, plus long `-`/`=`/`_` decorative rule rows for Tier H.
//! * [`pathological_aozora`] — deliberately malformed shapes that
//!   stress-test the parser's error path.
//! * A combined strategy — `prop_oneof![aozora_fragment, commonmark_adversarial]`
//!   for "Aozora × CommonMark interaction" coverage.
//!
//! Tier F (XSS prevention) lives in its own file because its payload
//! strategy is disjoint from shape-level generators. Tier C (heading
//! integrity) likewise has a dedicated file with heading-biased input.
//!
//! # What this property does *not* promise
//!
//! * Tier A only applies when the bracket pairing is well-formed (per
//!   [`aozora_flavored_markdown_test_support::check_no_bare_bracket`]'s
//!   own documented contract). Malformed inputs may legitimately leave
//!   bare `［＃` because the fallback classifier does not wrap them. For
//!   those inputs we only assert the predicate does not panic.
//!
//! Tier B is *not* on that list. A source may not contribute a sentinel
//! of its own — a PUA codepoint an author types is replaced with U+FFFD
//! before substitution — so a leak is a bug whatever diagnostics the
//! input raised, and the check runs on every draw, malformed ones
//! included.

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown::{Options, render as render_to_diagnostics, render_blocks_to_ir};
use aozora_flavored_markdown_test_support::config;
use aozora_flavored_markdown_test_support::generators::{
    aozora_fragment, commonmark_adversarial, pathological_aozora,
};
use aozora_flavored_markdown_test_support::{
    assert_html_invariants, check_no_bare_bracket, check_no_sentinel_leak,
};
use proptest::prelude::*;

/// Whether the render raised any diagnostic for `src`. Gates the Tier A
/// assertion so malformed-input boundary behaviour does not sabotage an
/// otherwise-valid property.
fn parse_is_well_formed(src: &str) -> bool {
    render_to_diagnostics(src, &Options::default())
        .diagnostics
        .is_empty()
}

/// Assert every always-on shape predicate, Tier B among them. Tier A is
/// asserted by the caller because it has an input precondition documented
/// above. Tier I is gated on
/// [`aozora_flavored_markdown_test_support::source_contains_html_entity_literal`]
/// inside [`assert_html_invariants`].
fn assert_always_on(html: &str, src: &str) {
    assert_html_invariants(src, html);
}

/// The per-block path's share of the same invariants, mirroring the
/// `render_blocks` fuzz target. Tier B is asserted per chunk, because a leak
/// into one chunk is what the reader sees; the rest is asserted on the
/// concatenation, since a paired container legitimately opens in one block
/// and closes in another and only the joined output owes tag balance.
fn assert_always_on_per_block(src: &str) {
    let (blocks, _) = render_blocks_to_ir(src, &Options::default());
    let mut joined = String::new();
    for block in &blocks {
        check_no_sentinel_leak(src, &block.html).unwrap_or_else(|e| {
            panic!(
                "Tier B (PUA sentinel leak) violated in one block for src={src:?}: {e:?}\n  \
                 block html = {:?}",
                block.html
            )
        });
        joined.push_str(&block.html);
    }
    assert_html_invariants(src, &joined);
}

/// Assert Tier A, which holds only where the bracket pairing is well-formed.
fn assert_gated(html: &str, src: &str) {
    if !parse_is_well_formed(src) {
        return;
    }
    check_no_bare_bracket(html)
        .unwrap_or_else(|e| panic!("Tier A (bare ［＃ leak) violated for src={src:?}: {e}"));
}

proptest! {
    #![proptest_config(config::default())]

    /// Mixed Aozora fragments: the workhorse shape. Covers long
    /// decorative rules (Tier H bait), unbalanced brackets, and a
    /// broad mix of trigger glyphs and plain text.
    #[test]
    fn html_shape_invariants_hold_for_aozora_fragments(src in aozora_fragment(16)) {
        let html = render_to_string(&src);
        assert_always_on(&html, &src);
        assert_gated(&html, &src);
    }

    /// Pathological shapes: deep bracket stacking, paired-container
    /// opens without closes, ruby permutations the classifier must
    /// reject gracefully. These routinely emit diagnostics, so Tier A is
    /// skipped here — the interesting property is that the always-on
    /// invariants, Tier B's "no sentinel survived the recovery path"
    /// among them, hold regardless of how malformed the input is.
    #[test]
    fn html_shape_invariants_hold_for_pathological_aozora(src in pathological_aozora(6)) {
        let html = render_to_string(&src);
        assert_always_on(&html, &src);
    }

    /// Aozora × CommonMark interaction: the two grammars collide (a
    /// heading's body carrying annotation markers, a blockquote
    /// containing ruby, a list containing page breaks). Shape
    /// invariants hold across all such mixes.
    #[test]
    fn html_shape_invariants_hold_for_mixed_cm_aozora(
        src in prop_oneof![aozora_fragment(12), commonmark_adversarial()]
    ) {
        let html = render_to_string(&src);
        assert_always_on(&html, &src);
        assert_gated(&html, &src);
    }

    /// The chunked path the wasm `renderBlocks` export drives owes the same
    /// shape guarantees. It carries state the document path does not — the
    /// code-block mask is restored a block at a time — so the same source can
    /// be clean in one shape and leaking in the other.
    #[test]
    fn html_shape_invariants_hold_per_block(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        assert_always_on_per_block(&src);
    }
}
