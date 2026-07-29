//! The `aozora-md-*` CSS class contract.
//!
//! This crate rewrites the parser's `aozora-*` classes to `aozora-md-*`
//! (ADR-0011), so the two lists are the same list under two brands: [`all`]
//! is derived from the parser's published `AOZORA_CLASSES` rather than
//! written out. A hand-kept copy drifts silently at every dependency bump;
//! deriving removes the failure mode instead of re-checking for it
//! (ADR-0020).

use std::sync::LazyLock;

/// What every class in the contract starts with.
pub const PREFIX: &str = "aozora-md-";

/// The parser's brand, which [`PREFIX`] replaces. Shared with
/// [`crate::fragment`], which performs the rewrite: a second copy there
/// could change on its own and leave the emitted classes disagreeing with
/// the contract derived here.
pub(crate) const BRAND: &str = "aozora-";

// Interned once and handed out as plain `&'static str`, so the laziness is
// this module's business rather than a `LazyLock` in the caller's signature.
static CLASSES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    aozora::AOZORA_CLASSES
        .iter()
        // The rewrite is a prefix swap, so a class the parser publishes
        // without its brand would reach the HTML unchanged; the default
        // carries it here the same way rather than inventing a prefix.
        .map(|class| {
            class
                .strip_prefix(BRAND)
                .map_or(*class, |stem| format!("{PREFIX}{stem}").leak())
        })
        .collect()
});

/// Every `aozora-md-*` CSS class the HTML renderer can emit.
///
/// The `theme` stylesheets style every entry, which the class-contract tests
/// hold them to.
///
/// ```
/// use aozora_flavored_markdown::classes;
///
/// assert!(classes::all().iter().all(|c| c.starts_with(classes::PREFIX)));
/// assert!(classes::all().contains(&"aozora-md-bouten"));
/// ```
#[must_use]
pub fn all() -> &'static [&'static str] {
    &CLASSES
}

/// Whether the renderer can emit `class`.
///
/// An open-ended family is carried in [`all`] by its stem alone, so a
/// numeric variant of one answers `true` without being listed.
///
/// ```
/// use aozora_flavored_markdown::classes::is_known;
///
/// assert!(is_known("aozora-md-indent"));
/// assert!(is_known("aozora-md-indent-2"));
/// assert!(!is_known("language-rust"));
/// ```
#[must_use]
pub fn is_known(class: &str) -> bool {
    if listed(class) {
        return true;
    }
    match class.rsplit_once('-') {
        Some((stem, suffix)) => {
            !suffix.is_empty()
                && suffix.bytes().all(|b| b.is_ascii_digit())
                && NUMERIC_FAMILIES.contains(&stem)
        }
        None => false,
    }
}

// The sibling renderer composes a numeric suffix only for indentation and
// end-alignment amounts. A listed fixed class such as `bouten-goma` does not
// become an open family merely because its name can be followed by digits.
const NUMERIC_FAMILIES: &[&str] = &[
    "aozora-md-indent",
    "aozora-md-align-end",
    "aozora-md-container-indent",
];

fn listed(class: &str) -> bool {
    all().contains(&class)
}
