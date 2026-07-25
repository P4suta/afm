//! Class-contract test between renderer and themes.
//!
//! `theme/` ships two CSS themes (`aozora-md-horizontal.css` and
//! `aozora-md-vertical.css`) whose class selectors must cover every
//! class token the renderer can emit. Without this contract a renderer
//! change silently ships unstyled markup.
//!
//! The pinned list (`AOZORA_MD_CLASSES`) is hand-curated so that adding
//! a class forces an explicit update here.

use core::str;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use aozora_flavored_markdown::html::render_to_string;
use aozora_flavored_markdown_test_support::{AOZORA_MD_CLASSES, check_css_class_contract};

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

#[test]
fn every_emitted_class_has_a_horizontal_theme_rule() {
    let css = fs::read_to_string(theme_path("aozora-md-horizontal.css"))
        .expect("aozora-md-horizontal.css must exist under theme/");
    let selectors = collect_class_selectors(&css);
    let missing: Vec<&&str> = AOZORA_MD_CLASSES
        .iter()
        .filter(|c| !selectors.contains(**c))
        .collect();
    assert!(
        missing.is_empty(),
        "aozora-md-horizontal.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn every_emitted_class_has_a_vertical_theme_rule() {
    let css = fs::read_to_string(theme_path("aozora-md-vertical.css"))
        .expect("aozora-md-vertical.css must exist under theme/");
    let selectors = collect_class_selectors(&css);
    let missing: Vec<&&str> = AOZORA_MD_CLASSES
        .iter()
        .filter(|c| !selectors.contains(**c))
        .collect();
    assert!(
        missing.is_empty(),
        "aozora-md-vertical.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn pinned_classes_are_sorted_and_unique() {
    // Hygiene: the pinned list is kept in sorted order + no dupes
    // for review friendliness. A sorted list also makes PR diffs
    // trivial to review.
    let mut copy: Vec<&str> = AOZORA_MD_CLASSES.to_vec();
    copy.sort_unstable();
    assert_eq!(
        AOZORA_MD_CLASSES.to_vec(),
        copy,
        "AOZORA_MD_CLASSES must stay sorted"
    );
    let mut seen: HashSet<&str> = HashSet::new();
    for &c in AOZORA_MD_CLASSES {
        assert!(seen.insert(c), "duplicate entry in AOZORA_MD_CLASSES: {c}");
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
// The theme tests above prove AOZORA_MD_CLASSES ⊆ CSS. The two tests below
// close the other half of the loop:
//
//   (1) every aozora-md-* class a known construct emits is recognised by
//       AOZORA_MD_CLASSES — a *new* upstream class surfaces here by name the
//       moment the corpus reaches it (pin bump), instead of shipping silent
//       unstyled markup; and
//   (2) every AOZORA_MD_CLASSES entry is actually emitted by some construct —
//       a *stale* entry (e.g. `aozora-md-double-ruby` after an upstream
//       rename) surfaces here — modulo the documented UNEXERCISED gaps.
//
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
    ("double ruby", "《《強調》》"),
];

/// `AOZORA_MD_CLASSES` entries the curated corpus does not (yet) emit at the
/// current pin, each with a justification. Keep this short — an entry here is
/// a known coverage gap, not license to skip new classes.
const UNEXERCISED: &[&str] = &[
    // 割り注 renders inline (`aozora-md-warichu`); the deprecated block form
    // `［＃ここから割り注］…` is not pinned in this corpus.
    "aozora-md-container-warichu",
    // The leaf indent span is emitted only via a directly-constructed
    // indent node — no source-string trigger is pinned. The
    // *container* form (`aozora-md-container-indent`) is exercised above.
    "aozora-md-indent",
    // Kanbun 返り点: no verified source trigger surfaces it through the render
    // path at the current pin. (Candidate follow-up.)
    "aozora-md-kaeriten",
];

/// Collect every `aozora-md-*` class token appearing in a `class="..."`
/// attribute in `html`. (The corpus emits no `<pre><code>` blocks, so a plain
/// scan is sufficient here.)
fn collect_class_tokens(html: &str, out: &mut HashSet<String>) {
    let mut rest = html;
    while let Some(i) = rest.find("class=\"") {
        let after = &rest[i + "class=\"".len()..];
        let Some(end) = after.find('"') else { break };
        for tok in after[..end].split_whitespace() {
            if tok.starts_with("aozora-md-") {
                out.insert(tok.to_owned());
            }
        }
        rest = &after[end + 1..];
    }
}

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

#[test]
fn every_class_is_exercised_by_the_corpus() {
    let mut emitted = HashSet::new();
    for (_label, src) in RENDER_CORPUS {
        collect_class_tokens(&render_to_string(src), &mut emitted);
    }
    let exercised = |base: &str| {
        emitted
            .iter()
            .any(|t| t == base || t.starts_with(&format!("{base}-")))
    };
    let stale: Vec<&str> = AOZORA_MD_CLASSES
        .iter()
        .copied()
        .filter(|base| !exercised(base) && !UNEXERCISED.contains(base))
        .collect();
    assert!(
        stale.is_empty(),
        "AOZORA_MD_CLASSES entries no construct emits — remove the stale entry, \
         add a corpus source, or document it in UNEXERCISED: {stale:?}"
    );
}
