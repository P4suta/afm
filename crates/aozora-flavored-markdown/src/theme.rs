//! The canonical `aozora-md-*` stylesheets, embedded from the crate's
//! `theme/` directory.
//!
//! The crate that emits the classes owns their CSS (ADR-0020), so a
//! downstream packager embeds these constants instead of vendoring a copy it
//! must track by hand. Both stylesheets define the same class set — writing
//! mode is the only difference — and apply under the `aozora-md-root` opt-in
//! class, so a host swaps one for the other without touching its markup.
//!
//! Coverage is tested against the parser this crate was released with, and
//! that dependency is a caret range: a semver-compatible parser release that
//! adds a class reaches the rendered HTML before these files have a rule for
//! it, and that class renders unstyled until this crate bumps. Host CSS
//! covers the gap in the meantime.

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
