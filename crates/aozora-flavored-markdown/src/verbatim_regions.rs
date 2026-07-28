//! Holds out of the sibling parser's reach what CommonMark owns. One rule
//! about a rule row, two mechanisms, because the crate's two halves owe their
//! callers different things. [`protect`] lifts a whole region out for
//! `crate::canonicalize`, which owes no offset, so the line structure inside
//! it (a blank-line run, a rule row, the `> ` a container puts in front of
//! every line) never reaches the canonicaliser; [`hide_rule_rows`] substitutes
//! one byte for one for `crate::render`, which keeps a construct's byte span
//! slicing the caller's text. comrak locating most of the first family keeps
//! it a splice rather than the block parser a fence scanner would become.

use core::iter::{once, repeat_n};
use core::ops::Range;

use comrak::nodes::{NodeHeading, NodeValue, Sourcepos};

use crate::code_block_mask::MASK_CHAR;
use crate::{Options, sentinels};

// The three characters a rule row can be made of, and what each becomes for
// the sibling parser's read — index for index.
//
// C0 controls, one byte each: the substitution is byte-for-byte, so every
// range the parser reports still addresses the caller's own text, which is
// the whole reason `render` cannot lift a region out the way `protect` does.
// None of them is whitespace, so a hidden row is still the non-blank line the
// block structure around it was read against, and none of them appears in
// prose — a source that carries one is left unhidden rather than guessed at.
const RULE_CHARS: [u8; 3] = [b'-', b'=', b'_'];
const HIDDEN_RULE_CHARS: [u8; 3] = [0x01, 0x02, 0x03];

// Hides every rule row from the sibling parser, which would otherwise push
// one onto a stanza of its own and split whichever block CommonMark had given
// the bytes to. `reveal_rule_rows` puts the row back before comrak parses, so
// the split never happens and comrak still reads the caller's own row.
//
// `None` where nothing was hidden — the source has no rule row, or it already
// carries a character the reveal would claim. A caller that gets `None` reads
// its own text and must not reveal.
pub(crate) fn hide_rule_rows(source: &str) -> Option<String> {
    if source.bytes().any(|byte| HIDDEN_RULE_CHARS.contains(&byte)) {
        return None;
    }
    let rows = rule_rows(source);
    if rows.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    for row in rows {
        out.push_str(&source[cursor..row.start]);
        cursor = row.end;
        let text = &source[row];
        match hidden_form(text) {
            Some(hidden) => out.extend(repeat_n(hidden, text.len())),
            None => out.push_str(text),
        }
    }
    out.push_str(&source[cursor..]);
    Some(out)
}

// Character for character, so it holds wherever the parser left the row —
// the text it canonicalised for its own read included, which no offset
// recorded against the source would still address.
pub(crate) fn reveal_rule_rows(text: &str) -> String {
    text.chars()
        .map(|ch| {
            u8::try_from(ch)
                .ok()
                .and_then(|byte| HIDDEN_RULE_CHARS.iter().position(|&h| h == byte))
                .map_or(ch, |idx| char::from(RULE_CHARS[idx]))
        })
        .collect()
}

// A row is a run of one character, so its first byte names the whole of it.
fn hidden_form(row: &str) -> Option<char> {
    let first = row.as_bytes().first()?;
    let idx = RULE_CHARS.iter().position(|rule| rule == first)?;
    Some(char::from(HIDDEN_RULE_CHARS[idx]))
}

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

// Everything the canonicaliser would rewrite that is not its to rewrite. Two
// families: what comrak read as anything but prose — code (fenced, indented,
// a span), raw HTML, a break, a setext heading's underline — and what the
// canonicaliser rewrites on sight whatever comrak made of it, which is a rule
// row and a codepoint this crate reserves. The second family is why this is
// not simply a walk of the AST: a rule row comrak claimed as nothing at all
// is still one the canonicaliser pushes onto a stanza of its own.
fn verbatim_ranges(source: &str) -> Vec<Range<usize>> {
    let arena = comrak::Arena::new();
    // The dialect `render` parses, so the block structure read here is the one
    // the other half reads. What each then holds verbatim differs by exactly
    // one family: a codepoint this crate reserves comes back from here and is
    // neutralised on the render path, which `sentinels` states as the contract
    // it is.
    let options = Options::default().comrak();
    let root = comrak::parse_document(&arena, source, &options);
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
        .chain(rule_rows(source))
        .chain(reserved_codepoints(source))
        .collect();
    // Widest first where two of them start together, so the region that
    // swallows the other is the one the pass below keeps.
    ranges.sort_unstable_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut disjoint: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if disjoint.last().is_none_or(|last| last.end <= range.start) {
            disjoint.push(range);
        }
    }
    disjoint
}

// A rule row is markup to one grammar or the other — CommonMark's thematic
// break, its setext underline, the canonicaliser's decorative rule — and
// prose to neither, so a length threshold here would only pin this crate to
// one that lives in the other parser. The canonicaliser pushes such a row
// onto a stanza of its own, which is right where CommonMark has not claimed
// the bytes and wrong where it has: the row can be a paragraph's own text, a
// line continuing the paragraph above it, or a table row, and a blank line in
// front of it splits the block that owned it. The indent stays outside the
// region — it is what tells comrak which block the row belongs to.
fn rule_rows(source: &str) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut at = 0;
    for line in source.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        let row = line.trim();
        if is_rule_row(row) {
            rows.push(at + indent..at + indent + row.len());
        }
        at += line.len();
    }
    rows
}

fn is_rule_row(row: &str) -> bool {
    let mut bytes = row.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    RULE_CHARS.contains(&first) && bytes.all(|byte| byte == first)
}

// Every codepoint this crate reserves, wherever the source already carried
// one. Four of the five are rewritten to U+FFFD on sight, and the fifth is
// the placeholder itself: lifting it as a region of its own is what keeps the
// restore walking exactly the placeholders it has originals for. Read off
// `sentinels::ALL` rather than re-listed, so one added later is covered here
// without editing this.
fn reserved_codepoints(source: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    for reserved in sentinels::ALL {
        found.extend(
            source
                .match_indices(reserved)
                .map(|(at, text)| at..at + text.len()),
        );
    }
    found
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
