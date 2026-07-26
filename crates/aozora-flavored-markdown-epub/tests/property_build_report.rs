//! The pipeline is diagnostic-preserving, for any manuscript.
//!
//! `build_epub.rs` states the same thing over a hand-written pool of chapters,
//! which is enough to catch a `render_all` that drops the vector outright and
//! not enough to catch the next narrower version of the same defect: keeping
//! only the last chapter's, only the first diagnostic of each, only the ones
//! whose severity is `Error`, or reporting them against the wrong chapter when
//! two files decode to different lengths. Every one of those passes a pool of
//! five and fails a book drawn at random.
//!
//! The comparison is against the renderer itself rather than a recorded
//! expectation, so a lexer that starts or stops reporting a shape moves both
//! sides at once and this stays a statement about the *pipeline*.

use std::fs;
use std::path::Path;

use aozora_flavored_markdown::{Options, render};
use aozora_flavored_markdown_epub::{BuildOptions, build};
use aozora_flavored_markdown_test_support::{config, generators};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

const BOOK: &str = "title = \"性質\"\ncreator = \"著者\"\nlanguage = \"ja\"\n";

/// Chapter file names are zero-padded so the lexicographic order discovery
/// sorts by is the order they were drawn in — the spine order the report is
/// supposed to follow.
fn chapter_name(idx: usize) -> String {
    format!("{:03}.md", idx + 1)
}

fn write_book(dir: &Path, chapters: &[String]) {
    fs::write(dir.join("book.toml"), BOOK).expect("write book.toml");
    let manuscript = dir.join("manuscript");
    fs::create_dir(&manuscript).expect("create manuscript dir");
    for (idx, body) in chapters.iter().enumerate() {
        fs::write(manuscript.join(chapter_name(idx)), body).expect("write chapter");
    }
}

/// A book of one to four chapters, each drawn from a different corner of the
/// input space: the Aozora atom pool (which produces unbalanced notation as
/// readily as balanced), plain kanji text the lexer has nothing to say about,
/// and the CommonMark-with-Aozora-inside pool. The mixture matters — the
/// property is about which chapters end up in the report, so a draw that made
/// every chapter noisy could not tell a correct report from one that lists
/// every chapter unconditionally.
fn chapters() -> impl Strategy<Value = Vec<String>> {
    let chapter = prop_oneof![
        generators::aozora_fragment(6),
        generators::kanji_fragment(8),
        generators::commonmark_adversarial(),
        generators::pathological_aozora(4),
    ];
    prop::collection::vec(chapter, 1..=4)
}

proptest! {
    #![proptest_config(config::default())]

    /// What `build` reports is what rendering those same chapters reports:
    /// same chapters, same order, same diagnostics, same text underneath the
    /// spans.
    #[test]
    fn a_build_reports_every_chapter_diagnostic_and_no_others(chapters in chapters()) {
        let dir = tempfile::tempdir().expect("create tempdir");
        write_book(dir.path(), &chapters);

        let report = build(&BuildOptions::new(
            &dir.path().join("manuscript"),
            &dir.path().join("book.toml"),
            &dir.path().join("book.epub"),
        ))
        .map_err(|e| TestCaseError::fail(format!("rendering is infallible, so a book of \
             well-formed UTF-8 chapters must build: {e}")))?;

        let expected: Vec<(String, String, usize)> = chapters
            .iter()
            .enumerate()
            .map(|(idx, body)| {
                let diagnostics = render(body, &Options::default()).diagnostics;
                (chapter_name(idx), body.clone(), diagnostics.len())
            })
            .filter(|(_, _, count)| *count > 0)
            .collect();

        let actual: Vec<(String, String, usize)> = report
            .chapters
            .iter()
            .map(|chapter| {
                (
                    chapter
                        .path
                        .file_name()
                        .expect("a discovered chapter has a file name")
                        .to_string_lossy()
                        .into_owned(),
                    chapter.text.clone(),
                    chapter.diagnostics.len(),
                )
            })
            .collect();

        prop_assert_eq!(
            &actual,
            &expected,
            "the report must name the chapters that raised, in spine order, with the text \
             they were rendered from"
        );
        prop_assert_eq!(
            report.diagnostic_count(),
            expected.iter().map(|(_, _, count)| count).sum::<usize>(),
            "the count must sum the chapters"
        );
        prop_assert_eq!(
            report.is_empty(),
            expected.is_empty(),
            "`is_empty` must mean no chapter raised, not no chapter existed"
        );

        // The diagnostics themselves, not merely how many: forwarding the
        // right count of the wrong chapter's would pass everything above.
        for chapter in &report.chapters {
            let body = chapters[chapter
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<usize>().ok())
                .expect("a chapter file stem is its 1-based index")
                - 1]
                .clone();
            prop_assert_eq!(
                &chapter.diagnostics,
                &render(&body, &Options::default()).diagnostics,
                "chapter {} must carry its own diagnostics",
                chapter.path.display()
            );
            for diagnostic in &chapter.diagnostics {
                let span = diagnostic.span();
                prop_assert!(
                    chapter
                        .text
                        .get(span.start as usize..span.end as usize)
                        .is_some(),
                    "{:?} does not slice the text reported with it",
                    span
                );
            }
        }
    }
}
