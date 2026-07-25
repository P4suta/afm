//! Error type (`thiserror` + `miette`). Each variant carries enough
//! context to root-cause without a stack trace (a path for IO, a TOML
//! span for metadata, the field name for invariant violations) and a
//! stable diagnostic code `aozora_flavored_markdown_epub::<phase>::<kind>`.
//!
//! `Error` is `#[non_exhaustive]`.

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;

use miette::Diagnostic;
use thiserror::Error;
use toml::de;
use zip::result::ZipError;

/// Result alias for the crate.
#[expect(
    clippy::absolute_paths,
    reason = "the prelude `Result` would self-reference this alias, so the std path is unavoidable"
)]
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to read manuscript root: {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::discover::io))]
    DiscoverIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse book metadata at {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::discover::metadata))]
    MetadataParse {
        path: PathBuf,
        #[source]
        source: de::Error,
    },

    #[error("metadata field {field:?} is invalid: {reason}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::compose::metadata))]
    MetadataInvalid { field: &'static str, reason: String },

    #[error("failed to build XML for the EPUB scaffolding: {0}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::compose::xml))]
    XmlBuild(Cow<'static, str>),

    #[error("render parse error in {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::render::parse))]
    RenderParse { path: PathBuf, message: String },

    #[error("EPUB packaging failed for {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::package::zip))]
    Package {
        path: PathBuf,
        #[source]
        source: ZipError,
    },

    #[error("EPUB packaging I/O error at {path}")]
    #[diagnostic(code(aozora_flavored_markdown_epub::package::io))]
    PackageIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("source bytes are not valid UTF-8")]
    #[diagnostic(code(aozora_flavored_markdown_epub::render::utf8))]
    Utf8(#[from] Utf8Error),

    #[error("Shift_JIS source could not be decoded")]
    #[diagnostic(code(aozora_flavored_markdown_epub::render::sjis))]
    Sjis(#[from] aozora::DecodeError),
}
