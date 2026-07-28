//! Integration tests for the forward-reference heading-hint path.
//!
//! Covers the end-to-end rendering contract for
//! `［＃「X」は(大|中|小)見出し］`: the bracket is classified as a heading hint,
//! the host paragraph is promoted to a heading, and the renderer emits
//! `<h1>/<h2>/<h3>` with the extracted target as the body.
//!
//! Also covers the promotion this crate does *not* do: a line of `-`/`=`/`_`
//! underlines the paragraph above it whatever its width, because the row is
//! held out of the sibling parser's reach and CommonMark reads it instead.

use aozora_flavored_markdown::to_html;
use aozora_flavored_markdown_test_support::{
    check_heading_integrity, check_no_bare_bracket, check_no_sentinel_leak,
};

#[test]
fn big_heading_is_rendered_as_h1() {
    // 大見出し → Markdown H1. Forward-reference target "第一篇" is
    // preceded by its own plain copy so the lexer's target-exists
    // gate passes.
    let out = to_html("第一篇［＃「第一篇」は大見出し］");
    assert!(
        out.contains("<h1>第一篇</h1>"),
        "expected <h1>第一篇</h1> in output; got: {out}"
    );
}

#[test]
fn medium_heading_is_rendered_as_h2() {
    let out = to_html("一［＃「一」は中見出し］");
    assert!(
        out.contains("<h2>一</h2>"),
        "expected <h2>一</h2> in output; got: {out}"
    );
}

#[test]
fn small_heading_is_rendered_as_h3() {
    let out = to_html("小題［＃「小題」は小見出し］");
    assert!(
        out.contains("<h3>小題</h3>"),
        "expected <h3>小題</h3> in output; got: {out}"
    );
}

#[test]
fn heading_with_preceding_indent_marker_still_becomes_heading() {
    // The 罪と罰 fixture shape: `［＃２字下げ］第一篇［＃「第一篇」は大見出し］`.
    // The post-process must strip the leading indent marker from the
    // paragraph so it doesn't leak into the promoted heading.
    let out = to_html("［＃２字下げ］第一篇［＃「第一篇」は大見出し］");
    assert!(
        out.contains("<h1>第一篇</h1>"),
        "expected <h1>第一篇</h1>; got: {out}"
    );
    // The heading body must be the target only — no indent marker
    // class, no annotation wrapper.
    assert!(
        !out.contains("<h1><span class=\"aozora-md-indent"),
        "indent marker must not leak into the heading: {out}"
    );
    assert!(
        !out.contains("<h1><span class=\"aozora-md-directive"),
        "annotation wrapper must not leak into the heading: {out}"
    );
}

#[test]
fn heading_hint_without_preceding_target_promotes_nothing() {
    // No preceding "第一篇" run, so the hint names no run of the
    // paragraph: it is its own text, sits mid-line, and there is nothing
    // to promote. Tier-A canary still holds — `［＃` never appears
    // outside a hidden wrapper.
    let input = "本文［＃「第一篇」は大見出し］";
    let out = to_html(input);
    assert!(
        !out.contains("<h1>"),
        "no heading should be promoted without a preceding target; got: {out}"
    );
    // Tier-A: the raw bracket characters must be inside a hidden
    // annotation wrapper, not bare in the output.
    assert!(
        !out.contains("］は大見出し］"),
        "bracket content should not leak bare; got: {out}"
    );
}

#[test]
fn heading_hint_inside_a_markdown_heading_leaves_the_heading_alone() {
    // Promotion is a paragraph's answer to a hint. A markdown heading is
    // not a paragraph, so the hint reaches the inline walk instead — and a
    // hint's own source run, alone among the notations, does not cover the
    // text it names, so re-parsing that run resolves it to an unknown
    // annotation. Emitting that would put an `aozora-md-directive` wrapper
    // inside the heading body, which Tier C bars.
    let out = to_html("# head第一篇［＃「第一篇」は大見出し］");
    assert_eq!(out, "<h1>head第一篇</h1>\n");
    assert!(
        check_heading_integrity(&out).is_ok(),
        "Tier C: {:?}",
        check_heading_integrity(&out)
    );
    assert!(check_no_bare_bracket(&out).is_ok(), "Tier A: {out}");
}

/// A `@` stands for the body under test. Between them, every heading a
/// caller writes in markdown rather than in 青空文庫 notation: ATX at the
/// ends and the middle of the level range, and a setext underline on both
/// sides of the width at which the sibling parser used to read the row as
/// decoration instead.
const MARKDOWN_HEADINGS: &[&str] = &[
    "# @\n",
    "## @\n",
    "###### @\n",
    "@\n---\n",
    "@\n===\n",
    "@\n----------\n",
    "@\n==================================\n",
];

/// Every 青空文庫 marker whose rendering is a wrapper rather than text, plus
/// the two shapes that carry no wrapper at all — an unknown annotation and a
/// truncated one, which reach the heading body by the recovery path instead.
const MARKERS: &[&str] = &[
    "",
    "［＃１字下げ］",
    "［＃２字下げ］",
    "［＃ここから２字下げ］",
    "［＃ここで字下げ終わり］",
    "［＃地付き］",
    "［＃改ページ］",
    "［＃ほげふが］",
    "［＃［＃２字下げ］",
];

#[test]
fn a_markdown_heading_body_carries_no_marker_and_no_sentinel() {
    // THE GRID `heading_hint_inside_a_markdown_heading_leaves_the_heading_alone`
    // is one cell of. That test asks what an ATX heading does with a heading
    // *hint* and answers correctly; nothing asked what one does with any of
    // the other markers, and two defects lived in the gap:
    //
    // * an indent marker rendered its `aozora-md-indent` wrapper into the
    //   heading body — Tier C, and reachable through `to_html` on `main` by
    //   typing `# ［＃２字下げ］漢字`. `inline_is_dropped` dropped a directive
    //   inside a heading and not an indent.
    // * a heading body the splice consumed *entirely* kept its original text
    //   node, sentinels and all — Tier B, reachable by `# ［＃「漢字」は大見出し］`,
    //   where the hint names a target the heading has no room for.
    //
    // Both tiers judge any `<hN>` correctly. Neither had ever been shown a
    // markdown heading with a marker in it: the Tier C property generator
    // builds its headings out of hints alone, and the Tier B generators have no
    // `#` in their atom pool and draw their rule rows at widths the sibling
    // parser used to swallow — so a setext heading could not form there either.
    let mut cells = 0usize;
    for heading in MARKDOWN_HEADINGS {
        for marker in MARKERS {
            for body in ["", "漢字"] {
                for hint in ["", "［＃「漢字」は大見出し］"] {
                    let src = heading.replace('@', &format!("{marker}{body}{hint}"));
                    let out = to_html(&src);
                    check_heading_integrity(&out)
                        .unwrap_or_else(|e| panic!("Tier C for src={src:?}, html={out:?}: {e}"));
                    check_no_sentinel_leak(&src, &out)
                        .unwrap_or_else(|e| panic!("Tier B for src={src:?}, html={out:?}: {e}"));
                    cells += 1;
                }
            }
        }
    }
    assert_eq!(
        cells,
        MARKDOWN_HEADINGS.len() * MARKERS.len() * 2 * 2,
        "the grid stopped covering what it enumerates"
    );

    // The two defects, spelled out, because a grid of "must never" says nothing
    // about severity — and, between them, they are what keeps the grid from
    // passing vacuously: a renderer that emitted no heading at all would
    // satisfy both tiers everywhere above and fail here.
    assert_eq!(
        to_html("# ［＃２字下げ］漢字"),
        "<h1>漢字</h1>\n",
        "an indent marker has nothing to indent on a one-line heading, so it is dropped"
    );
    assert_eq!(
        to_html("# ［＃「漢字」は大見出し］"),
        "<h1></h1>\n",
        "a heading whose whole body was consumed is empty — an empty ATX heading is \
         legitimate markdown (Tier L is about a *promoted* one), and it is what CommonMark \
         renders for `#` with nothing after it"
    );
    assert_eq!(
        to_html("［＃２字下げ］漢字\n----------\n"),
        "<h2>漢字</h2>\n",
        "the setext arm of the grid answers the same way, and it exists at all only \
         because the rule row is now CommonMark's to read"
    );
}

#[test]
fn heading_hint_inside_a_table_cell_renders_nothing() {
    // Same reasoning one context over: nothing to promote, so the
    // directive renders as what it is — an instruction, not content.
    let out = to_html("| a |\n| - |\n| 第一篇［＃「第一篇」は大見出し］ |");
    assert!(
        out.contains("<td>第一篇</td>"),
        "the cell keeps its text and loses the directive: {out}"
    );
    assert!(check_no_bare_bracket(&out).is_ok(), "Tier A: {out}");
}

#[test]
fn a_long_hyphen_rule_underlines_the_line_above_it() {
    // Direct analogue of the `spec/aozora/fixtures/56656/input.utf8.txt`
    // front-matter shape: a prose line followed by a long `-` run.
    //
    // These two tests used to assert the opposite, on the sibling parser's
    // rule-isolation pass — which reads a run of ten or more as a decorative
    // separator and pushes it onto a stanza of its own, so the prose above
    // stays a paragraph and the row becomes a `<hr>`. That is the 青空文庫
    // reading, and this crate no longer takes it on the render path: the
    // dialect is documented as a superset of CommonMark, and CommonMark says
    // the row underlines the line above it whatever its width. The row is
    // held out of the parser's reach instead (`crate::verbatim_regions`), so
    // both halves of the crate answer the same way and the length threshold
    // stops being observable at all.
    let input = "凡例です。\n-----------------------------------\n本文";
    let out = to_html(input);
    assert!(
        out.contains("<h2>凡例です。</h2>"),
        "the row underlines the paragraph CommonMark gave it to; got: {out}"
    );
    assert!(
        !out.contains("<hr"),
        "a setext underline is not also a thematic break; got: {out}"
    );
}

#[test]
fn a_long_equals_rule_underlines_the_line_above_it() {
    let input = "凡例です。\n=====================================\n本文";
    let out = to_html(input);
    assert!(
        out.contains("<h1>凡例です。</h1>"),
        "an `=` row is a setext H1 underline at any width; got: {out}"
    );
}

#[test]
fn short_setext_heading_still_works() {
    // The control that always passed: a 3-character underline was below the
    // sibling parser's threshold even before the row was held out of its
    // reach, so this is what tells a reader the two cases now agree rather
    // than merely both being green.
    let input = "Heading\n---\nbody";
    let out = to_html(input);
    assert!(
        out.contains("<h2>Heading</h2>"),
        "short `---` must still act as a setext underline; got: {out}"
    );
}

#[test]
fn empty_title_heading_hint_never_emits_an_empty_heading() {
    // Tier L (no empty promoted heading), pinned at the unit level
    // because it has no sound rendered-HTML witness: an empty *promoted*
    // heading `<hN></hN>` is byte-identical to a legitimate empty ATX
    // heading (`##`), so an always-on HTML predicate cannot tell them
    // apart (see the catalog note in aozora-flavored-markdown-test-support).
    //
    // A degenerate hint with an empty target `「」` must not promote the
    // host paragraph into an empty heading. The target-exists gate has
    // no non-empty preceding run to match, so the hint stays an
    // annotation — never `<hN></hN>`.
    for input in [
        "［＃「」は大見出し］",
        "本文［＃「」は中見出し］",
        "　［＃「　」は小見出し］",
    ] {
        let out = to_html(input);
        for level in 1..=6 {
            assert!(
                !out.contains(&format!("<h{level}></h{level}>")),
                "empty <h{level}> heading leaked for input {input:?}; got: {out}"
            );
        }
    }
}

#[test]
fn heading_hint_round_trips_through_canonicalize() {
    // I3 (canonicalize ∘ parse fixed point) demands that a heading hint
    // reconstructs its original `［＃「…」は…見出し］` form through
    // the canonicaliser even though the HTML pipeline promotes the host
    // paragraph to `<h{level}>`. The canonicaliser works off the lexer's
    // placeholder registry, so the heading's HTML-side promotion does
    // not lose round-trip information.
    let input = "第一篇［＃「第一篇」は大見出し］";
    let canonical =
        aozora_flavored_markdown::canonicalize(input).expect("in-budget source canonicalises");
    assert!(
        canonical.contains("［＃「第一篇」は大見出し］"),
        "heading-hint markup must survive round-trip; got: {canonical}"
    );
    let second =
        aozora_flavored_markdown::canonicalize(&canonical).expect("a canonical form settles");
    assert_eq!(
        canonical, second,
        "canonicalize ∘ parse must be a fixed point after one iteration"
    );
}
