//! Shared helpers for the integration tests. Cargo does not treat

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture and EPUB inspection helpers stop at the first malformed artifact"
)]
//! `tests/common/mod.rs` as its own test target, so this stays a plain
//! module other test files pull in via `mod common;`.

use std::fs;
use std::io::Read;
use std::path::Path;

use aozora_flavored_markdown_test_support::check_well_formed;
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
///
/// Every read runs [`package_violations`] first, so the rules there hold for
/// *every* EPUB this suite builds rather than for whichever fixture a given
/// test happened to look inside. That is the difference that matters here:
/// each test below asserts the presence of what it expects — an `itemref` it
/// named, a chapter it wrote — and a package can satisfy every one of those
/// while being invalid in a way no test names. A manuscript directory with no
/// sources produced exactly that: `<spine></spine>`, a nav with an empty
/// `<ol>`, and a `build` that returned `Ok`.
pub(crate) fn read_epub(path: &Path) -> Vec<Entry> {
    let entries = unpack(path);
    let violations = package_violations(&entries);
    assert!(
        violations.is_empty(),
        "{} is not a spec-valid EPUB container:\n  - {}",
        path.display(),
        violations.join("\n  - ")
    );
    entries
}

fn unpack(path: &Path) -> Vec<Entry> {
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

/// What makes the produced archive an EPUB rather than a ZIP of XHTML.
///
/// One line per rule, empty when the package is sound.
fn package_violations(entries: &[Entry]) -> Vec<String> {
    let mut bad = Vec::new();

    // OCF §3.3 — the ZIP is identified by its first member, uncompressed.
    let mimetype_ok = entries.first().is_some_and(|first| {
        first.name == "mimetype"
            && first.compression == zip::CompressionMethod::Stored
            && first.bytes.as_slice() == b"application/epub+zip"
    });
    if !mimetype_ok {
        bad.push(
            "OCF §3.3: the first archive member must be a Stored `mimetype` holding exactly \
             `application/epub+zip`"
                .to_owned(),
        );
    }

    let Some(opf) = entries.iter().find(|e| e.name == "OEBPS/package.opf") else {
        bad.push("OCF §3.5.1: no OEBPS/package.opf in the container".to_owned());
        return bad;
    };
    let opf = String::from_utf8_lossy(&opf.bytes);
    bad.extend(spine_violations(entries, &opf));
    bad.extend(xhtml_violations(entries));
    bad
}

/// The reading order, and everything that has to line up behind it.
fn spine_violations(entries: &[Entry], opf: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let manifest: Vec<(String, String)> = tag_bodies(opf, "item")
        .iter()
        .filter_map(|body| Some((attr(body, "id")?, attr(body, "href")?)))
        .collect();
    let idrefs: Vec<String> = tag_bodies(opf, "itemref")
        .iter()
        .filter_map(|body| attr(body, "idref"))
        .collect();

    // EPUB 3.3 §3.4.12: `spine` has one or more `itemref` children. A book
    // with no chapters is not a book, and this is where that stops being an
    // opinion.
    if idrefs.is_empty() {
        bad.push("EPUB 3.3 §3.4.12: <spine> carries no <itemref>".to_owned());
    }

    let mut reading_order = Vec::new();
    for idref in &idrefs {
        match manifest.iter().find(|(id, _)| id == idref) {
            Some((_, href)) => reading_order.push(href.clone()),
            None => bad.push(format!("spine itemref {idref:?} names no manifest item")),
        }
    }
    for (_, href) in &manifest {
        if !entries.iter().any(|e| e.name == format!("OEBPS/{href}")) {
            bad.push(format!("manifest href {href:?} is not in the archive"));
        }
    }

    // The table of contents lists the same documents the spine reads, in the
    // same order — a nav that disagrees sends a reader to the wrong chapter.
    match entries.iter().find(|e| e.name == "OEBPS/nav.xhtml") {
        None => bad.push("EPUB 3.3 §5.4: no OEBPS/nav.xhtml in the container".to_owned()),
        Some(nav) => {
            let nav = String::from_utf8_lossy(&nav.bytes);
            let listed: Vec<String> = tag_bodies(&nav, "a")
                .iter()
                .filter_map(|body| attr(body, "href"))
                .collect();
            if listed != reading_order {
                bad.push(format!(
                    "the navigation document lists {listed:?}, the spine reads {reading_order:?}"
                ));
            }
        }
    }
    bad
}

/// Every XHTML document the package ships is tag-balanced.
///
/// The chapter envelope is the one document this crate builds by string
/// interpolation rather than through `quick_xml`, so it is the one that can
/// go malformed from a chapter title alone.
fn xhtml_violations(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| {
            Path::new(&e.name)
                .extension()
                .is_some_and(|ext| ext == "xhtml")
        })
        .flat_map(|entry| {
            let text = String::from_utf8_lossy(&entry.bytes).into_owned();
            check_well_formed(&text)
                .into_iter()
                .map(|err| format!("{}: {err}", entry.name))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The inside of every `<tag …>` start tag, in document order.
///
/// Deliberately not an XML parser: it reads markup this crate wrote, and a
/// checker with its own parse tree would be the second implementation of the
/// thing under test. `<item` must not match `<itemref`, which is the only
/// subtlety.
fn tag_bodies<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    for (lt, _) in xml.match_indices('<') {
        let after = &xml[lt + 1..];
        let Some(gt) = after.find('>') else { continue };
        let Some(rest) = after[..gt].strip_prefix(tag) else {
            continue;
        };
        if rest.is_empty() || rest.starts_with([' ', '\t', '\n', '\r', '/']) {
            out.push(rest);
        }
    }
    out
}

fn attr(body: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = body.find(&needle)? + needle.len();
    let len = body[start..].find('"')?;
    Some(body[start..start + len].to_owned())
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
