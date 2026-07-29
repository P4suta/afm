//! Per-top-level-block render with `data-aozora-md-source-line` injection,
//! behind `Options::with_source_line_anchors`.
//!
//! Formatting each top-level child into its own buffer makes the Nth child
//! the Nth anchored tag: no depth tracking, no full-document HTML scan, and
//! no ambiguity about which open tag is top-level. The two-pass alternative
//! needed a hand-rolled tag walker with quote and void-tag tracking.

use comrak::nodes::AstNode;

use crate::constructs::saturating_u32;

/// One anchored `data-aozora-md-source-line="N"` per top-level child.
pub(crate) fn format_root_with_anchors<'a>(
    root: &'a AstNode<'a>,
    options: &comrak::Options<'static>,
) -> String {
    let children: Vec<&AstNode<'a>> = root.children().collect();
    let mut out = String::with_capacity(children.len() * 64);
    for child in children {
        out.push_str(&format_block_with_anchor(child, options));
    }
    out
}

/// The per-child half, shared with the streaming block renderer.
#[allow(
    clippy::expect_used,
    reason = "comrak writes through std::fmt::Write into String, whose write operations are infallible"
)]
pub(crate) fn format_block_with_anchor<'a>(
    child: &'a AstNode<'a>,
    options: &comrak::Options<'static>,
) -> String {
    let line = saturating_u32(child.data.borrow().sourcepos.start.line).max(1);
    let mut buf = String::new();
    comrak::format_html(child, options, &mut buf).expect("formatting to a String never fails");
    inject_anchor_into_first_open_tag(&mut buf, line);
    buf
}

/// "First opening tag" means the first `<X` where `X` is an ASCII letter,
/// skipping comments, doctypes and processing instructions — which comrak
/// does not emit, but a raw HTML block can carry verbatim from the source.
/// With no eligible tag the buffer is left untouched.
fn inject_anchor_into_first_open_tag(buf: &mut String, line: u32) {
    let bytes = buf.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && let Some(&next) = bytes.get(i + 1)
            && next.is_ascii_alphabetic()
        {
            let mut j = i + 1;
            while j < bytes.len() && !matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')
            {
                j += 1;
            }
            let attr = format!(r#" data-aozora-md-source-line="{line}""#);
            buf.insert_str(j, &attr);
            return;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_after_simple_open_tag_name() {
        let mut buf = String::from("<p>hello</p>");
        inject_anchor_into_first_open_tag(&mut buf, 1);
        assert_eq!(buf, r#"<p data-aozora-md-source-line="1">hello</p>"#);
    }

    #[test]
    fn injects_after_self_closing_void_tag() {
        let mut buf = String::from("<hr />");
        inject_anchor_into_first_open_tag(&mut buf, 5);
        assert_eq!(buf, r#"<hr data-aozora-md-source-line="5" />"#);
    }

    #[test]
    fn injects_after_void_open_without_slash() {
        let mut buf = String::from("<hr>after");
        inject_anchor_into_first_open_tag(&mut buf, 9);
        assert_eq!(buf, r#"<hr data-aozora-md-source-line="9">after"#);
    }

    #[test]
    fn preserves_existing_attributes() {
        let mut buf = String::from(r#"<p class="x">y</p>"#);
        inject_anchor_into_first_open_tag(&mut buf, 2);
        assert_eq!(buf, r#"<p data-aozora-md-source-line="2" class="x">y</p>"#);
    }

    #[test]
    fn injects_on_block_with_leading_whitespace() {
        let mut buf = String::from("\n<h1>title</h1>\n");
        inject_anchor_into_first_open_tag(&mut buf, 3);
        assert_eq!(buf, "\n<h1 data-aozora-md-source-line=\"3\">title</h1>\n");
    }

    #[test]
    fn skips_html_comments_and_doctype() {
        let mut buf = String::from("<!-- note -->\n<p>x</p>");
        inject_anchor_into_first_open_tag(&mut buf, 1);
        assert_eq!(
            buf,
            "<!-- note -->\n<p data-aozora-md-source-line=\"1\">x</p>"
        );
    }

    #[test]
    fn no_op_when_no_open_tag_present() {
        let mut buf = String::from("just text");
        inject_anchor_into_first_open_tag(&mut buf, 1);
        assert_eq!(buf, "just text");
    }

    #[test]
    fn injects_only_into_first_top_level_block() {
        // The walker injects per top-level block; this helper only
        // touches the first tag of the buffer it sees. The caller
        // (`format_root_with_anchors`) feeds it one child at a time,
        // so child-internal nested tags never match the "first open
        // tag" of *their* buffer slice.
        let mut buf = String::from("<blockquote><p>x</p></blockquote>");
        inject_anchor_into_first_open_tag(&mut buf, 1);
        assert_eq!(
            buf,
            r#"<blockquote data-aozora-md-source-line="1"><p>x</p></blockquote>"#
        );
    }
}
