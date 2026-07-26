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
//!
//! Run with: `just fuzz serialize_round_trip -- -runs=10000`

#![no_main]

use aozora_flavored_markdown::serialize;
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
});
