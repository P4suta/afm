//! Phase 2 — decode each source at the encoding its extension names in
//! `discover`, render it, and wrap it in an XHTML envelope carrying the
//! manuscript language and a stylesheet link.

use std::str;

use aozora_flavored_markdown::{Options, escape_html, render};

use crate::discover::{Encoding, Manuscript, SourceFile, encoding_of};
use crate::{ChapterReport, Error, Result};

#[derive(Debug, Clone)]
pub(crate) struct SpineItem {
    /// Filename used inside the EPUB, e.g. `chapter-001.xhtml`.
    pub href: String,
    /// `<title>` element of the chapter.
    pub title: String,
    /// Already HTML-escaped by the renderer.
    pub xhtml: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderOutput {
    pub items: Vec<SpineItem>,
    // What each chapter's render observed. Only the chapters that saw
    // something are in here, so the happy path carries nothing.
    pub chapters: Vec<ChapterReport>,
}

pub(crate) fn render_all(manuscript: &Manuscript) -> Result<RenderOutput> {
    let opts = Options::default();
    let mut items = Vec::with_capacity(manuscript.sources.len());
    let mut chapters = Vec::new();
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
        // The decoded text moves into the report rather than being cloned:
        // it is what the spans were measured against, and nothing after
        // this point reads it.
        if !rendered.diagnostics.is_empty() {
            chapters.push(ChapterReport {
                path: source.path.clone(),
                text,
                diagnostics: rendered.diagnostics,
            });
        }
    }
    Ok(RenderOutput { items, chapters })
}

fn decode_source(source: &SourceFile) -> Result<String> {
    // The extension table in `discover` is the only thing that says Shift_JIS,
    // so the set this branches on and the set a directory sweep collects are
    // the same set by construction rather than by two lists agreeing.
    if encoding_of(&source.path) == Some(Encoding::ShiftJis) {
        aozora::decode_sjis(&source.bytes).map_err(|e| Error::sjis(source.path.clone(), e))
    } else {
        str::from_utf8(&source.bytes)
            .map(str::to_owned)
            .map_err(|e| Error::Utf8 {
                path: source.path.clone(),
                source: e,
            })
    }
}

fn wrap_xhtml(title: &str, body_html: &str, lang: &str) -> String {
    // The renderer owns the escape table; a copy here could gain a character
    // on its own and leave one of the two sides open.
    let title = escape_html(title);
    let lang = escape_html(lang);
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
    use aozora_flavored_markdown_test_support::{check_well_formed, config};
    use proptest::prelude::*;
    use quick_xml::XmlVersion;
    use quick_xml::escape::resolve_predefined_entity;
    use quick_xml::events::{BytesRef, Event};
    use quick_xml::reader::Reader;

    use super::*;

    // Titles a chapter file stem can really carry. `<` and `&` are legal in a
    // POSIX filename, so every one of these is one `touch` away.
    const HOSTILE_TITLES: &[&str] = &[
        "plain",
        "<script>alert(1)</script>",
        "a & b",
        "\"quoted\"",
        "it's",
        "&amp;",
        "図 <b>",
    ];

    // The alphabet the two slots are quantified over: the five characters the
    // escape table owns, the entity syntax itself (so an escaper that skips
    // what already looks like an entity gives itself away), and ordinary
    // text. Two classes are deliberately absent, and each absence is a bug
    // this crate still has rather than a property of the escaper:
    //
    //  * `\t`, `\n`, `\r`. Attribute-value normalisation (XML 1.0 §3.3.3)
    //    turns each into a space before a parser hands the value over, and
    //    end-of-line normalisation (§2.11) rewrites CR in text content to LF.
    //    Markup significance is all `escape_html` covers, so a `book.toml`
    //    `language = "ja\n"` and a chapter file named `序\r.md` both reach
    //    the reader altered. Only numeric references (`&#9;` `&#10;` `&#13;`)
    //    survive normalisation, and nothing emits them.
    //  * C0 controls and the non-characters. XML 1.0 §2.2 forbids them
    //    outright, so a file stem holding one yields a chapter that is not
    //    well-formed XML at all. `quick_xml` does not enforce that
    //    constraint (its own docs say so of `resolve_char_ref`) and neither
    //    does anything else here — but epubcheck and a reading system do.
    const ENVELOPE_ATOMS: &[&str] = &[
        "&", "<", ">", "\"", "'", "amp;", "#39;", "lt", "a", "第", " ", "-",
    ];

    // The hostile pool stays reachable by selection rather than being
    // replaced: the shapes someone thought to write down keep running on
    // every draw budget, and the generated ones extend past them.
    fn envelope_text() -> impl Strategy<Value = String> {
        prop_oneof![
            prop::sample::select(HOSTILE_TITLES).prop_map(str::to_owned),
            prop::collection::vec(prop::sample::select(ENVELOPE_ATOMS), 0..8)
                .prop_map(|parts| parts.concat()),
        ]
    }

    // What a conforming XML parser hands back for the two interpolated slots.
    // `quick_xml` is the oracle rather than `check_well_formed` because
    // balance is only half of what the envelope owes: a title that arrives as
    // `&amp;amp;` leaves every tag matched and is still corrupt, and a
    // language tag that lost its escaping is only visible as *imbalance* when
    // the payload happens to open a tag.
    fn read_back(xhtml: &str) -> (String, String) {
        let mut reader = Reader::from_str(xhtml);
        let decoder = reader.decoder();
        let (mut title, mut lang) = (String::new(), String::new());
        let mut in_title = false;
        loop {
            match reader.read_event().expect("the envelope must parse as XML") {
                Event::Eof => break,
                Event::Start(tag) => {
                    in_title = tag.name().as_ref() == b"title";
                    if tag.name().as_ref() == b"html" {
                        for attr in tag.attributes() {
                            let attr = attr.expect("every attribute must parse");
                            if attr.key.as_ref() == b"lang" {
                                lang = attr
                                    .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
                                    .expect("the language tag must decode")
                                    .into_owned();
                            }
                        }
                    }
                }
                Event::End(_) => in_title = false,
                Event::Text(text) if in_title => title.push_str(
                    &text
                        .xml10_content()
                        .expect("the title text must decode as XML 1.0"),
                ),
                Event::GeneralRef(reference) if in_title => {
                    title.push_str(&resolve(&reference));
                }
                _ => {}
            }
        }
        (title, lang)
    }

    // `&amp;` and its four siblings arrive as events of their own, so a title
    // is reassembled from text and references rather than read whole.
    fn resolve(reference: &BytesRef<'_>) -> String {
        if let Some(ch) = reference
            .resolve_char_ref()
            .expect("a numeric reference must resolve")
        {
            return ch.into();
        }
        let name = reference.decode().expect("a reference name must decode");
        resolve_predefined_entity(&name)
            .expect("the envelope emits no entity outside the predefined five")
            .to_owned()
    }

    // Every other XHTML in the package goes through `quick_xml`, which escapes
    // on the caller's behalf. This wrapper is the one document built by string
    // interpolation, so the title and the language tag are the two places
    // user-controlled text reaches markup with only `escape_html` in front of
    // it — and an unescaped `<` there closes `<title>` early and unbalances
    // the whole document. The body is a real render rather than a literal, so
    // the check covers the seam between the envelope and the HTML too.
    //
    // Balance is the whole of what it sees, and less than it looks: a `"`
    // that escapes its attribute moves no tag boundary at all, because the
    // same table escapes the `<` that would have opened one. Drop the `"` arm
    // and this test still passes. The property below is the other half.
    #[test]
    fn the_xhtml_envelope_stays_balanced_whatever_the_title_and_language_hold() {
        for title in HOSTILE_TITLES {
            for lang in ["ja", "en-US", "\"><x"] {
                let body = render(&format!("# {title}\n\n{title}\n"), &Options::default()).html;
                let xhtml = wrap_xhtml(title, &body, lang);
                let errors = check_well_formed(&xhtml);
                assert!(
                    errors.is_empty(),
                    "title {title:?} / lang {lang:?} produced ill-formed XHTML: {errors:?}\n\
                     {xhtml}"
                );
                assert!(
                    !xhtml.contains("<script>"),
                    "an unescaped tag reached the envelope: {xhtml}"
                );
            }
        }
    }

    proptest! {
        #![proptest_config(config::default())]

        // Balance is what the pool above states; fidelity is what only a real
        // parser can. Both slots are quantified because they sit in different
        // XML contexts — element content and a quoted attribute value — and
        // the crate now trusts one escape table to be right in both. A table
        // that stopped escaping `"` keeps `<title>` intact and blows the
        // attribute open; one that double-encoded would keep every document
        // balanced and hand the reader the wrong book title.
        #[test]
        fn the_envelope_hands_back_the_title_and_language_it_was_given(
            title in envelope_text(),
            lang in envelope_text(),
        ) {
            let xhtml = wrap_xhtml(&title, "<p>本文</p>", &lang);
            let (read_title, read_lang) = read_back(&xhtml);
            prop_assert_eq!(read_title, title);
            prop_assert_eq!(read_lang, lang);
        }
    }

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
