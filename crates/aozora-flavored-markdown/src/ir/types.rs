//! Public IR type definitions. The `serde` attributes are the single source
//! of the wire shape: under the `tsify` feature these derive the TypeScript
//! declarations directly, so there is no hand-written `.d.ts` to keep in sync
//! (ADR-0017).
//!
//! The Markdown vocabulary is **owned here** — one typed variant each,
//! because this crate decides what they mean. The 青空文庫 vocabulary is
//! **not**: every notation collapses to [`Block::Aozora`] /
//! [`Inline::Aozora`] with an opaque `kind` tag, because mirroring the
//! sibling parser's type vocabulary would own it twice (ADR-0021).

#[doc(inline)]
pub use crate::diagnostics::Span;

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
// ADR-0013 sealed the IR enums; the containers around them are sealed for the
// same reason — document-level metadata (a source digest, a schema tag) is
// exactly the kind of field a later release adds.
#[non_exhaustive]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(
    feature = "serde",
    serde(
        tag = "kind",
        rename_all = "camelCase",
        rename_all_fields = "camelCase"
    )
)]
// New Markdown constructs land as new variants; `#[non_exhaustive]`
// (ADR-0013) lets that happen in a minor release without breaking external
// `match`es. New 青空文庫 notations do not need a variant at all — they
// arrive as a new `kind` on `Block::Aozora` (ADR-0022).
#[non_exhaustive]
pub enum Block {
    Paragraph {
        children: Vec<Inline>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Heading {
        level: u8,
        children: Vec<Inline>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Blockquote {
        children: Vec<Block>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    List {
        ordered: bool,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        start: Option<u32>,
        items: Vec<ListItem>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    // `Block::CodeBlock` stutters once the enum is `Block` (the same reason
    // the types lost their `Ir`), and `Inline::Code` is already the inline
    // half of the pair. The wire tag stays `codeBlock` — the tag names the
    // node across a union that has both halves in it, where `code` alone
    // would be ambiguous.
    #[cfg_attr(feature = "serde", serde(rename = "codeBlock"))]
    Code {
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        lang: Option<String>,
        value: String,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    ThematicBreak {
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Table {
        header: TableRow,
        rows: Vec<TableRow>,
        align: Vec<TableAlign>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    /// A 青空文庫 construct occupying a whole block.
    ///
    /// Containers are **not** nested here: their two markers are separate
    /// blocks in document order, so concatenating `html` reproduces the
    /// nesting without this crate re-deriving it.
    Aozora {
        /// Opaque notation tag. See [`Inline::Aozora`]'s `kind`.
        #[cfg_attr(feature = "serde", serde(rename = "aozoraKind"))]
        kind: String,
        /// As [`Inline::Aozora`]'s `span`, and additionally `None` for a
        /// close marker synthesised because the document ended with the
        /// container still open.
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        span: Option<Span>,
        /// Rebranded to `aozora-md-*` classes (ADR-0011).
        html: String,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        source_line: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
// See `Document`: a row gains fields (a header flag, a span) without the
// variants around it changing.
#[non_exhaustive]
pub struct TableRow {
    pub cells: Vec<Vec<Inline>>,
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub range: Option<Range>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
// As `TableRow`: GFM task-list state is the obvious next field here.
#[non_exhaustive]
pub struct ListItem {
    pub children: Vec<Block>,
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub range: Option<Range>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
// The IR enum ADR-0013 missed. `Default` here is the column's *absence* of an
// alignment marker, which is also the variant a fresh column starts from.
#[non_exhaustive]
pub enum TableAlign {
    Left,
    Center,
    Right,
    #[default]
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(
    feature = "serde",
    serde(
        tag = "kind",
        rename_all = "camelCase",
        rename_all_fields = "camelCase"
    )
)]
// See `Block`: `#[non_exhaustive]` (ADR-0013) keeps new inline Markdown
// constructs additive for external consumers.
#[non_exhaustive]
pub enum Inline {
    Text {
        value: String,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Code {
        value: String,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Strong {
        children: Vec<Inline>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Emphasis {
        children: Vec<Inline>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    Link {
        href: String,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        title: Option<String>,
        children: Vec<Inline>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    /// CommonMark image. `alt` carries the alt-text inlines as comrak parses
    /// them, so it is a list rather than a string.
    Image {
        url: String,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        title: Option<String>,
        alt: Vec<Inline>,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        range: Option<Range>,
    },
    LineBreak {
        hard: bool,
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
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
        #[cfg_attr(feature = "serde", serde(rename = "aozoraKind"))]
        kind: String,
        /// End-exclusive byte range: slicing the source you passed in
        /// recovers the text the author wrote.
        ///
        /// `None` when that promise cannot be kept. The parser measures
        /// spans against its *normalised* text, and normalisation moves
        /// bytes (BOM stripped, `\r\n` folded, accent digraphs combined,
        /// decorative rules given a blank line), so the offsets would
        /// address a different — possibly mid-codepoint — position.
        #[cfg_attr(feature = "serde", serde(default))]
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
// Left open for literal construction for the reason `Span` is — see its
// definition in `crate::diagnostics`.
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    /// No ordering is imposed on the pair; `sourcepos_to_range` checks it.
    #[must_use]
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// `column` is grapheme-blind, matching comrak, so it suits editor surfaces
/// but not byte slicing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
// Left open for literal construction; see `Range`.
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl Position {
    /// Both coordinates are 1-based, as comrak reports them.
    #[must_use]
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

// Every assertion below reads a serialised value, so the module follows the
// feature that puts one on the wire — as `diagnostics::miette_impl` follows
// `miette`. The gate is what keeps a `--no-default-features` build of this
// crate's own tests compiling.
#[cfg(all(test, feature = "serde"))]
mod tests {
    use core::fmt::Debug;

    use serde::Serialize;
    use serde::de::DeserializeOwned;

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
        let value = json(Inline::Aozora {
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
        let value = json(Block::Aozora {
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

    // -----------------------------------------------------------------
    // the tag alphabet — the whole wire format, not two of its members
    // -----------------------------------------------------------------
    //
    // ADR-0017 calls the IR a stable wire format, and until this section
    // existed nothing anywhere asserted a single Markdown tag: the two tests
    // above cover `aozora` alone. A `rename_all` edit, a variant rename or a
    // dropped `#[serde(rename)]` rewrote what every JS `switch` dispatches on
    // and left the whole suite green — which is exactly the shape of edit
    // `Block::CodeBlock` -> `Block::Code` is.
    //
    // The `match`es below are exhaustive with no wildcard, so a new variant
    // is a compile error until its wire tag is written down. The frozen
    // alphabet beside each one is then what the samples are held to, so the
    // decision cannot be made twice.

    // The tag is the contract; the Rust identifier beside it is not.
    fn block_tag(block: &Block) -> &'static str {
        match block {
            Block::Paragraph { .. } => "paragraph",
            Block::Heading { .. } => "heading",
            Block::Blockquote { .. } => "blockquote",
            Block::List { .. } => "list",
            Block::Code { .. } => "codeBlock",
            Block::ThematicBreak { .. } => "thematicBreak",
            Block::Table { .. } => "table",
            Block::Aozora { .. } => "aozora",
        }
    }

    fn inline_tag(inline: &Inline) -> &'static str {
        match inline {
            Inline::Text { .. } => "text",
            Inline::Code { .. } => "code",
            Inline::Strong { .. } => "strong",
            Inline::Emphasis { .. } => "emphasis",
            Inline::Link { .. } => "link",
            Inline::Image { .. } => "image",
            Inline::LineBreak { .. } => "lineBreak",
            Inline::Aozora { .. } => "aozora",
        }
    }

    // Every tag the `Block` union may carry. Add a variant above and this
    // list is what forces a sample for it into `one_of_every_block`.
    const BLOCK_TAGS: &[&str] = &[
        "paragraph",
        "heading",
        "blockquote",
        "list",
        "codeBlock",
        "thematicBreak",
        "table",
        "aozora",
    ];

    const INLINE_TAGS: &[&str] = &[
        "text",
        "code",
        "strong",
        "emphasis",
        "link",
        "image",
        "lineBreak",
        "aozora",
    ];

    fn text(value: &str) -> Inline {
        Inline::Text {
            value: value.to_owned(),
            range: None,
        }
    }

    fn one_of_every_block() -> Vec<Block> {
        vec![
            Block::Paragraph {
                children: vec![text("p")],
                source_line: None,
                range: None,
            },
            Block::Heading {
                level: 1,
                children: vec![text("h")],
                source_line: None,
                range: None,
            },
            Block::Blockquote {
                children: Vec::new(),
                source_line: None,
                range: None,
            },
            Block::List {
                ordered: false,
                start: None,
                items: vec![ListItem {
                    children: Vec::new(),
                    range: None,
                }],
                source_line: None,
                range: None,
            },
            Block::Code {
                lang: None,
                value: "x".to_owned(),
                source_line: None,
                range: None,
            },
            Block::ThematicBreak {
                source_line: None,
                range: None,
            },
            Block::Table {
                header: TableRow {
                    cells: Vec::new(),
                    range: None,
                },
                rows: Vec::new(),
                align: vec![TableAlign::Default],
                source_line: None,
                range: None,
            },
            Block::Aozora {
                kind: "pageBreak".to_owned(),
                span: None,
                html: String::new(),
                source_line: None,
            },
        ]
    }

    fn one_of_every_inline() -> Vec<Inline> {
        vec![
            text("t"),
            Inline::Code {
                value: "c".to_owned(),
                range: None,
            },
            Inline::Strong {
                children: Vec::new(),
                range: None,
            },
            Inline::Emphasis {
                children: Vec::new(),
                range: None,
            },
            Inline::Link {
                href: "https://example.com".to_owned(),
                title: None,
                children: Vec::new(),
                range: None,
            },
            Inline::Image {
                url: "https://example.com/a.png".to_owned(),
                title: None,
                alt: Vec::new(),
                range: None,
            },
            Inline::LineBreak {
                hard: true,
                range: None,
            },
            Inline::Aozora {
                kind: "ruby".to_owned(),
                span: None,
                html: String::new(),
            },
        ]
    }

    fn assert_tags_are(observed: Vec<&str>, frozen: &[&str], union: &str) {
        let mut observed = observed;
        observed.sort_unstable();
        let mut frozen = frozen.to_vec();
        frozen.sort_unstable();
        assert_eq!(
            observed, frozen,
            "the `{union}` union's wire tags are not the frozen alphabet; a consumer's \
             `switch` dispatches on exactly these strings"
        );
    }

    #[test]
    fn every_block_variant_serialises_under_its_frozen_wire_tag() {
        let blocks = one_of_every_block();
        let mut seen = Vec::new();
        for block in &blocks {
            let tag = block_tag(block);
            assert_eq!(
                json(block)["kind"],
                tag,
                "the `Block` variant tagged {tag} does not serialise under it: {block:?}"
            );
            seen.push(tag);
        }
        assert_tags_are(seen, BLOCK_TAGS, "Block");
    }

    #[test]
    fn every_inline_variant_serialises_under_its_frozen_wire_tag() {
        let inlines = one_of_every_inline();
        let mut seen = Vec::new();
        for inline in &inlines {
            let tag = inline_tag(inline);
            assert_eq!(
                json(inline)["kind"],
                tag,
                "the `Inline` variant tagged {tag} does not serialise under it: {inline:?}"
            );
            seen.push(tag);
        }
        assert_tags_are(seen, INLINE_TAGS, "Inline");
    }

    // The one tag whose Rust identifier deliberately disagrees with it, and
    // the reason the disagreement exists: `code` is already taken by the
    // inline half of the pair, and a JS host that dispatches one union after
    // the other on `kind` would not be able to tell them apart.
    #[test]
    fn the_block_and_inline_code_nodes_keep_distinguishable_tags() {
        let block = json(&Block::Code {
            lang: Some("rust".to_owned()),
            value: "fn main() {}".to_owned(),
            source_line: None,
            range: None,
        });
        let inline = json(&Inline::Code {
            value: "fn".to_owned(),
            range: None,
        });
        assert_eq!(block["kind"], "codeBlock");
        assert_eq!(inline["kind"], "code");
        assert_ne!(
            block["kind"], inline["kind"],
            "the two code nodes must stay distinguishable by `kind` alone"
        );
    }

    // Every alignment a table column can carry, tagged. The `#[default]`
    // variant is spelled `default` on the wire, which reads as a placeholder
    // and is a real value — a consumer that treats it as "absent" gets a
    // left-aligned column right by accident and nothing else.
    #[test]
    fn every_table_alignment_serialises_under_its_frozen_wire_tag() {
        for (align, tag) in [
            (TableAlign::Left, "left"),
            (TableAlign::Center, "center"),
            (TableAlign::Right, "right"),
            (TableAlign::Default, "default"),
        ] {
            assert_eq!(
                json(align),
                tag,
                "the `TableAlign` variant tagged {tag} does not serialise under it"
            );
        }
    }

    // -----------------------------------------------------------------
    // the read-back half — a format only one side can read is a dead letter
    // -----------------------------------------------------------------
    //
    // Everything above asserts what these types *write*. ADR-0017 calls the
    // IR a stable wire format and ADR-0012 says the same of the diagnostic
    // envelope, and a format is a claim about two processes: what one writes,
    // another reads. Nothing in the workspace asserted the second half, and
    // nothing could — no type here derived `Deserialize`, so a test that tried
    // did not compile. Every serde assertion in the suite therefore held for
    // a write-only format, which is what these types shipped as.
    //
    // The samples are the ones the tag alphabet above already forces to be
    // exhaustive: `block_tag` / `inline_tag` are wildcard-free `match`es, so
    // a variant added to either union is a compile error until it is named,
    // and naming it enrols it here too. No second list to keep in step.
    //
    // Every optional field in those samples is `None`, so every key
    // `skip_serializing_if` can drop is dropped: what these assert is that a
    // document with the keys missing still reads. The `Some` half is rendered
    // output rather than a literal, and is asserted over generated sources by
    // `tests/property_ir_value_identity.rs`.
    //
    // The `#[serde(default)]` beside each skip is not what makes this pass —
    // the fields are `Option`, and serde's derive already resolves a missing
    // one to `None`. It is the declaration that will matter for the first
    // omittable field that is not an `Option`, and the source-text rule in
    // `tests/public_type_contract.rs` is what holds it there.

    // Read back equal, and rewrite what it was read from. The second half is
    // what catches a `#[serde(default)]` that supplies something other than
    // the value `skip_serializing_if` omitted: the pair is then a round trip
    // in neither direction, and only re-serialising says so.
    fn assert_reads_back<T: Serialize + DeserializeOwned + PartialEq + Debug>(
        value: &T,
        label: &str,
    ) {
        let written = json(value);
        let read: T = match serde_json::from_value(written.clone()) {
            Ok(read) => read,
            Err(err) => panic!("{label} did not read back from {written}: {err}"),
        };
        assert_eq!(
            &read, value,
            "{label} read back as a different value from {written}"
        );
        assert_eq!(
            json(&read),
            written,
            "{label} read back does not rewrite the JSON it was read from"
        );
    }

    #[test]
    fn every_block_variant_reads_back_from_the_wire_form_it_writes() {
        for block in &one_of_every_block() {
            assert_reads_back(block, block_tag(block));
        }
    }

    #[test]
    fn every_inline_variant_reads_back_from_the_wire_form_it_writes() {
        for inline in &one_of_every_inline() {
            assert_reads_back(inline, inline_tag(inline));
        }
    }

    #[test]
    fn every_table_alignment_reads_back_from_its_frozen_wire_tag() {
        for align in [
            TableAlign::Left,
            TableAlign::Center,
            TableAlign::Right,
            TableAlign::Default,
        ] {
            assert_reads_back(&align, "TableAlign");
        }
    }

    // The shapes the two unions travel inside. `Document` is what a consumer
    // is actually handed; `ListItem` and `TableRow` are reachable only
    // through a variant, so an unpaired attribute on either shows up not as a
    // bad list item but as a whole document that will not read. The geometry
    // types come along because they are the leaves every other shape's
    // optional keys resolve to.
    #[test]
    fn the_containers_and_the_geometry_read_back_too() {
        assert_reads_back(
            &Document {
                blocks: one_of_every_block(),
            },
            "Document",
        );
        assert_reads_back(
            &ListItem {
                children: one_of_every_block(),
                range: Some(Range::new(Position::new(1, 1), Position::new(2, 4))),
            },
            "ListItem",
        );
        assert_reads_back(
            &TableRow {
                cells: vec![one_of_every_inline()],
                range: None,
            },
            "TableRow",
        );
        assert_reads_back(
            &Range::new(Position::new(1, 1), Position::new(9, 3)),
            "Range",
        );
        assert_reads_back(&Position::new(4, 12), "Position");
        assert_reads_back(&Span::new(0, 7), "Span");
    }
}
