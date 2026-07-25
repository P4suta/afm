//! The HTML one 青空文庫 construct renders to.
//!
//! The parser renders documents, not constructs, so this module asks it the
//! only question it answers: it parses the construct's own source run as a
//! document of its own and keeps the HTML that comes back. Exactly one
//! construct fills that run — [`crate::constructs`] shows that before it
//! publishes the run — so what comes back is that construct's markup and
//! nothing else.
//!
//! Two adjustments follow, and they are the whole module:
//!
//! * The paragraph wrapper an inline construct arrives in is dropped. Block
//!   structure belongs to comrak here, not to the sub-document; a construct
//!   that renders as a block of its own (a page break, a container marker)
//!   arrives without a wrapper and passes straight through.
//! * The classes are rebranded (ADR-0011): the parser emits its own
//!   `aozora-*` brand, this crate's HTML uses `aozora-md-*`. Only class
//!   attribute values are rewritten — a fragment is the construct's markup
//!   wrapped around the *author's* text, and that text may say `aozora-`
//!   for reasons of its own. This crate's own name does.
//!
//! Asking the renderer is what keeps the two sides from drifting. The
//! alternative — deriving the markup from the construct's classification —
//! would make this crate a second owner of the notation, and every notation
//! the parser grew would silently render as something else here.

/// The wrapper the renderer puts around a document whose whole content is
/// inline. The pair is the one HTML shape this crate reads.
const PARAGRAPH_OPEN: &str = "<p>";
/// Closing half of [`PARAGRAPH_OPEN`], newline included: the renderer ends
/// every paragraph with one.
const PARAGRAPH_CLOSE: &str = "</p>\n";
/// Start of the one attribute whose value carries the parser's brand.
const CLASS_ATTRIBUTE: &str = "class=\"";
/// The parser's brand, and this crate's (ADR-0011).
const BRAND: &str = "aozora-";
/// This crate's brand.
const REBRAND: &str = "aozora-md-";

/// The fragment `run` renders to, where `run` is the source text of exactly
/// one construct.
pub(crate) fn render(run: &str) -> String {
    let html = aozora::Document::new(run).parse().to_html();
    let body = html
        .strip_prefix(PARAGRAPH_OPEN)
        .and_then(|inner| inner.strip_suffix(PARAGRAPH_CLOSE))
        .unwrap_or(html.as_str());
    rebrand(body)
}

/// `fragment` with every class token under the parser's brand rewritten to
/// this crate's.
///
/// Scanning for the attribute rather than for the brand is what keeps the
/// rewrite off the author's own text: the parser puts its brand in class
/// attributes and nowhere else, so a `aozora-` that reaches here outside
/// one belongs to the document.
fn rebrand(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(at) = rest.find(CLASS_ATTRIBUTE) {
        let value = at + CLASS_ATTRIBUTE.len();
        out.push_str(&rest[..value]);
        rest = &rest[value..];
        let Some(end) = rest.find('"') else {
            break;
        };
        out.push_str(&rest[..end].replace(BRAND, REBRAND));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_construct_loses_its_paragraph_wrapper() {
        assert_eq!(
            render("｜青梅《おうめ》"),
            "<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>"
        );
    }

    #[test]
    fn block_construct_arrives_unwrapped() {
        assert_eq!(
            render("［＃改ページ］"),
            r#"<div class="aozora-md-page-break"></div>"#
        );
    }

    #[test]
    fn container_markers_render_to_their_own_halves() {
        assert!(
            render("［＃ここから字下げ］").starts_with(r#"<div class="aozora-md-container"#),
            "container open: {}",
            render("［＃ここから字下げ］")
        );
        assert_eq!(render("［＃ここで字下げ終わり］"), "</div>");
    }

    #[test]
    fn a_forward_reference_resolves_inside_its_own_run() {
        // The run a forward reference occupies covers the text it points
        // back at, so parsing it on its own reaches the same target the
        // whole document would.
        assert_eq!(
            render("可哀想［＃「可哀想」に傍点］"),
            r#"<em class="aozora-md-bouten aozora-md-bouten-goma aozora-md-bouten-right">可哀想</em>"#
        );
    }

    #[test]
    fn brand_rewrite_leaves_unbranded_markup_alone() {
        assert_eq!(render("ただの文"), "ただの文");
    }

    #[test]
    fn brand_rewrite_leaves_the_authors_own_text_alone() {
        // This crate's own name, written by the author, is not a class.
        assert_eq!(
            render("｜aozora-flavored《エーエフエム》"),
            "<ruby>aozora-flavored<rp>(</rp><rt>エーエフエム</rt><rp>)</rp></ruby>"
        );
        assert_eq!(
            render("aozora-md［＃「aozora-md」に傍点］"),
            r#"<em class="aozora-md-bouten aozora-md-bouten-goma aozora-md-bouten-right">aozora-md</em>"#
        );
    }

    #[test]
    fn brand_rewrite_survives_an_unterminated_attribute() {
        // Not markup this renderer produces, but the scan must terminate
        // and keep what it was given either way.
        assert_eq!(rebrand("<i class=\"aozora-x"), "<i class=\"aozora-x");
    }
}
