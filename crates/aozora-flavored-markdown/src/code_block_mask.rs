//! Hides 青空文庫 trigger characters inside CommonMark fenced code blocks.
//!
//! The sibling parser rewrites every candidate trigger into a sentinel
//! before comrak sees the source — right for prose, wrong inside a fence
//! where every byte must reach `<pre><code>` literally. The parser is
//! CommonMark-blind by design (ADR-0010), so teaching it about code-block
//! context lives here: mask each trigger inside a fence with [`MASK_CHAR`],
//! record the original in source order, restore once the mask is back out —
//! `comrak::format_html` never disturbs it. One character for one, because
//! `render` keeps a construct's byte span slicing the caller's text;
//! `crate::serialize` owes no caller an offset and lifts whole regions out
//! instead (`crate::verbatim_regions`), reaching what this cannot.
//!
//! **Indented code blocks (CommonMark §4.4) are deliberately not masked**:
//! their boundaries depend on paragraph context. A notation inside one
//! becomes a sentinel that `crate::ast_splice` writes back as it does for an
//! inline code span, so both spellings read the same.
//!
//! **A source that already contains [`MASK_CHAR`] skips masking entirely**,
//! returning a borrowed `Cow` and no originals. The sibling parser
//! neutralizes U+E001..U+E004 only, so such a codepoint reaches the output
//! as the author's own byte, with no ambiguity of origin to resolve.

use core::cmp::min;
use std::borrow::Cow;

/// Distinct from the four construct sentinels (U+E001..U+E004), so masking
/// cannot collide with them.
pub(crate) const MASK_CHAR: char = '\u{E000}';

/// Mirrors the sibling tokeniser; if the upstream list grows, so must this.
const AOZORA_TRIGGERS: &[char] = &['｜', '《', '》', '［', '］', '※', '〔', '〕', '「', '」'];

/// Returns the replaced characters in source order, for [`unmask`]. Borrows
/// without allocating when the source has no fence at all.
#[must_use]
pub(crate) fn mask_code_block_triggers(source: &str) -> (Cow<'_, str>, Vec<char>) {
    if source.contains(MASK_CHAR) || !source.contains(['`', '~']) {
        return (Cow::Borrowed(source), Vec::new());
    }

    let mut out = String::with_capacity(source.len());
    let mut originals: Vec<char> = Vec::new();
    let mut phase = Phase::Outside;
    let mut masked_anything = false;

    for line in source.split_inclusive('\n') {
        match phase {
            Phase::Outside => {
                out.push_str(line);
                if let Some(fence) = parse_fence_open(line) {
                    phase = Phase::InFence(fence);
                }
            }
            Phase::InFence(open) => {
                if is_fence_close(line, open) {
                    out.push_str(line);
                    phase = Phase::Outside;
                } else {
                    for ch in line.chars() {
                        if AOZORA_TRIGGERS.contains(&ch) {
                            originals.push(ch);
                            out.push(MASK_CHAR);
                            masked_anything = true;
                        } else {
                            out.push(ch);
                        }
                    }
                }
            }
        }
    }

    if masked_anything {
        (Cow::Owned(out), originals)
    } else {
        (Cow::Borrowed(source), Vec::new())
    }
}

#[must_use]
pub(crate) fn unmask<'a>(text: &'a str, originals: &[char]) -> Cow<'a, str> {
    let mut cursor = originals;
    unmask_from(text, &mut cursor)
}

/// Restores in source-scan order — the order the masks appear in `text` —
/// advancing `originals` past what it consumed, so a caller formatting one
/// block at a time resumes instead of replaying. Extra masks flow through.
#[must_use]
pub(crate) fn unmask_from<'a>(text: &'a str, originals: &mut &[char]) -> Cow<'a, str> {
    if originals.is_empty() || !text.contains(MASK_CHAR) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut idx = 0;
    for ch in text.chars() {
        if ch == MASK_CHAR && idx < originals.len() {
            out.push(originals[idx]);
            idx += 1;
        } else {
            out.push(ch);
        }
    }
    *originals = &originals[idx..];
    Cow::Owned(out)
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Outside,
    InFence(FenceOpen),
}

#[derive(Debug, Clone, Copy)]
struct FenceOpen {
    /// Backtick or tilde, as chosen on the open line.
    marker: u8,
    width: usize,
}

/// CommonMark allows up to 3 leading spaces before the fence run.
fn parse_fence_open(line: &str) -> Option<FenceOpen> {
    let stripped = trim_leading_indent(line, 3);
    let bytes = stripped.as_bytes();
    let &first = bytes.first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let width = bytes.iter().take_while(|&&b| b == first).count();
    (width >= 3).then_some(FenceOpen {
        marker: first,
        width,
    })
}

/// Same marker as `open`, at least as wide, and nothing but whitespace after.
fn is_fence_close(line: &str, open: FenceOpen) -> bool {
    let stripped = trim_leading_indent(line, 3);
    let bytes = stripped.as_bytes();
    let run = bytes.iter().take_while(|&&b| b == open.marker).count();
    if run < open.width {
        return false;
    }
    bytes[run..]
        .iter()
        .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

/// Tabs are deliberately not expanded. CommonMark counts them in the indent
/// budget, but this is a pre-pass rather than a conformance check, so a
/// tab-led line simply fails fence detection — a strict subset, enough here.
fn trim_leading_indent(line: &str, max: usize) -> &str {
    let bytes = line.as_bytes();
    let cap = min(bytes.len(), max);
    let consumed = bytes.iter().take(cap).take_while(|&&b| b == b' ').count();
    &line[consumed..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_owned(src: &str) -> (String, Vec<char>) {
        let (cow, originals) = mask_code_block_triggers(src);
        (cow.into_owned(), originals)
    }

    #[test]
    fn no_code_block_no_mask() {
        let (cow, originals) = mask_code_block_triggers("｜青梅《おうめ》");
        // No fence open chars in source: borrowed fast path.
        assert!(matches!(cow, Cow::Borrowed(_)));
        assert_eq!(cow.as_ref(), "｜青梅《おうめ》");
        assert!(originals.is_empty());
    }

    #[test]
    fn fenced_code_triggers_get_masked() {
        let src = "before\n```\n｜青梅《おうめ》\n```\nafter";
        let (out, originals) = mask_owned(src);
        assert!(!out.contains('｜'), "trigger leaked: {out:?}");
        assert!(!out.contains('《'), "trigger leaked: {out:?}");
        assert!(!out.contains('》'), "trigger leaked: {out:?}");
        // before / after stay untouched
        assert!(out.starts_with("before\n```\n"));
        assert!(out.ends_with("\n```\nafter"));
        assert_eq!(originals, vec!['｜', '《', '》']);
    }

    #[test]
    fn tilde_fence_works_too() {
        let src = "~~~\n［＃改ページ］\n~~~";
        let (out, originals) = mask_owned(src);
        assert!(!out.contains('［'));
        assert_eq!(originals, vec!['［', '］']);
    }

    #[test]
    fn close_fence_must_match_marker() {
        // Opened with ``` but closed with ~~~ → still inside the
        // fence; everything to EOF stays masked.
        let src = "```\n｜inside\n~~~\n｜still\n";
        let (_, originals) = mask_owned(src);
        assert_eq!(originals, vec!['｜', '｜']);
    }

    #[test]
    fn close_fence_must_be_at_least_as_wide() {
        // Opened with ````, closed with only ``` → not closed.
        let src = "````\n｜inside\n```\n｜still\n";
        let (_, originals) = mask_owned(src);
        assert_eq!(originals, vec!['｜', '｜']);
    }

    #[test]
    fn outside_text_is_left_alone() {
        let src = "｜prose《outside》\n```\n｜inside\n```\n｜after《tail》";
        let (out, originals) = mask_owned(src);
        assert!(out.contains("｜prose《outside》"), "out: {out}");
        assert!(out.contains("｜after《tail》"), "out: {out}");
        assert_eq!(originals, vec!['｜']);
    }

    #[test]
    fn pre_existing_mask_char_disables_masking() {
        // If the source already contains MASK_CHAR, we cannot
        // distinguish a masked trigger from a literal PUA char on the
        // unmask side, so we bail out and leave the sibling parser's
        // own PUA-collision diagnostic in charge.
        let src = "\u{E000}\n```\n｜trigger\n```";
        let (cow, originals) = mask_code_block_triggers(src);
        assert!(matches!(cow, Cow::Borrowed(_)));
        assert_eq!(cow.as_ref(), src);
        assert!(originals.is_empty());
    }

    #[test]
    fn unmask_round_trips_fenced_triggers() {
        let src = "```\n｜青梅《おうめ》\n```";
        let (masked, originals) = mask_owned(src);
        // Pretend comrak emitted the masked content verbatim inside a
        // <pre><code> block (which is exactly what it does).
        let pseudo_html = format!(
            "<pre><code>{}\n</code></pre>\n",
            &masked[4..masked.len() - 4]
        );
        let restored = unmask(&pseudo_html, &originals);
        assert!(restored.contains('｜'), "got: {restored}");
        assert!(restored.contains('《'));
        assert!(restored.contains('》'));
    }

    #[test]
    fn unmask_with_empty_originals_is_a_noop() {
        assert_eq!(unmask("hello", &[]).as_ref(), "hello");
    }

    #[test]
    fn unmask_handles_more_mask_chars_than_originals_gracefully() {
        // Edge case: comrak somehow emitted more mask chars than we
        // recorded. The extras flow through verbatim — benign.
        let originals = vec!['｜'];
        let masked = format!("{MASK_CHAR}{MASK_CHAR}");
        let restored = unmask(&masked, &originals);
        assert_eq!(restored.chars().filter(|&c| c == '｜').count(), 1);
        assert_eq!(restored.chars().filter(|&c| c == MASK_CHAR).count(), 1);
    }

    #[test]
    fn indent_up_to_three_spaces_does_not_break_fence_detection() {
        let src = "   ```\n｜inside\n   ```\nafter";
        let (_, originals) = mask_owned(src);
        assert_eq!(originals, vec!['｜']);
    }

    #[test]
    fn indent_of_four_spaces_disables_the_fence() {
        // Four leading spaces: the line is not a fence open per
        // CommonMark (it would be an indented code block instead, but
        // we don't mask indented code blocks). The trigger remains.
        let src = "    ```\n｜prose\n    ```";
        let (out, originals) = mask_owned(src);
        assert!(out.contains('｜'), "out: {out}");
        assert!(originals.is_empty());
    }

    #[test]
    fn crlf_line_endings_are_preserved_through_the_fence() {
        // Carriage-return + line-feed should not derail fence-open or
        // close detection. The split_inclusive('\n') loop hands each
        // line with its trailing `\r\n` intact; trim_leading_indent
        // operates on leading bytes only, and is_fence_close treats
        // `\r` as trailing whitespace.
        let src = "```\r\n｜inside\r\n```\r\nafter";
        let (out, originals) = mask_owned(src);
        assert!(!out.contains('｜'), "trigger leaked: {out:?}");
        assert_eq!(originals, vec!['｜']);
        assert!(out.contains("\r\nafter"));
    }
}

#[cfg(test)]
mod proptests {
    //! The unit tests above pin hand-curated shapes; these close the gap
    //! with arbitrary Aozora-shaped and CommonMark-adversarial input.

    use super::*;
    use aozora_flavored_markdown_test_support::config::default_config;
    use aozora_flavored_markdown_test_support::generators::{
        aozora_fragment, commonmark_adversarial,
    };
    use proptest::prelude::*;

    /// Aozora fragments mixed with CommonMark-adversarial constructs.
    fn aozora_or_commonmark() -> impl Strategy<Value = String> {
        prop_oneof![aozora_fragment(40), commonmark_adversarial()]
    }

    /// Mirrors the fence-state machine in [`mask_code_block_triggers`], so
    /// the count covers exactly the characters masking leaves alone.
    fn outside_fences(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut phase = Phase::Outside;
        for line in s.split_inclusive('\n') {
            match phase {
                Phase::Outside => {
                    out.push_str(line);
                    if let Some(fence) = parse_fence_open(line) {
                        phase = Phase::InFence(fence);
                    }
                }
                Phase::InFence(open) => {
                    if is_fence_close(line, open) {
                        phase = Phase::Outside;
                    }
                    // body of a fenced code block is dropped here on
                    // purpose — we only want the prose context.
                }
            }
        }
        out
    }

    fn count_triggers(s: &str) -> usize {
        s.chars().filter(|c| AOZORA_TRIGGERS.contains(c)).count()
    }

    proptest! {
        #![proptest_config(default_config())]

        /// Sources without `` ` `` or `~` round-trip untouched.
        #[test]
        fn no_fence_input_is_borrowed_with_no_originals(s in aozora_fragment(40)) {
            let scrubbed: String = s.chars().filter(|c| *c != '`' && *c != '~').collect();
            let (masked, originals) = mask_code_block_triggers(&scrubbed);
            prop_assert!(matches!(masked, Cow::Borrowed(_)));
            prop_assert!(originals.is_empty());
            prop_assert_eq!(&*masked, &scrubbed);
        }

        /// A source already carrying [`MASK_CHAR`] short-circuits, so the
        /// parser's `SourceContainsPua` diagnostic stays meaningful.
        #[test]
        fn pre_existing_mask_char_short_circuits(s in aozora_fragment(40)) {
            let mut with_mask = String::with_capacity(s.len() + 1);
            with_mask.push(MASK_CHAR);
            with_mask.push_str(&s);
            let (masked, originals) = mask_code_block_triggers(&with_mask);
            prop_assert!(matches!(masked, Cow::Borrowed(_)));
            prop_assert!(originals.is_empty());
            prop_assert_eq!(&*masked, &with_mask);
        }

        /// The round-trip the whole pass exists to provide.
        #[test]
        fn mask_then_unmask_is_identity(src in aozora_or_commonmark()) {
            let (masked, originals) = mask_code_block_triggers(&src);
            let restored = unmask(&masked, &originals);
            prop_assert_eq!(&*restored, &src);
        }

        /// Only the fence interior is substituted, so the masked output
        /// keeps at least as many triggers as the exterior projection has.
        #[test]
        fn outside_fence_triggers_are_preserved(src in aozora_or_commonmark()) {
            let outside_count = count_triggers(&outside_fences(&src));
            let (masked, _) = mask_code_block_triggers(&src);
            let masked_count = count_triggers(&masked);
            prop_assert!(
                masked_count >= outside_count,
                "outside-fence triggers were not preserved: outside={outside_count} masked={masked_count}\n\
                 source: {src:?}\nmasked: {masked:?}"
            );
        }
    }
}
