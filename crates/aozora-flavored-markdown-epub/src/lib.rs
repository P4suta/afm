//! Convert Aozora Flavored Markdown sources into an [EPUB 3.3] package.
//!
//! [`build`] runs the `discover` → `validate` → `render` → `compose` →
//! `package` phases. [`check`] runs the same work without the final write.
//!
//! [EPUB 3.3]: https://www.w3.org/TR/epub-33/

#![doc(html_logo_url = "https://github.com/P4suta/aozora-flavored-markdown-epub")]
#![forbid(unsafe_code)]

// This crate's README carries the `build` call a reader copies first, so it is
// compiled as a doctest by `just test-doc` — the same guard the library crate
// puts on its own quick-start. `#[cfg(doctest)]` keeps the `include_str!` out
// of normal builds and out of `cargo doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

use std::path::{Path, PathBuf};

mod compose;
mod discover;
mod error;
mod package;
mod render;
mod validate;
mod xml;

// A chapter's diagnostics are the renderer's, so they are re-exported rather
// than copied into a shadow type: a host reads one vocabulary whether it
// renders the HTML itself or asks for an EPUB.
pub use aozora_flavored_markdown::{ByteSpan, Diagnostic, DiagnosticSource, Severity};
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

/// Inputs for one non-writing [`check`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CheckOptions<'a> {
    /// Directory or single file containing Aozora Flavored Markdown sources.
    pub input: &'a Path,
    /// Path to `book.toml` metadata.
    pub metadata: &'a Path,
}

impl<'a> CheckOptions<'a> {
    /// Both input paths are required.
    #[must_use]
    pub const fn new(input: &'a Path, metadata: &'a Path) -> Self {
        Self { input, metadata }
    }
}

/// What one chapter's render observed, and the text those spans index into.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChapterReport {
    /// Source file as discovered, before decoding.
    pub path: PathBuf,
    /// Decoded chapter text. A [`ByteSpan`] indexes into this, not into the
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
    let (bundle, report) = prepare(opts)?;
    package::write(opts.output, &bundle)?;
    Ok(report)
}

/// Validate and render a book without creating an EPUB file.
///
/// # Errors
///
/// Any discovery, validation, rendering, or composition failure.
pub fn check(opts: &CheckOptions<'_>) -> Result<BuildReport> {
    let build_opts = BuildOptions::new(opts.input, opts.metadata, Path::new(""));
    let (_, report) = prepare(&build_opts)?;
    Ok(report)
}

fn prepare(opts: &BuildOptions<'_>) -> Result<(compose::Bundle, BuildReport)> {
    let manuscript = discover::collect(opts)?;
    validate::validate(&manuscript)?;
    let rendered = render::render_all(&manuscript)?;
    let bundle = compose::compose(&manuscript, &rendered)?;
    let report = BuildReport {
        chapters: rendered.chapters,
    };
    Ok((bundle, report))
}
