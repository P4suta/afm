//! Fuzz target — Shift_JIS decoding + full render pipeline.
//!
//! Arbitrary bytes are fed into `decode_sjis`. Failures (non-SJIS
//! input) skip this iteration. Successful decodes are pushed through
//! the full render pipeline and every always-on invariant predicate
//! is asserted via [`assert_html_invariants`]. Targets encoding-
//! boundary bugs (truncated trail bytes, lead-byte-at-EOF, SJIS-
//! adjacent codepoints that decode to Aozora trigger glyphs after
//! mapping).
//!
//! Run with the same `just fuzz-{quick,deep,triage,promote}` family
//! as `parse_render`.

#![no_main]

use aozora::decode_sjis;
use aozora_flavored_markdown::to_html;
use aozora_flavored_markdown_test_support::assert_html_invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = decode_sjis(data) else {
        return;
    };
    let html = to_html(&text);
    assert_html_invariants(&text, &html);
});
