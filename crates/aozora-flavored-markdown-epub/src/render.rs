//! Phase 2 — render aozora-flavored-markdown sources into XHTML spine items.
//!
//! Each source is decoded (UTF-8, or Shift_JIS for `.sjis` /
//! `.shift_jis`), rendered by [`aozora_flavored_markdown::render`], and
//! wrapped in an XHTML (HTML5 doctype) envelope carrying the manuscript
//! language and a stylesheet link.

use std::str;

use aozora_flavored_markdown::{Options, render};

use crate::discover::{Manuscript, SourceFile};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub(crate) struct SpineItem {
    /// Filename used inside the EPUB, e.g. `chapter-001.xhtml`.
    pub href: String,
    /// `<title>` element of the chapter.
    pub title: String,
    /// Full XHTML document — already HTML-escaped by aozora-flavored-markdown.
    pub xhtml: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderOutput {
    pub items: Vec<SpineItem>,
}

pub(crate) fn render_all(manuscript: &Manuscript) -> Result<RenderOutput> {
    let opts = Options::default();
    let mut items = Vec::with_capacity(manuscript.sources.len());
    for (idx, source) in manuscript.sources.iter().enumerate() {
        let text = decode_source(source)?;
        let rendered = render(&text, &opts);
        let title = source
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_owned();
        let xhtml = wrap_xhtml(&title, &rendered.html, &manuscript.metadata.language);
        items.push(SpineItem {
            href: format!("chapter-{:03}.xhtml", idx + 1),
            title,
            xhtml,
        });
    }
    Ok(RenderOutput { items })
}

fn decode_source(source: &SourceFile) -> Result<String> {
    let ext = source
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("sjis" | "shift_jis" | "shift-jis")) {
        aozora::decode_sjis(&source.bytes).map_err(Error::from)
    } else {
        str::from_utf8(&source.bytes)
            .map(str::to_owned)
            .map_err(Error::from)
    }
}

fn wrap_xhtml(title: &str, body_html: &str, lang: &str) -> String {
    let title = escape_attr(title);
    let lang = escape_attr(lang);
    // The body opts into the bundled theme via `aozora-md-root`. The
    // writing mode (horizontal vs. vertical) is decided by which theme
    // `aozora-md.css` carries, selected per book in `compose`, so the
    // XHTML itself is writing-mode agnostic.
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{lang}" lang="{lang}">
  <head>
    <meta charset="utf-8" />
    <title>{title}</title>
    <link rel="stylesheet" type="text/css" href="css/aozora-md.css" />
  </head>
  <body class="aozora-md-root">
{body_html}
  </body>
</html>
"#,
    )
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    // The stylesheets this crate bundles come from the renderer's `theme`
    // feature, and so does the class contract they answer, so whether they
    // cover it is settled there — `css_class_contract.rs` sweeps both
    // directions with a selector tokeniser (ADR-0020). Restating that here
    // as a substring search would read as a second gate while passing on
    // any prefix: `.aozora-md-font-large` is a substring of the unrelated
    // `.aozora-md-font-larger` rule, and a fifth of the contract has a
    // longer sibling like that. What is this crate's own to check is the
    // wrapper it writes around the rendered HTML.
    use std::path::PathBuf;

    use aozora_flavored_markdown::theme;

    use super::*;

    /// The wrapper opts into the bundled theme via the `aozora-md-root`
    /// body class and the `aozora-md.css` link; both themes must
    /// define that root selector or the theme never applies.
    #[test]
    fn wrapper_opts_into_the_bundled_theme() {
        let xhtml = wrap_xhtml("title", "", "ja");
        assert!(xhtml.contains("<body class=\"aozora-md-root\">"), "{xhtml}");
        assert!(xhtml.contains("href=\"css/aozora-md.css\""), "{xhtml}");
        assert!(theme::HORIZONTAL_CSS.contains(".aozora-md-root"));
        assert!(theme::VERTICAL_CSS.contains(".aozora-md-root"));
    }

    /// A `.sjis` source is decoded through the `Shift_JIS` branch:
    /// `[0x82, 0xA0]` is the SJIS encoding of `"あ"`.
    #[test]
    fn decode_source_decodes_shift_jis_extension() {
        let source = SourceFile {
            path: PathBuf::from("x.sjis"),
            bytes: vec![0x82, 0xA0],
        };
        let text = decode_source(&source).expect("valid Shift_JIS should decode");
        assert_eq!(text, "あ");
    }

    /// A plain `.md` source takes the UTF-8 branch and is decoded verbatim.
    #[test]
    fn decode_source_decodes_markdown_as_utf8() {
        let source = SourceFile {
            path: PathBuf::from("chapter.md"),
            bytes: "あ".as_bytes().to_vec(),
        };
        let text = decode_source(&source).expect("valid UTF-8 should decode");
        assert_eq!(text, "あ");
    }
}
