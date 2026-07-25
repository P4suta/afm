//! The canonical `aozora-md-*` stylesheets, as embedded strings.
//!
//! The crate that emits the classes owns their CSS (ADR-0020): a
//! downstream packager — the EPUB generator here, a PDF one later — embeds
//! these constants instead of vendoring a copy that has to track
//! [`AOZORA_MD_CLASSES`](crate::AOZORA_MD_CLASSES) by hand.
//!
//! Pure data behind the default-off `theme` feature, so a parser-only
//! consumer pays nothing for it and the sources stay editable as plain
//! `.css` files under the crate's `theme/` directory.
//!
//! Both stylesheets define the same class set — the writing mode is the
//! only difference — so a host page can swap one for the other without
//! touching its markup. Styling applies under the `aozora-md-root` opt-in
//! class, which the host puts on the element wrapping the rendered HTML.
//!
//! The coverage these files owe [`AOZORA_MD_CLASSES`](crate::AOZORA_MD_CLASSES)
//! is tested against the parser this crate was released with, and the
//! dependency on it is a caret range. A semver-compatible parser release that
//! adds a class will reach the rendered HTML before these stylesheets have a
//! rule for it, so that class renders unstyled until this crate bumps
//! (ADR-0020). Host CSS of your own is the way to cover the gap in the
//! meantime.

/// The horizontal (left-to-right) writing-mode theme.
///
/// ```
/// use aozora_flavored_markdown::theme;
///
/// assert!(theme::HORIZONTAL_CSS.contains(".aozora-md-root"));
/// ```
pub const HORIZONTAL_CSS: &str = include_str!("../theme/aozora-md-horizontal.css");

/// The vertical (tategaki, `writing-mode: vertical-rl`) theme.
///
/// ```
/// use aozora_flavored_markdown::theme;
///
/// assert!(theme::VERTICAL_CSS.contains("writing-mode: vertical-rl"));
/// ```
pub const VERTICAL_CSS: &str = include_str!("../theme/aozora-md-vertical.css");
