//! Native (non-wasm) smoke tests for the aozora-flavored-markdown-wasm crate.
//!
//! `cargo test -p aozora-flavored-markdown-wasm` builds the crate as a regular `rlib`
//! (the `[lib].crate-type` includes `rlib` for exactly this reason)
//! so we can validate the underlying logic without spinning up a
//! browser / Node WASM runtime.
//!
//! These tests do NOT exercise the wasm-bindgen marshalling path —
//! that's covered by aozora-flavored-markdown-obsidian's `from-wasm.test.ts` against a
//! built `.wasm` artefact.
//!
//! What they do exercise is the claim this crate is left making once its own
//! size guard and its own shadow of `Options` are gone: that every export is
//! a *forwarder*. The envelopes are `Serialize`-only wrappers around the
//! library's own output, so "forwards it" is checkable field for field
//! against a direct library call — and it has to be, because nothing else
//! looks. `just wasm-build` only has to produce a file, `just
//! playground-build` type-checks the playground against whatever `.d.ts` came
//! out, and the library's own test suite has never heard of this crate.
//!
//! The `cfg` below is the title line, stated where it binds. `just test-wasm`
//! builds this crate's test targets for wasm32 as well, and a plain `#[test]`
//! there is a function wasm-bindgen's runner does not collect — silently, so
//! the file would read as passing while running nothing. The wasm half of this
//! crate's suite is `tests/wasm.rs`.

#![cfg(not(target_arch = "wasm32"))]

use aozora_flavored_markdown::{
    Options, RenderedBlocks, render_blocks as render_blocks_core, render_to_ir,
};
use aozora_flavored_markdown_wasm::{
    hash_source, render, render_aozora_only, render_blocks as render_blocks_wasm,
};
use serde_json::{Value, to_value};

#[test]
fn hash_source_is_deterministic() {
    assert_eq!(hash_source("hello"), hash_source("hello"));
}

#[test]
fn hash_source_differs_for_different_inputs() {
    assert_ne!(hash_source("hello"), hash_source("world"));
}

#[test]
fn hash_source_is_nonzero_for_typical_input() {
    assert_ne!(hash_source(""), hash_source("｜漢字《かんじ》"));
}

// ---------------------------------------------------------------------------
// the exports are forwarders
// ---------------------------------------------------------------------------

/// One document that every knob changes the rendering of — ruby, a soft
/// break, ASCII quotes, emphasis across CJK punctuation, a table, a
/// strikethrough, a bare URL, a task item — with an orphaned 《 closer on the
/// end so the render also carries a diagnostic.
///
/// Sensitivity is the point, and `the_option_corpus_renders_a_different_document_for_each_member`
/// holds it: an equality against the library is worth nothing on a source the
/// options do not move, and "the options do not move it" is exactly how an
/// export that dropped them would look.
const KNOB_SENSITIVE: &str = concat!(
    "｜青梅《おうめ》\n続き\n\n",
    "これは**「強調」**です \"quoted\" ~~x~~ https://example.com/\n\n",
    "| a | b |\n| - | - |\n| 1 | 2 |\n\n",
    "- [ ] todo\n\n",
    "orphan》close\n",
);

/// Configurations that differ from one another in what they render, reached
/// through both doors — three constructors, two single-knob builders, and one
/// object of the kind a browser host actually sends.
///
/// Deliberately not a knob-by-knob table: that the nine knobs each survive
/// the wire is the library's own contract, held in
/// `tests/options_surface_contract.rs`, and the reason this crate no longer
/// needs its own copy is the change under test. It used to translate JS into
/// a two-field shadow struct by hand, which is where seven knobs went
/// missing; it now hands the library the whole `Options` value it was given,
/// and a whole value has no knobs to drop.
fn option_corpus() -> Vec<(&'static str, Options)> {
    vec![
        ("default()", Options::default()),
        ("commonmark()", Options::commonmark()),
        ("gfm()", Options::gfm()),
        (
            "default().with_smart_punctuation(true)",
            Options::default().with_smart_punctuation(true),
        ),
        (
            "default().with_source_line_anchors(true)",
            Options::default().with_source_line_anchors(true),
        ),
        (
            r#"{"aozora": false, "tables": false}"#,
            serde_json::from_str(r#"{"aozora": false, "tables": false}"#)
                .expect("the wire form must decode"),
        ),
    ]
}

fn json_of<T: serde::Serialize>(value: &T) -> Value {
    to_value(value).expect("the library's own types serialise")
}

#[test]
fn the_option_corpus_renders_a_different_document_for_each_member() {
    let rendered: Vec<(&str, String)> = option_corpus()
        .into_iter()
        .map(|(label, opts)| (label, render_to_ir(KNOB_SENSITIVE, &opts).html))
        .collect();
    for (i, (left_label, left)) in rendered.iter().enumerate() {
        for (right_label, right) in &rendered[i + 1..] {
            assert_ne!(
                left, right,
                "{left_label} and {right_label} render the same document, so an export that \
                 ignored its `options` argument would satisfy the forwarding rules below"
            );
        }
    }
}

#[test]
fn render_forwards_what_the_library_returned_field_for_field() {
    let mut saw_a_diagnostic = false;
    for (label, opts) in option_corpus() {
        let direct = render_to_ir(KNOB_SENSITIVE, &opts);
        let bridged = json_of(&render(KNOB_SENSITIVE, Some(opts)));
        assert_eq!(
            bridged["ir"],
            json_of(&direct.ir),
            "ir differs under {label}"
        );
        assert_eq!(
            bridged["html"],
            json_of(&direct.html),
            "html differs under {label}"
        );
        // The diagnostic channel is the whole reason this crate's own size
        // guard could go: an oversize source degrades in the library to an
        // empty render carrying `source_too_large`, and this crate is what
        // has to carry that verdict out to JS rather than answering with a
        // bail of its own. Pinned on a diagnostic a test can provoke,
        // because the oversize one cannot be: the budget is `u32::MAX`
        // bytes, which on `wasm32` is `usize::MAX` — so on the target that
        // ships, no `&str` can exceed it and the branch is unreachable by
        // arithmetic. What is checkable is that whatever the library
        // reports arrives unaltered, which is the same forwarding path.
        assert_eq!(
            bridged["diagnostics"],
            json_of(&direct.diagnostics),
            "diagnostics differ under {label}"
        );
        saw_a_diagnostic |= !direct.diagnostics.is_empty();
    }
    // Only the configurations that run the notation pass have anything to
    // report — `aozora(false)` skips the lexer, so its diagnostics are empty
    // by construction and forwarding them proves nothing. One member has to
    // carry a real one or the equality above is satisfied by two empty
    // vectors.
    assert!(
        saw_a_diagnostic,
        "no member of the corpus produced a diagnostic, so the forwarding above is vacuous"
    );
}

#[test]
fn render_blocks_forwards_every_block_and_the_documents_diagnostics() {
    for (label, opts) in option_corpus() {
        let RenderedBlocks {
            blocks,
            diagnostics,
            ..
        } = render_blocks_core(KNOB_SENSITIVE, &opts);
        let bridged = json_of(&render_blocks_wasm(KNOB_SENSITIVE, Some(opts)));
        assert_eq!(
            bridged["diagnostics"],
            json_of(&diagnostics),
            "diagnostics differ under {label}"
        );
        let bridged_blocks = bridged["blocks"]
            .as_array()
            .expect("the envelope carries an array of blocks");
        assert_eq!(
            bridged_blocks.len(),
            blocks.len(),
            "block count differs under {label}"
        );
        for (index, (got, want)) in bridged_blocks.iter().zip(&blocks).enumerate() {
            assert_eq!(
                got["ir"],
                json_of(&want.ir),
                "block {index} ir under {label}"
            );
            assert_eq!(
                got["html"],
                json_of(&want.html),
                "block {index} html under {label}"
            );
            // `sourceLine`, not `source_line`: the field is renamed on the
            // way out and a host reads the renamed one.
            assert_eq!(
                got["sourceLine"],
                json_of(&want.source_line),
                "block {index} source line under {label}"
            );
        }
    }
}

#[test]
fn render_aozora_only_is_render_with_no_options_at_all() {
    // The wrapper's whole contract, and a claim the signature cannot make:
    // it takes no options, so the only thing that could go wrong is it
    // choosing a different default than `render` does.
    assert_eq!(
        json_of(&render_aozora_only(KNOB_SENSITIVE)),
        json_of(&render(KNOB_SENSITIVE, None)),
        "the aozora-only wrapper must be `render` with the default options"
    );
    assert_eq!(
        json_of(&render(KNOB_SENSITIVE, None)),
        json_of(&render(KNOB_SENSITIVE, Some(Options::default()))),
        "omitting `options` must be `Options::default()`, which is what the `.d.ts` promises a \
         host that leaves the argument out"
    );
}
