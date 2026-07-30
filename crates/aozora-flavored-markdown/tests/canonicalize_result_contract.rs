//! What the `Result` on `canonicalize` buys, read from a consumer crate.
//!
//! The entry point it replaced returned a bare `String` and spelled every
//! failure `""` — the value an empty document canonicalises to. A source one
//! byte past the parser's `u32` span budget and a document with nothing in it
//! were therefore *one answer*. No caller could
//! branch on them, and no gate could see the difference either: the fuzz
//! target's fixed point held vacuously, `""` being a fixed point of anything
//! that answers `""`.
//!
//! An integration test is where this has to be stated. `CanonicalizeError` is
//! `#[non_exhaustive]`, so only from outside the defining crate does a
//! `match` on it carry the wildcard arm the attribute forces on consumers,
//! and only from outside is the signature the one crates.io publishes.
//!
//! The guard itself — that a source past the budget is refused at all, and
//! that the refusal names the length — is exercised where a budget can be
//! made small enough to reach, in the library's own
//! `a_source_past_the_budget_is_refused_and_named_by_its_length`.

use std::collections::HashSet;
use std::error::Error as StdError;

use aozora_flavored_markdown::{
    CanonicalizeError, Options, Rendered, RenderedBlocks, RenderedIr, canonicalize, render,
    render_blocks, render_to_ir, to_html,
};

/// One byte past the budget on a 64-bit target: the length a refusal reports.
const OVER_BUDGET: usize = u32::MAX as usize + 1;

/// The branch a host writes over the answer. `Unrecognised` is not padding:
/// `#[non_exhaustive]` *requires* a consumer to write it, and a variant added
/// later must land there rather than break the build.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Empty,
    Canonical(String),
    Refused { len: usize },
    Unrecognised,
}

fn classify(answer: Result<String, CanonicalizeError>) -> Answer {
    match answer {
        Ok(out) if out.is_empty() => Answer::Empty,
        Ok(out) => Answer::Canonical(out),
        Err(CanonicalizeError::SourceTooLarge { len }) => Answer::Refused { len },
        Err(_) => Answer::Unrecognised,
    }
}

#[test]
fn success_and_refusal_are_values_a_caller_can_tell_apart() {
    // Empty input is the success side of the split, by decision: nothing to
    // canonicalise is not a failure to canonicalise.
    assert_eq!(canonicalize(""), Ok(String::new()));
    assert_eq!(classify(canonicalize("")), Answer::Empty);
    assert_eq!(
        classify(canonicalize("彼は｜青梅《おうめ》に行った。")),
        Answer::Canonical("彼は青梅《おうめ》に行った。".to_owned())
    );
    // The refusal, through the same match. Handed in rather than provoked:
    // `SourceTooLarge` needs a 4 GiB source.
    assert_eq!(
        classify(Err(CanonicalizeError::SourceTooLarge { len: OVER_BUDGET })),
        Answer::Refused { len: OVER_BUDGET }
    );
    assert_ne!(
        canonicalize(""),
        Err(CanonicalizeError::SourceTooLarge { len: 0 })
    );
}

#[test]
fn every_leading_bom_is_preserved_and_is_not_an_empty_document() {
    for src in ["\u{feff}", "\u{feff}\u{feff}", "\u{feff}\u{feff}body"] {
        assert_eq!(
            canonicalize(src),
            Ok(src.to_owned()),
            "leading BOM run changed for {src:?}"
        );
    }
    assert_ne!(classify(canonicalize("\u{feff}")), Answer::Empty);
}

#[test]
fn a_failure_propagates_as_the_error_trait_a_host_already_holds() {
    // The shape a binding or a CLI writes. `?` into `Box<dyn Error>` needs
    // `Error: std::error::Error + 'static`, so this line is what pins the
    // trait impl a `String` return could not have had.
    fn round_trip(src: &str) -> Result<String, Box<dyn StdError>> {
        Ok(canonicalize(src)?)
    }
    assert_eq!(
        round_trip("｜青梅《おうめ》").expect("an in-budget source canonicalises"),
        "青梅《おうめ》"
    );

    // A refusal a host logs must say which length it refused, or the operator
    // is back to guessing what an empty answer meant.
    let boxed: Box<dyn StdError> = Box::new(CanonicalizeError::SourceTooLarge { len: OVER_BUDGET });
    let message = boxed.to_string();
    assert!(
        message.contains(&OVER_BUDGET.to_string()),
        "a refusal must report the length: {message}"
    );
}

#[test]
fn an_error_is_a_value_a_host_can_copy_compare_and_key_on() {
    let refused = CanonicalizeError::SourceTooLarge { len: OVER_BUDGET };
    // `Copy`, so `refused` is still live below — a host holding one in a
    // per-document cache entry does not have to clone it.
    let copied = refused;
    assert_eq!(refused, copied);
    assert_ne!(
        refused,
        CanonicalizeError::SourceTooLarge {
            len: OVER_BUDGET + 1
        },
        "the length is part of the value, not decoration"
    );
    let mut seen = HashSet::new();
    assert!(seen.insert(refused));
    assert!(!seen.insert(copied), "equal errors must hash equal");
}

/// Sources the lexer has something to say about. Any one of them producing a
/// diagnostic is enough; the claim below is about the pool.
const MALFORMED: &[&str] = &["｜青梅《", "［＃", "［＃ここから字下げ］", "《》"];

#[test]
fn the_rendering_entry_points_gained_no_result_of_their_own() {
    // The other half of the decision, and the half a well-meaning follow-up
    // would undo. CommonMark is a total grammar, so what the lexer saw comes
    // back as diagnostics *beside* a rendered document rather than instead of
    // one — a rustc warning's standing, not an error's. Every binding below
    // is annotated with the type it expects, so each stops compiling the day
    // its entry point starts returning a `Result`.
    let mut seen = 0usize;
    for src in MALFORMED {
        let rendered: Rendered = render(src, &Options::default());
        let ir: RenderedIr = render_to_ir(src, &Options::default());
        let streamed: RenderedBlocks = render_blocks(src, &Options::default());
        let direct: String = to_html(src);
        assert!(
            !rendered.html.is_empty()
                && !ir.html.is_empty()
                && !streamed.blocks.is_empty()
                && !direct.is_empty(),
            "malformed notation must still render a document: {src:?}"
        );
        seen += rendered.diagnostics.len() + ir.diagnostics.len() + streamed.diagnostics.len();
    }
    assert!(
        seen > 0,
        "no malformed sample produced a diagnostic; the pool is stale and the \
         claim that rendering reports rather than fails is untested"
    );
}
