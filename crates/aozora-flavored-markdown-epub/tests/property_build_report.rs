//! The pipeline is diagnostic-preserving, for any manuscript.

#![allow(
    clippy::expect_used,
    reason = "integration-test setup helpers stop immediately when a temporary fixture cannot be created"
)]
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
//!
//! The draw covers the order too, because `book.toml`'s `spine` made order a
//! decision rather than a consequence of `sort`. See [`book`].

use std::fs;
use std::path::Path;

use aozora_flavored_markdown::{Options, render};
use aozora_flavored_markdown_epub::{BuildOptions, build};
use aozora_flavored_markdown_test_support::{config, generators};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

const BOOK: &str = "title = \"性質\"\ncreator = \"著者\"\nlanguage = \"ja\"\n";

/// Chapter file names are zero-padded so the lexicographic order the sweep
/// sorts by is the order they were drawn in — the spine order the report is
/// supposed to follow when `book.toml` names none of its own.
fn chapter_name(idx: usize) -> String {
    format!("{:03}.md", idx + 1)
}

/// `spine` absent leaves the order to the sweep; present, it *is* the order,
/// and the list of chapters with it.
fn write_book(dir: &Path, chapters: &[String], spine: Option<&[usize]>) {
    let book = spine.map_or_else(
        || BOOK.to_owned(),
        |order| {
            let listed: Vec<String> = order
                .iter()
                .map(|&idx| format!("{:?}", chapter_name(idx)))
                .collect();
            format!("{BOOK}spine = [{}]\n", listed.join(", "))
        },
    );
    fs::write(dir.join("book.toml"), book).expect("write book.toml");
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

/// A book, and the order `book.toml` asks for it in: either nothing (the
/// sweep decides, lexicographically) or a non-empty subset of the chapters in
/// a drawn order.
///
/// Quantifying over orderings is what makes this a statement about the
/// manifest rather than about `sort`. Until `spine` existed, sorting was the
/// only order the code could produce, so every rule below held just as well
/// for an implementation that read the manifest and one that ignored it —
/// which is what shipped. A *subset* rather than a permutation carries the
/// other half: a file on disk the spine leaves out is not a chapter, so an
/// implementation that appended the leftovers, or fell back to the sweep when
/// the spine was short, fails the equality below rather than passing it.
fn book() -> impl Strategy<Value = (Vec<String>, Option<Vec<usize>>)> {
    chapters().prop_flat_map(|chapters| {
        let indices: Vec<usize> = (0..chapters.len()).collect();
        let order = prop_oneof![
            Just(None),
            prop::sample::subsequence(indices, 1..=chapters.len())
                .prop_shuffle()
                .prop_map(Some),
        ];
        (Just(chapters), order)
    })
}

proptest! {
    #![proptest_config(config::default())]

    /// What `build` reports is what rendering those same chapters reports:
    /// same chapters, same order the manifest asked for, same diagnostics,
    /// same text underneath the spans.
    #[test]
    fn a_build_reports_every_chapter_diagnostic_and_no_others((chapters, spine) in book()) {
        let dir = tempfile::tempdir().expect("create tempdir");
        write_book(dir.path(), &chapters, spine.as_deref());

        let report = build(&BuildOptions::new(
            &dir.path().join("manuscript"),
            &dir.path().join("book.toml"),
            &dir.path().join("book.epub"),
        ))
        .map_err(|e| TestCaseError::fail(format!("rendering is infallible, so a book of \
             well-formed UTF-8 chapters must build: {e}")))?;

        // The order the book is in: whatever `book.toml` asked for, or the
        // sweep's when it asked for nothing.
        let reading_order = spine.unwrap_or_else(|| (0..chapters.len()).collect());

        let expected: Vec<(String, String, usize)> = reading_order
            .iter()
            .map(|&idx| {
                let body = &chapters[idx];
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
