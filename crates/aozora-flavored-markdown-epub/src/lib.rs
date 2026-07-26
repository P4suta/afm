//! Convert Aozora Flavored Markdown sources into an [EPUB 3.3] package.
//!
//! [`build`] is the only entry point; it runs the `discover` → `render` →
//! `compose` → `package` phases, each of which cites the spec section it
//! implements, and reports what the renderer saw in every chapter.
//!
//! [EPUB 3.3]: https://www.w3.org/TR/epub-33/

#![doc(html_logo_url = "https://github.com/P4suta/aozora-flavored-markdown-epub")]
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

mod compose;
mod discover;
mod error;
mod package;
mod render;

// A chapter's diagnostics are the renderer's, so they are re-exported rather
// than copied into a shadow type: a host reads one vocabulary whether it
// renders the HTML itself or asks for an EPUB.
pub use aozora_flavored_markdown::{Diagnostic, DiagnosticSource, Severity, Span};
pub use error::{Cause, Error, Result};

/// Inputs for one [`build`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BuildOptions<'a> {
    /// Directory or single file containing Aozora Flavored Markdown sources.
    pub input: &'a Path,
    /// Path to `book.toml` metadata.
    pub metadata: &'a Path,
    /// Output `.epub` path.
    pub output: &'a Path,
}

impl<'a> BuildOptions<'a> {
    /// All three paths are required, so this is the only way to build one.
    #[must_use]
    pub const fn new(input: &'a Path, metadata: &'a Path, output: &'a Path) -> Self {
        Self {
            input,
            metadata,
            output,
        }
    }
}

/// What one chapter's render observed, and the text those spans index into.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChapterReport {
    /// Source file as discovered, before decoding.
    pub path: PathBuf,
    /// Decoded chapter text. A [`Span`] indexes into this, not into the
    /// bytes on disk, which may have been Shift_JIS.
    pub text: String,
    /// Never empty — a clean chapter contributes no report at all.
    pub diagnostics: Vec<Diagnostic>,
}

/// What one [`build`] observed, beyond the file it wrote.
///
/// Rendering is infallible — a diagnostic is an observation, not a refusal —
/// so this can be non-empty for an EPUB that was written in full. Whether a
/// diagnostic fails the run is the host's call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildReport {
    /// One entry per chapter that raised at least one diagnostic, in spine
    /// order.
    pub chapters: Vec<ChapterReport>,
}

impl BuildReport {
    /// True when every chapter rendered clean.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty()
    }

    /// Summed across every chapter.
    #[must_use]
    pub fn diagnostic_count(&self) -> usize {
        self.chapters.iter().map(|c| c.diagnostics.len()).sum()
    }
}

/// # Errors
///
/// Any failing phase. Errors carry source spans where applicable.
pub fn build(opts: &BuildOptions<'_>) -> Result<BuildReport> {
    let manuscript = discover::collect(opts)?;
    let rendered = render::render_all(&manuscript)?;
    let bundle = compose::compose(&manuscript, &rendered)?;
    package::write(opts.output, &bundle)?;
    Ok(BuildReport {
        chapters: rendered.chapters,
    })
}
