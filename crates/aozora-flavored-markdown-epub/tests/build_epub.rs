//! End-to-end tests for the public [`build`] entry point: run it on a

#![allow(
    clippy::expect_used,
    reason = "integration-test fixture helpers stop immediately when setup or the expected outcome fails"
)]
//! fixture manuscript and inspect the produced EPUB container — and, from
//! [`a_build_report_carries_exactly_what_a_direct_render_saw`] down, the
//! report it hands back beside that container.
//!
//! Everything above that line inspects the ZIP, which is why the pipeline
//! could throw away every chapter diagnostic for as long as it did: `render`
//! is infallible, a dropped diagnostic changes no byte of the EPUB, and an
//! entry point returning `Result<()>` has nothing for a test to disagree
//! with. Coverage saw nothing either — reading a field is not a region, so
//! *not* reading `rendered.diagnostics` left no arm uncovered.

mod common;

use core::error::Error as StdError;
use std::fs;
use std::path::{Path, PathBuf};

use aozora_flavored_markdown::{Diagnostic, DiagnosticSource, Options, render};
use aozora_flavored_markdown_epub::{BuildOptions, BuildReport, Error, build};
use common::{Entry, entry_text, fixture, fixture_bytes, read_epub};

const HORIZONTAL_BOOK: &str = "\
title = \"Test Book\"
creator = \"Test Author\"
language = \"ja\"
writing_mode = \"horizontal\"
";

const VERTICAL_BOOK: &str = "\
title = \"縦書きの本\"
creator = \"著者\"
language = \"ja\"
writing_mode = \"vertical\"
";

fn build_into(dir: &Path, out_name: &str) -> PathBuf {
    report_from(dir, out_name).1
}

/// The report and the path it wrote, from one build of a [`fixture`] dir.
fn report_from(dir: &Path, out_name: &str) -> (BuildReport, PathBuf) {
    let out = dir.join(out_name);
    let report = build(&BuildOptions::new(
        &dir.join("manuscript"),
        &dir.join("book.toml"),
        &out,
    ))
    .expect("build succeeds");
    (report, out)
}

fn opf(entries: &[Entry]) -> String {
    entry_text(entries, "OEBPS/package.opf")
}

#[test]
fn produces_spec_compliant_ocf_container() {
    let dir = fixture(
        HORIZONTAL_BOOK,
        &[
            ("001-intro.md", "# Intro\n\nHello.\n"),
            ("002-body.md", "# Body\n\nWorld.\n"),
        ],
    );
    let out = build_into(dir.path(), "book.epub");
    let entries = read_epub(&out);

    // mimetype must be the first entry, Stored, exactly 20 bytes.
    assert_eq!(entries[0].name, "mimetype");
    assert_eq!(entries[0].compression, zip::CompressionMethod::Stored);
    assert_eq!(entries[0].bytes, b"application/epub+zip");

    // container.xml is the second entry.
    assert_eq!(entries[1].name, "META-INF/container.xml");

    // every entry uses only Stored or Deflated (OCF constraint).
    for e in &entries {
        assert!(
            matches!(
                e.compression,
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            ),
            "{} uses an unexpected compression method",
            e.name
        );
    }

    // all required members are present.
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    for required in [
        "OEBPS/package.opf",
        "OEBPS/nav.xhtml",
        "OEBPS/css/aozora-md.css",
        "OEBPS/chapter-001.xhtml",
        "OEBPS/chapter-002.xhtml",
    ] {
        assert!(names.contains(&required), "missing {required}");
    }

    // container.xml points at the OPF root.
    let container = entry_text(&entries, "META-INF/container.xml");
    assert!(container.contains("OEBPS/package.opf"));
}

#[test]
fn spine_and_manifest_agree_in_lexicographic_order() {
    let dir = fixture(
        HORIZONTAL_BOOK,
        &[("001-intro.md", "# Intro\n"), ("002-body.md", "# Body\n")],
    );
    let out = build_into(dir.path(), "book.epub");
    let opf = opf(&read_epub(&out));

    // chapters keep lexicographic order in both manifest and spine.
    assert!(opf.find("ch001").unwrap() < opf.find("ch002").unwrap());
    // every spine idref is backed by a manifest item.
    assert!(opf.contains(r#"<item id="ch001""#));
    assert!(opf.contains(r#"<item id="ch002""#));
    assert!(opf.contains(r#"<itemref idref="ch001""#));
    assert!(opf.contains(r#"<itemref idref="ch002""#));
}

#[test]
fn nav_lists_every_chapter() {
    let dir = fixture(
        HORIZONTAL_BOOK,
        &[("001-a.md", "# A\n"), ("002-b.md", "# B\n")],
    );
    let entries = read_epub(&build_into(dir.path(), "book.epub"));
    let nav = entry_text(&entries, "OEBPS/nav.xhtml");
    assert_eq!(nav.matches("chapter-001.xhtml").count(), 1);
    assert_eq!(nav.matches("chapter-002.xhtml").count(), 1);
}

#[test]
fn vertical_book_binds_right_to_left() {
    let dir = fixture(VERTICAL_BOOK, &[("001.md", "# 章\n\n本文。\n")]);
    let opf = opf(&read_epub(&build_into(dir.path(), "v.epub")));
    assert!(
        opf.contains(r#"<spine page-progression-direction="rtl">"#),
        "opf: {opf}"
    );
}

#[test]
fn horizontal_book_binds_left_to_right() {
    let dir = fixture(HORIZONTAL_BOOK, &[("001.md", "# Ch\n")]);
    let opf = opf(&read_epub(&build_into(dir.path(), "h.epub")));
    assert!(
        opf.contains(r#"<spine page-progression-direction="ltr">"#),
        "opf: {opf}"
    );
}

#[test]
fn rejects_invalid_language() {
    let dir = fixture(
        "title = \"T\"\ncreator = \"A\"\nlanguage = \"japanese\"\n",
        &[("001.md", "x")],
    );
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(matches!(
        err,
        Error::MetadataInvalid {
            field: "language",
            ..
        }
    ));
}

#[test]
fn rejects_malformed_metadata_toml() {
    let dir = fixture("= not valid toml =", &[("001.md", "x")]);
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(matches!(err, Error::MetadataParse { .. }));
}

#[test]
fn reports_missing_input_directory() {
    let dir = fixture(HORIZONTAL_BOOK, &[]);
    let err = build(&BuildOptions::new(
        &dir.path().join("does-not-exist"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(matches!(err, Error::DiscoverIo { .. }));
}

// ---------------------------------------------------------------------------
// discovery — which files become chapters, and in what order
// ---------------------------------------------------------------------------

/// 「あ」 in Shift_JIS. Not valid UTF-8, so a `.sjis` chapter read down the
/// UTF-8 branch fails loudly instead of arriving as mojibake.
const SJIS_A: &[u8] = b"\x82\xA0";

/// Distinct enough that "chapter 2 holds what chapter 3 should" is visible.
const THREE_CHAPTERS: &[(&str, &str)] = &[
    ("001-a.md", "# 甲\n"),
    ("002-b.md", "# 乙\n"),
    ("003-c.md", "# 丙\n"),
];

fn book_with_spine(entries: &str) -> String {
    format!("{HORIZONTAL_BOOK}spine = [{entries}]\n")
}

fn chapter(entries: &[Entry], n: usize) -> String {
    entry_text(entries, &format!("OEBPS/chapter-{n:03}.xhtml"))
}

/// A `spine` in `book.toml` is the reading order, and the list of chapters.
///
/// `spine_and_manifest_agree_in_lexicographic_order` above could not state
/// this: sorting the sweep was the *only* order the code could produce, so
/// that test passed identically whether the manifest was consulted or
/// ignored — which it was. `deny_unknown_fields` was absent too, so the key
/// did not even have to be spelled right to be silently dropped.
#[test]
fn a_configured_spine_is_the_reading_order_and_omitting_a_file_drops_it() {
    let dir = fixture(
        &book_with_spine(r#""003-c.md", "001-a.md""#),
        THREE_CHAPTERS,
    );
    let entries = read_epub(&build_into(dir.path(), "book.epub"));

    assert!(
        chapter(&entries, 1).contains("丙") && chapter(&entries, 2).contains("甲"),
        "the spine order must be the book order, got {:?} then {:?}",
        chapter(&entries, 1),
        chapter(&entries, 2)
    );
    // The title is the source file's stem, so this says *which file* landed
    // where rather than only which text did.
    assert!(
        chapter(&entries, 1).contains("<title>003-c</title>"),
        "the first chapter must be the first spine entry: {}",
        chapter(&entries, 1)
    );
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        !names.contains(&"OEBPS/chapter-003.xhtml"),
        "a file the spine omits is not a chapter, got {names:?}"
    );
    assert!(
        !entries
            .iter()
            .any(|e| String::from_utf8_lossy(&e.bytes).contains('乙')),
        "the omitted file's text must not reach the package at all"
    );
}

/// A spine entry with a typo is the failure the whole manifest existed to
/// stop being silent, so it names the path it could not read rather than
/// quietly shipping a shorter book.
#[test]
fn a_spine_entry_that_is_not_there_is_named_rather_than_skipped() {
    let dir = fixture(
        &book_with_spine(r#""001-a.md", "004-typo.md""#),
        THREE_CHAPTERS,
    );
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    let missing = dir.path().join("manuscript").join("004-typo.md");
    assert!(
        matches!(err, Error::DiscoverIo { ref path, .. } if path == &missing),
        "a spine entry that is not on disk must name itself, got {err:?}"
    );
}

/// A single-file input has no directory for a spine to order, so a manifest
/// that carries one is describing a build that is not happening.
#[test]
fn a_spine_beside_a_single_file_input_is_refused() {
    let dir = fixture(&book_with_spine(r#""001-a.md""#), THREE_CHAPTERS);
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript").join("001-a.md"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(
        matches!(err, Error::MetadataInvalid { field: "spine", .. }),
        "a configured spine must not be silently unused, got {err:?}"
    );
}

/// The other half of `deny_unknown_fields`: a key this crate does not
/// implement is a key whose author expected something, so it is reported with
/// the name they typed.
#[test]
fn an_unknown_book_toml_key_is_refused_and_named() {
    let dir = fixture(
        &format!("{HORIZONTAL_BOOK}spien = [\"001-a.md\"]\n"),
        THREE_CHAPTERS,
    );
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(matches!(err, Error::MetadataParse { .. }), "got {err:?}");
    let chain = chain_of(&err);
    assert!(
        chain[1].contains("spien"),
        "the cause must name the key the author actually wrote, got {chain:?}"
    );
}

/// Directory discovery took `.md` alone while the decoder branched on three
/// Shift_JIS spellings, so a `.sjis` chapter beside a `.md` one left the book
/// without a word. Both sites read one table now; this is that table's
/// user-visible half.
#[test]
fn a_directory_sweep_takes_shift_jis_chapters_beside_markdown_ones() {
    let dir = fixture_bytes(
        HORIZONTAL_BOOK,
        &[("001.md", "# 甲\n".as_bytes()), ("002.sjis", SJIS_A)],
    );
    let entries = read_epub(&build_into(dir.path(), "mixed.epub"));
    assert!(
        chapter(&entries, 1).contains("甲"),
        "the UTF-8 chapter is still the first: {}",
        chapter(&entries, 1)
    );
    assert!(
        chapter(&entries, 2).contains("あ"),
        "the swept Shift_JIS chapter must be in the book, decoded: {}",
        chapter(&entries, 2)
    );
}

/// An empty book is not a book: EPUB 3.3 §3.4.12 wants one `itemref` or more,
/// and `compose` had no opinion — it wrote `<spine></spine>` and `build`
/// handed it back as a success. `reports_missing_input_directory` above looks
/// adjacent but is a different failure: that directory does not exist, and an
/// `fs::read_dir` that fails never reaches this.
#[test]
fn an_empty_manuscript_directory_is_refused_rather_than_written() {
    let dir = fixture(HORIZONTAL_BOOK, &[]);
    let out = dir.path().join("empty.epub");
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &out,
    ))
    .unwrap_err();
    assert!(
        matches!(err, Error::NoSources { ref path, .. } if path == &dir.path().join("manuscript")),
        "an empty manuscript must be refused by name, got {err:?}"
    );
    assert!(
        !out.exists(),
        "a refused build must leave no spec-invalid EPUB behind"
    );
}

/// The same refusal by the other route — files are there, none of them is a
/// source. This is what says the emptiness check reads what discovery
/// *collected* rather than whether the directory has entries in it.
#[test]
fn a_directory_holding_nothing_this_crate_reads_is_the_same_refusal() {
    let dir = fixture(
        HORIZONTAL_BOOK,
        &[("notes.txt", "x"), ("README", "x"), ("cover.svg", "x")],
    );
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(matches!(err, Error::NoSources { .. }), "got {err:?}");
}

// ---------------------------------------------------------------------------
// the report — what the renderer saw, chapter by chapter
// ---------------------------------------------------------------------------

/// A book whose chapters are deliberately mixed: some the lexer has nothing
/// to say about, some it does. Which is which is *not* written down here —
/// every rule below asks `render` and compares — so a lexer that stops
/// reporting one of these shapes moves the expectation rather than breaking
/// a hand-copied list. What is written down is that the pool must keep at
/// least one of each, which is what stops the comparison passing on two empty
/// vectors.
const MIXED_CHAPTERS: &[(&str, &str)] = &[
    ("001-clean.md", "# 第一章\n\n本文。\n"),
    ("002-orphan-close.md", "orphan》close\n"),
    ("003-ruby.md", "｜青梅《おうめ》へ。\n"),
    ("004-unclosed.md", "｜青梅《\n"),
    ("005-bare-annotation.md", "前［＃\n"),
];

/// What a chapter's report is expected to hold: its path, the text it was
/// rendered from, and the diagnostics a *direct* render of that text produces.
fn expected_reports(
    dir: &Path,
    chapters: &[(&str, &str)],
) -> Vec<(PathBuf, String, Vec<Diagnostic>)> {
    chapters
        .iter()
        .map(|(name, body)| {
            (
                dir.join("manuscript").join(name),
                (*body).to_owned(),
                render(body, &Options::default()).diagnostics,
            )
        })
        .filter(|(_, _, diagnostics)| !diagnostics.is_empty())
        .collect()
}

fn actual_reports(report: &BuildReport) -> Vec<(PathBuf, String, Vec<Diagnostic>)> {
    report
        .chapters
        .iter()
        .map(|chapter| {
            (
                chapter.path.clone(),
                chapter.text.clone(),
                chapter.diagnostics.clone(),
            )
        })
        .collect()
}

/// The rule the pipeline used to break: `render_all` read `rendered.html` and
/// let `rendered.diagnostics` fall out of scope.
///
/// Stated as an equality against the renderer rather than as "at least one
/// diagnostic arrives", because the defect had degrees — forwarding the first
/// chapter's, or the last one's, or all of them with the paths mixed up would
/// each have satisfied a weaker test. Equality also pins the *order* (spine
/// order) and the text the spans were measured against, which is what the CLI
/// slices to draw a caret.
#[test]
fn a_build_report_carries_exactly_what_a_direct_render_saw() {
    let dir = fixture(HORIZONTAL_BOOK, MIXED_CHAPTERS);
    let (report, out) = report_from(dir.path(), "book.epub");
    let expected = expected_reports(dir.path(), MIXED_CHAPTERS);

    assert!(
        expected.len() >= 2,
        "fewer than two chapters of the pool still raise anything; the canaries are stale and \
         this comparison is about to start passing on empty vectors"
    );
    assert!(
        expected.len() < MIXED_CHAPTERS.len(),
        "every chapter of the pool raises something, so `chapters` holding *all* of them would \
         also pass; the pool must keep a clean chapter to leave out"
    );

    assert_eq!(
        actual_reports(&report),
        expected,
        "the report must be, chapter for chapter, what rendering those chapters directly says"
    );
    assert_eq!(
        report.diagnostic_count(),
        expected
            .iter()
            .map(|(_, _, diagnostics)| diagnostics.len())
            .sum::<usize>(),
        "the count must sum the chapters rather than count them"
    );
    assert!(!report.is_empty(), "a book that raised is not empty");
    assert!(
        out.exists(),
        "a diagnostic is an observation, not a refusal: the EPUB must still be written"
    );
}

#[test]
fn a_clean_book_reports_nothing_at_all() {
    // The other half of the equality above, and the one that says a clean
    // chapter contributes *no entry* rather than an entry with an empty
    // vector — which is what `is_empty` is answering for the CLI.
    let dir = fixture(
        HORIZONTAL_BOOK,
        &[("001.md", "# 第一章\n\n本文。\n"), ("002.md", "後書き。\n")],
    );
    let (report, out) = report_from(dir.path(), "clean.epub");
    assert!(
        report.chapters.is_empty(),
        "a clean book must contribute no chapter entries, got {:?}",
        report.chapters
    );
    assert!(report.is_empty(), "`is_empty` must agree");
    assert_eq!(report.diagnostic_count(), 0, "and so must the count");
    assert!(out.exists(), "the EPUB is written either way");
}

/// Every span a consumer is handed must slice the text it is handed beside it.
///
/// This is the half that makes the report usable rather than merely present:
/// the CLI turns `span.start` into a line and column and asks miette to draw
/// a caret into `chapter.text`. A span measured against anything else — the
/// bytes on disk, the previous chapter, the concatenated book — indexes into
/// this string at a byte that is not a character boundary and takes the
/// reporter down with it.
#[test]
fn every_reported_span_slices_the_text_it_was_reported_with() {
    let dir = fixture(HORIZONTAL_BOOK, MIXED_CHAPTERS);
    let (report, _) = report_from(dir.path(), "book.epub");
    let mut checked = 0usize;
    for chapter in &report.chapters {
        for diagnostic in &chapter.diagnostics {
            if diagnostic.source() != DiagnosticSource::Source {
                continue;
            }
            let span = diagnostic.span();
            let range = span.start as usize..span.end as usize;
            checked += 1;
            assert!(
                chapter.text.get(range.clone()).is_some(),
                "{}: {span:?} does not slice the {} bytes of text reported with it \
                 ({diagnostic:?})",
                chapter.path.display(),
                chapter.text.len()
            );
        }
    }
    assert!(
        checked > 0,
        "no source-anchored diagnostic in the pool; this rule would pass on a report that \
         carried no spans at all"
    );
}

/// 「あ》あ」 in Shift_JIS: `あ` is `0x82 0xA0`, `》` is `0x81 0x74`.
///
/// A two-byte lead character before the offending one is the whole point —
/// `》` starts at byte 2 on disk and at byte 3 once decoded, so a report that
/// handed back the file's bytes instead of the decoded string would be off by
/// one *and* land mid-character.
const SJIS_WITH_ORPHAN_CLOSE: &[u8] = b"\x82\xA0\x81\x74\x82\xA0";

#[test]
fn a_shift_jis_chapter_reports_against_the_decoded_text() {
    let dir = fixture_bytes(HORIZONTAL_BOOK, &[("001.sjis", SJIS_WITH_ORPHAN_CLOSE)]);
    // Named outright rather than swept for: what this rule is about is the
    // decoded text the spans are measured against, and the single-file input
    // reaches the `.sjis` branch without depending on what discovery collects.
    let input = dir.path().join("manuscript").join("001.sjis");
    let report = build(&BuildOptions::new(
        &input,
        &dir.path().join("book.toml"),
        &dir.path().join("sjis.epub"),
    ))
    .expect("a decodable Shift_JIS chapter builds");

    assert_eq!(report.chapters.len(), 1, "one chapter, one entry");
    let chapter = &report.chapters[0];
    assert_eq!(
        chapter.path, input,
        "the entry names the file as discovered"
    );
    assert_eq!(
        chapter.text, "あ》あ",
        "the text beside the spans must be the decoded string"
    );
    let diagnostic = chapter
        .diagnostics
        .first()
        .expect("an orphan 》 must still be reported through the Shift_JIS branch");
    let span = diagnostic.span();
    let slice = chapter
        .text
        .get(span.start as usize..span.end as usize)
        .expect("the span must slice the decoded text, not the bytes on disk");
    assert!(
        slice.contains('》'),
        "the span must cover the character complained about, got {slice:?} from {diagnostic:?}"
    );
}

// ---------------------------------------------------------------------------
// the error surface — an opaque cause is still a cause
// ---------------------------------------------------------------------------

/// Every `Display` down the `source()` chain, starting with the error itself.
fn chain_of(err: &dyn StdError) -> Vec<String> {
    let mut out = vec![err.to_string()];
    let mut next = err.source();
    while let Some(cause) = next {
        out.push(cause.to_string());
        next = cause.source();
    }
    out
}

/// One failure the public entry point can be driven to, and whether a
/// dependency's own diagnosis has to survive underneath it.
#[derive(Debug)]
struct Failure {
    what: &'static str,
    err: Error,
    cause: bool,
}

/// The failures a consumer can reach through `build`, each built here rather
/// than constructed: `Error` is `#[non_exhaustive]` and its `Cause`-bearing
/// variants have no public constructor, so a caller only ever meets one the
/// pipeline raised.
///
/// `Error::Package` and `Error::XmlBuild` are missing because nothing can
/// drive `build` to them — the ZIP and the XML are written into an in-memory
/// sink whose `io::Write` cannot fail (ADR-0018 records the same reasoning for
/// the coverage carve-out on those two modules).
fn reachable_failures() -> Vec<Failure> {
    let mut all = manifest_failures();
    all.extend(source_failures());
    all
}

/// One case: what it is, the three paths to hand `build`, and whether a
/// dependency's own diagnosis has to survive underneath the refusal.
type Case = (&'static str, PathBuf, PathBuf, PathBuf, bool);

fn run(cases: Vec<Case>) -> Vec<Failure> {
    cases
        .into_iter()
        .map(|(what, input, metadata, output, cause)| Failure {
            what,
            err: build(&BuildOptions::new(&input, &metadata, &output)).expect_err(what),
            cause,
        })
        .collect()
}

/// The refusals `book.toml` and the shape of the input can raise, before a
/// single chapter byte is read.
fn manifest_failures() -> Vec<Failure> {
    let missing_dir = fixture(HORIZONTAL_BOOK, &[]);
    let malformed_toml = fixture("= not valid toml =", &[("001.md", "x")]);
    let unknown_key = fixture(
        &format!("{HORIZONTAL_BOOK}spien = [\"001.md\"]\n"),
        &[("001.md", "x")],
    );
    let bad_language = fixture(
        "title = \"T\"\ncreator = \"A\"\nlanguage = \"japanese\"\n",
        &[("001.md", "x")],
    );
    let spined_file = fixture(&book_with_spine(r#""001.md""#), &[("001.md", "x")]);
    let empty_dir = fixture(HORIZONTAL_BOOK, &[]);

    run(vec![
        (
            "a manuscript root that is not there",
            missing_dir.path().join("does-not-exist"),
            missing_dir.path().join("book.toml"),
            missing_dir.path().join("o.epub"),
            true,
        ),
        (
            "a book.toml the parser rejects",
            malformed_toml.path().join("manuscript"),
            malformed_toml.path().join("book.toml"),
            malformed_toml.path().join("o.epub"),
            true,
        ),
        (
            "a book.toml key this crate does not implement",
            unknown_key.path().join("manuscript"),
            unknown_key.path().join("book.toml"),
            unknown_key.path().join("o.epub"),
            true,
        ),
        (
            "a language tag this crate rejects itself",
            bad_language.path().join("manuscript"),
            bad_language.path().join("book.toml"),
            bad_language.path().join("o.epub"),
            false,
        ),
        (
            "a spine beside a single-file input",
            spined_file.path().join("manuscript").join("001.md"),
            spined_file.path().join("book.toml"),
            spined_file.path().join("o.epub"),
            false,
        ),
        (
            "a manuscript directory with no sources in it",
            empty_dir.path().join("manuscript"),
            empty_dir.path().join("book.toml"),
            empty_dir.path().join("o.epub"),
            false,
        ),
    ])
}

/// The refusals the chapter bytes and the output path can raise.
fn source_failures() -> Vec<Failure> {
    let blocked_output = fixture(HORIZONTAL_BOOK, &[("001.md", "x")]);
    let bad_utf8 = fixture_bytes(HORIZONTAL_BOOK, &[("001.md", &[0x80, 0x81])]);
    let bad_sjis = fixture_bytes(HORIZONTAL_BOOK, &[("001.sjis", &[0xFF, 0xFF, 0xFF])]);

    // The output path aims *inside* a regular file, so creating its parent
    // directory fails.
    let blocker = blocked_output.path().join("blocker");
    fs::write(&blocker, b"not a directory").expect("write blocker");

    run(vec![
        (
            "an output path under a regular file",
            blocked_output.path().join("manuscript"),
            blocked_output.path().join("book.toml"),
            blocker.join("o.epub"),
            true,
        ),
        (
            "a .md chapter that is not UTF-8",
            bad_utf8.path().join("manuscript"),
            bad_utf8.path().join("book.toml"),
            bad_utf8.path().join("o.epub"),
            true,
        ),
        (
            "a .sjis chapter the decoder rejects",
            bad_sjis.path().join("manuscript").join("001.sjis"),
            bad_sjis.path().join("book.toml"),
            bad_sjis.path().join("o.epub"),
            true,
        ),
    ])
}

/// Boxing a dependency's error behind an opaque `Cause` must change what a
/// consumer can *read*, not what they can *reach*.
///
/// Nothing asked before this: while the fields were `toml::de::Error` and
/// `zip::ZipError`, `#[source]` on a concrete type made the chain true by
/// construction, so replacing them with a wrapper that swallowed the chain —
/// a `Display`-only newtype, a `source()` that returned `None`, an inherent
/// `source` shadowing the trait one — would have satisfied every gate in the
/// workspace and left `anyhow`, `miette` and `{:?}` reporting a bare
/// "failed to parse book metadata" with no parser message under it.
#[test]
fn every_error_a_build_can_raise_still_hands_over_the_chain_underneath_it() {
    let failures = reachable_failures();
    assert!(
        failures.len() >= 9,
        "only {} failures reached; this rule is a sweep, not a sample",
        failures.len()
    );
    let mut with_cause = 0usize;
    for Failure { what, err, cause } in &failures {
        let chain = chain_of(err);
        assert_eq!(
            chain[0],
            err.to_string(),
            "{what}: the head of the chain is the error itself"
        );
        assert_eq!(
            chain.len() > 1,
            *cause,
            "{what}: expected a cause underneath: {cause}, got the chain {chain:?}"
        );
        if !cause {
            continue;
        }
        with_cause += 1;
        assert_ne!(
            chain[1], chain[0],
            "{what}: the cause must be the dependency's own diagnosis, not this crate's message \
             repeated one level down"
        );
        assert!(
            !chain[1].is_empty(),
            "{what}: an empty cause is a chain a consumer can walk and learn nothing from"
        );
        // No level restates the one above it. This is what says `Cause`
        // *stands in for* the error it holds rather than sitting on top of
        // it: a `source()` that returned the boxed error instead of that
        // error's own source would print the dependency's message twice,
        // once as the `Cause` and once as itself.
        for pair in chain.windows(2) {
            assert_ne!(
                pair[0], pair[1],
                "{what}: a level of the chain restates the one above it: {chain:?}"
            );
        }
        // The wrapper has to be invisible in both renderings. `Cause` delegates
        // `Display` *and* `Debug` to the error it holds for exactly this
        // reason: a reporter that prints the type name would show a consumer a
        // name that is not the failure's.
        for line in &chain {
            assert!(
                !line.contains("Cause"),
                "{what}: the opaque wrapper leaked into a rendering: {chain:?}"
            );
        }
        assert!(
            !format!("{err:?}").contains("Cause("),
            "{what}: the opaque wrapper leaked into `Debug`: {err:?}"
        );
    }
    assert!(
        with_cause >= 4,
        "only {with_cause} failures carried a cause; the chain half of this rule must not \
         narrow to the one variant somebody remembered"
    );
}

#[test]
fn the_toml_parsers_own_diagnosis_survives_the_boxing() {
    // The sharpest of the three: `toml::de::Error` is the one whose message
    // carries the line and column, and it is the one whose type left the
    // public fields. Losing it turns "TOML parse error at line 1, column 18"
    // into "failed to parse book metadata at /tmp/…" and nothing else.
    let dir = fixture("= not valid toml =", &[("001.md", "x")]);
    let err = build(&BuildOptions::new(
        &dir.path().join("manuscript"),
        &dir.path().join("book.toml"),
        &dir.path().join("o.epub"),
    ))
    .unwrap_err();
    assert!(matches!(err, Error::MetadataParse { .. }));
    let chain = chain_of(&err);
    assert!(
        chain[1].contains("TOML parse error"),
        "the cause must still be the parser's own report, got {chain:?}"
    );
    assert!(
        chain[1].contains("line 1"),
        "and it must still carry the position it found, got {chain:?}"
    );
}
