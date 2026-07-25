//! The `aozora-md-*` CSS class contract.
//!
//! This crate rewrites the parser's `aozora-*` classes to `aozora-md-*`
//! (ADR-0011), so the two lists are the same list under two brands:
//! [`AOZORA_MD_CLASSES`] is derived from the parser's published
//! `AOZORA_CLASSES` rather than written out. A hand-kept copy drifts
//! silently at every dependency bump; deriving removes the failure mode
//! instead of re-checking for it (ADR-0020).

use std::sync::LazyLock;

/// The parser's brand (ADR-0011).
///
/// Shared with [`crate::fragment`], which performs the rewrite these two
/// spellings describe: a second copy there could be changed on its own and
/// leave the emitted classes disagreeing with the contract derived here.
pub(crate) const BRAND: &str = "aozora-";
/// This crate's brand, the one [`BRAND`] is rewritten to.
pub(crate) const REBRAND: &str = "aozora-md-";

/// Every `aozora-md-*` CSS class the HTML renderer can emit.
///
/// Open-ended numeric variants appear as their stem — `aozora-md-indent-2`
/// is in the contract as `aozora-md-indent` — so a consumer matching a
/// class token should accept a trailing `-<n>` on a listed stem;
/// [`is_contract_class`] answers the exact-spelling question.
///
/// The canonical stylesheets — `theme::HORIZONTAL_CSS` and
/// `theme::VERTICAL_CSS`, behind the default-off `theme` feature — style
/// every entry, which the class-contract tests hold them to.
///
/// ```
/// use aozora_flavored_markdown::AOZORA_MD_CLASSES;
///
/// assert!(AOZORA_MD_CLASSES.iter().all(|class| class.starts_with("aozora-md-")));
/// assert!(AOZORA_MD_CLASSES.iter().any(|class| class == "aozora-md-bouten"));
/// ```
pub static AOZORA_MD_CLASSES: LazyLock<Vec<String>> = LazyLock::new(|| {
    aozora::AOZORA_CLASSES
        .iter()
        // The rewrite is a prefix swap, so a class the parser publishes
        // without its brand would reach the HTML unchanged; the fallback
        // carries it here the same way rather than inventing a prefix.
        .map(|class| {
            class
                .strip_prefix(BRAND)
                .map_or_else(|| (*class).to_owned(), |stem| format!("{REBRAND}{stem}"))
        })
        .collect()
});

/// Whether `class` is a class the renderer can emit, exactly as spelled.
///
/// Exact membership, so the numeric variants of an open-ended family
/// (`aozora-md-indent-2`) answer `false` — their stem is what the
/// contract carries.
///
/// ```
/// use aozora_flavored_markdown::is_contract_class;
///
/// assert!(is_contract_class("aozora-md-indent"));
/// assert!(!is_contract_class("aozora-md-indent-2"));
/// assert!(!is_contract_class("language-rust"));
/// ```
#[must_use]
pub fn is_contract_class(class: &str) -> bool {
    AOZORA_MD_CLASSES.iter().any(|known| known == class)
}
