//! Phase 4 — package the bundle into a `.epub` ZIP.
//!
//! [OCF §3.4](https://www.w3.org/TR/epub-33/#sec-container-abstract) makes
//! the entry order and compression load-bearing: `mimetype` must be *first*,
//! Stored (method 0) with no extra fields, holding exactly the 20 bytes
//! `application/epub+zip`. `META-INF/container.xml` follows, Deflated like
//! everything after it per OCF §3.5.
//!
//! `zip` is pulled with `default-features = false` + `deflate`, so it brings
//! in neither `time` nor compressors nothing here uses.

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use zip::write::{SimpleFileOptions, ZipWriter};

use crate::compose::Bundle;
use crate::{Error, Result};

pub(crate) fn write(out: &Path, bundle: &Bundle) -> Result<()> {
    // Assembly has no filesystem beneath it: every zip write targets the
    // infallible `Cursor<Vec<u8>>` below. Only the completed archive crosses
    // the filesystem boundary, so an output-device failure is always
    // `PackageIo`, never an archiver-shaped `Package`.
    let archive = assemble(out, bundle)?;

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::PackageIo {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(out, archive).map_err(|source| Error::PackageIo {
        path: out.to_path_buf(),
        source,
    })
}

type ArchiveWriter = ZipWriter<Cursor<Vec<u8>>>;

fn assemble(out: &Path, bundle: &Bundle) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));

    write_stored(&mut zip, "mimetype", bundle.mimetype.as_bytes(), out)?;
    write_deflated(
        &mut zip,
        "META-INF/container.xml",
        bundle.container.as_bytes(),
        out,
    )?;
    write_deflated(
        &mut zip,
        "OEBPS/package.opf",
        bundle.package_opf.as_bytes(),
        out,
    )?;
    write_deflated(
        &mut zip,
        "OEBPS/nav.xhtml",
        bundle.nav_xhtml.as_bytes(),
        out,
    )?;
    for asset in &bundle.assets {
        write_deflated(&mut zip, &asset.path, &asset.contents, out)?;
    }
    for item in &bundle.spine {
        write_deflated(&mut zip, &item.path, &item.contents, out)?;
    }
    let archive = zip
        .finish()
        .map_err(|source| Error::package(out.to_path_buf(), source))?;
    Ok(archive.into_inner())
}

fn write_stored(zip: &mut ArchiveWriter, name: &str, bytes: &[u8], out_path: &Path) -> Result<()> {
    let opts: SimpleFileOptions =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(name, opts)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    zip.write_all(bytes)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    Ok(())
}

fn write_deflated(
    zip: &mut ArchiveWriter,
    name: &str,
    bytes: &[u8],
    out_path: &Path,
) -> Result<()> {
    let opts: SimpleFileOptions =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(name, opts)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    zip.write_all(bytes)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::Bundle;

    fn minimal_bundle() -> Bundle {
        Bundle {
            mimetype: "application/epub+zip",
            container: String::new(),
            package_opf: String::new(),
            nav_xhtml: String::new(),
            spine: vec![],
            assets: vec![],
        }
    }

    #[test]
    fn write_produces_a_nonempty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("book.epub");

        write(&out, &minimal_bundle()).expect("write should succeed");

        assert!(out.exists(), "the .epub file should exist");
        let len = fs::metadata(&out).expect("metadata").len();
        assert!(len > 0, "the .epub file should be non-empty");
    }

    #[test]
    fn write_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("nested/sub/book.epub");

        write(&out, &minimal_bundle()).expect("write should create parents");

        assert!(out.exists(), "the nested .epub file should exist");
    }

    #[test]
    fn write_fails_when_parent_is_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Create a regular file, then aim the output *inside* it so that
        // `create_dir_all` on the parent must fail (NotADirectory).
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"not a directory").expect("write blocker");
        let out = blocker.join("book.epub");

        let err = write(&out, &minimal_bundle()).expect_err("write should fail");
        assert!(
            matches!(err, Error::PackageIo { ref path, .. } if path == &blocker),
            "expected PackageIo for the parent path, got {err:?}",
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_finished_archive_write_failure_is_package_io() {
        let out = Path::new("/dev/full");

        let err = write(out, &minimal_bundle()).expect_err("/dev/full rejects every write");
        assert!(
            matches!(err, Error::PackageIo { ref path, .. } if path == out),
            "the filesystem boundary must report PackageIo, got {err:?}",
        );
    }
}
