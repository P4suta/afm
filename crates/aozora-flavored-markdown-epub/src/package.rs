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
    // infallible `Cursor<Vec<u8>>` below, and `write_deflated` reserves ZIP64
    // before the selected encoder's upper bound can cross the classic limit.
    // Only the completed archive crosses the filesystem boundary, so an
    // output-device failure is always `PackageIo`, never an archiver-shaped
    // `Package`.
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

    write_mimetype(&mut zip, out)?;
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

fn write_mimetype(zip: &mut ArchiveWriter, out_path: &Path) -> Result<()> {
    // Keep this entry's name, contents and options closed over here. OCF
    // requires it to be first, Stored, and free of every extra field,
    // including ZIP64.
    let opts: SimpleFileOptions =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", opts)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    zip.write_all(b"application/epub+zip")
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    Ok(())
}

fn write_deflated(
    zip: &mut ArchiveWriter,
    name: &str,
    bytes: &[u8],
    out_path: &Path,
) -> Result<()> {
    let opts: SimpleFileOptions = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(DEFLATE_LEVEL))
        .large_file(deflated_entry_needs_zip64(bytes.len() as u64));
    zip.start_file(name, opts)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    zip.write_all(bytes)
        .map_err(|source| Error::package(out_path.to_path_buf(), source))?;
    Ok(())
}

/// The Deflate level every entry is written at. Named and passed explicitly
/// rather than left to `SimpleFileOptions::default()`, because it is the
/// premise [`deflated_entry_needs_zip64`] reasons from: an archiver default
/// that moved under us would move the encoder's upper bound with it, and would
/// change the bytes of an otherwise unchanged `.epub` besides. Six is what the
/// backend picks today, so pinning it holds output byte-identical.
const DEFLATE_LEVEL: i64 = 6;

/// Whether the selected Deflate encoder might cross the classic ZIP entry
/// size limit for an input whose complete size is already known.
///
/// The workspace's `zip/deflate` feature uses flate2's zlib-rs backend at
/// [`DEFLATE_LEVEL`]. Its conservative bound is the source length plus one bit
/// per input byte, rounded up, and a small header/wrapper constant. ZIP stores
/// raw Deflate, so retaining sixteen bytes of headroom is more conservative
/// still.
///
/// Checking the encoded upper bound, rather than only `input_len`, also
/// covers an incompressible input just below 4 GiB whose Deflate stream is
/// slightly larger than its source. Saturation fails toward ZIP64.
const fn deflated_entry_needs_zip64(input_len: u64) -> bool {
    let one_bit_per_byte = input_len.saturating_add(7) / 8;
    let encoded_upper_bound = input_len
        .saturating_add(one_bit_per_byte)
        .saturating_add(16);
    encoded_upper_bound > zip::ZIP64_BYTES_THR
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::compose::Bundle;

    fn minimal_bundle() -> Bundle {
        Bundle {
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
    fn mimetype_keeps_the_ocf_header_while_small_entries_avoid_zip64() {
        let bytes = assemble(Path::new("book.epub"), &minimal_bundle()).expect("assemble");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("read archive");

        let mimetype = archive.by_index(0).expect("mimetype is first");
        assert_eq!(mimetype.name(), "mimetype");
        assert_eq!(mimetype.compression(), zip::CompressionMethod::Stored);
        assert!(
            mimetype.extra_data().is_none_or(<[u8]>::is_empty),
            "OCF forbids every extra field on mimetype"
        );
        drop(mimetype);

        let mut container = archive
            .by_name("META-INF/container.xml")
            .expect("container");
        let mut contents = String::new();
        container
            .read_to_string(&mut contents)
            .expect("read container");
        assert!(
            container.extra_data().is_none_or(<[u8]>::is_empty),
            "ordinary small entries should not pay for ZIP64"
        );
    }

    #[test]
    fn deflated_size_bound_enables_zip64_before_classic_headers_can_overflow() {
        assert!(!deflated_entry_needs_zip64(0));
        assert!(!deflated_entry_needs_zip64(1024));

        let classic_limit = zip::ZIP64_BYTES_THR;
        assert!(
            deflated_entry_needs_zip64(classic_limit - 1),
            "Deflate can expand an input that is itself just below the limit"
        );
        assert!(deflated_entry_needs_zip64(classic_limit));
        assert!(
            deflated_entry_needs_zip64(u64::MAX),
            "overflow must fail toward ZIP64"
        );
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
