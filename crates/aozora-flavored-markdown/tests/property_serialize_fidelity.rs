//! Source-fidelity properties for `aozora_flavored_markdown::serialize`.
//!
//! The serializer had exactly one property: I3, the fixed point
//! (`serialize(serialize(x)) == serialize(x)`), asserted by the
//! `serialize_round_trip` fuzz target and by `tests/fuzz_regressions.rs`.
//! I3 relates the output to *itself*, never to the input, so a rewrite that
//! is consistently wrong satisfies it — which is how a `serialize` that never
//! called the code-block mask canonicalised `｜青梅《おうめ》` to
//! `青梅《おうめ》` inside a fence without any gate noticing.
//!
//! I5 is the missing half: what the input said inside a fence, the output
//! must still say. Two shapes assert it, because each covers the other's
//! blind spot:
//!
//! * `fenced_*` build the fence themselves, so the interior is known to the
//!   test and the assertion cannot be skipped by a carve-out.
//! * `mixed_*` hand whole documents — including the shared
//!   `commonmark_adversarial` pool, whose atoms have carried a fenced
//!   `｜青梅《おうめ》` all along without any property ever serializing one —
//!   to [`check_fence_fidelity`], the same predicate the fuzz target runs.

use aozora_flavored_markdown::serialize;
use aozora_flavored_markdown_test_support::check_fence_fidelity;
use aozora_flavored_markdown_test_support::config::default_config;
use aozora_flavored_markdown_test_support::generators::{aozora_fragment, commonmark_adversarial};
use proptest::prelude::*;

/// I3 and I5 together, the pair the fuzz target asserts.
fn assert_serialize_invariants(src: &str) {
    let first = serialize(src);
    let second = serialize(&first);
    assert_eq!(
        first, second,
        "I3 fixed-point broken for src={src:?}\n  first  = {first:?}\n  second = {second:?}"
    );
    check_fence_fidelity(src, &first)
        .unwrap_or_else(|e| panic!("I5 (fence fidelity) violated for src={src:?}: {e}"));
}

/// Drop what `check_fence_fidelity` carves out — CRLF and runs of three or
/// more newlines — so a draw exercises the predicate instead of skipping it.
fn without_carve_outs(src: &str) -> String {
    let mut out = src.replace('\r', "");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

/// Notation the canonicaliser demonstrably rewrites when it is *not* masked:
/// a ruby loses its explicit base marker, a block construct gains blank lines
/// around it, and an indent's full-width digit is normalised to ASCII — code
/// silently corrupted, not merely markup. One is planted in every payload, so
/// the property below cannot pass by drawing inert text.
fn canonicalising_notation() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("｜青梅《おうめ》".to_owned()),
        Just("｜漢字《かんじ》".to_owned()),
        Just("［＃改ページ］".to_owned()),
        Just("［＃ここから２字下げ］".to_owned()),
    ]
}

/// A payload that can sit inside a fence without changing its shape: no fence
/// marker to close it early, no blank line (it would collapse against the
/// opening fence's own newline), and no decorative rule row, which the
/// canonicaliser separates with a blank line wherever it appears. The last two
/// are line structure, which no character mask reaches — see the carve-outs in
/// [`check_fence_fidelity`].
fn fence_payload() -> impl Strategy<Value = String> {
    (canonicalising_notation(), aozora_fragment(8)).prop_map(|(planted, drawn)| {
        let mut out = format!("{planted}\n{drawn}").replace(['\r', '`', '~', '-', '=', '_'], "");
        while out.contains("\n\n") {
            out = out.replace("\n\n", "\n");
        }
        out
    })
}

/// One draw of surrounding prose. Both grammars, because the bug only shows
/// where the two meet: notation the lexer *must* rewrite outside the fence and
/// must not rewrite inside it, in one document.
fn prose() -> impl Strategy<Value = String> {
    prop_oneof![aozora_fragment(6), commonmark_adversarial()]
}

/// A document with one fence whose interior the test knows exactly.
fn fenced_document() -> impl Strategy<Value = (String, String)> {
    (prose(), fence_payload(), prose()).prop_map(|(before, payload, after)| {
        let doc = format!("{before}\n\n```\n{payload}\n```\n\n{after}\n");
        (without_carve_outs(&doc), payload)
    })
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn fenced_ruby_is_not_canonicalised() {
    // The acceptance case: the ruby's explicit base marker is dropped
    // everywhere else, and must survive here.
    let src = "```\n｜青梅《おうめ》\n```";
    assert_eq!(serialize(src), src);
    assert_serialize_invariants(src);
}

#[test]
fn the_same_notation_is_rewritten_outside_the_fence_and_kept_inside() {
    let src = "｜青梅《おうめ》\n\n```\n｜青梅《おうめ》\n```\n";
    assert_eq!(
        serialize(src),
        "青梅《おうめ》\n\n```\n｜青梅《おうめ》\n```\n"
    );
    assert_serialize_invariants(src);
}

#[test]
fn every_shape_of_canonicalisation_stops_at_the_fence() {
    // One case per rewrite an unmasked lexer applies to a fence body, since
    // the dropped ruby marker of the report is only the visible one: a block
    // construct also gets separated by blank lines, and a full-width digit
    // normalised to ASCII — a silent corruption of code, not just of markup.
    for src in [
        "```\n｜青梅《おうめ》\n```\n",
        "```\n［＃改ページ］\n```\n",
        "```\n［＃ここから２字下げ］\n```\n",
    ] {
        assert_eq!(serialize(src), src, "fence body rewritten: {src:?}");
    }
}

#[test]
fn tilde_and_wide_fences_are_masked_too() {
    for src in [
        "~~~\n｜青梅《おうめ》\n~~~\n",
        "````\n｜青梅《おうめ》\n````\n",
        "```rust\n// ｜青梅《おうめ》\n```\n",
        "  ```\n  ｜青梅《おうめ》\n  ```\n",
    ] {
        assert_eq!(serialize(src), src, "fence not masked: {src:?}");
        assert_serialize_invariants(src);
    }
}

#[test]
fn a_fence_bearing_document_from_the_shared_pool_holds() {
    // The atom that has been in `commonmark_adversarial` since the pool was
    // written, never once handed to `serialize`.
    assert_serialize_invariants("```\n｜青梅《おうめ》\n［＃改ページ］\n```\n");
}

proptest! {
    #![proptest_config(default_config())]

    /// The interior the generator put in comes back out byte for byte. Known
    /// to the test rather than rediscovered by a scanner, so no carve-out can
    /// quietly turn this property into a no-op.
    #[test]
    fn a_fenced_payload_survives_verbatim((src, payload) in fenced_document()) {
        let out = serialize(&src);
        prop_assert!(
            out.contains(&payload),
            "fenced payload {payload:?} lost from src={src:?}\n  out = {out:?}",
        );
    }

    /// …and the surrounding document still satisfies I3 and I5.
    #[test]
    fn a_fenced_document_satisfies_both_serialize_invariants((src, _) in fenced_document()) {
        assert_serialize_invariants(&src);
    }

    /// Whole documents from the shared pools, scanned for fences the way the
    /// fuzz target scans arbitrary bytes.
    #[test]
    fn mixed_documents_satisfy_both_serialize_invariants(src in prose()) {
        assert_serialize_invariants(&without_carve_outs(&src));
    }
}
