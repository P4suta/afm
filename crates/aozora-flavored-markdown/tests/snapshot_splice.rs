//! Snapshot the post-splice HTML for a curated set of aozora-flavored-markdown sources.
//!
//! Sibling to the upstream renderer's own golden snapshots — those pin the
//! *standalone* renderer's `aozora-*` output; this one pins what
//! aozora-flavored-markdown actually ships: comrak-wrapped HTML with the
//! Aozora sentinels spliced in and the brand rewrite (`aozora-` →
//! `aozora-md-`, ADR-0011) applied. A renderer or post-splice regression surfaces
//! here as a one-test `cargo insta` diff instead of an opaque
//! `assert_eq!` mismatch buried in the integration suite.
//!
//! Coverage rationale: each case pins one *kind* of splice path in
//! isolation —
//!
//! * `plain` — the no-sentinel floor (comrak passthrough; the splicer
//!   walks the AST and rewrites nothing).
//! * `inline_ruby` — an inline sentinel spliced inside a `<p>` (the
//!   `INLINE_SENTINEL` → `Raw` inline-node path).
//! * `block_page_break` — a sole-block sentinel promoted to a
//!   block-level `<div class="aozora-md-page-break">` between two paragraphs
//!   (the `BLOCK_LEAF` path + brand rewrite).
//! * `unknown_annotation` — an orphan `［＃…］` the lexer never claimed,
//!   wrapped in a hidden `aozora-md-annotation` span (Tier-A canary: no bare
//!   bracket leaks into body text).
//!
//! Each case passes an explicit snapshot name so the on-disk
//! `.snap` files read as `snapshot_splice__<case>.snap` rather than
//! the default doubled `snapshot_splice__snapshot_<case>.snap` insta
//! would derive from the `snapshot_*` test-function names.
//!
//! Regenerate after an intentional renderer change with
//! `cargo insta accept` (or `INSTA_UPDATE=always cargo test`) inside
//! the dev container — `just test` runs nextest, which does not honour
//! insta's update flow, so accept the pending snapshots with cargo-insta
//! directly.

use aozora_flavored_markdown::{Options, render as render_full};

/// Render an aozora-flavored-markdown source through the full aozora-flavored-markdown path (lexer pre-pass +
/// comrak + AST splice), returning just the HTML.
fn render(source: &str) -> String {
    render_full(source, &Options::default()).html
}

#[test]
fn snapshot_plain() {
    insta::assert_snapshot!("plain", render("hello, world"));
}

#[test]
fn snapshot_inline_ruby() {
    insta::assert_snapshot!("inline_ruby", render("｜青梅《おうめ》へ"));
}

#[test]
fn snapshot_block_page_break() {
    insta::assert_snapshot!("block_page_break", render("前［＃改ページ］後"));
}

#[test]
fn snapshot_unknown_annotation() {
    // Tier-A canary in snapshot form: the bracket text survives only
    // inside the hidden `aozora-md-annotation` wrapper, never bare in body.
    insta::assert_snapshot!("unknown_annotation", render("前［＃ほげふが］後"));
}
