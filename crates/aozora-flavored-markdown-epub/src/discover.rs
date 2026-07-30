//! Phase 1 — collect the sources in spine order (lexicographic, unless
//! `book.toml` overrides it) and parse that manifest into [`Metadata`].

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::{BuildOptions, Error, Result};

/// One book's worth of inputs after discovery.
#[derive(Debug, Clone)]
pub(crate) struct Manuscript {
    pub metadata: Metadata,
    pub metadata_path: PathBuf,
    pub sources: Vec<SourceFile>,
}

// `deny_unknown_fields` because a key this crate does not know is a key the
// author expected to do something. Silence is the failure mode this whole
// manifest had: `spine` parsed, deserialised into nothing, and ordered the
// book lexicographically anyway.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub title: String,
    pub creator: String,
    pub language: String,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default = "default_mode")]
    pub writing_mode: WritingMode,
    // `None` means no explicit order and selects the directory sweep.
    // `Some([])` is deliberately distinct: an explicitly empty EPUB spine is
    // invalid rather than another spelling of the default.
    #[serde(default)]
    pub spine: Option<Vec<PathBuf>>,
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

// How a source file's bytes decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    Utf8,
    ShiftJis,
}

// Every extension a manuscript source can carry, and the encoding its bytes
// are in. The sweep below collects exactly these rows and `render` decodes
// from the same ones, so an extension cannot become collectable without also
// becoming decodable. The two were written out separately once, and the sweep
// took `.md` alone while the decoder branched on three Shift_JIS spellings —
// a `.sjis` chapter beside a `.md` one was dropped from the book without a
// word.
const SOURCE_EXTENSIONS: &[(&str, Encoding)] = &[
    ("md", Encoding::Utf8),
    ("sjis", Encoding::ShiftJis),
    ("shift_jis", Encoding::ShiftJis),
    ("shift-jis", Encoding::ShiftJis),
];

// `None` for anything the sweep does not collect. A file named outright still
// reads — naming it *is* the choice the sweep's filter stands in for — and
// takes the UTF-8 branch.
pub(crate) fn encoding_of(path: &Path) -> Option<Encoding> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    SOURCE_EXTENSIONS
        .iter()
        .find_map(|&(name, encoding)| (name == ext).then_some(encoding))
}

pub(crate) fn collect(opts: &BuildOptions<'_>) -> Result<Manuscript> {
    let metadata_text = fs::read_to_string(opts.metadata).map_err(|source| Error::DiscoverIo {
        path: opts.metadata.to_path_buf(),
        source,
    })?;
    let metadata: Metadata = toml::from_str(&metadata_text)
        .map_err(|source| Error::metadata_parse(opts.metadata.to_path_buf(), source))?;

    let sources = match (opts.input.is_file(), metadata.spine.as_deref()) {
        // An explicitly empty reading order describes an empty book
        // regardless of whether `input` names a directory or one file.
        (_, Some([])) => {
            return Err(Error::NoSources {
                path: opts.input.to_path_buf(),
            });
        }
        // A spine names files inside a manuscript directory, so there is
        // nothing here for it to order. Refusing beats building the one file
        // while silently ignoring a non-empty chapter list.
        (true, Some(_)) => {
            return Err(Error::SpineInvalid {
                path: opts.input.to_path_buf(),
                reason: "`spine` cannot be used with a single-file input".to_owned(),
            });
        }
        (true, None) => vec![read_source(opts.input)?],
        (false, None) => sweep(opts.input)?,
        (false, Some(entries)) => entries
            .iter()
            .map(|entry| {
                let path = validate_spine_path(opts.input, entry)?;
                read_source(&path)
            })
            .collect::<Result<Vec<_>>>()?,
    };

    // An empty book is not a book. Left to run, `compose` writes a package
    // whose `<spine>` holds no `itemref` — a shape EPUB 3.3 does not allow —
    // and `build` hands it back as a success.
    if sources.is_empty() {
        return Err(Error::NoSources {
            path: opts.input.to_path_buf(),
        });
    }

    Ok(Manuscript {
        metadata,
        metadata_path: opts.metadata.to_path_buf(),
        sources,
    })
}

fn validate_spine_path(root: &Path, entry: &Path) -> Result<PathBuf> {
    if entry.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(Error::SpineInvalid {
            path: entry.to_path_buf(),
            reason: "entries must be relative paths contained by the manuscript root".to_owned(),
        });
    }

    let canonical_root = fs::canonicalize(root).map_err(|source| Error::DiscoverIo {
        path: root.to_path_buf(),
        source,
    })?;
    let candidate = root.join(entry);
    let canonical = fs::canonicalize(&candidate).map_err(|source| Error::DiscoverIo {
        path: candidate.clone(),
        source,
    })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(Error::SpineInvalid {
            path: candidate,
            reason: "the resolved path escapes the manuscript root".to_owned(),
        });
    }
    if !canonical.is_file() {
        return Err(Error::SpineInvalid {
            path: candidate,
            reason: "the resolved path is not a regular file".to_owned(),
        });
    }
    Ok(candidate)
}

// Lexicographic by full path, which is what zero-padded chapter numbers are
// named for.
fn sweep(dir: &Path) -> Result<Vec<SourceFile>> {
    let entries = fs::read_dir(dir).map_err(|source| Error::DiscoverIo {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| encoding_of(p).is_some())
        .collect();
    paths.sort();
    paths.iter().map(|p| read_source(p)).collect()
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
    use crate::render::render_all;

    use super::*;

    fn book_toml(dir: &Path) -> PathBuf {
        let p = dir.join("book.toml");
        fs::write(&p, "title = \"T\"\ncreator = \"A\"\nlanguage = \"ja\"\n").expect("write");
        p
    }

    fn opts<'a>(input: &'a Path, metadata: &'a Path) -> BuildOptions<'a> {
        BuildOptions {
            input,
            metadata,
            output: Path::new("unused.epub"),
        }
    }

    #[test]
    fn sweeps_sources_in_lexicographic_order_and_skips_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let meta = book_toml(dir.path());
        let src = dir.path().join("manuscript");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("002-b.md"), "b").unwrap();
        fs::write(src.join("001-a.md"), "a").unwrap();
        fs::write(src.join("notes.txt"), "ignored").unwrap();
        fs::write(src.join("README"), "no extension at all").unwrap();
        let m = collect(&opts(&src, &meta)).unwrap();
        assert_eq!(m.sources.len(), 2, "only the two sources are collected");
        assert!(m.sources[0].path.ends_with("001-a.md"), "sorted by path");
        assert!(m.sources[1].path.ends_with("002-b.md"), "sorted by path");
        assert!(
            matches!(m.metadata.writing_mode, WritingMode::Horizontal),
            "the default writing mode is horizontal"
        );
    }

    // The statement two hand-written extension lists could not make. It walks
    // `SOURCE_EXTENSIONS` rather than naming extensions, so a row added later
    // is covered the day it is added: the sweep has to collect it, and `render`
    // has to decode it at the encoding the row declares.
    //
    // Both directions fail loudly if the two sites disagree. Shift_JIS `あ` is
    // not valid UTF-8, so a `.sjis` row read down the UTF-8 branch is an
    // `Error::Utf8`; UTF-8 `あ` decoded as Shift_JIS is mojibake, not `あ`.
    #[test]
    fn every_extension_the_sweep_collects_decodes_at_the_encoding_the_table_declares() {
        let dir = tempfile::tempdir().unwrap();
        let meta = book_toml(dir.path());
        let src = dir.path().join("manuscript");
        fs::create_dir(&src).unwrap();
        for (idx, (ext, encoding)) in SOURCE_EXTENSIONS.iter().enumerate() {
            let bytes: &[u8] = match *encoding {
                Encoding::Utf8 => "あ".as_bytes(),
                Encoding::ShiftJis => &[0x82, 0xA0],
            };
            fs::write(src.join(format!("{idx:03}.{ext}")), bytes).unwrap();
        }

        let manuscript = collect(&opts(&src, &meta)).expect("the sweep takes every row");
        assert_eq!(
            manuscript.sources.len(),
            SOURCE_EXTENSIONS.len(),
            "an extension in the table that the sweep drops leaves a chapter out of the book"
        );
        let rendered = render_all(&manuscript).expect("every row decodes");
        for (idx, (ext, _)) in SOURCE_EXTENSIONS.iter().enumerate() {
            let item = &rendered.items[idx];
            assert_eq!(
                item.title,
                format!("{idx:03}"),
                ".{ext} landed out of order"
            );
            assert!(
                item.xhtml.contains('あ'),
                ".{ext} decoded to {:?} rather than あ",
                item.xhtml
            );
        }
    }

    #[test]
    fn an_extension_the_table_does_not_carry_is_not_a_source() {
        assert!(encoding_of(Path::new("README")).is_none(), "no extension");
        assert!(
            encoding_of(Path::new("notes.txt")).is_none(),
            "not a source"
        );
        assert_eq!(
            encoding_of(Path::new("CH.MD")),
            Some(Encoding::Utf8),
            "the table is matched case-insensitively"
        );
    }

    // A filename is bytes on Unix, so an extension need not be text at all.
    #[cfg(unix)]
    #[test]
    fn an_extension_that_is_not_utf8_is_not_a_source() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(b"chapter.\xFF".to_vec()));
        assert!(
            encoding_of(&path).is_none(),
            "an extension that is not UTF-8 cannot match a row of the table"
        );
    }

    #[test]
    fn a_spine_is_the_reading_order_and_the_whole_chapter_list() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("book.toml");
        fs::write(
            &meta,
            "title = \"T\"\ncreator = \"A\"\nlanguage = \"ja\"\nspine = [\"c.md\", \"a.md\"]\n",
        )
        .unwrap();
        let src = dir.path().join("manuscript");
        fs::create_dir(&src).unwrap();
        for name in ["a.md", "b.md", "c.md"] {
            fs::write(src.join(name), name).unwrap();
        }
        let m = collect(&opts(&src, &meta)).unwrap();
        let names: Vec<_> = m
            .sources
            .iter()
            .map(|s| s.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["c.md", "a.md"],
            "the spine order wins over the sweep's, and b.md is not in it"
        );
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
