//! Shared helpers for the integration tests. Cargo does not treat
//! `tests/common/mod.rs` as its own test target, so this stays a plain
//! module other test files pull in via `mod common;`.

use std::fs;
use std::io::Read;
use std::path::Path;

use tempfile::TempDir;

/// One ZIP entry of a produced EPUB, in archive order.
pub(crate) struct Entry {
    pub name: String,
    pub compression: zip::CompressionMethod,
    pub bytes: Vec<u8>,
}

/// Write a `book.toml` plus named markdown sources (under `manuscript/`)
/// into a fresh temp dir. The dir is removed when the returned
/// [`TempDir`] is dropped.
pub(crate) fn fixture(book_toml: &str, sources: &[(&str, &str)]) -> TempDir {
    let bytes: Vec<(&str, &[u8])> = sources
        .iter()
        .map(|(name, body)| (*name, body.as_bytes()))
        .collect();
    fixture_bytes(book_toml, &bytes)
}

/// [`fixture`] over raw bytes, for the chapters that are not UTF-8: a `.sjis`
/// source, or one that is not decodable at all.
pub(crate) fn fixture_bytes(book_toml: &str, sources: &[(&str, &[u8])]) -> TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    fs::write(dir.path().join("book.toml"), book_toml).expect("write book.toml");
    let manuscript = dir.path().join("manuscript");
    fs::create_dir(&manuscript).expect("create manuscript dir");
    for (name, body) in sources {
        fs::write(manuscript.join(name), body).expect("write source");
    }
    dir
}

/// Open a produced `.epub` and return every entry in archive order.
pub(crate) fn read_epub(path: &Path) -> Vec<Entry> {
    let file = fs::File::open(path).expect("open epub");
    let mut zip = zip::ZipArchive::new(file).expect("read epub zip");
    let mut out = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("zip entry");
        let name = entry.name().to_owned();
        let compression = entry.compression();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read entry");
        out.push(Entry {
            name,
            compression,
            bytes,
        });
    }
    out
}

/// Read one entry's contents as UTF-8 text, panicking if absent.
pub(crate) fn entry_text(entries: &[Entry], name: &str) -> String {
    let bytes = &entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("entry {name} not found"))
        .bytes;
    String::from_utf8(bytes.clone()).expect("entry is valid UTF-8")
}
