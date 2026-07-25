//! Class-contract test between renderer and themes.
//!
//! `theme/` ships two CSS themes (`aozora-md-horizontal.css` and
//! `aozora-md-vertical.css`) whose class selectors must cover every
//! class token the renderer can emit. Without this contract a renderer
//! change silently ships unstyled markup.
//!
//! `AOZORA_MD_CLASSES` is derived from the parser's own `AOZORA_CLASSES`
//! (ADR-0011), so the contract cannot drift from what the renderer emits.
//! What can drift is the *themes*: a class the parser grows arrives here
//! with no CSS behind it. `UNSTYLED_CLASSES` names the ones this repo has
//! not styled yet (the ADR-0020 follow-up), and the tests below hold that
//! list to shrinking only.

use core::str;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown_test_support::{
    AOZORA_MD_CLASSES, UNSTYLED_CLASSES, check_css_class_contract, styled_classes,
};

/// Absolute path to one of the repo-root `theme/` CSS files. Resolving
/// via `CARGO_MANIFEST_DIR` keeps the test stable regardless of the
/// runner's working directory.
fn theme_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/aozora-flavored-markdown → crates/
    p.pop(); // crates/ → repo root
    p.push("theme");
    p.push(name);
    p
}

/// Return every `.aozora-md-…` class selector name appearing in `css`.
///
/// A tokeniser that extracts the identifier immediately after each
/// `.aozora-md-` prefix. Accepts lowercase ASCII letters, digits, and
/// hyphens; stops at any other character. Intentionally trivial —
/// the project's CSS doesn't use namespace prefixes or escaped
/// selectors.
fn collect_class_selectors(css: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let bytes = css.as_bytes();
    let mut i = 0usize;
    while i + 11 <= bytes.len() {
        if &bytes[i..i + 11] == b".aozora-md-" {
            let start = i + 1; // after the '.'
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-') {
                end += 1;
            }
            // Trim trailing hyphens from a ".aozora-md-" prefix with no
            // body — shouldn't occur but be tolerant.
            let token = str::from_utf8(&bytes[start..end]).expect("ASCII");
            if token.len() > "aozora-md-".len() && !token.ends_with('-') {
                out.insert(token.to_owned());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// The `.aozora-md-…` selectors both shipped themes define.
fn theme_selectors(name: &str) -> HashSet<String> {
    let css = fs::read_to_string(theme_path(name))
        .unwrap_or_else(|_| panic!("{name} must exist under theme/"));
    collect_class_selectors(&css)
}

#[test]
fn every_styled_class_has_a_horizontal_theme_rule() {
    let selectors = theme_selectors("aozora-md-horizontal.css");
    let missing: Vec<&str> = styled_classes()
        .into_iter()
        .filter(|c| !selectors.contains(*c))
        .collect();
    assert!(
        missing.is_empty(),
        "aozora-md-horizontal.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn every_styled_class_has_a_vertical_theme_rule() {
    let selectors = theme_selectors("aozora-md-vertical.css");
    let missing: Vec<&str> = styled_classes()
        .into_iter()
        .filter(|c| !selectors.contains(*c))
        .collect();
    assert!(
        missing.is_empty(),
        "aozora-md-vertical.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn the_unstyled_backlog_only_shrinks() {
    // An entry that turns out to be styled has to leave the list, or the
    // list stops meaning what it says and the coverage tests above stop
    // covering it.
    let horizontal = theme_selectors("aozora-md-horizontal.css");
    let vertical = theme_selectors("aozora-md-vertical.css");
    let styled: Vec<&str> = UNSTYLED_CLASSES
        .iter()
        .copied()
        .filter(|c| horizontal.contains(*c) && vertical.contains(*c))
        .collect();
    assert!(
        styled.is_empty(),
        "UNSTYLED_CLASSES entries both themes already style — drop them: {styled:?}"
    );
}

#[test]
fn the_unstyled_backlog_names_real_classes() {
    // A stale entry would silently exempt nothing while looking like work
    // that is still owed.
    let contract: HashSet<&str> = AOZORA_MD_CLASSES.iter().map(String::as_str).collect();
    let unknown: Vec<&str> = UNSTYLED_CLASSES
        .iter()
        .copied()
        .filter(|c| !contract.contains(*c))
        .collect();
    assert!(
        unknown.is_empty(),
        "UNSTYLED_CLASSES entries the renderer cannot emit — drop them: {unknown:?}"
    );
}

#[test]
fn the_contract_is_the_parsers_own_list_rebranded() {
    // The derivation itself, pinned: every class the parser publishes has
    // exactly one branded counterpart here, in the same order.
    let expected: Vec<String> = aozora::AOZORA_CLASSES
        .iter()
        .map(|class| class.replacen("aozora-", "aozora-md-", 1))
        .collect();
    assert_eq!(*AOZORA_MD_CLASSES, expected);
    let mut seen: HashSet<&str> = HashSet::new();
    for class in AOZORA_MD_CLASSES.iter() {
        assert!(seen.insert(class), "duplicate entry: {class}");
    }
}

#[test]
fn collect_class_selectors_extracts_basic_rules() {
    // Self-test for the tokeniser — a regression here would
    // silently weaken every other test in this file.
    let css = ".aozora-md-foo { color: red; }\n.aozora-md-bar-baz, .aozora-md-qux { }\n.foo { }";
    let selectors = collect_class_selectors(css);
    assert!(selectors.contains("aozora-md-foo"));
    assert!(selectors.contains("aozora-md-bar-baz"));
    assert!(selectors.contains("aozora-md-qux"));
    assert!(!selectors.contains("foo"));
}

#[test]
fn collect_class_selectors_tolerates_trailing_hyphen() {
    // `.aozora-md-` alone (no body) must not emit a token.
    let css = ".aozora-md- { }";
    let selectors = collect_class_selectors(css);
    assert!(selectors.is_empty());
}

// ---------------------------------------------------------------------------
// Render-direction contract: construct → aozora-md-* class → AOZORA_MD_CLASSES
//
// The theme tests above prove the styled classes ⊆ CSS. The test below
// closes the other half of the loop: every aozora-md-* class a known
// construct emits is recognised by the contract — including the classes
// this crate authors itself (an orphan bracket's `aozora-md-directive`
// wrapper) and the family-suffix variants the parser composes at render
// time (`aozora-md-indent-2`), neither of which appears in the derived
// list verbatim.
// Sources are copied verbatim from existing passing tests / the sibling
// renderer's own tests at the pinned SHA — none authored from memory.
// ---------------------------------------------------------------------------

/// One verified source per class-emitting aozora construct.
const RENDER_CORPUS: &[(&str, &str)] = &[
    ("ruby (explicit)", "｜青梅《おうめ》"),
    ("forward bouten (goma/right)", "対象［＃「対象」に傍点］"),
    ("left bouten", "X［＃「X」の左に傍点］"),
    ("tcy", "20［＃「20」は縦中横］"),
    ("gaiji", "※［＃二の字点、1-2-22］"),
    ("kaeriten", "学［＃二、レ点］而時習之"),
    ("unknown annotation", "前［＃ほげふが］後"),
    ("page break", "前\n\n［＃改ページ］\n\n後"),
    ("section break (choho)", "前\n\n［＃改丁］\n\n後"),
    ("indent leaf", "前［＃地から１字下げ］後"),
    ("align-end leaf", "前［＃地付き］末尾"),
    (
        "indent container",
        "［＃ここから字下げ］\n本文\n［＃ここで字下げ終わり］",
    ),
    (
        "align-end container",
        "［＃ここから地付き］\n後書き\n［＃ここで地付き終わり］",
    ),
    (
        "keigakomi container",
        "［＃罫囲み］\n引用\n［＃罫囲み終わり］",
    ),
    (
        "warichu (inline)",
        "黄色い鑑札（［＃割り注］淫売婦の鑑札［＃割り注終わり］）をもって",
    ),
    ("body end", "本文\n\n［＃本文終わり］\n\n奥付"),
    ("illustration", "［＃挿絵（fig1.png）入る］"),
];

#[test]
fn every_rendered_class_is_recognised() {
    for (label, src) in RENDER_CORPUS {
        let html = render_to_string(src);
        if let Err(violation) = check_css_class_contract(&html) {
            panic!(
                "corpus item {label:?} emitted an aozora-md-* class not in \
                 AOZORA_MD_CLASSES:\n  {violation}\n  src = {src:?}\n  html = {html}"
            );
        }
    }
}
