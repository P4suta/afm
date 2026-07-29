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
//! ## Why the knob table can not quietly go stale
//!
//! The bitmask is a hand-written listing of the public surface, and this
//! crate sits outside the workspace, so no test over there compiles it and
//! `options_surface_contract.rs` cannot be imported. Both files therefore
//! read the same source of truth instead of each other: that file's
//! `public_options_api()` scans `src/lib.rs` for the `pub fn`s of
//! `impl Options`, and the `const` assertions below scan the same text,
//! `include_str!`'d, for the same declarations. A knob added to `src/lib.rs`
//! and not to [`KNOBS`] fails this crate's COMPILE, which is `just
//! fuzz-build`, which is a `[group('gate')]` recipe — so it fails a pull
//! request rather than the next person to run a fuzzer.
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

/// The library source both this target and `options_surface_contract.rs`
/// enumerate the `Options` surface from.
///
/// Read at compile time, so the assertions over it are compile errors. The
/// path reaches out of this crate's directory on purpose: the fuzz crate is
/// its own workspace and has no other way to see the library it fuzzes as
/// text. It is `publish = false`, so nothing packages this file.
const LIB_RS: &str = include_str!("../../src/lib.rs");

/// One public `with_*` knob: the name it is declared under, and the setter.
struct Knob {
    /// The `pub fn` name in `impl Options`, checked against [`LIB_RS`].
    name: &'static str,
    /// The setter itself, so the bit is wired to the method rather than to a
    /// field this crate cannot see.
    set: fn(Options, bool) -> Options,
}

/// Every public knob, one bit each, lowest bit first.
const KNOBS: &[Knob] = &[
    Knob {
        name: "with_aozora",
        set: |o, on| o.with_aozora(on),
    },
    Knob {
        name: "with_hardbreaks",
        set: |o, on| o.with_hardbreaks(on),
    },
    Knob {
        name: "with_smart_punctuation",
        set: |o, on| o.with_smart_punctuation(on),
    },
    Knob {
        name: "with_cjk_friendly_emphasis",
        set: |o, on| o.with_cjk_friendly_emphasis(on),
    },
    Knob {
        name: "with_source_line_anchors",
        set: |o, on| o.with_source_line_anchors(on),
    },
    Knob {
        name: "with_tables",
        set: |o, on| o.with_tables(on),
    },
    Knob {
        name: "with_strikethrough",
        set: |o, on| o.with_strikethrough(on),
    },
    Knob {
        name: "with_autolinks",
        set: |o, on| o.with_autolinks(on),
    },
    Knob {
        name: "with_task_lists",
        set: |o, on| o.with_task_lists(on),
    },
];

/// One public constructor: the name it is declared under, and the call.
struct Constructor {
    /// The `pub fn` name in `impl Options`, checked against [`LIB_RS`].
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

/// Whether `needle` sits at `at` in `haystack`.
const fn matches_at(haystack: &[u8], at: usize, needle: &[u8]) -> bool {
    if at + needle.len() > haystack.len() {
        return false;
    }
    let mut i = 0;
    while i < needle.len() {
        if haystack[at + i] != needle[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The `pub fn` declarations at `impl` indentation in `src`, as
/// `(total, of which `with_*`)`.
///
/// Every one of them belongs to `impl Options`: the free functions of that
/// module sit at column zero, and it is the same shape
/// `options_surface_contract.rs::public_options_api` reads with a `String`
/// scanner it can afford.
///
/// One pass, and the shape of it is load-bearing rather than tidy. Two of
/// these (one per needle) tripped `long_running_const_eval` on a 46 KB file —
/// const evaluation is budgeted, and a scan that runs out of budget is a
/// build that fails for a reason unrelated to what it was checking.
const fn public_options_methods(src: &[u8]) -> (usize, usize) {
    const DECL: &[u8] = b"\n    pub fn ";
    let mut total = 0;
    let mut knobs = 0;
    let mut at = 0;
    while at < src.len() {
        // The cheap test first: the inner comparison then runs at the ~1,200
        // line starts rather than at all 46,000 byte offsets.
        if src[at] == b'\n' && matches_at(src, at, DECL) {
            total += 1;
            if matches_at(src, at + DECL.len(), b"with_") {
                knobs += 1;
            }
        }
        at += 1;
    }
    (total, knobs)
}

/// What `src/lib.rs` declares, measured once at compile time.
const DECLARED: (usize, usize) = public_options_methods(LIB_RS.as_bytes());

// A knob ADDED to `src/lib.rs` and not given a bit here. That is the whole
// failure mode: the target would go on reporting a full sweep of a space
// missing an axis.
//
// The other two directions need no scan, because the tables are not strings.
// `KNOBS` holds calls to `o.with_aozora(on)` and `CONSTRUCTORS` holds
// `Options::commonmark` itself, so a knob renamed or removed upstream is a
// name that fails to resolve — this crate stops compiling before any
// assertion here is reached.
const _: () = assert!(
    DECLARED.1 == KNOBS.len(),
    "`src/lib.rs` declares a different number of public `with_*` knobs than KNOBS lists. A knob \
     added there needs a bit here, or this target sweeps a space that is missing one axis while \
     reporting full coverage of it."
);

const _: () = assert!(
    DECLARED.0 == KNOBS.len() + CONSTRUCTORS.len(),
    "`impl Options` declares a public method that is neither a `with_*` knob nor one of the \
     constructors CONSTRUCTORS lists. Either it is a new base configuration this target should \
     sweep from, or `public_options_methods` is reading something it was never meant to."
);

/// Which bit of the mask turns the aozora dialect on.
///
/// The one knob whose value changes which PIPELINE runs rather than which
/// flag comrak is handed, so the assertions below have to read it back.
const AOZORA_BIT: usize = 0;

const _: () = assert!(
    matches_at(KNOBS[AOZORA_BIT].name.as_bytes(), 0, b"with_aozora"),
    "AOZORA_BIT no longer indexes `with_aozora`, so the dialect carve-out below is reading some \
     other knob's bit and the tiers it gates are asserted against the wrong configuration."
);

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

    // Tier B reads a reserved codepoint in the output as one the lexer
    // substituted and never resolved. That argument belongs to the lexer:
    // with the dialect off there is no lexer pass and no substitution, so a
    // reserved codepoint the AUTHOR typed comes back as the author's own
    // byte — which is what `canonicalize`'s I8 states positively over on
    // `canonicalize_round_trip`. This target found that within a minute of first
    // running, which is the finding it was built for and not a defect: the
    // tier's precondition is the dialect, and no sweep had ever asked it
    // with the dialect off.
    if mask & (1u16 << AOZORA_BIT) == 0 && src.chars().any(|c| sentinels::ALL.contains(&c)) {
        return;
    }

    let ctor = &CONSTRUCTORS[(mask as usize >> KNOBS.len()) % CONSTRUCTORS.len()];
    let mut options = (ctor.build)();
    for (bit, knob) in KNOBS.iter().enumerate() {
        options = (knob.set)(options, mask & (1u16 << bit) != 0);
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
