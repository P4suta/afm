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
use std::borrow::Cow;

use crate::classes::{BRAND, PREFIX};

/// The wrapper the renderer puts around an all-inline document.
const PARAGRAPH_OPEN: &str = "<p>";
const PARAGRAPH_CLOSE: &str = "</p>";
/// The one attribute whose value carries the parser's brand.
const CLASS_ATTRIBUTE: &str = "class=\"";
const DIRECTIVE_CLASS: &str = "aozora-md-directive";

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

/// Removes a directive-bearing ruby's reading while retaining its parent
/// text, which is the only meaningful part of that notation in a Markdown
/// heading.
///
/// The sibling renderer emits ruby fallback parentheses in `<rp>` and the
/// reading in `<rt>`. Keeping only text outside those elements yields the
/// parent text without teaching this crate how ruby bases are inferred.
#[must_use]
pub(crate) fn for_markdown_heading(fragment: &str) -> Cow<'_, str> {
    if !fragment.contains(DIRECTIVE_CLASS) || !fragment.contains("<ruby") {
        return Cow::Borrowed(fragment);
    }

    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    let mut skipped_depth = 0usize;
    while let Some(at) = rest.find('<') {
        if skipped_depth == 0 {
            out.push_str(&rest[..at]);
        }
        rest = &rest[at..];
        let Some(end) = rest.find('>') else {
            if skipped_depth == 0 {
                out.push_str(rest);
            }
            return Cow::Owned(out);
        };
        let tag = &rest[1..end];
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        match (closing, name) {
            (false, "rt" | "rp") => skipped_depth += 1,
            (true, "rt" | "rp") => skipped_depth = skipped_depth.saturating_sub(1),
            // The ruby framing itself is discarded; parent text between its
            // tags is copied by the text arm above.
            (_, "ruby") => {}
            // A directive-bearing reading is wholly inside `rt`, so markup
            // outside it belongs to the parent text and remains intact.
            _ if skipped_depth == 0 => out.push_str(&rest[..=end]),
            _ => {}
        }
        rest = &rest[end + 1..];
    }
    if skipped_depth == 0 {
        out.push_str(rest);
    }
    Cow::Owned(out)
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

    #[test]
    fn a_directive_reading_in_a_heading_keeps_only_the_ruby_parent() {
        let fragment = concat!(
            "改<ruby>ページ<rp>(</rp><rt>",
            r#"<span class="aozora-md-directive" hidden>［＃２字下げ］</span>"#,
            "</rt><rp>)</rp></ruby>"
        );
        assert_eq!(for_markdown_heading(fragment), "改ページ");
        assert!(matches!(
            for_markdown_heading("<ruby>漢字<rt>かんじ</rt></ruby>"),
            Cow::Borrowed(_)
        ));
    }
}
