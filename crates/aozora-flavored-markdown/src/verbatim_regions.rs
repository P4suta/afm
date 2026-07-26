//! Lifts every region comrak has already claimed out of the canonicaliser's
//! reach, for `crate::serialize` — the stronger half of the split
//! `crate::code_block_mask` documents. A whole region leaves as one
//! placeholder, so the line structure inside it (a blank-line run, a rule row,
//! the `> ` a container puts in front of every line) never reaches the
//! canonicaliser at all. comrak locating the regions is what keeps this a
//! splice rather than the block parser a fence scanner would have to become.

use core::iter::once;
use core::ops::Range;

use comrak::nodes::{NodeHeading, NodeValue, Sourcepos};

use crate::Options;
use crate::code_block_mask::MASK_CHAR;

/// Returns the lifted regions in source order, for [`restore`].
#[must_use]
pub(crate) fn protect(source: &str) -> (String, Vec<&str>) {
    let ranges = verbatim_ranges(source);
    let mut text = String::with_capacity(source.len());
    let mut originals = Vec::with_capacity(ranges.len());
    let mut cursor = 0;
    for range in ranges {
        text.push_str(&source[cursor..range.start]);
        text.push(MASK_CHAR);
        cursor = range.end;
        originals.push(&source[range]);
    }
    text.push_str(&source[cursor..]);
    (text, originals)
}

/// Puts each original back where its placeholder came out; extras flow
/// through, as in the character mask.
#[must_use]
pub(crate) fn restore(canonical: &str, originals: &[&str]) -> String {
    let mut out = String::with_capacity(canonical.len());
    let mut idx = 0;
    for ch in canonical.chars() {
        match originals.get(idx) {
            Some(original) if ch == MASK_CHAR => {
                out.push_str(original);
                idx += 1;
            }
            _ => out.push(ch),
        }
    }
    out
}

// The bytes comrak reads as anything but prose: code — fenced, indented,
// spans — raw HTML, and the rule rows both grammars claim, where CommonMark
// owns the block structure. Isolating one with blank lines would not only
// rewrite it, it would demote the setext heading above it to a paragraph and
// take the next block's protection with it. A placeholder codepoint already
// in the source is lifted as its own region, restoring to itself, so what the
// restore walks over is exactly what was taken out.
fn verbatim_ranges(source: &str) -> Vec<Range<usize>> {
    let arena = comrak::Arena::new();
    // The dialect `render` parses, so what one holds verbatim the other does.
    let options = Options::default();
    let root = comrak::parse_document(&arena, source, &options.comrak);
    let line_starts = line_starts(source);
    let mut ranges: Vec<Range<usize>> = root
        .descendants()
        .filter_map(|node| {
            let data = node.data.borrow();
            let pos = data.sourcepos;
            match data.value {
                NodeValue::Code(_)
                | NodeValue::CodeBlock(_)
                | NodeValue::HtmlBlock(_)
                | NodeValue::HtmlInline(_)
                | NodeValue::ThematicBreak => byte_range(source, &line_starts, pos),
                // A setext heading's own text is prose; the row under it is
                // the one that also reads as a rule.
                NodeValue::Heading(NodeHeading { setext: true, .. }) => {
                    byte_range(source, &line_starts, underline_row(pos))
                }
                _ => None,
            }
        })
        .chain(
            source
                .match_indices(MASK_CHAR)
                .map(|(at, found)| at..at + found.len()),
        )
        .collect();
    ranges.sort_unstable_by_key(|range| range.start);
    let mut disjoint: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if disjoint.last().is_none_or(|last| last.end <= range.start) {
            disjoint.push(range);
        }
    }
    disjoint
}

// The heading's last line, starting where its text did: a container prefix is
// the same width on both rows.
fn underline_row(pos: Sourcepos) -> Sourcepos {
    let mut start = pos.end;
    start.column = pos.start.column;
    Sourcepos { start, ..pos }
}

// comrak reports 1-based lines and byte columns, the end column inclusive — 0
// for a block ending on a blank line, whose own line break is trimmed back
// off: a break the canonicaliser cannot see is one it re-inserts, and the
// round trip would grow a blank line every pass. A pair that does not slice
// the source is dropped rather than trusted.
fn byte_range(source: &str, line_starts: &[usize], pos: Sourcepos) -> Option<Range<usize>> {
    let offset = |line: usize, column: usize| {
        line_starts
            .get(line.checked_sub(1)?)
            .copied()?
            .checked_add(column)
    };
    let start = offset(pos.start.line, pos.start.column.saturating_sub(1))?;
    let end = offset(pos.end.line, pos.end.column)?;
    let text = source.get(start..end)?.trim_end_matches(['\n', '\r']);
    (!text.is_empty()).then_some(start..start + text.len())
}

fn line_starts(source: &str) -> Vec<usize> {
    once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use comrak::nodes::LineColumn;

    const fn at(line: usize, column: usize) -> LineColumn {
        LineColumn { line, column }
    }

    #[test]
    fn a_placeholder_the_source_already_carried_is_a_region_of_its_own() {
        // Otherwise the restore walks one placeholder more than it has
        // originals and puts every region back one position early.
        let source = format!("{MASK_CHAR}\n```\n｜青梅《おうめ》\n```\n");
        let (protected, originals) = protect(&source);
        let mask = String::from(MASK_CHAR);
        assert_eq!(originals, [mask.as_str(), "```\n｜青梅《おうめ》\n```"]);
        assert_eq!(protected, format!("{MASK_CHAR}\n{MASK_CHAR}\n"));
        assert_eq!(restore(&protected, &originals), source);
    }

    #[test]
    fn a_placeholder_with_no_original_behind_it_is_left_alone() {
        // The canonicaliser may add one of its own; an extra is text, and
        // consuming an original for it would shift every later splice.
        let canonical = format!("a{MASK_CHAR}b{MASK_CHAR}c");
        assert_eq!(
            restore(&canonical, &["X"]),
            format!("aXb{MASK_CHAR}c"),
            "an unmatched placeholder must flow through, not steal an original",
        );
    }

    #[test]
    fn a_placeholder_inside_a_region_is_not_lifted_a_second_time() {
        // Both the code block and the codepoint the source typed inside it
        // claim these bytes. Taking the inner one out as well would leave the
        // restore one original short of the placeholders it walks, and every
        // later region would splice back one position early.
        let source = format!("```\n{MASK_CHAR}\n```\n");
        let (protected, originals) = protect(&source);
        assert_eq!(originals, [format!("```\n{MASK_CHAR}\n```")]);
        assert_eq!(restore(&protected, &originals), source);
    }

    #[test]
    fn a_position_that_does_not_slice_the_source_is_dropped() {
        // Protecting a range computed from a position that does not address
        // the text would splice bytes back at the wrong offset, so the pair
        // is dropped instead of trusted.
        let source = "abc\n";
        let starts = line_starts(source);
        for pos in [
            Sourcepos {
                start: at(9, 1),
                end: at(9, 3),
            },
            Sourcepos {
                start: at(0, 1),
                end: at(1, 3),
            },
            Sourcepos {
                start: at(1, 1),
                end: at(1, 99),
            },
            // The line break alone: a break the canonicaliser cannot see is
            // one it re-inserts, so there is nothing here to hold.
            Sourcepos {
                start: at(1, 4),
                end: at(1, 4),
            },
            // A column that overflows the offset it is added to.
            Sourcepos {
                start: at(1, 1),
                end: at(2, usize::MAX),
            },
        ] {
            assert_eq!(byte_range(source, &starts, pos), None, "{pos:?}");
        }
    }
}
