//! Class-contract test between renderer and themes.
//!
//! This crate ships two CSS themes (`theme/aozora-md-horizontal.css` and
//! `theme/aozora-md-vertical.css`, published as `theme::HORIZONTAL_CSS` /
//! `theme::VERTICAL_CSS` behind the `theme` feature) whose class selectors
//! must cover every class token the renderer can emit — and cover nothing
//! else. Without this contract a renderer change silently ships unstyled
//! markup, or leaves a rule behind for a class that has been renamed away.
//!
//! `classes::all()` is derived from the parser's own `AOZORA_CLASSES`
//! (ADR-0011), so the contract cannot drift from what the renderer emits;
//! the themes are what can drift, in either direction, and the two sweeps
//! below pin both.

use core::{ptr, str};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use aozora_flavored_markdown::classes;
use aozora_flavored_markdown::to_html;
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

/// Whether a selector a theme defines is one it is allowed to define: a
/// class the renderer can emit — `is_known` already carries the numeric
/// modifiers of an open-ended family — or the themes' own opt-in root.
fn is_theme_selector(class: &str) -> bool {
    classes::is_known(class) || THEME_ONLY_CLASSES.contains(&class)
}

#[test]
fn every_class_has_a_horizontal_theme_rule() {
    let selectors = theme_selectors("aozora-md-horizontal.css");
    let missing: Vec<&str> = classes::all()
        .iter()
        .copied()
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
    let missing: Vec<&str> = classes::all()
        .iter()
        .copied()
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
    assert_eq!(classes::all(), expected);
    let mut seen: HashSet<&str> = HashSet::new();
    for class in classes::all() {
        assert!(seen.insert(*class), "duplicate entry: {class}");
    }
}

#[test]
fn the_class_list_is_interned_once_and_handed_out_by_reference() {
    // `all()` replaced a `pub static … : LazyLock<Vec<String>>`, and the
    // interning it hides is the reason a consumer no longer reads a laziness
    // wrapper out of the signature. `&'static [&'static str]` is also exactly
    // the return type an implementation that leaked a fresh `String` per call
    // would have — the leak would be invisible to every other test in this
    // file, and to rustdoc. Pointer identity across two calls is what says
    // the interning happened once.
    let (first, second) = (classes::all(), classes::all());
    assert!(
        ptr::eq(first, second),
        "all() handed back a different slice on the second call, so it is re-interning \
         (and leaking) per call rather than once"
    );
    assert!(
        first.iter().zip(second).all(|(a, b)| ptr::eq(*a, *b)),
        "the entries are re-leaked per call even though the slice is not"
    );
}

#[test]
fn contract_membership_covers_the_numeric_family() {
    // The listed stem, a numeric variant of it that the list does not carry
    // verbatim, and the parser's own brand — which this crate never emits.
    assert!(classes::is_known("aozora-md-indent"));
    assert!(classes::is_known("aozora-md-indent-2"));
    assert!(!classes::is_known("aozora-indent"));
}

#[test]
fn membership_is_the_listed_class_plus_a_numeric_suffix_and_nothing_else() {
    // Quantified over the whole list rather than over the two or three names
    // somebody remembered, because the rule is the parser's and applies to
    // every entry: `AOZORA_CLASSES` carries each slug family member verbatim
    // (`aozora-bouten-goma`) and collapses only the open-ended *numeric*
    // variants to their stem. So a suffix means a number, or it means the
    // token is not one the renderer can emit.
    for &class in classes::all() {
        assert!(classes::is_known(class), "a listed class must be known");
        for n in ["0", "1", "2", "10", "07", "4294967296"] {
            let numeric = format!("{class}-{n}");
            assert!(
                classes::is_known(&numeric),
                "the numeric family member {numeric} must be known"
            );
        }
        // `zzq` is not a slug the parser publishes, so `<listed>-zzq` is a
        // token no renderer emits. Accepting it was the slack that let the
        // checkers paper over a predicate rejecting `aozora-md-indent-2`.
        let slug = format!("{class}-zzq");
        assert!(
            !classes::is_known(&slug),
            "{slug} is neither listed nor numeric, so it must not be known"
        );
        // Two numeric segments never compose: the parser writes one amount.
        let doubled = format!("{class}-2-3");
        assert!(!classes::is_known(&doubled), "{doubled} must not be known");
    }
    // Shapes with nothing to split on, or a split that yields an empty half.
    for class in ["", "indent", "aozora", "-", "-2", "aozora-md-indent-"] {
        assert!(!classes::is_known(class), "{class:?} must not be known");
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
// Render-direction contract: construct → aozora-md-* class → classes::all()
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
    // The numeric family, reached from source rather than asserted on a
    // hand-written token: this one renders `aozora-md-indent-2`, the class
    // the contract predicate used to answer `false` for.
    ("indent leaf (numeric family)", "［＃２字下げ］見出し"),
    (
        "align-end leaf (numeric family)",
        "本文［＃地から２字上げ］",
    ),
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
    let mut emitted: HashSet<String> = HashSet::new();
    for (label, src) in RENDER_CORPUS {
        let html = to_html(src);
        if let Err(violation) = check_css_class_contract(&html) {
            panic!(
                "corpus item {label:?} emitted an aozora-md-* class \
                 classes::is_known() rejects:\n  {violation}\n  src = {src:?}\n  html = {html}"
            );
        }
        emitted.extend(rendered_class_tokens(&html));
    }
    // Anti-vacuity, and the half of the contract the sweep alone cannot
    // state: the corpus must keep reaching the open-ended numeric family,
    // whose members the derived list does not carry verbatim. Without this
    // the sweep goes quiet the day the notation for `［＃２字下げ］` changes,
    // and the family stops being checked against a real render.
    let numeric: Vec<&String> = emitted.iter().filter(|c| ends_in_a_number(c)).collect();
    assert!(
        !numeric.is_empty(),
        "no corpus item emitted a numeric-family class any more; \
         retarget the corpus rather than deleting the guard. Emitted: {emitted:?}"
    );
    for class in numeric {
        assert!(
            !classes::all().contains(&class.as_str()),
            "{class} is carried verbatim now, so it no longer exercises the family rule"
        );
    }
}

/// Every `class="…"` token in rendered HTML, tag-position-blind because the
/// renderer emits the attribute in one fixed shape and the corpus above is
/// small enough to read.
fn rendered_class_tokens(html: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut rest = html;
    while let Some(at) = rest.find("class=\"") {
        let after = &rest[at + "class=\"".len()..];
        let Some(end) = after.find('"') else { break };
        out.extend(after[..end].split_whitespace().map(ToOwned::to_owned));
        rest = &after[end..];
    }
    out
}

/// A class whose last hyphen-separated segment is a number — the open-ended
/// family the parser publishes by stem only.
fn ends_in_a_number(class: &str) -> bool {
    class
        .rsplit_once('-')
        .is_some_and(|(_, suffix)| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
}
