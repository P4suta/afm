//! Hides 青空文庫 trigger characters inside CommonMark fenced code blocks.
//!
//! The sibling parser is CommonMark-blind and rewrites every candidate
//! trigger into an internal sentinel. A fenced code block is literal
//! CommonMark, so this module asks the same comrak configuration as the
//! render pass which byte ranges are fenced, masks triggers only inside
//! those ranges, and snapshots the fields comrak normalised from the
//! original source.
//!
//! Restoration is structural rather than textual: after source positions
//! have been rebound to the caller's source, a final fenced `CodeBlock`
//! receives the snapshot with the exact same byte range. There is no shared
//! cursor, so a missing or reshaped block cannot consume the values of a
//! later one. An unmatched block is fail-closed: an introduced mask becomes
//! U+FFFD rather than reaching HTML or IR as a private-use codepoint.
//!
//! **Indented code blocks are deliberately not masked.** Their boundaries
//! depend on paragraph context. A notation inside one becomes a construct
//! sentinel that the existing literal-context recovery in the AST and IR
//! walkers writes back as source.
//!
//! **A source that already contains [`MASK_CHAR`] still stands that
//! codepoint down.** No additional U+E000 is introduced. Fenced triggers are
//! hidden with same-width U+FFFD instead, then a matching snapshot restores
//! the author's fields. The author's U+E000 therefore comes back as written
//! without letting fenced constructs shift the shared construct cursor.

use core::ops::Range;
use std::borrow::Cow;

use comrak::nodes::{AstNode, NodeValue};

use crate::constructs::is_sentinel_char;
use crate::verbatim_regions;

/// Distinct from the four construct sentinels (U+E001..U+E004), so masking
/// cannot collide with them on a source that does not already contain it.
pub(crate) const MASK_CHAR: char = '\u{E000}';

const REPLACEMENT_CHAR: char = '\u{FFFD}';

/// Mirrors the sibling tokeniser; if its trigger list grows, so must this.
const AOZORA_TRIGGERS: &[char] = &[
    '｜', '《', '》', '≪', '≫', '［', '］', '＃', '※', '〔', '〕', '「', '」',
];

#[derive(Debug, Default)]
pub(crate) struct FencedCodeBlocks {
    snapshots: Vec<FencedCodeBlock>,
    /// True only when this call introduced at least one [`MASK_CHAR`].
    introduced_masks: bool,
    /// Whether a fence marker occurs in the source at all — the one thing
    /// that has to be true for the *rendered* text to hold a fenced block.
    /// Substitution only ever replaces a run of source with one sentinel, so
    /// it cannot add a marker; a source without one cannot grow a fence, and
    /// [`restore_ast`] can skip its walk. It can, however, *remove* the
    /// backtick that CommonMark §4.5 forbids in a backtick fence's info
    /// string, and that turns a line the source read as prose into a fence
    /// nothing snapshotted.
    source_may_fence: bool,
}

impl FencedCodeBlocks {
    /// Whether downstream construct-owned strings must neutralise U+E000.
    ///
    /// The mask normally never leaves a restored fenced `CodeBlock`, but a
    /// malformed Aozora construct can begin before a fence and end after it.
    /// Such a construct owns the intervening mask bytes rather than the final
    /// AST block. Callers use this bit to replace only masks introduced by
    /// this pass; an author-written U+E000 keeps the stand-down contract.
    pub(crate) const fn introduced_masks(&self) -> bool {
        self.introduced_masks
    }
}

#[derive(Debug)]
struct FencedCodeBlock {
    range: Range<usize>,
    info: String,
    literal: String,
}

/// Parse `source` with the render's actual comrak options, snapshot every
/// fenced block, and mask 青空文庫 triggers only inside those exact ranges.
///
/// Every trigger, [`MASK_CHAR`] and U+FFFD is three UTF-8 bytes, so
/// substitution preserves the byte ranges and source positions used by the
/// later parse.
#[must_use]
pub(crate) fn mask_code_block_triggers<'a>(
    source: &'a str,
    options: &comrak::Options<'_>,
) -> (Cow<'a, str>, FencedCodeBlocks) {
    if !source.contains(['`', '~']) {
        return (Cow::Borrowed(source), FencedCodeBlocks::default());
    }

    let snapshots = snapshots(source, options);
    if snapshots.is_empty() {
        return (
            Cow::Borrowed(source),
            FencedCodeBlocks {
                snapshots,
                introduced_masks: false,
                source_may_fence: true,
            },
        );
    }

    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut changed = false;
    let replacement = if source.contains(MASK_CHAR) {
        REPLACEMENT_CHAR
    } else {
        MASK_CHAR
    };
    for snapshot in &snapshots {
        out.push_str(&source[cursor..snapshot.range.start]);
        changed |= mask_triggers(&source[snapshot.range.clone()], &mut out, replacement);
        cursor = snapshot.range.end;
    }
    out.push_str(&source[cursor..]);
    let introduced_masks = changed && replacement == MASK_CHAR;

    if changed {
        (
            Cow::Owned(out),
            FencedCodeBlocks {
                snapshots,
                introduced_masks,
                source_may_fence: true,
            },
        )
    } else {
        (
            Cow::Borrowed(source),
            FencedCodeBlocks {
                snapshots,
                introduced_masks,
                source_may_fence: true,
            },
        )
    }
}

fn snapshots(source: &str, options: &comrak::Options<'_>) -> Vec<FencedCodeBlock> {
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, source, options);
    let line_starts = verbatim_regions::line_starts(source);
    let mut snapshots: Vec<FencedCodeBlock> = root
        .descendants()
        .filter_map(|node| {
            let data = node.data.borrow();
            let NodeValue::CodeBlock(code) = &data.value else {
                return None;
            };
            if !code.fenced {
                return None;
            }
            Some(FencedCodeBlock {
                range: verbatim_regions::byte_range(source, &line_starts, data.sourcepos)?,
                // comrak has already trimmed/unescaped the info string and
                // normalised the literal's line endings. Preserve that
                // compiler-owned representation, except for author-written
                // construct sentinels whose public contract is U+FFFD.
                info: neutralize_construct_sentinels(&code.info),
                literal: neutralize_construct_sentinels(&code.literal),
            })
        })
        .collect();
    snapshots.sort_unstable_by_key(|snapshot| snapshot.range.start);
    snapshots
}

fn neutralize_construct_sentinels(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if is_sentinel_char(ch) {
                REPLACEMENT_CHAR
            } else {
                ch
            }
        })
        .collect()
}

fn mask_triggers(text: &str, out: &mut String, replacement: char) -> bool {
    let mut changed = false;
    for ch in text.chars() {
        if AOZORA_TRIGGERS.contains(&ch) {
            out.push(replacement);
            changed = true;
        } else {
            out.push(ch);
        }
    }
    changed
}

/// Restore fenced fields by exact source range.
///
/// `root` must already have had its source positions rebound to `source`.
/// Exact lookup makes restoration independent of traversal depth and output
/// shape. In particular, an IR walker that truncates an over-deep container
/// cannot shift a later code block's values.
///
/// The walk is skipped only when the source holds no fence marker at all.
/// Skipping it whenever there was nothing to *restore* was the same mistake
/// in the other direction: a source with no fenced block still reaches this
/// with an unmatched one whenever substitution removed the backtick §4.5
/// forbids from a backtick fence's info string, and that block is exactly the
/// one whose fail-closed pass below is the only thing standing between a
/// construct sentinel and the HTML.
pub(crate) fn restore_ast<'a>(root: &'a AstNode<'a>, source: &str, fenced: &FencedCodeBlocks) {
    if !fenced.source_may_fence {
        return;
    }
    let line_starts = verbatim_regions::line_starts(source);
    for node in root.descendants() {
        let mut data = node.data.borrow_mut();
        let sourcepos = data.sourcepos;
        let NodeValue::CodeBlock(code) = &mut data.value else {
            continue;
        };
        if !code.fenced {
            continue;
        }
        let snapshot = verbatim_regions::byte_range(source, &line_starts, sourcepos)
            .and_then(|range| find_snapshot(&fenced.snapshots, &range));
        if let Some(snapshot) = snapshot {
            code.info.clone_from(&snapshot.info);
            code.literal.clone_from(&snapshot.literal);
            continue;
        }

        // Do not borrow a different snapshot merely because it is next in
        // document order. Author-written construct sentinels are always
        // reserved; U+E000 is replaced only when this call introduced it,
        // so the raw-U+E000 stand-down contract remains intact.
        code.info = neutralize_unmatched_fence(&code.info, fenced.introduced_masks);
        code.literal = neutralize_unmatched_fence(&code.literal, fenced.introduced_masks);
    }
}

fn find_snapshot<'a>(
    snapshots: &'a [FencedCodeBlock],
    range: &Range<usize>,
) -> Option<&'a FencedCodeBlock> {
    snapshots
        .binary_search_by(|snapshot| {
            snapshot
                .range
                .start
                .cmp(&range.start)
                .then(snapshot.range.end.cmp(&range.end))
        })
        .ok()
        .map(|idx| &snapshots[idx])
}

fn neutralize_unmatched_fence(text: &str, introduced_masks: bool) -> String {
    text.chars()
        .map(|ch| {
            if is_sentinel_char(ch) || (introduced_masks && ch == MASK_CHAR) {
                REPLACEMENT_CHAR
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use comrak::nodes::Sourcepos;

    fn options() -> comrak::Options<'static> {
        crate::Options::default().comrak()
    }

    fn mask_owned(src: &str) -> (String, FencedCodeBlocks) {
        let (masked, fenced) = mask_code_block_triggers(src, &options());
        (masked.into_owned(), fenced)
    }

    fn code_fields<'a>(root: &'a AstNode<'a>) -> Vec<(String, String)> {
        root.descendants()
            .filter_map(|node| {
                let data = node.data.borrow();
                match &data.value {
                    NodeValue::CodeBlock(code) if code.fenced => {
                        Some((code.info.clone(), code.literal.clone()))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    #[test]
    fn no_fence_is_a_borrowed_fast_path() {
        let src = "｜青梅《おうめ》";
        let (masked, fenced) = mask_code_block_triggers(src, &options());
        assert!(matches!(masked, Cow::Borrowed(_)));
        assert!(fenced.snapshots.is_empty());
    }

    #[test]
    fn compiler_ranges_mask_container_fences_and_restore_their_fields() {
        for src in [
            "> ```漢字《かんじ》\n> ｜body《literal》\n> ```\n",
            "- item\n\n  ~~~漢字《かんじ》 extra《ignored》\n  ｜body《literal》\n  ~~~\n",
        ] {
            let (masked, fenced) = mask_owned(src);
            assert_eq!(masked.len(), src.len());
            assert!(
                !masked.contains(['｜', '《', '》']),
                "a fenced trigger reached the lexer: {masked:?}"
            );
            let arena = comrak::Arena::new();
            let opts = options();
            let root = comrak::parse_document(&arena, &masked, &opts);
            restore_ast(root, src, &fenced);
            let fields = code_fields(root);
            assert_eq!(fields.len(), 1, "{src:?}");
            assert!(fields[0].0.contains("漢字《かんじ》"));
            assert!(fields[0].1.contains("｜body《literal》"));
        }
    }

    #[test]
    fn every_parser_trigger_is_hidden_and_structurally_restored() {
        let triggers: String = AOZORA_TRIGGERS.iter().collect();
        let src = format!("```{triggers}\n{triggers}\n```\n");
        let (masked, fenced) = mask_owned(&src);
        assert_eq!(masked.len(), src.len());
        assert!(
            AOZORA_TRIGGERS
                .iter()
                .all(|trigger| !masked.contains(*trigger)),
            "a parser trigger reached the lexer: {masked:?}"
        );

        let arena = comrak::Arena::new();
        let opts = options();
        let root = comrak::parse_document(&arena, &masked, &opts);
        restore_ast(root, &src, &fenced);
        assert_eq!(
            code_fields(root),
            vec![(triggers.clone(), format!("{triggers}\n"))]
        );
    }

    #[test]
    fn multiple_and_unclosed_fences_restore_by_range() {
        let src = "```\n｜first\n```\n\n~~~lang《x》\n［second］\n";
        let (masked, fenced) = mask_owned(src);
        let arena = comrak::Arena::new();
        let opts = options();
        let root = comrak::parse_document(&arena, &masked, &opts);
        restore_ast(root, src, &fenced);
        assert_eq!(
            code_fields(root),
            vec![
                (String::new(), "｜first\n".to_owned()),
                ("lang《x》".to_owned(), "［second］\n".to_owned()),
            ]
        );
    }

    #[test]
    fn every_commonmark_line_ending_is_restored_from_the_original_snapshot() {
        for ending in ["\n", "\r\n", "\r"] {
            let src = format!("```lang《x》{ending}｜body{ending}```{ending}");
            let (masked, fenced) = mask_owned(&src);
            let arena = comrak::Arena::new();
            let opts = options();
            let root = comrak::parse_document(&arena, &masked, &opts);
            restore_ast(root, &src, &fenced);
            assert_eq!(
                code_fields(root),
                vec![("lang《x》".to_owned(), format!("｜body{ending}"))],
                "{ending:?}"
            );
        }
    }

    #[test]
    fn raw_construct_sentinels_and_info_entities_are_neutralized() {
        for sentinel in ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
            let src = format!("```lang&#x{:X};\nraw{sentinel}\n```\n", sentinel as u32);
            let (masked, fenced) = mask_owned(&src);
            let arena = comrak::Arena::new();
            let opts = options();
            let root = comrak::parse_document(&arena, &masked, &opts);
            restore_ast(root, &src, &fenced);
            let fields = code_fields(root);
            assert_eq!(fields, vec![("lang�".to_owned(), "raw�\n".to_owned())]);
        }
    }

    #[test]
    fn pre_existing_mask_uses_replacement_mask_and_comes_back_as_written() {
        let src = "\u{E000}\n```\n｜trigger\u{E000}\n```\n";
        let (masked, fenced) = mask_code_block_triggers(src, &options());
        assert!(matches!(masked, Cow::Owned(_)));
        assert!(masked.contains("�trigger\u{E000}"));
        assert!(!fenced.introduced_masks);
        let arena = comrak::Arena::new();
        let opts = options();
        let root = comrak::parse_document(&arena, &masked, &opts);
        restore_ast(root, src, &fenced);
        assert_eq!(
            code_fields(root),
            vec![(String::new(), "｜trigger\u{E000}\n".to_owned())]
        );
    }

    /// `render_blocks` reduced a 61-byte artifact to a lone fence line whose
    /// info string is a ruby whose reading is a backtick. The line
    /// is prose in the source — CommonMark §4.5 admits no backtick in a
    /// backtick fence's info string — so nothing is snapshotted. Substituting
    /// the ruby takes that backtick away, and the text comrak parses opens a
    /// fence whose info string *is* the sentinel. The fail-closed pass covers
    /// it; skipping the walk when there was nothing to restore is what let it
    /// through to the HTML.
    #[test]
    fn a_fence_substitution_created_is_neutralized_though_nothing_was_snapshotted() {
        let src = "```｜⢅《`》";
        let (masked, fenced) = mask_owned(src);
        assert_eq!(masked, src, "prose holds no fenced range to mask");
        assert!(fenced.snapshots.is_empty());
        assert!(fenced.source_may_fence, "the source does hold a marker");

        // What the render pass hands comrak: the ruby replaced by its
        // sentinel, which leaves a line that reads as a fence.
        let tiled = "```\u{E001}";
        let arena = comrak::Arena::new();
        let opts = options();
        let root = comrak::parse_document(&arena, tiled, &opts);
        restore_ast(root, src, &fenced);
        assert_eq!(code_fields(root), vec![("�".to_owned(), String::new())]);
    }

    #[test]
    fn a_source_without_a_fence_marker_skips_the_walk() {
        let (_, fenced) = mask_owned("｜青梅《おうめ》");
        assert!(
            !fenced.source_may_fence,
            "substitution cannot add a marker the source lacks"
        );
    }

    #[test]
    fn a_mismatch_fails_closed_without_consuming_the_later_snapshot() {
        let src = "```\n｜first\n```\n\n```\n［second］\n```\n";
        let (masked, fenced) = mask_owned(src);
        let arena = comrak::Arena::new();
        let opts = options();
        let root = comrak::parse_document(&arena, &masked, &opts);
        let first = root
            .descendants()
            .find(|node| {
                matches!(
                    &node.data.borrow().value,
                    NodeValue::CodeBlock(code) if code.fenced
                )
            })
            .expect("first fenced block");
        first.data.borrow_mut().sourcepos = Sourcepos::from((99, 1, 99, 1));

        restore_ast(root, src, &fenced);
        assert_eq!(
            code_fields(root),
            vec![
                (String::new(), "�first\n".to_owned()),
                (String::new(), "［second］\n".to_owned()),
            ]
        );
    }
}
