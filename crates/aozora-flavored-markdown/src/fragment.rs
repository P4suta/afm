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
//! Two adjustments follow: the paragraph wrapper an inline construct arrives
//! in is dropped (block structure is comrak's here, not the sub-document's),
//! and classes are rebranded `aozora-*` → `aozora-md-*` (ADR-0011). Only class
//! attribute values are rewritten, because the author's own text may say
//! `aozora-` for reasons of its own — this crate's name does.
//!
//! The same rule the first adjustment rests on is also a question this module
//! answers, [`is_inline_unit`]: a run whose render carries block structure of
//! its own has no paragraph wrapper to drop and no inline position to be
//! spliced into. Both live here because both read the same markup.

// The two brands live next to the contract they define — the published
// `classes::all()` is the parser's own list under the rewritten brand — so
// the rewrite below cannot be changed without the contract following it.
use std::borrow::Cow;

use crate::classes::{BRAND, PREFIX};

/// The wrapper the renderer puts around an all-inline document.
const PARAGRAPH_OPEN: &str = "<p>";
const PARAGRAPH_CLOSE: &str = "</p>";
/// Every block-level tag the sibling renderer opens. A fragment carrying one
/// is a document in its own right rather than a phrase, which is what
/// [`is_inline_unit`] is asked about. Headings are spelled out because the
/// renderer computes the level, and `<p>` is only ever emitted bare.
const BLOCK_TAGS: &[&str] = &[
    PARAGRAPH_OPEN,
    PARAGRAPH_CLOSE,
    "<div",
    "<figure",
    "<h1",
    "<h2",
    "<h3",
    "<h4",
    "<h5",
    "<h6",
];
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
        // A *single* paragraph's wrapper is the only one that is ours to
        // drop. Taking the outermost pair off a document that renders as
        // several paragraphs leaves the boundary between them behind —
        // `A</p>\n<p>B` — which is no longer a fragment at all: spliced into
        // a line, its stray close closes whatever comrak had open, and a
        // `<li>` ends up closed by a `</p>` (Tier D). A run reaches that
        // shape by covering a blank line, which one aozora node's own span
        // can do without the fold ever widening it.
        .filter(|inner| !inner.contains(PARAGRAPH_CLOSE))
        .unwrap_or(html);
    rebrand(body)
}

/// Whether `fragment` can be woven into a line comrak owns.
///
/// Block structure is comrak's — the rule [`crate::constructs`]'s fold states
/// as "a fold never crosses a line". A run can carry block structure without
/// the fold widening it, though: a single node's span may cover a blank line,
/// or a block notation. Reading it back off the rendered fragment catches both
/// spellings, and is what keeps our tags from interleaving with comrak's.
pub(crate) fn is_inline_unit(fragment: &str) -> bool {
    !BLOCK_TAGS.iter().any(|tag| fragment.contains(tag))
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

    /// A run reaching over a blank line renders as two paragraphs, and the
    /// wrapper of the first is not the wrapper of the document.
    #[test]
    fn a_multi_paragraph_run_keeps_the_paragraphs_it_rendered() {
        let two = render("あ\n\nい");
        assert_eq!(two, "<p>あ</p>\n<p>い</p>");
        assert!(
            !is_inline_unit(&two),
            "two paragraphs cannot be woven into a line: {two}"
        );

        // A single newline is still one paragraph, so the wrapper is ours to
        // drop — the case `of` exists for.
        let one = render("あ\nい");
        assert_eq!(one, "あ<br />\nい");
        assert!(is_inline_unit(&one), "one paragraph is a phrase: {one}");
    }

    #[test]
    fn is_inline_unit_reads_every_block_tag_the_renderer_opens() {
        assert!(is_inline_unit(""));
        assert!(is_inline_unit("<ruby>青梅<rt>おうめ</rt></ruby>"));
        assert!(is_inline_unit(
            r#"<span class="aozora-md-directive" hidden>［＃改丁］</span>"#
        ));
        assert!(is_inline_unit("あ<br />\nい"));
        // The shape this guard was written for: a stray close and a stray
        // open, left behind by unwrapping a two-paragraph document.
        assert!(!is_inline_unit("あ</p>\n<p>い"));
        // A block notation the run swallowed, in every spelling the sibling
        // renderer emits.
        assert!(!is_inline_unit(&render("［＃改ページ］")));
        for level in 1..=6 {
            assert!(!is_inline_unit(&format!("<h{level}>章</h{level}>")));
        }
        assert!(!is_inline_unit("<figure><img /></figure>"));
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
