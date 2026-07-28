//! Property test — Tier C: promoted headings carry only legitimate content.
//!
//! `［＃「X」は大見出し／中見出し／小見出し］` promotes a paragraph
//! into an `<h1>` / `<h2>` / `<h3>`. The contract is that any random
//! composition of indent markers and heading hints must produce
//! headings whose bodies carry only the target text (no `aozora-md-indent`,
//! `aozora-md-container-indent`, or `aozora-md-directive` tokens). A regression
//! test in `tests/heading_promotion.rs` guards a specific shape; this
//! property test generalises.
//!
//! # Generator strategy
//!
//! The strategy builds a heading-biased Aozora fragment by
//! concatenating:
//!
//! 1. An indent / align decorator (`［＃N字下げ］`, `［＃ここから字下げ］`,
//!    `［＃地付き］`) chosen from a short list.
//! 2. A target literal (1–5 kanji codepoints).
//! 3. A heading hint (`［＃「target」は大見出し］` / `…中見出し］` /
//!    `…小見出し］`) referencing the target.
//! 4. Optional trailing body text.
//!
//! The lexer's forward-reference classifier requires the target
//! literal to appear before the hint, which the generator provides by
//! construction — this keeps the proptest exercising the promotion
//! path rather than the "unknown annotation" fallback.
//!
//! The generator over-samples the indent-followed-by-heading shape so
//! the indent-leakage failure mode is exercised heavily.
//!
//! # The second source, and why the first was not enough
//!
//! Every draw renders twice: the fragment above on its own, and the same
//! fragment inside a *markdown* heading. Steps 1–4 compose a heading out of
//! 青空文庫 parts, which is one of the two ways a `<hN>` gets made here and
//! was, for a long time, the only one this file drew. The other — `#`, and a
//! setext underline — reaches the same tier through a different splice, and
//! the omission hid two defects at once; `tests/heading_promotion.rs`'s
//! `a_markdown_heading_body_carries_no_marker_and_no_sentinel` says which.
//! Tier B is asserted alongside Tier C for the same reason: a heading body
//! the splice consumes entirely is where a sentinel survived, and that shape
//! is a markdown heading's alone.

use aozora_flavored_markdown::to_html;
use aozora_flavored_markdown_test_support::config;
use aozora_flavored_markdown_test_support::generators::kanji_fragment;
use aozora_flavored_markdown_test_support::{check_heading_integrity, check_no_sentinel_leak};
use proptest::prelude::*;

/// Generate an indent / alignment decorator as a single atom.
fn indent_atom() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("［＃１字下げ］".to_owned()),
        Just("［＃２字下げ］".to_owned()),
        Just("［＃３字下げ］".to_owned()),
        Just("［＃ここから２字下げ］".to_owned()),
        Just("［＃ここで字下げ終わり］".to_owned()),
        Just("［＃地付き］".to_owned()),
    ]
}

/// Generate a heading-hint suffix (`大`/`中`/`小`) that will wrap the
/// given target.
fn heading_kind() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("大"), Just("中"), Just("小")]
}

/// The markdown heading syntaxes the composed fragment gets wrapped in, a
/// `@` standing for the fragment: ATX at the ends and the middle of the level
/// range, and a setext underline on both sides of the width at which the
/// sibling parser used to read the row as decoration instead. The bare shape
/// is not in this pool because the property below checks it unconditionally —
/// widening the file must not thin out what it already drew.
fn markdown_heading_shape() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("# @"),
        Just("### @"),
        Just("###### @"),
        Just("@\n---"),
        Just("@\n=========="),
    ]
}

/// Compose a heading-biased source twice over: `[decorator][target][hint]`
/// bare — promoted to a heading by the hint, as this file always drew it — and
/// the same fragment inside a markdown heading. Both carry the same trailing
/// body text. The generator picks the decorator, the heading kind and the
/// wrapper independently so every combination of shape × kind × syntax gets
/// exercised.
fn heading_biased_srcs() -> impl Strategy<Value = [String; 2]> {
    (
        indent_atom(),
        kanji_fragment(5),
        heading_kind(),
        kanji_fragment(5),
        markdown_heading_shape(),
    )
        .prop_map(|(deco, target, kind, trailing, shape)| {
            let composed = format!("{deco}{target}［＃「{target}」は{kind}見出し］");
            [
                format!("{composed}\n\n{trailing}"),
                format!("{}\n\n{trailing}", shape.replace('@', &composed)),
            ]
        })
}

proptest! {
    #![proptest_config(config::default())]

    /// For every heading-biased input, promoted and markdown alike, the
    /// rendered `<h1>`/`<h2>`/`<h3>` must carry only the target text — no
    /// `aozora-md-indent` / `aozora-md-container-indent` / `aozora-md-directive`
    /// class should appear inside the heading body — and no PUA sentinel may
    /// survive anywhere in the output.
    #[test]
    fn heading_body_never_carries_forbidden_classes(srcs in heading_biased_srcs()) {
        for src in srcs {
            let html = to_html(&src);
            check_heading_integrity(&html)
                .unwrap_or_else(|e| panic!("Tier C violated for src={src:?}, html={html}: {e}"));
            check_no_sentinel_leak(&src, &html)
                .unwrap_or_else(|e| panic!("Tier B violated for src={src:?}, html={html}: {e}"));
        }
    }
}
