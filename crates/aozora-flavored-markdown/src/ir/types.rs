//! Public IR type definitions.
//!
//! Every type here is part of the `aozora_flavored_markdown::ir` public
//! surface. Under the `tsify` feature (enabled by aozora-flavored-markdown-wasm)
//! each derives `tsify::Tsify`, so wasm-pack emits the matching TypeScript
//! `IRDocument` — consumed by the playground and aozora-flavored-markdown-obsidian
//! — straight from these definitions, with no hand-written `.d.ts` to keep in
//! sync (ADR-0017). The `serde` attributes are the single source of the wire
//! shape.

use serde::Serialize;

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
// New Aozora notations land as new variants; `#[non_exhaustive]` (ADR-0013)
// lets that happen in a minor release without breaking external `match`es.
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
    // ----- Aozora-specific block variants -----
    /// Paired-container wrapper. `indent_level` is set to `Some(n)` for
    /// [`ContainerSubtype::Indent`] (字下げ amount) and
    /// [`ContainerSubtype::AlignEnd`] (地上げ offset); `None` otherwise.
    Container {
        subtype: ContainerSubtype,
        children: Vec<IrBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        indent_level: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    PageBreak {
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// `［＃改丁／改段／改見開き］`. See [`SectionSubtype`]. `［＃改ページ］` is
    /// its own block — see [`IrBlock::PageBreak`].
    SectionBreak {
        subtype: SectionSubtype,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
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
// See `IrBlock`: `#[non_exhaustive]` (ADR-0013) keeps new inline notations
// additive for external consumers.
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
    // ----- Aozora-specific variants (mirror TS IRInline) -----
    /// Furigana. `reading` is the flattened reading text;
    /// `explicit` is `true` when the source used the explicit
    /// `｜base《reading》` opener.
    Ruby {
        base: Vec<IrInline>,
        reading: String,
        explicit: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// `《《…》》` double-bracket bouten. Upstream's `DoubleRuby`
    /// carries a single `content` payload — that payload becomes
    /// `base` here. The shape is intentionally minimal: any future
    /// upstream addition (e.g., explicit ring-style metadata) lands
    /// as a new optional field rather than re-using empty strings as
    /// placeholders.
    DoubleRuby {
        base: Vec<IrInline>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// Emphasis dots / sidelines. See [`BoutenStyle`] and [`BoutenPosition`].
    Bouten {
        children: Vec<IrInline>,
        style: BoutenStyle,
        position: BoutenPosition,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Gaiji {
        #[serde(skip_serializing_if = "Option::is_none")]
        codepoint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fallback_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    Tcy {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// Generic annotation. `payload` is the raw bytes between `［＃` and
    /// `］`. `resolved` carries the [`AnnotationKind`] classification when
    /// the upstream lexer recognised the annotation; `None` for future
    /// non-exhaustive variants aozora-flavored-markdown hasn't seen yet.
    Annotation {
        payload: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolved: Option<AnnotationKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
}

// ---------------------------------------------------------------------
// Aozora classification enums
//
// These mirror the sibling parser's own classifications but are owned
// here, so the public IR surface is decoupled from upstream's
// semver. Each `#[serde(rename_all = "camelCase")]` variant serializes to
// the exact wire string the previous stringly-typed fields produced, so
// the JSON is byte-identical. `#[non_exhaustive]` keeps them additive
// (ADR-0013); `Unknown` is the wire value emitted when the upstream lexer
// produces a variant aozora-flavored-markdown does not classify yet.

/// Paired-container subtype, mirroring the upstream container kind
/// (minus the numeric payload, which rides in
/// [`IrBlock::Container`]'s `indent_level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ContainerSubtype {
    /// 字下げ — left indent.
    Indent,
    /// 地上げ — right-aligned (trailing) block.
    AlignEnd,
    /// 罫囲み — ruled box.
    Keigakomi,
    /// 割り注 — interlinear note.
    Warichu,
    /// An upstream variant aozora-flavored-markdown does not classify yet.
    Unknown,
}

/// Section-break subtype, mirroring the upstream section kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SectionSubtype {
    /// 改丁.
    Choho,
    /// 改段.
    Dan,
    /// 改見開き.
    Spread,
    /// An upstream variant aozora-flavored-markdown does not classify yet.
    Unknown,
}

/// Emphasis-dot / sideline style, mirroring the upstream bouten kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BoutenStyle {
    /// ゴマ点.
    Goma,
    /// 白ゴマ点.
    WhiteSesame,
    /// 丸.
    Circle,
    /// 白丸.
    WhiteCircle,
    /// 二重丸.
    DoubleCircle,
    /// 蛇の目.
    Janome,
    /// ばつ.
    Cross,
    /// 白三角.
    WhiteTriangle,
    /// 波線（脇線）.
    WavyLine,
    /// 傍線.
    UnderLine,
    /// 二重傍線.
    DoubleUnderLine,
    /// An upstream variant aozora-flavored-markdown does not classify yet.
    Unknown,
}

/// Which side of the text a bouten sits on, mirroring the upstream
/// bouten position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BoutenPosition {
    /// Right side (vertical) / above (horizontal) — the default.
    Right,
    /// Left side (vertical) / below (horizontal).
    Left,
    /// An upstream variant aozora-flavored-markdown does not classify yet.
    Unknown,
}

/// Resolved annotation classification, mirroring the upstream
/// annotation kind.
///
/// Carried as `Option` on [`IrInline::Annotation`]'s `resolved`: `None`
/// means the upstream lexer produced a variant aozora-flavored-markdown
/// hasn't seen yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum AnnotationKind {
    /// The lexer saw the annotation but could not classify it.
    Unknown,
    /// `［＃「…」はママ］` — leave as-is.
    AsIs,
    /// A textual editorial note.
    TextualNote,
    /// A ruby span the lexer flagged as invalid.
    InvalidRubySpan,
    /// 割り注 open marker.
    WarichuOpen,
    /// 割り注 close marker.
    WarichuClose,
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

    /// Lock the camelCase wire strings the stringly-typed fields used to
    /// produce, so the enum migration stays byte-identical on the JSON side.
    #[test]
    fn classification_enums_serialize_to_stable_wire_strings() {
        use serde_json::to_value;

        for (style, wire) in [
            (BoutenStyle::Goma, "goma"),
            (BoutenStyle::WhiteSesame, "whiteSesame"),
            (BoutenStyle::Circle, "circle"),
            (BoutenStyle::WhiteCircle, "whiteCircle"),
            (BoutenStyle::DoubleCircle, "doubleCircle"),
            (BoutenStyle::Janome, "janome"),
            (BoutenStyle::Cross, "cross"),
            (BoutenStyle::WhiteTriangle, "whiteTriangle"),
            (BoutenStyle::WavyLine, "wavyLine"),
            (BoutenStyle::UnderLine, "underLine"),
            (BoutenStyle::DoubleUnderLine, "doubleUnderLine"),
            (BoutenStyle::Unknown, "unknown"),
        ] {
            assert_eq!(to_value(style).unwrap(), wire);
        }

        for (position, wire) in [
            (BoutenPosition::Right, "right"),
            (BoutenPosition::Left, "left"),
            (BoutenPosition::Unknown, "unknown"),
        ] {
            assert_eq!(to_value(position).unwrap(), wire);
        }

        for (subtype, wire) in [
            (ContainerSubtype::Indent, "indent"),
            (ContainerSubtype::AlignEnd, "alignEnd"),
            (ContainerSubtype::Keigakomi, "keigakomi"),
            (ContainerSubtype::Warichu, "warichu"),
            (ContainerSubtype::Unknown, "unknown"),
        ] {
            assert_eq!(to_value(subtype).unwrap(), wire);
        }

        for (subtype, wire) in [
            (SectionSubtype::Choho, "choho"),
            (SectionSubtype::Dan, "dan"),
            (SectionSubtype::Spread, "spread"),
            (SectionSubtype::Unknown, "unknown"),
        ] {
            assert_eq!(to_value(subtype).unwrap(), wire);
        }

        for (kind, wire) in [
            (AnnotationKind::Unknown, "unknown"),
            (AnnotationKind::AsIs, "asIs"),
            (AnnotationKind::TextualNote, "textualNote"),
            (AnnotationKind::InvalidRubySpan, "invalidRubySpan"),
            (AnnotationKind::WarichuOpen, "warichuOpen"),
            (AnnotationKind::WarichuClose, "warichuClose"),
        ] {
            assert_eq!(to_value(kind).unwrap(), wire);
        }
    }
}
