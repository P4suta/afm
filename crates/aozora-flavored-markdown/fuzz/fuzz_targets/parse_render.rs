//! Fuzz target — `aozora_flavored_markdown::to_html` on arbitrary UTF-8.
//!
//! Arbitrary bytes are decoded as UTF-8 (invalid sequences skip this
//! iteration). The resulting source is pushed through
//! `to_html` and every always-on invariant predicate is
//! asserted via [`assert_html_invariants`]. A crash artifact's
//! Debug-formatted panic message is therefore self-contained: tier
//! label + source + html excerpt + violation details — no manual
//! triage needed.
//!
//! Run with:
//! - `just fuzz-quick parse_render` (60 s) — inner-loop smoke
//! - `just fuzz-deep  parse_render` (5 min) — release pre-flight
//! - `just fuzz-triage parse_render`         — replay every artifact
//! - `just fuzz-promote parse_render <hash>` — lift to permanent
//!   regression set under `tests/fuzz_regressions/`

#![no_main]

use aozora_flavored_markdown::to_html;
use aozora_flavored_markdown_test_support::assert_html_invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let html = to_html(src);
    assert_html_invariants(src, &html);
});
