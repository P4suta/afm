//! Fuzz target — `aozora_flavored_markdown::render_blocks` on arbitrary UTF-8.
//!
//! The chunked path the wasm `renderBlocks` export drives. It carries state
//! the whole-document path does not — the code-block mask is restored with a
//! cursor that walks block by block — so the same source can be clean in one
//! shape and leaking in the other.
//!
//! Tier B is asserted per block, because a leak into one chunk is what the
//! reader sees; the remaining tiers are asserted on the concatenation, since
//! a paired container legitimately opens in one block and closes in another
//! and only the joined output owes tag balance.
//!
//! Run with:
//! - `just fuzz-quick render_blocks` (60 s) — inner-loop smoke
//! - `just fuzz-deep  render_blocks` (5 min) — release pre-flight
//! - `just fuzz-triage render_blocks`         — replay every artifact
//! - `just fuzz-promote render_blocks <hash>` — lift to permanent
//!   regression set under `tests/fuzz_regressions/`

#![no_main]

use aozora_flavored_markdown::{Options, RenderedBlocks, render_blocks};
use aozora_flavored_markdown_test_support::{assert_html_invariants, check_no_sentinel_leak};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let RenderedBlocks { blocks, .. } = render_blocks(src, &Options::default());
    let mut joined = String::new();
    for block in &blocks {
        if let Err(e) = check_no_sentinel_leak(src, &block.html) {
            panic!(
                "Tier B (PUA sentinel leak) violated in one block:\n  src = {src:?}\n  \
                 block html = {:?}\n  details = {e:?}",
                block.html
            );
        }
        joined.push_str(&block.html);
    }
    assert_html_invariants(src, &joined);
});
