//! Fuzz target — `aozora_flavored_markdown::render` on arbitrary UTF-8.
//!
//! Arbitrary bytes are decoded as UTF-8 (invalid sequences skip this
//! iteration). The resulting source is pushed through
//! `render` and every always-on invariant predicate is
//! asserted via [`assert_html_invariants`]. A crash artifact's
//! Debug-formatted panic message is therefore self-contained: tier
//! label + source + html excerpt + violation details — no manual
//! triage needed.
//!
//! The same call answers a second question. `diagnose` is what the CLI's
//! `check` sub-command asks now that it no longer renders, and its whole
//! contract is that it returns what this render would have returned — a
//! claim about two code paths over every source, which is a fuzzer's
//! question and not an example's. `render` rather than `to_html` for that
//! reason: `to_html` is `render(…).html` with the diagnostics dropped, so
//! nothing is lost and the other half of the result becomes reachable.
//!
//! Run with:
//! - `just fuzz-quick parse_render` (60 s) — inner-loop smoke
//! - `just fuzz-deep  parse_render` (5 min) — release pre-flight
//! - `just fuzz-triage parse_render`         — replay every artifact
//! - `just fuzz-promote parse_render <hash>` — lift to permanent
//!   regression set under `tests/fuzz_regressions/`

#![no_main]

use aozora_flavored_markdown::{Options, diagnose, render};
use aozora_flavored_markdown_test_support::assert_html_invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let rendered = render(src, &Options::default());
    assert_html_invariants(src, &rendered.html);
    assert_eq!(
        diagnose(src, &Options::default()),
        rendered.diagnostics,
        "`diagnose` and `render` disagree about {src:?}"
    );
});
