//! Fuzz target — arbitrary bytes against a *non-default* `Options`.
//!
//! The other four targets render with `Options::default()` and nothing else,
//! so the whole non-default configuration space was reached by no fuzzer at
//! all. `tests/options_surface_contract.rs` sweeps that space with proptest,
//! over a generator pool and a fixed payload corpus; this is the same space
//! over the input class proptest cannot reach — arbitrary bytes, mutated
//! against a committed corpus.
//!
//! The input is its own format: [`MASK_BYTES`] little-endian bytes of option
//! mask, then the source as UTF-8. The low [`KNOBS`]`.len()` bits of the mask
//! set each `with_*` knob and the bits above them choose the base
//! constructor. A new input shape rather than an edit to an existing target,
//! which is what keeps the other four corpora valid — the reason DEV-246 was
//! deferred out of #172 in the first place.
//!
//! Spelled by hand rather than taken through `arbitrary`, so that the bytes
//! of a promoted crash artifact say what configuration found it with no
//! decoder to agree on: `tests/fuzz_regressions.rs` replays one by stripping
//! the same fixed-width prefix, and `just fuzz-triage` replays it by handing
//! the whole file back to this binary.
//!
//! Each row stores an actual method call. Renaming or removing an option
//! therefore fails `just fuzz-build` in the compiler, while the integration
//! suite checks the same typed table against its rendering and JSON effects.
//!
//! Run with:
//! - `just fuzz-quick options_space` (60 s) — inner-loop smoke
//! - `just fuzz-deep  options_space` (5 min) — release pre-flight
//! - `just fuzz-triage options_space`         — replay every artifact
//! - `just fuzz-promote options_space <hash>` — lift to permanent
//!   regression set under `tests/fuzz_regressions/`

#![no_main]

use aozora_flavored_markdown::{
    Options, RenderedBlocks, diagnose, render, render_blocks, render_to_ir, sentinels,
};
use aozora_flavored_markdown_test_support::{assert_html_invariants, check_no_sentinel_leak};
use libfuzzer_sys::fuzz_target;

/// One public `with_*` knob: the name it is declared under, and the setter.
struct Knob {
    /// Human-readable spelling used in crash diagnostics.
    name: &'static str,
    /// The setter itself, so the bit is wired to the method rather than to a
    /// field this crate cannot see.
    set: fn(Options, bool) -> Options,
    /// This setter selects whether the Aozora preprocessing pipeline runs.
    aozora_pipeline: bool,
}

/// Every public knob, one bit each, lowest bit first.
const KNOBS: &[Knob] = &[
    Knob {
        name: "with_aozora",
        set: |o, on| o.with_aozora(on),
        aozora_pipeline: true,
    },
    Knob {
        name: "with_hardbreaks",
        set: |o, on| o.with_hardbreaks(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_smart_punctuation",
        set: |o, on| o.with_smart_punctuation(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_cjk_friendly_emphasis",
        set: |o, on| o.with_cjk_friendly_emphasis(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_source_line_anchors",
        set: |o, on| o.with_source_line_anchors(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_tables",
        set: |o, on| o.with_tables(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_strikethrough",
        set: |o, on| o.with_strikethrough(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_autolinks",
        set: |o, on| o.with_autolinks(on),
        aozora_pipeline: false,
    },
    Knob {
        name: "with_task_lists",
        set: |o, on| o.with_task_lists(on),
        aozora_pipeline: false,
    },
];

/// One public constructor: the name it is declared under, and the call.
struct Constructor {
    /// Human-readable spelling used in crash diagnostics.
    name: &'static str,
    /// The constructor itself.
    build: fn() -> Options,
}

/// Every public constructor. A knob mask is a delta from one of these, and
/// which one matters: `commonmark()` turns the aozora lexer off, which is a
/// different pipeline rather than a different flag.
const CONSTRUCTORS: &[Constructor] = &[
    Constructor {
        name: "new",
        build: Options::new,
    },
    Constructor {
        name: "commonmark",
        build: Options::commonmark,
    },
    Constructor {
        name: "gfm",
        build: Options::gfm,
    },
];

/// How many leading bytes of the input are the option mask. Named because
/// `tests/fuzz_regressions.rs` strips exactly this many off a promoted
/// artifact before replaying it.
const MASK_BYTES: usize = 2;

// The mask has to hold one bit per knob and still leave room to select a
// constructor above them.
const _: () = assert!(
    KNOBS.len() + 2 <= MASK_BYTES * 8,
    "the knob mask has outgrown the prefix the target reads it out of"
);

fuzz_target!(|data: &[u8]| {
    let Some((mask, rest)) = data
        .split_first_chunk::<MASK_BYTES>()
        .map(|(prefix, rest)| (u16::from_le_bytes(*prefix), rest))
    else {
        return;
    };
    let Ok(src) = core::str::from_utf8(rest) else {
        return;
    };

    let ctor = &CONSTRUCTORS[(mask as usize >> KNOBS.len()) % CONSTRUCTORS.len()];
    let mut options = (ctor.build)();
    let mut aozora_pipeline = None;
    for (bit, knob) in KNOBS.iter().enumerate() {
        let on = mask & (1u16 << bit) != 0;
        options = (knob.set)(options, on);
        if knob.aozora_pipeline {
            assert!(
                aozora_pipeline.replace(on).is_none(),
                "the option table marks more than one Aozora pipeline selector"
            );
        }
    }
    let aozora_pipeline =
        aozora_pipeline.expect("the option table must mark its Aozora pipeline selector");

    // Tier B reads a reserved codepoint in the output as one the lexer
    // substituted and never resolved. That argument belongs to the lexer:
    // with the dialect off there is no lexer pass and no substitution, so a
    // reserved codepoint the AUTHOR typed comes back as the author's own
    // byte — which is what `canonicalize`'s I8 states positively over on
    // `canonicalize_round_trip`. This target found that within a minute of first
    // running, which is the finding it was built for and not a defect: the
    // tier's precondition is the dialect, and no sweep had ever asked it
    // with the dialect off.
    if !aozora_pipeline && src.chars().any(|c| sentinels::ALL.contains(&c)) {
        return;
    }
    // Built only when something has already failed: `assert*!` formats its
    // message lazily, and this target runs the format-free path per exec.
    let label = || {
        let mut out = format!("{}()", ctor.name);
        for (bit, knob) in KNOBS.iter().enumerate() {
            out.push_str(&format!(".{}({})", knob.name, mask & (1u16 << bit) != 0));
        }
        out
    };

    let rendered = render(src, &options);
    assert_html_invariants(src, &rendered.html);

    // The same tiers on the other two renderers. They are not redundant: the
    // IR path projects the AST before splicing, and the per-block path
    // restores the code-block mask with a cursor that walks block by block —
    // so a leak can exist in one shape and not the other under the very same
    // options.
    let ir = render_to_ir(src, &options);
    assert_html_invariants(src, &ir.html);

    let RenderedBlocks {
        blocks,
        diagnostics: block_diagnostics,
        ..
    } = render_blocks(src, &options);
    let mut joined = String::new();
    for block in &blocks {
        if let Err(e) = check_no_sentinel_leak(src, &block.html) {
            panic!(
                "Tier B (PUA sentinel leak) violated in one block under {}:\n  src = {src:?}\n  \
                 block html = {:?}\n  details = {e:?}",
                label(),
                block.html
            );
        }
        joined.push_str(&block.html);
    }
    assert_html_invariants(src, &joined);

    // One document, one set of diagnostics — `options_surface_contract.rs`'s
    // rule, quantified over arbitrary bytes instead of over a fixed corpus.
    // `diagnose` is the entry point that reaches its answer without
    // rendering, and it has its own early return for `with_aozora(false)`,
    // which this mask reaches on half its inputs.
    let expected = diagnose(src, &options);
    for (name, actual) in [
        ("render", &rendered.diagnostics),
        ("render_to_ir", &ir.diagnostics),
        ("render_blocks", &block_diagnostics),
    ] {
        assert_eq!(
            actual,
            &expected,
            "`{name}` and `diagnose` disagree about {src:?} under {}",
            label()
        );
    }
});
