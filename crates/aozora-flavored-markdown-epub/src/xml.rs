//! EPUB-private XML escaping.
//!
//! HTML and XML share the five markup-significant escapes, so that table
//! stays owned by the renderer. XML 1.0 adds one preservation requirement:
//! literal TAB/LF/CR in attributes are normalised by a reader, and literal CR
//! in character data is end-of-line normalised. Numeric references keep the
//! authored scalar in both locations.

use aozora_flavored_markdown::escape_html;

pub(crate) fn escape(value: &str) -> String {
    let escaped = escape_html(value);
    let mut output = String::with_capacity(escaped.len());
    for ch in escaped.chars() {
        match ch {
            '\t' => output.push_str("&#9;"),
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            _ => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_and_xml_whitespace_are_escaped_once() {
        assert_eq!(
            escape("a&<>'\"\t\n\r"),
            "a&amp;&lt;&gt;&#39;&quot;&#9;&#10;&#13;"
        );
    }
}
