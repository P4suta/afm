//! End-to-end tests for the source-line anchor option.

use aozora_flavored_markdown::{Options, render};

#[test]
fn anchors_emit_only_when_option_is_on() {
    let src = "para line 1\n\npara line 3\n";
    let off = render(src, &Options::default());
    let on = render(src, &Options::default().with_source_line_anchors(true));
    assert!(!off.html.contains("data-aozora-md-source-line"));
    assert!(on.html.contains(r#"<p data-aozora-md-source-line=""#));
}

#[test]
fn anchors_are_one_based() {
    let on = render(
        "first\n\nsecond\n",
        &Options::default().with_source_line_anchors(true),
    );
    assert!(on.html.contains(r#"data-aozora-md-source-line="1""#));
    assert!(on.html.contains(r#"data-aozora-md-source-line="3""#));
}

#[test]
fn anchors_apply_to_headings() {
    let on = render(
        "# Title\n\nbody\n",
        &Options::default().with_source_line_anchors(true),
    );
    assert!(on.html.contains(r#"<h1 data-aozora-md-source-line="1""#));
    assert!(on.html.contains(r#"<p data-aozora-md-source-line="3""#));
}

#[test]
fn anchors_not_applied_when_aozora_disabled_and_option_off() {
    // commonmark() has source_line_anchors: false. Adding the
    // builder afterwards should still flip the bit.
    let on = render("p\n", &Options::commonmark().with_source_line_anchors(true));
    assert!(on.html.contains(r#"<p data-aozora-md-source-line=""#));
}

#[test]
fn block_notation_does_not_shift_a_following_anchor() {
    let on = render(
        "一行目\n\n［＃改ページ］\n\n五行目\n",
        &Options::default().with_source_line_anchors(true),
    );
    assert!(
        on.html
            .contains(r#"<p data-aozora-md-source-line="5">五行目</p>"#),
        "anchor must address the caller's fifth line: {}",
        on.html
    );
}
