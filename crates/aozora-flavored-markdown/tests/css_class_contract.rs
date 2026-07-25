//! Class-contract test between renderer and themes.
//!
//! This crate ships two CSS themes (`theme/aozora-md-horizontal.css` and
//! `theme/aozora-md-vertical.css`, published as `theme::HORIZONTAL_CSS` /
//! `theme::VERTICAL_CSS` behind the `theme` feature) whose class selectors
//! must cover every class token the renderer can emit — and cover nothing
//! else. Without this contract a renderer change silently ships unstyled
//! markup, or leaves a rule behind for a class that has been renamed away.
//!
//! `AOZORA_MD_CLASSES` is derived from the parser's own `AOZORA_CLASSES`
//! (ADR-0011), so the contract cannot drift from what the renderer emits;
//! the themes are what can drift, in either direction, and the two sweeps
//! below pin both.

use core::str;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown::{AOZORA_MD_CLASSES, is_contract_class};
use aozora_flavored_markdown_test_support::check_css_class_contract;

/// Classes the themes define that the renderer never emits: the host-page
/// opt-in root the themes scope every other rule under. Kept as an explicit
/// list so a stale selector cannot hide behind it.
const THEME_ONLY_CLASSES: &[&str] = &["aozora-md-root"];

/// The two shipped themes, by file name.
const THEMES: &[&str] = &["aozora-md-horizontal.css", "aozora-md-vertical.css"];

/// Absolute path to one of this crate's `theme/` CSS files. Resolving via
/// `CARGO_MANIFEST_DIR` keeps the test stable regardless of the runner's
/// working directory, and reads the same bytes the `theme` feature embeds.
fn theme_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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

/// The `.aozora-md-…` selectors one shipped theme defines.
fn theme_selectors(name: &str) -> HashSet<String> {
    let css = fs::read_to_string(theme_path(name))
        .unwrap_or_else(|_| panic!("{name} must exist under this crate's theme/"));
    collect_class_selectors(&css)
}

/// Whether a selector a theme defines is one it is allowed to define: an
/// emitted class exactly, a numeric modifier of an open-ended family
/// (`aozora-md-indent-2` for the `aozora-md-indent` stem), or the themes'
/// own opt-in root.
fn is_theme_selector(class: &str) -> bool {
    if is_contract_class(class) || THEME_ONLY_CLASSES.contains(&class) {
        return true;
    }
    match class.rsplit_once('-') {
        Some((stem, suffix)) => {
            !suffix.is_empty()
                && suffix.bytes().all(|b| b.is_ascii_digit())
                && is_contract_class(stem)
        }
        None => false,
    }
}

#[test]
fn every_class_has_a_horizontal_theme_rule() {
    let selectors = theme_selectors("aozora-md-horizontal.css");
    let missing: Vec<&str> = AOZORA_MD_CLASSES
        .iter()
        .map(String::as_str)
        .filter(|c| !selectors.contains(*c))
        .collect();
    assert!(
        missing.is_empty(),
        "aozora-md-horizontal.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn every_class_has_a_vertical_theme_rule() {
    let selectors = theme_selectors("aozora-md-vertical.css");
    let missing: Vec<&str> = AOZORA_MD_CLASSES
        .iter()
        .map(String::as_str)
        .filter(|c| !selectors.contains(*c))
        .collect();
    assert!(
        missing.is_empty(),
        "aozora-md-vertical.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn the_themes_style_nothing_outside_the_contract() {
    // The other half of the drift gate: a class the parser drops or renames
    // leaves its rule behind, styling markup that is never emitted again.
    // Without this sweep only *added* classes are caught.
    for theme in THEMES {
        let stale: Vec<String> = theme_selectors(theme)
            .into_iter()
            .filter(|class| !is_theme_selector(class))
            .collect();
        assert!(
            stale.is_empty(),
            "{theme} styles classes the renderer cannot emit — drop them: {stale:?}"
        );
    }
}

/// Both themes define the same class set, so a host can swap one for the
/// other without touching its markup.
#[test]
fn the_two_themes_define_the_same_classes() {
    let horizontal = theme_selectors("aozora-md-horizontal.css");
    let vertical = theme_selectors("aozora-md-vertical.css");
    let mut only_horizontal: Vec<&str> = horizontal
        .difference(&vertical)
        .map(String::as_str)
        .collect();
    let mut only_vertical: Vec<&str> = vertical
        .difference(&horizontal)
        .map(String::as_str)
        .collect();
    only_horizontal.sort_unstable();
    only_vertical.sort_unstable();
    assert!(
        only_horizontal.is_empty() && only_vertical.is_empty(),
        "the themes disagree on their class set\n  horizontal only: {only_horizontal:?}\n  vertical only:   {only_vertical:?}"
    );
}

/// The embedded constants and the editable files are the same bytes — the
/// `theme` feature ships what a contributor edits, not a stale copy.
#[cfg(feature = "theme")]
#[test]
fn the_embedded_themes_are_the_theme_files() {
    use aozora_flavored_markdown::theme;

    for (name, embedded) in [
        ("aozora-md-horizontal.css", theme::HORIZONTAL_CSS),
        ("aozora-md-vertical.css", theme::VERTICAL_CSS),
    ] {
        let on_disk = fs::read_to_string(theme_path(name)).expect("theme file must exist");
        assert_eq!(on_disk, embedded, "{name} differs from its embedded const");
    }
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
fn contract_membership_is_exact() {
    // The public predicate answers exact spelling only: the numeric
    // variants of an open-ended family are recognised by their stem, which
    // is what `is_theme_selector` above (and the Tier G predicate) rely on.
    assert!(is_contract_class("aozora-md-indent"));
    assert!(!is_contract_class("aozora-md-indent-2"));
    assert!(!is_contract_class("aozora-indent"));
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
// The theme tests above prove the emitted classes ⊆ CSS ⊆ the contract. The
// test below closes the other half of the loop: every aozora-md-* class a
// known construct emits is recognised by the contract — including the classes
// this crate authors itself (an orphan bracket's `aozora-md-directive`
// wrapper) and the family-suffix variants the parser composes at render
// time (`aozora-md-indent-2`), neither of which appears in the derived
// list verbatim.
// Sources are copied verbatim from existing passing tests / the sibling
// renderer's own tests at the pinned version — none authored from memory.
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
    ("section break (kaicho)", "前\n\n［＃改丁］\n\n後"),
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
