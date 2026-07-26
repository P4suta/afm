//! Integration tests for the forward-reference heading-hint path.
//!
//! Covers the end-to-end rendering contract for
//! `［＃「X」は(大|中|小)見出し］`: the bracket is classified as a heading hint,
//! the host paragraph is promoted to a heading, and the renderer emits
//! `<h1>/<h2>/<h3>` with the extracted target as the body.
//!
//! Also covers the adjacent sanitize rule: a line of ≥ 10 repeats of
//! `-`/`=`/`_` is isolated from the preceding paragraph so CommonMark does not
//! promote it to a setext heading (`<h2>`).

use aozora_flavored_markdown::to_html;
use aozora_flavored_markdown_test_support::{check_heading_integrity, check_no_bare_bracket};

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
fn long_hyphen_rule_does_not_turn_paragraph_into_setext_heading() {
    // Direct analogue of the `spec/aozora/fixtures/56656/input.utf8.txt`
    // front-matter shape: a prose line followed by a long `-` run.
    // Without phase0's rule-isolation pass, CommonMark would promote
    // the prose to a setext H2.
    let input = "凡例です。\n-----------------------------------\n本文";
    let out = to_html(input);
    assert!(
        out.contains("<p>凡例です。</p>"),
        "preceding prose must remain a paragraph; got: {out}"
    );
    assert!(
        !out.contains("<h2>凡例です。</h2>"),
        "preceding prose must not become a setext heading; got: {out}"
    );
    // The rule itself should render as a thematic break.
    assert!(
        out.contains("<hr"),
        "decorative rule should render as <hr>; got: {out}"
    );
}

#[test]
fn long_equals_rule_does_not_turn_paragraph_into_setext_heading() {
    let input = "凡例です。\n=====================================\n本文";
    let out = to_html(input);
    assert!(
        out.contains("<p>凡例です。</p>"),
        "preceding prose must remain a paragraph; got: {out}"
    );
    assert!(
        !out.contains("<h1>凡例です。</h1>"),
        "preceding prose must not become a setext H1; got: {out}"
    );
}

#[test]
fn short_setext_heading_still_works() {
    // Regression canary for the rule-isolation threshold. A standard
    // 3-character setext underline is shorter than
    // `DECORATIVE_RULE_MIN_LEN` (10) and therefore untouched — the
    // CommonMark idiom of `Heading\n---\n` still promotes to H2.
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
