//! Public IR type definitions. The `serde` attributes are the single source
//! of the wire shape: under the `tsify` feature these derive the TypeScript
//! `IRDocument` directly, so there is no hand-written `.d.ts` to keep in sync
//! (ADR-0017).
//!
//! The Markdown vocabulary is **owned here** — one typed variant each,
//! because this crate decides what they mean. The 青空文庫 vocabulary is
//! **not**: every notation collapses to [`IrBlock::Aozora`] /
//! [`IrInline::Aozora`] with an opaque `kind` tag, because mirroring the
//! sibling parser's type vocabulary would own it twice (ADR-0021).

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
    /// A 青空文庫 construct occupying a whole block.
    ///
    /// Containers are **not** nested here: their two markers are separate
    /// blocks in document order, so concatenating `html` reproduces the
    /// nesting without this crate re-deriving it.
    Aozora {
        /// Opaque notation tag. See [`IrInline::Aozora`]'s `kind`.
        #[serde(rename = "aozoraKind")]
        kind: String,
        /// As [`IrInline::Aozora`]'s `span`, and additionally `None` for a
        /// close marker synthesised because the document ended with the
        /// container still open.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
        /// Rebranded to `aozora-md-*` classes (ADR-0011).
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
    /// CommonMark image. `alt` carries the alt-text inlines as comrak parses
    /// them, so it is a list rather than a string.
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
    /// A 青空文庫 construct inside a text run: ruby, bouten, 縦中横, 外字,
    /// 返り点, a bracket annotation, …
    ///
    /// One variant covers all of them on purpose — the notation vocabulary
    /// is the sibling parser's to define (ADR-0021).
    Aozora {
        /// Opaque notation tag, serialised as `aozoraKind` because `kind` is
        /// already the union's discriminant. An **open** string set: an
        /// unrecognised tag just means a notation newer than the consumer,
        /// and `html` still renders it.
        #[serde(rename = "aozoraKind")]
        kind: String,
        /// End-exclusive byte range: slicing the source you passed in
        /// recovers the text the author wrote.
        ///
        /// `None` when that promise cannot be kept. The parser measures
        /// spans against its *normalised* text, and normalisation moves
        /// bytes (BOM stripped, `\r\n` folded, accent digraphs combined,
        /// decorative rules given a blank line), so the offsets would
        /// address a different — possibly mid-codepoint — position.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
        /// Rebranded to `aozora-md-*` classes (ADR-0011), and byte-identical
        /// to the run this notation contributes to [`crate::render`] —
        /// empty included. A notation the HTML suppresses in context is
        /// suppressed here too, so rendering from the IR cannot produce
        /// markup the document does not have.
        html: String,
    },
}

/// Source-position range, end-exclusive.
///
/// Carries comrak's own 1-based line / column coordinates, so a `CodeMirror`
/// bridge maps them to editor positions without UTF-8 byte arithmetic —
/// which the previous pseudo-byte representation silently broke for
/// multi-byte CJK.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// `column` is grapheme-blind, matching comrak, so it suits editor surfaces
/// but not byte slicing.
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

    /// The error is unreachable: `serde_json` only fails for a custom
    /// `Serialize` that errors, which none of these have.
    fn json<T: Serialize>(value: T) -> serde_json::Value {
        match serde_json::to_value(value) {
            Ok(v) => v,
            Err(err) => panic!("IR values must serialise: {err}"),
        }
    }

    /// Locks both keys: a plain `kind` field would emit a duplicate JSON
    /// key and the last one would silently win on the JS side.
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

    /// An absent span is omitted rather than emitted as `null`, the
    /// contract every optional IR field follows.
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
