//! Public IR type definitions.
//!
//! Every type here is part of the `aozora_flavored_markdown::ir` public
//! surface. Under the `tsify` feature (enabled by aozora-flavored-markdown-wasm)
//! each derives `tsify::Tsify`, so wasm-pack emits the matching TypeScript
//! `IRDocument` — consumed by the playground and aozora-flavored-markdown-obsidian
//! — straight from these definitions, with no hand-written `.d.ts` to keep in
//! sync (ADR-0017). The `serde` attributes are the single source of the wire
//! shape.
//!
//! # Two halves, two rules
//!
//! The Markdown vocabulary is **owned here**: paragraphs, headings, lists,
//! tables, code, links and images each get their own typed variant, because
//! this crate is the thing that decides what they mean.
//!
//! The 青空文庫 vocabulary is **not**. Every notation collapses to one
//! variant per level — [`IrBlock::Aozora`] and [`IrInline::Aozora`] — carrying
//! an opaque `kind` tag, the source `span`, and the rendered `html` fragment.
//! Mirroring the notation's own type vocabulary here would own it twice
//! (ADR-0021); a new notation upstream now lands in the IR as a new `kind`
//! string instead of a new Rust variant.

use serde::Serialize;

#[doc(inline)]
pub use crate::diagnostics::Span;

#[derive(Debug, Default, Clone, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub struct IrDocument {
    pub blocks: Vec<IrBlock>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
// New Markdown constructs land as new variants; `#[non_exhaustive]`
// (ADR-0013) lets that happen in a minor release without breaking external
// `match`es. New 青空文庫 notations do not need a variant at all — they
// arrive as a new `kind` on `IrBlock::Aozora` (ADR-0022).
#[non_exhaustive]
pub enum IrBlock {
    Paragraph {
        children: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Heading {
        level: u8,
        children: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Blockquote {
        children: Vec<IrBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    List {
        ordered: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u32>,
        items: Vec<IrListItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    CodeBlock {
        #[serde(skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        value: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    ThematicBreak {
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Table {
        header: IrTableRow,
        rows: Vec<IrTableRow>,
        align: Vec<IrTableAlign>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// A 青空文庫 construct that occupies a whole block: `［＃改ページ］`,
    /// `［＃改丁］`, an illustration, or one marker of a paired container.
    ///
    /// Containers are **not** nested here. Their open and close markers are
    /// two separate blocks (`kind` = `"containerOpen"` / `"containerClose"`)
    /// carrying the opening and closing HTML, in the same document order the
    /// rendered HTML uses — so concatenating `html` across the document
    /// reproduces the nesting without this crate re-deriving it.
    Aozora {
        /// Opaque notation tag. See [`IrInline::Aozora`]'s `kind`.
        #[serde(rename = "aozoraKind")]
        kind: String,
        /// Byte range of the marker in the source, end-exclusive, under the
        /// same rules as [`IrInline::Aozora`]'s `span`. Additionally `None`
        /// for a close marker this crate synthesised because the document
        /// ended with the container still open.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
        /// Rendered HTML for this marker, already rebranded to
        /// `aozora-md-*` classes (ADR-0011).
        html: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
pub struct IrTableRow {
    pub cells: Vec<Vec<IrInline>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
pub struct IrListItem {
    pub children: Vec<IrBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub enum IrTableAlign {
    Left,
    Center,
    Right,
    Default,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
// See `IrBlock`: `#[non_exhaustive]` (ADR-0013) keeps new inline Markdown
// constructs additive for external consumers.
#[non_exhaustive]
pub enum IrInline {
    Text {
        value: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Code {
        value: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Strong {
        children: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Emphasis {
        children: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Link {
        href: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        children: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// CommonMark image. `alt` carries the alt-text inlines exactly
    /// as comrak parses them (typically a single `Text`). `url` is
    /// the image source; `title` is the optional `"…"` argument.
    Image {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        alt: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    LineBreak {
        hard: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// A 青空文庫 construct sitting inside a text run: ruby, emphasis
    /// dots, 縦中横, 外字, a 返り点, a bracket annotation, …
    ///
    /// One variant covers all of them on purpose. The notation's own
    /// vocabulary is the sibling parser's to define (ADR-0021); reproducing
    /// it as Rust variants here would own it twice and force this crate to
    /// grow a variant every time upstream grows a notation.
    Aozora {
        /// Opaque notation tag — `"ruby"`, `"bouten"`, `"gaiji"`, … —
        /// serialised as `aozoraKind` because `kind` is already the
        /// union's discriminant. Treat it as an open string set: an
        /// unrecognised tag means a notation newer than the consumer,
        /// and `html` still renders it correctly.
        #[serde(rename = "aozoraKind")]
        kind: String,
        /// Byte range of the notation in the source, end-exclusive —
        /// slicing the source you passed in recovers the text the author
        /// wrote.
        ///
        /// `None` when that promise cannot be kept: the parser measures
        /// spans against its normalised text, and normalisation moves bytes
        /// (a leading BOM is stripped, `\r\n` folds to `\n`, accent
        /// digraphs inside `〔…〕` combine, decorative rules gain a blank
        /// line). On such an input the offsets would address a different —
        /// possibly mid-codepoint — position in your source, so no span is
        /// reported rather than a wrong one.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
        /// Rendered HTML for this notation, already rebranded to
        /// `aozora-md-*` classes (ADR-0011).
        ///
        /// Byte-identical to the run the same notation contributes to
        /// [`crate::render`]'s output — including the case where that run
        /// is empty: a notation the HTML suppresses in context (an
        /// annotation inside a heading body, which would contaminate it
        /// with `aozora-md-directive` markup) is suppressed here too, so
        /// rendering from the IR cannot produce markup the document does
        /// not have.
        html: String,
    },
}

/// Source-position range, end-exclusive.
///
/// `start` and `end` carry 1-based line / column coordinates straight
/// from comrak's `Sourcepos`. JS-side consumers (aozora-flavored-markdown-obsidian's
/// `CodeMirror` bridge) can map these to editor positions without
/// re-doing UTF-8 byte arithmetic, which the previous pseudo-byte
/// representation silently broke for multi-byte CJK content.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// 1-based line / column tuple. `column` is a UTF-8 grapheme-blind
/// column count (matching comrak's `Sourcepos`), so it is suitable
/// for editor surfaces but not for byte slicing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise an IR value, turning the (unreachable) error into a test
    /// failure. `serde_json` only fails here for types with a custom
    /// `Serialize` that errors, which none of these have.
    fn json<T: Serialize>(value: T) -> serde_json::Value {
        match serde_json::to_value(value) {
            Ok(v) => v,
            Err(err) => panic!("IR values must serialise: {err}"),
        }
    }

    /// The collapsed Aozora variants share the union's `kind` discriminant
    /// with their own notation tag, so the tag rides under `aozoraKind`.
    /// Lock both keys: a plain `kind` field here would silently emit a
    /// duplicate JSON key and the last one would win on the JS side.
    #[test]
    fn aozora_inline_wire_shape_separates_tag_from_discriminant() {
        let value = json(IrInline::Aozora {
            kind: "ruby".to_owned(),
            span: Some(Span { start: 3, end: 21 }),
            html: "<ruby>青梅<rt>おうめ</rt></ruby>".to_owned(),
        });

        assert_eq!(value["kind"], "aozora");
        assert_eq!(value["aozoraKind"], "ruby");
        assert_eq!(value["span"]["start"], 3);
        assert_eq!(value["span"]["end"], 21);
        assert_eq!(value["html"], "<ruby>青梅<rt>おうめ</rt></ruby>");
    }

    /// The block half uses the same two-key split, keeps `sourceLine`, and
    /// omits an absent span rather than emitting `null` — the same
    /// `skip_serializing_if` contract every other optional IR field follows.
    #[test]
    fn aozora_block_wire_shape_omits_absent_span() {
        let value = json(IrBlock::Aozora {
            kind: "containerClose".to_owned(),
            span: None,
            html: "</div>".to_owned(),
            source_line: Some(7),
        });

        assert_eq!(value["kind"], "aozora");
        assert_eq!(value["aozoraKind"], "containerClose");
        assert_eq!(value["html"], "</div>");
        assert_eq!(value["sourceLine"], 7);
        assert!(
            value.get("span").is_none(),
            "absent span must not serialise: {value}"
        );
    }
}
