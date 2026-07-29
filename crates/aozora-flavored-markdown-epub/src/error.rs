//! Error type (`thiserror` + `miette`). Each variant carries enough
//! context to root-cause without a stack trace (the path it happened in,
//! the field name for invariant violations) and a stable diagnostic code
//! `aozora_flavored_markdown_epub::<phase>::<kind>`.
//!
//! `Error` and each of its struct variants are `#[non_exhaustive]`.

use core::error::Error as StdError;
use core::fmt;
use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;

use miette::Diagnostic;
use thiserror::Error;

/// Result alias for the crate.
#[expect(
    clippy::absolute_paths,
    reason = "the prelude `Result` would self-reference this alias, so the std path is unavoidable"
)]
pub type Result<T> = std::result::Result<T, Error>;

/// A failure raised by a dependency, held opaquely.
///
/// `toml`, `zip` and `aozora` all release majors on their own clocks, so
/// naming their error types in a field here would hand them this crate's
/// `SemVer`. `Display` and [`StdError::source`] delegate to the wrapped
/// error, leaving the chain a caller walks exactly as it was.
pub struct Cause(Box<dyn StdError + Send + Sync + 'static>);

impl Cause {
    fn new<E: StdError + Send + Sync + 'static>(err: E) -> Self {
        Self(Box::new(err))
    }
}

// Both delegate rather than wrap, so neither rendering shows this type: a
// `Cause` is meant to read as the error it holds.
impl fmt::Debug for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl StdError for Cause {
    // The wrapped error's own cause, not the wrapped error itself — this
    // type stands in for it rather than sitting above it.
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

/// Every way a build can fail, tagged by the phase that raised it.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    /// Discovery could not read the manuscript tree.
    #[error("failed to read manuscript root: {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::discover::io))]
    #[non_exhaustive]
    DiscoverIo {
        /// The entry that would not read.
        path: PathBuf,
        /// What the filesystem said.
        #[source]
        source: io::Error,
    },

    /// `book.toml` is present but not valid TOML.
    #[error("failed to parse book metadata at {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::discover::metadata))]
    #[non_exhaustive]
    MetadataParse {
        /// The metadata file that would not parse.
        path: PathBuf,
        /// The parser's own failure, held opaquely.
        #[source]
        source: Cause,
    },

    /// Metadata parsed, but a field cannot go into an EPUB package document.
    #[error("metadata field {field:?} is invalid: {reason}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::validate::metadata))]
    #[non_exhaustive]
    MetadataInvalid {
        /// Name of the offending field, as spelled in `book.toml`.
        field: &'static str,
        /// Why that value is not usable.
        reason: String,
    },

    /// An explicit spine entry is empty, rooted, or escapes its manuscript.
    #[error("invalid EPUB spine entry {path}: {reason}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::validate::spine))]
    #[non_exhaustive]
    SpineInvalid {
        /// The manuscript root or entry that failed validation.
        path: PathBuf,
        /// The containment or shape rule it violated.
        reason: String,
    },

    /// User-controlled text contains a character XML 1.0 cannot represent.
    #[error("XML 1.0 forbids U+{codepoint:04X} in {field} at byte {byte_offset}: {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::validate::xml_character))]
    #[non_exhaustive]
    XmlCharacter {
        /// Metadata file or chapter containing the value.
        path: PathBuf,
        /// Logical field whose value is invalid.
        field: &'static str,
        /// UTF-8 byte offset in the value.
        byte_offset: usize,
        /// Unicode scalar value rejected by XML 1.0.
        codepoint: u32,
    },

    /// The OPF / NAV writer rejected what it was asked to emit.
    #[error("failed to build XML for the EPUB scaffolding: {0}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::compose::xml))]
    XmlBuild(
        /// What the XML writer objected to.
        Cow<'static, str>,
    ),

    /// The tree held no chapter, so there is no spine to write.
    #[error("no chapter sources under {path}")]
    #[diagnostic(
        code(aozora_flavored_markdown_epub::discover::empty),
        help("EPUB requires a spine of one item or more, so a book needs at least one chapter")
    )]
    #[non_exhaustive]
    NoSources {
        /// The root that was searched.
        path: PathBuf,
    },

    /// The ZIP container could not be assembled.
    #[error("EPUB packaging failed for {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::package::zip))]
    #[non_exhaustive]
    Package {
        /// The archive being written.
        path: PathBuf,
        /// The archiver's own failure, held opaquely.
        #[source]
        source: Cause,
    },

    /// The finished archive could not be written out.
    #[error("EPUB packaging I/O error at {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::package::io))]
    #[non_exhaustive]
    PackageIo {
        /// The archive being written.
        path: PathBuf,
        /// What the filesystem said.
        #[source]
        source: io::Error,
    },

    /// A chapter's bytes are neither UTF-8 nor recoverable as `Shift_JIS`.
    #[error("source bytes are not valid UTF-8: {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::render::utf8))]
    #[non_exhaustive]
    Utf8 {
        /// The chapter that would not decode.
        path: PathBuf,
        /// Where in the byte stream decoding stopped.
        #[source]
        source: Utf8Error,
    },

    /// A chapter looked like `Shift_JIS` and still would not decode.
    #[error("Shift_JIS source could not be decoded: {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::render::sjis))]
    #[non_exhaustive]
    Sjis {
        /// The chapter that would not decode.
        path: PathBuf,
        /// The decoder's own failure, held opaquely.
        #[source]
        source: Cause,
    },
}

impl Error {
    // The three constructors below are the only way a dependency's error
    // reaches `Cause`, which keeps `Cause::new` off the public surface.
    pub(crate) fn metadata_parse<E: StdError + Send + Sync + 'static>(
        path: PathBuf,
        err: E,
    ) -> Self {
        Self::MetadataParse {
            path,
            source: Cause::new(err),
        }
    }

    pub(crate) fn package<E: StdError + Send + Sync + 'static>(path: PathBuf, err: E) -> Self {
        Self::Package {
            path,
            source: Cause::new(err),
        }
    }

    pub(crate) fn sjis<E: StdError + Send + Sync + 'static>(path: PathBuf, err: E) -> Self {
        Self::Sjis {
            path,
            source: Cause::new(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    // A failure with a cause of its own, so the delegation is observable.
    // `Cause::source` answers "inner" here; a wrapper that handed back the
    // error it holds instead would answer "outer" and print it twice.
    #[derive(Debug, Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, Error)]
    #[error("inner")]
    struct Inner;

    fn chain(err: &dyn StdError) -> Vec<String> {
        let mut out = vec![err.to_string()];
        let mut next = err.source();
        while let Some(cause) = next {
            out.push(cause.to_string());
            next = cause.source();
        }
        out
    }

    #[test]
    fn a_cause_stands_in_for_the_error_it_holds_rather_than_above_it() {
        let err = Error::metadata_parse(PathBuf::from("book.toml"), Outer(Inner));
        assert_eq!(
            chain(&err),
            [
                "failed to parse book metadata at book.toml",
                "outer",
                "inner",
            ],
            "boxing must leave the chain the same three levels deep it was"
        );
    }

    #[test]
    fn a_cause_renders_as_the_error_it_holds() {
        let err = Error::sjis(PathBuf::from("x.sjis"), Outer(Inner));
        let Error::Sjis { source, .. } = &err else {
            panic!("expected a Sjis, got {err:?}");
        };
        assert_eq!(source.to_string(), "outer", "`Display` delegates");
        assert_eq!(
            format!("{source:?}"),
            format!("{:?}", Outer(Inner)),
            "`Debug` delegates too, so a debug report shows the failure and not this type"
        );
        assert!(
            !format!("{err:?}").contains("Cause("),
            "the wrapper must not name itself anywhere: {err:?}"
        );
    }

    // Nothing can drive `build` to a zip failure — the archive is assembled
    // in memory through a sink whose writes cannot fail, which is the same
    // reason `package.rs` sits outside the coverage gate (ADR-0018). The
    // constructor still has a contract, so it is exercised directly rather
    // than left as the one arm of this module nothing reaches.
    #[test]
    fn the_packaging_constructor_keeps_the_path_and_the_cause() {
        let err = Error::package(PathBuf::from("out.epub"), Outer(Inner));
        assert_eq!(
            chain(&err),
            ["EPUB packaging failed for out.epub", "outer", "inner"],
            "the packaging path must hand over the same chain the others do"
        );
    }
}
