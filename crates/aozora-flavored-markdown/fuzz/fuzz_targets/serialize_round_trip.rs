//! Fuzz target — `aozora_flavored_markdown::serialize` on arbitrary UTF-8.
//!
//! Two invariants, because the first alone proved insufficient:
//!
//! * **I3, fixed point.** `serialize(serialize(src))` must be byte-identical
//!   to `serialize(src)`: the lex pipeline canonicalises the source on the
//!   first pass; oscillation on the second would mean the classifier and the
//!   serializer disagree on the canonical form.
//! * **I5, fence fidelity.** Every fenced code interior of `src` must reappear
//!   in the output byte for byte. I3 cannot see this class of bug at all — a
//!   *consistently* wrong rewrite is still a fixed point — which is how a
//!   `serialize` that skipped the code-block mask fuzzed clean for a release
//!   cycle while canonicalising fence bodies as prose.
//! * **I8, reserved-codepoint fidelity.** Every codepoint the crate reserves
//!   appears in the output exactly as often as in the source. Four of the five
//!   are overwritten with `U+FFFD` by the sibling parser on sight, so an
//!   unprotected one is not markup moved but the author's byte destroyed —
//!   and both I3 and I5 are blind to it: the rewrite is consistent, and it
//!   happens outside any fence. Every input carrying one is UTF-8 the fuzzer
//!   reaches directly, which is what makes this target the right home for it.
//!
//! Run with: `just fuzz serialize_round_trip -- -runs=10000`

#![no_main]

use aozora_flavored_markdown::{sentinels, serialize};
use aozora_flavored_markdown_test_support::check_fence_fidelity;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let first = serialize(src);
    let second = serialize(&first);
    assert_eq!(
        first, second,
        "I3 fixed-point broken for src={src:?}: first vs second differ"
    );
    if let Err(e) = check_fence_fidelity(src, &first) {
        panic!("I5 (fence fidelity) violated:\n  src = {src:?}\n  details = {e:?}");
    }
    // Read off the crate's own reserved set, so a codepoint added there is
    // covered here without editing this.
    for reserved in sentinels::ALL {
        let before = src.matches(reserved).count();
        let after = first.matches(reserved).count();
        assert_eq!(
            before, after,
            "I8 (reserved codepoint fidelity) violated for U+{:04X}:\n  src = {src:?}\n  out = {first:?}",
            reserved as u32
        );
    }
});
