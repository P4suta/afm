//! The HTML one 青空文庫 construct renders to.
//!
//! The parser renders documents, not constructs, so this asks it the only
//! question it answers: parse the construct's own source run as a document
//! and keep the HTML. Exactly one construct fills that run, which
//! [`crate::constructs`] establishes before publishing it.
//!
//! Asking the renderer is what keeps the two sides from drifting. Deriving
//! the markup from a classification instead would make this crate a second
//! owner of the notation, and every notation the parser grew would silently
//! render as something else here.
//!
//! Two adjustments follow, and they are the whole module: the paragraph
//! wrapper an inline construct arrives in is dropped (block structure is
//! comrak's here, not the sub-document's), and classes are rebranded
//! `aozora-*` → `aozora-md-*` (ADR-0011). Only class attribute values are
//! rewritten, because the author's own text may say `aozora-` for reasons of
//! its own — this crate's name does.

// The two brands live next to the contract they define — the published
// `classes::all()` is the parser's own list under the rewritten brand — so
// the rewrite below cannot be changed without the contract following it.
use crate::classes::{BRAND, PREFIX};

/// The wrapper the renderer puts around an all-inline document.
const PARAGRAPH_OPEN: &str = "<p>";
const PARAGRAPH_CLOSE: &str = "</p>";
/// The one attribute whose value carries the parser's brand.
const CLASS_ATTRIBUTE: &str = "class=\"";

/// Takes a snapshot rather than the run itself because the same reading
/// answers the other question [`crate::constructs`] asks of a run.
pub(crate) fn of(snapshot: &aozora::Snapshot) -> String {
    let html = snapshot.to_html();
    // The renderer ends a document with a newline; a fragment is woven into
    // a line of comrak's making, so the document's own line break is not
    // ours to keep.
    let html = html.trim_end_matches('\n');
    let body = html
        .strip_prefix(PARAGRAPH_OPEN)
        .and_then(|inner| inner.strip_suffix(PARAGRAPH_CLOSE))
        .unwrap_or(html);
    rebrand(body)
}

/// A container marker with nothing inside renders as one empty element, so
/// the opening markup is everything before its closing tag. Reading both off
/// the *open* is what lets a close — which renders to nothing on its own —
/// be spliced at all.
pub(crate) fn halves(fragment: &str) -> (&str, &str) {
    fragment
        .rfind("</")
        .map_or((fragment, ""), |at| fragment.split_at(at))
}

/// Scanning for the attribute rather than for the brand keeps the rewrite
/// off the author's own text: the parser puts its brand in class attributes
/// and nowhere else, so an `aozora-` outside one belongs to the document.
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
        out.push_str(&rest[..end].replace(BRAND, PREFIX));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run past the parser's span budget renders to nothing, exactly as
    /// it does on the production path.
    fn render(run: &str) -> String {
        aozora::parse(run.to_owned())
            .map_or_else(|_| String::new(), |document| of(&document.snapshot()))
    }

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
        let fragment = render("［＃ここから字下げ］");
        let (open, close) = halves(&fragment);
        assert!(
            open.starts_with(r#"<div class="aozora-md-container"#),
            "container open: {open}"
        );
        assert_eq!(close, "</div>");
        // A close has no open to close, so on its own it renders to
        // nothing — which is why the halves are read off the open.
        assert_eq!(render("［＃ここで字下げ終わり］"), "");
    }

    #[test]
    fn halves_of_an_unpaired_fragment_are_the_whole_and_nothing() {
        assert_eq!(halves("plain"), ("plain", ""));
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
