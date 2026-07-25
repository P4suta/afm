//! Convert Aozora Flavored Markdown sources into an [EPUB 3.3] package.
//!
//! [`build`] is the only entry point; it runs the `discover` → `render` →
//! `compose` → `package` phases, each of which cites the spec section it
//! implements.
//!
//! [EPUB 3.3]: https://www.w3.org/TR/epub-33/

#![doc(html_logo_url = "https://github.com/P4suta/aozora-flavored-markdown-epub")]
#![forbid(unsafe_code)]

use std::path::Path;

mod compose;
mod discover;
mod error;
mod package;
mod render;

pub use error::{Error, Result};

/// Inputs for one [`build`].
#[derive(Debug, Clone)]
pub struct BuildOptions<'a> {
    /// Directory or single file containing Aozora Flavored Markdown sources.
    pub input: &'a Path,
    /// Path to `book.toml` metadata.
    pub metadata: &'a Path,
    /// Output `.epub` path.
    pub output: &'a Path,
}

/// # Errors
///
/// Any failing phase. Errors carry source spans where applicable.
pub fn build(opts: &BuildOptions<'_>) -> Result<()> {
    let manuscript = discover::collect(opts)?;
    let rendered = render::render_all(&manuscript)?;
    let bundle = compose::compose(&manuscript, &rendered)?;
    package::write(opts.output, &bundle)
}
