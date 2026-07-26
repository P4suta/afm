//! Phase 1 — collect the sources in spine order (lexicographic, unless
//! `book.toml` overrides it) and parse that manifest into [`Metadata`].

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{BuildOptions, Error, Result};

/// One book's worth of inputs after discovery.
#[derive(Debug, Clone)]
pub(crate) struct Manuscript {
    pub metadata: Metadata,
    pub sources: Vec<SourceFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Metadata {
    pub title: String,
    pub creator: String,
    pub language: String,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default = "default_mode")]
    pub writing_mode: WritingMode,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WritingMode {
    Horizontal,
    Vertical,
}

const fn default_mode() -> WritingMode {
    WritingMode::Horizontal
}

#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

pub(crate) fn collect(opts: &BuildOptions<'_>) -> Result<Manuscript> {
    let metadata_text = fs::read_to_string(opts.metadata).map_err(|source| Error::DiscoverIo {
        path: opts.metadata.to_path_buf(),
        source,
    })?;
    let metadata: Metadata = toml::from_str(&metadata_text)
        .map_err(|source| Error::metadata_parse(opts.metadata.to_path_buf(), source))?;

    let mut sources = Vec::new();
    if opts.input.is_file() {
        sources.push(read_source(opts.input)?);
    } else {
        let entries = fs::read_dir(opts.input).map_err(|source| Error::DiscoverIo {
            path: opts.input.to_path_buf(),
            source,
        })?;
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
            .collect();
        paths.sort();
        for p in paths {
            sources.push(read_source(&p)?);
        }
    }

    Ok(Manuscript { metadata, sources })
}

fn read_source(path: &Path) -> Result<SourceFile> {
    let bytes = fs::read(path).map_err(|source| Error::DiscoverIo {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(SourceFile {
        path: path.to_path_buf(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book_toml(dir: &Path) -> PathBuf {
        let p = dir.join("book.toml");
        fs::write(&p, "title = \"T\"\ncreator = \"A\"\nlanguage = \"ja\"\n").expect("write");
        p
    }

    #[test]
    fn collects_markdown_sorted_and_ignores_non_md() {
        let dir = tempfile::tempdir().unwrap();
        let meta = book_toml(dir.path());
        let src = dir.path().join("manuscript");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("002-b.md"), "b").unwrap();
        fs::write(src.join("001-a.md"), "a").unwrap();
        fs::write(src.join("notes.txt"), "ignored").unwrap();
        let opts = BuildOptions {
            input: &src,
            metadata: &meta,
            output: Path::new("unused.epub"),
        };
        let m = collect(&opts).unwrap();
        assert_eq!(m.sources.len(), 2, "the .txt file must be ignored");
        assert!(m.sources[0].path.ends_with("001-a.md"));
        assert!(m.sources[1].path.ends_with("002-b.md"));
        assert!(matches!(m.metadata.writing_mode, WritingMode::Horizontal));
    }

    #[test]
    fn accepts_a_single_file_input() {
        let dir = tempfile::tempdir().unwrap();
        let meta = book_toml(dir.path());
        fs::write(dir.path().join("only.md"), "x").unwrap();
        let opts = BuildOptions {
            input: &dir.path().join("only.md"),
            metadata: &meta,
            output: Path::new("unused.epub"),
        };
        let m = collect(&opts).unwrap();
        assert_eq!(m.sources.len(), 1);
    }

    #[test]
    fn missing_metadata_file_is_discover_io_not_parse() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("only.md");
        fs::write(&src, "x").unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        let opts = BuildOptions {
            input: &src,
            metadata: &missing,
            output: Path::new("unused.epub"),
        };
        let err = collect(&opts).unwrap_err();
        assert!(
            matches!(err, Error::DiscoverIo { ref path, .. } if path == &missing),
            "a missing metadata file must surface as DiscoverIo, got {err:?}"
        );
    }

    #[test]
    fn malformed_metadata_is_metadata_parse() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("book.toml");
        fs::write(&meta, "title = \"unterminated\ncreator =").unwrap();
        let src = dir.path().join("only.md");
        fs::write(&src, "x").unwrap();
        let opts = BuildOptions {
            input: &src,
            metadata: &meta,
            output: Path::new("unused.epub"),
        };
        let err = collect(&opts).unwrap_err();
        assert!(
            matches!(err, Error::MetadataParse { ref path, .. } if path == &meta),
            "malformed book.toml must surface as MetadataParse, got {err:?}"
        );
    }
}
