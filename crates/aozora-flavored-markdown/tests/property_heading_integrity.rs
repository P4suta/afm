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

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown_test_support::check_heading_integrity;
use aozora_flavored_markdown_test_support::config;
use aozora_flavored_markdown_test_support::generators::kanji_fragment;
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

/// Compose a heading-biased source: `[decorator][target][hint][trailing]`.
/// The generator picks the decorator and heading kind independently so
/// every combination of shape × kind gets exercised.
fn heading_biased_src() -> impl Strategy<Value = String> {
    (
        indent_atom(),
        kanji_fragment(5),
        heading_kind(),
        kanji_fragment(5),
    )
        .prop_map(|(deco, target, kind, trailing)| {
            format!("{deco}{target}［＃「{target}」は{kind}見出し］\n\n{trailing}")
        })
}

proptest! {
    #![proptest_config(config::default())]

    /// For every heading-biased input, the rendered `<h1>`/`<h2>`/`<h3>`
    /// must carry only the target text — no `aozora-md-indent` /
    /// `aozora-md-container-indent` / `aozora-md-directive` class should appear
    /// inside the heading body.
    #[test]
    fn heading_body_never_carries_forbidden_classes(src in heading_biased_src()) {
        let html = render_to_string(&src);
        check_heading_integrity(&html)
            .unwrap_or_else(|e| panic!("Tier C violated for src={src:?}, html={html}: {e}"));
    }
}
