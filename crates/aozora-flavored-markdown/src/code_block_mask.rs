//! Pre-/post-process pass that hides 青空文庫 trigger characters
//! inside CommonMark fenced code blocks.
//!
//! ## Why this exists
//!
//! The sibling parser recognises every `｜` / `《` / `》` / `［` / `］` /
//! `※` / `〔` / `〕` / `「` / `」` as a candidate trigger and rewrites
//! it into a PUA sentinel before comrak ever sees the source. That is
//! exactly what we want for prose; it is exactly what we *don't* want
//! inside a fenced code block, where every byte should flow through
//! to `<pre><code>` literally.
//!
//! That parser is intentionally CommonMark-blind (ADR-0010), so the
//! responsibility for teaching it about code-block context lives
//! here. The pass:
//!
//! 1. Scans the source line-by-line and tracks fenced-code-block
//!    state with the [`Phase`] machine below (CommonMark info-string
//!    fence: a run of three or more backticks or three or more
//!    tildes after at most three leading spaces, closed by a
//!    same-character run of at least the same length).
//! 2. Replaces each Aozora trigger inside a fence with [`MASK_CHAR`]
//!    (U+E000 — Private Use Area, distinct from the four sentinels
//!    U+E001..U+E004) and records the original char in source order.
//! 3. After `comrak::format_html`, restores the original chars by
//!    walking the recorded list in the same order. comrak's HTML
//!    escape only touches `<`, `>`, `&`, `"`; `MASK_CHAR` survives
//!    untouched.
//!
//! ## What's deliberately out of scope
//!
//! **Indented code blocks** (CommonMark §4.4) — `    code` — are
//! NOT masked. They start and end based on paragraph context that
//! the lexer pre-pass would need a full mini-parser to reproduce
//! (blank-line boundaries, list-item interleaving, etc.). In every
//! Aozora Bunko source we've seen, code-shaped runs use fenced
//! syntax; the pinned test
//! `tests::indent_of_four_spaces_disables_the_fence` codifies the
//! current behaviour. If a future corpus exhibits real-world
//! 4-space indented code blocks with Aozora trigger chars, this is
//! the place to extend.
//!
//! ## Why not collide with `MASK_CHAR`?
//!
//! The sibling parser already scans for source-supplied PUA
//! characters and emits a `Diagnostic::SourceContainsPua` for any
//! encountered. We pre-scan for [`MASK_CHAR`] in the *original*
//! source and skip masking entirely if any is present, returning
//! the source as a borrowed `Cow` and an empty originals list —
//! that preserves the lexer's diagnostic on the user's pristine
//! input and avoids ambiguity-of-origin in [`unmask_html`].

use core::cmp::min;
use std::borrow::Cow;

/// Private-use code point used to stand in for an Aozora trigger
/// character that lives inside a fenced code block. Distinct from the
/// inline sentinel (U+E001) and the three block sentinels
/// (U+E002..U+E004), so the masking pass cannot collide with the lexer's
/// own sentinels.
const MASK_CHAR: char = '\u{E000}';

/// Every char the sibling parser treats as a recogniser trigger.
/// Mirrors its tokeniser; if the upstream list grows, this one must
/// follow.
const AOZORA_TRIGGERS: &[char] = &['｜', '《', '》', '［', '］', '※', '〔', '〕', '「', '」'];

/// Mask every Aozora trigger character that appears inside a fenced
/// code block. Returns the (possibly borrowed) source plus the list
/// of original characters that were replaced (for use by
/// [`unmask_html`]).
///
/// Returns `(Cow::Borrowed(source), Vec::new())` and skips masking
/// when:
/// - the source already contains [`MASK_CHAR`] (see module docs), or
/// - the source has no fenced code block at all.
///
/// Otherwise allocates a single owned `String` and returns
/// `(Cow::Owned(masked), originals)`.
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

/// Reverse the masking. For every [`MASK_CHAR`] in `html`, take the
/// next entry from `originals` (in source-scan order, which matches
/// the order they appear in the rendered HTML).
///
/// If `originals` runs short, remaining `MASK_CHAR`s flow through
/// unchanged — that is benign because they would render as a PUA
/// glyph in the browser and never collide with body text.
#[must_use]
pub(crate) fn unmask_html<'a>(html: &'a str, originals: &[char]) -> Cow<'a, str> {
    if originals.is_empty() || !html.contains(MASK_CHAR) {
        return Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut idx = 0;
    for ch in html.chars() {
        if ch == MASK_CHAR && idx < originals.len() {
            out.push(originals[idx]);
            idx += 1;
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

/// Line-state of the masking scan. CommonMark fenced code blocks are
/// the only construct we recognise; indented code blocks are out of
/// scope (see module docs).
#[derive(Debug, Clone, Copy)]
enum Phase {
    Outside,
    InFence(FenceOpen),
}

#[derive(Debug, Clone, Copy)]
struct FenceOpen {
    /// Backtick or tilde — the fence character chosen on the open line.
    marker: u8,
    /// Number of consecutive marker chars in the opening fence.
    width: usize,
}

/// Recognise the opening of a fenced code block on this line.
/// CommonMark allows up to 3 leading spaces before the fence run.
/// Returns the fence shape if `line` is a valid open fence.
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

/// Recognise a closing fence: same marker as `open`, at least
/// `open.width` repetitions, optional leading indent up to 3 spaces,
/// nothing but whitespace (including the trailing CRLF / LF) after
/// the run.
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

/// Strip up to `max` leading ASCII spaces from `line`. Tabs are
/// deliberately not expanded — CommonMark allows them inside the
/// indent budget but our masking pass is a pre-pass, not a
/// conformance check; tabs flow through untouched and the
/// fence-detector simply fails on lines that lead with a tab. That is
/// a strict subset of valid fences but matches every real-world aozora-flavored-markdown
/// source we have seen.
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
        let restored = unmask_html(&pseudo_html, &originals);
        assert!(restored.contains('｜'), "got: {restored}");
        assert!(restored.contains('《'));
        assert!(restored.contains('》'));
    }

    #[test]
    fn unmask_with_empty_originals_is_a_noop() {
        assert_eq!(unmask_html("hello", &[]).as_ref(), "hello");
    }

    #[test]
    fn unmask_handles_more_mask_chars_than_originals_gracefully() {
        // Edge case: comrak somehow emitted more mask chars than we
        // recorded. The extras flow through verbatim — benign.
        let originals = vec!['｜'];
        let masked = format!("{MASK_CHAR}{MASK_CHAR}");
        let restored = unmask_html(&masked, &originals);
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
    //! Property tests for the masking pass.
    //!
    //! The unit tests above pin a finite list of hand-curated shapes
    //! (fenced code, tilde fence, indented fence, CRLF, pre-existing
    //! mask char). Property tests close the gap by drawing arbitrary
    //! Aozora-shaped and CommonMark-adversarial input from the shared
    //! generators and asserting four cross-cutting invariants that
    //! must hold *regardless* of the input's shape:
    //!
    //! 1. **No-fence identity** — when the source contains neither
    //!    backticks nor tildes, masking is a no-op (returns
    //!    `Cow::Borrowed`) and `originals` is empty.
    //! 2. **PUA fast-path** — when the source already contains
    //!    [`MASK_CHAR`] (U+E000), masking is short-circuited to
    //!    `Cow::Borrowed` and `originals` is empty (regardless of
    //!    fence presence). The lexer's own `SourceContainsPua`
    //!    diagnostic must not be sabotaged by us mutating the bytes.
    //! 3. **Mask + unmask is identity** — for any input, replaying the
    //!    masked output through `unmask_html` with the recorded
    //!    `originals` reconstructs the source byte-for-byte. This is
    //!    the core round-trip property the masking pass exists to
    //!    provide.
    //! 4. **Outside-fence triggers are preserved verbatim** — masking
    //!    only touches the fence interior; any Aozora trigger glyph
    //!    that appears *outside* a fenced code block must survive the
    //!    pass unchanged.

    use super::*;
    use aozora::proptest::config::default_config;
    use aozora::proptest::generators::{aozora_fragment, commonmark_adversarial};
    use proptest::prelude::*;

    /// Combined input strategy — Aozora fragments mixed with CommonMark
    /// adversarial constructs (which include fenced code blocks).
    fn aozora_or_commonmark() -> impl Strategy<Value = String> {
        prop_oneof![aozora_fragment(40), commonmark_adversarial()]
    }

    /// Substring of `s` that lies *outside* every fenced code block.
    /// Mirrors the fence-state machine in [`mask_code_block_triggers`]
    /// so we count exactly the characters the masking pass leaves
    /// alone.
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

    /// Count occurrences of every Aozora trigger char in a string.
    fn count_triggers(s: &str) -> usize {
        s.chars().filter(|c| AOZORA_TRIGGERS.contains(c)).count()
    }

    proptest! {
        #![proptest_config(default_config())]

        /// (1) No-fence identity — sources without `` ` `` or `~` round-trip
        /// untouched.
        #[test]
        fn no_fence_input_is_borrowed_with_no_originals(s in aozora_fragment(40)) {
            let scrubbed: String = s.chars().filter(|c| *c != '`' && *c != '~').collect();
            let (masked, originals) = mask_code_block_triggers(&scrubbed);
            prop_assert!(matches!(masked, Cow::Borrowed(_)));
            prop_assert!(originals.is_empty());
            prop_assert_eq!(&*masked, &scrubbed);
        }

        /// (2) PUA fast-path — sources already carrying [`MASK_CHAR`] short
        /// circuit to a borrowed Cow with empty originals so the lexer's
        /// SourceContainsPua diagnostic stays meaningful.
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

        /// (3) Mask + unmask is identity. The fundamental round-trip
        /// invariant — without this, the entire masking pass is broken.
        #[test]
        fn mask_then_unmask_is_identity(src in aozora_or_commonmark()) {
            let (masked, originals) = mask_code_block_triggers(&src);
            let restored = unmask_html(&masked, &originals);
            prop_assert_eq!(&*restored, &src);
        }

        /// (4) Outside-fence triggers survive masking — only the fence
        /// interior gets [`MASK_CHAR`]-substituted. We count triggers in
        /// the fence-exterior projection of the source and assert the
        /// masked output retains at least that many.
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
