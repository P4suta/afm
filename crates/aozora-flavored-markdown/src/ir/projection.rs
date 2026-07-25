//! Construct → IR projection helpers + sourcepos / table-align /
//! enum-tag mappers.
//!
//! Every helper here is **pure**: a function of its arguments alone, no
//! walker / cursor state. The walker (`IrWalker` in `crate::ir`) composes
//! these to produce `IrBlock` / `IrInline` values from the upstream
//! constructs.
//!
//! ## Non-exhaustive enum guards
//!
//! All upstream payload enums are `#[non_exhaustive]`. The trailing
//! wildcard arm fires only when a future upstream release adds a
//! variant before aozora-flavored-markdown bumps its dep, so we keep its return value
//! **distinct** from every named variant: the wildcard returns
//! `"unknown"` (or `None`), and named variants return their own
//! semantic mapping. That way a future-variant hit is observable in
//! the IR rather than silently coinciding with a known variant's
//! output. Clippy's `match_same_arms` would otherwise flag any
//! explicit arm that happens to share the wildcard body — but we
//! don't have to silence the lint because our values are genuinely
//! distinct everywhere.

use aozora::encoding::gaiji::Resolved;
use aozora::syntax::borrowed::{
    Annotation as AozoraAnnotation, AozoraNode, Bouten as AozoraBouten, Content,
    DoubleRuby as AozoraDoubleRuby, Gaiji as AozoraGaiji, Ruby as AozoraRuby, Segment, TateChuYoko,
};
use aozora::syntax::{
    AnnotationKind as AozoraAnnotationKind, BoutenKind, BoutenPosition as AozoraBoutenPosition,
    ContainerKind, SectionKind,
};
use comrak::nodes::{Sourcepos, TableAlignment};

use crate::sentinel_stream::saturating_u32;

use super::types::{
    AnnotationKind, BoutenPosition, BoutenStyle, ContainerSubtype, IrBlock, IrInline, IrTableAlign,
    Position, Range, SectionSubtype,
};

pub(super) fn project_inline(node: AozoraNode<'_>) -> Option<IrInline> {
    match node {
        AozoraNode::Ruby(r) => Some(project_ruby(r)),
        AozoraNode::DoubleRuby(d) => Some(project_double_ruby(d)),
        AozoraNode::Bouten(b) => Some(project_bouten(b)),
        AozoraNode::TateChuYoko(t) => Some(project_tcy(t)),
        AozoraNode::Gaiji(g) => Some(project_gaiji(g)),
        AozoraNode::Annotation(a) => Some(project_annotation(a)),
        // HeadingHint is consumed at the paragraph level, never inline.
        // Other variants (`Indent` leaf, `AlignEnd` leaf, `Warichu`,
        // `Sashie`, `Kaeriten`, `AozoraHeading`, `Keigakomi`) exist as
        // block markers in upstream and don't have a v0.2 inline
        // projection. They appear in the HTML but drop from the IR.
        _ => None,
    }
}

pub(super) fn project_block_leaf(node: AozoraNode<'_>, source_line: u32) -> Option<IrBlock> {
    match node {
        AozoraNode::PageBreak => Some(IrBlock::PageBreak {
            source_line: Some(source_line),
            range: None,
        }),
        AozoraNode::SectionBreak(kind) => Some(IrBlock::SectionBreak {
            subtype: section_subtype(kind),
            source_line: Some(source_line),
            range: None,
        }),
        // Other block-leaf variants (`Sashie`, `AozoraHeading`, …)
        // have no v0.2 IR projection. The HTML still carries them.
        _ => None,
    }
}

fn project_ruby(r: &AozoraRuby<'_>) -> IrInline {
    IrInline::Ruby {
        base: project_content_inlines(r.base.get()),
        reading: content_to_string(r.reading.get()),
        explicit: r.delim_explicit,
        range: None,
    }
}

fn project_double_ruby(d: &AozoraDoubleRuby<'_>) -> IrInline {
    IrInline::DoubleRuby {
        base: project_content_inlines(d.content.get()),
        range: None,
    }
}

fn project_bouten(b: &AozoraBouten<'_>) -> IrInline {
    IrInline::Bouten {
        children: project_content_inlines(b.target.get()),
        style: bouten_style(b.kind),
        position: bouten_position(b.position),
        range: None,
    }
}

fn project_tcy(t: &TateChuYoko<'_>) -> IrInline {
    IrInline::Tcy {
        text: content_to_string(t.text.get()),
        range: None,
    }
}

fn project_gaiji(g: &AozoraGaiji<'_>) -> IrInline {
    IrInline::Gaiji {
        codepoint: g.ucs.map(resolved_to_string),
        description: (!g.description.is_empty()).then(|| g.description.to_owned()),
        fallback_text: None,
        range: None,
    }
}

fn project_annotation(a: &AozoraAnnotation<'_>) -> IrInline {
    IrInline::Annotation {
        payload: a.raw.as_str().to_owned(),
        resolved: annotation_resolved(a.kind),
        range: None,
    }
}

pub(super) fn project_content_inlines(content: Content<'_>) -> Vec<IrInline> {
    match content {
        Content::Plain(s) if !s.is_empty() => vec![IrInline::Text {
            value: s.to_owned(),
            range: None,
        }],
        Content::Segments(segs) => {
            let mut out = Vec::with_capacity(segs.len());
            for seg in segs {
                match *seg {
                    Segment::Text(t) if !t.is_empty() => out.push(IrInline::Text {
                        value: t.to_owned(),
                        range: None,
                    }),
                    Segment::Gaiji(g) => out.push(project_gaiji(g)),
                    Segment::Annotation(a) => out.push(project_annotation(a)),
                    // Empty `Segment::Text` plus any future
                    // non-exhaustive variant: drop quietly.
                    _ => {}
                }
            }
            out
        }
        // `Content::Plain("")` plus any future non-exhaustive variant:
        // produce no IR.
        _ => Vec::new(),
    }
}

pub(super) fn content_to_string(content: Content<'_>) -> String {
    match content {
        Content::Plain(s) => s.to_owned(),
        Content::Segments(segs) => {
            let mut out = String::new();
            for seg in segs {
                if let Segment::Text(t) = seg {
                    out.push_str(t);
                }
            }
            out
        }
        _ => String::new(),
    }
}

pub(super) fn resolved_to_string(r: Resolved) -> String {
    match r {
        Resolved::Char(c) => c.to_string(),
        Resolved::Multi(s) => s.to_owned(),
    }
}

pub(super) const fn bouten_style(k: BoutenKind) -> BoutenStyle {
    match k {
        BoutenKind::Goma => BoutenStyle::Goma,
        BoutenKind::WhiteSesame => BoutenStyle::WhiteSesame,
        BoutenKind::Circle => BoutenStyle::Circle,
        BoutenKind::WhiteCircle => BoutenStyle::WhiteCircle,
        BoutenKind::DoubleCircle => BoutenStyle::DoubleCircle,
        BoutenKind::Janome => BoutenStyle::Janome,
        BoutenKind::Cross => BoutenStyle::Cross,
        BoutenKind::WhiteTriangle => BoutenStyle::WhiteTriangle,
        BoutenKind::WavyLine => BoutenStyle::WavyLine,
        BoutenKind::UnderLine => BoutenStyle::UnderLine,
        BoutenKind::DoubleUnderLine => BoutenStyle::DoubleUnderLine,
        _ => BoutenStyle::Unknown,
    }
}

pub(super) const fn bouten_position(p: AozoraBoutenPosition) -> BoutenPosition {
    match p {
        AozoraBoutenPosition::Right => BoutenPosition::Right,
        AozoraBoutenPosition::Left => BoutenPosition::Left,
        _ => BoutenPosition::Unknown,
    }
}

pub(super) const fn section_subtype(kind: SectionKind) -> SectionSubtype {
    match kind {
        SectionKind::Choho => SectionSubtype::Choho,
        SectionKind::Dan => SectionSubtype::Dan,
        SectionKind::Spread => SectionSubtype::Spread,
        _ => SectionSubtype::Unknown,
    }
}

pub(super) const fn container_subtype(kind: ContainerKind) -> ContainerSubtype {
    match kind {
        ContainerKind::Indent { .. } => ContainerSubtype::Indent,
        ContainerKind::Warichu => ContainerSubtype::Warichu,
        ContainerKind::Keigakomi => ContainerSubtype::Keigakomi,
        ContainerKind::AlignEnd { .. } => ContainerSubtype::AlignEnd,
        _ => ContainerSubtype::Unknown,
    }
}

pub(super) const fn container_indent_level(kind: ContainerKind) -> Option<u32> {
    // Only the size-carrying variants emit a depth. `Warichu` and
    // `Keigakomi` (and any future non-exhaustive variant) fall
    // through the wildcard with `None`.
    match kind {
        ContainerKind::Indent { amount } => Some(amount as u32),
        ContainerKind::AlignEnd { offset } => Some(offset as u32),
        _ => None,
    }
}

pub(super) const fn annotation_resolved(k: AozoraAnnotationKind) -> Option<AnnotationKind> {
    // Named annotation kinds project to their aozora-flavored-markdown `AnnotationKind`.
    // `Unknown` deliberately differs from a future-variant hit:
    // `Some(Unknown)` says the upstream classifier saw the annotation but
    // couldn't classify it, whereas `None` says aozora-flavored-markdown doesn't
    // know about this variant of upstream `AnnotationKind` yet.
    match k {
        AozoraAnnotationKind::Unknown => Some(AnnotationKind::Unknown),
        AozoraAnnotationKind::AsIs => Some(AnnotationKind::AsIs),
        AozoraAnnotationKind::TextualNote => Some(AnnotationKind::TextualNote),
        AozoraAnnotationKind::InvalidRubySpan => Some(AnnotationKind::InvalidRubySpan),
        AozoraAnnotationKind::WarichuOpen => Some(AnnotationKind::WarichuOpen),
        AozoraAnnotationKind::WarichuClose => Some(AnnotationKind::WarichuClose),
        _ => None,
    }
}

pub(super) fn table_align(a: TableAlignment) -> IrTableAlign {
    match a {
        TableAlignment::Left => IrTableAlign::Left,
        TableAlignment::Center => IrTableAlign::Center,
        TableAlignment::Right => IrTableAlign::Right,
        TableAlignment::None => IrTableAlign::Default,
    }
}

pub(super) fn sourcepos_to_range(s: &Sourcepos) -> Option<Range> {
    // comrak source positions are 1-based line / column. Map the
    // pair through `Position` directly — no pseudo-byte arithmetic.
    let start = Position {
        line: saturating_u32(s.start.line),
        column: saturating_u32(s.start.column),
    };
    let end = Position {
        line: saturating_u32(s.end.line),
        column: saturating_u32(s.end.column),
    };
    // `Position` derives `Ord` lexicographically (line first, then
    // column), so the comparison works for malformed inputs where
    // `end` precedes `start`.
    (end >= start).then_some(Range { start, end })
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure projection helpers.
    //!
    //! These cover the match arms inside the `const fn` projectors
    //! that are otherwise reachable only through specific Aozora
    //! input patterns — enumerating every input grammar in
    //! integration tests would be both noisy and fragile against
    //! upstream parser evolution. Calling the projectors directly
    //! with synthetic enum values pins each match arm to a value, so
    //! an upstream rename or variant removal fails the build at the
    //! call site rather than silently in the IR.

    use super::*;
    use aozora::syntax::AlignEnd;
    use comrak::nodes::{LineColumn, Sourcepos};

    #[test]
    fn bouten_style_covers_every_upstream_variant() {
        let cases = [
            (BoutenKind::Goma, BoutenStyle::Goma),
            (BoutenKind::WhiteSesame, BoutenStyle::WhiteSesame),
            (BoutenKind::Circle, BoutenStyle::Circle),
            (BoutenKind::WhiteCircle, BoutenStyle::WhiteCircle),
            (BoutenKind::DoubleCircle, BoutenStyle::DoubleCircle),
            (BoutenKind::Janome, BoutenStyle::Janome),
            (BoutenKind::Cross, BoutenStyle::Cross),
            (BoutenKind::WhiteTriangle, BoutenStyle::WhiteTriangle),
            (BoutenKind::WavyLine, BoutenStyle::WavyLine),
            (BoutenKind::UnderLine, BoutenStyle::UnderLine),
            (BoutenKind::DoubleUnderLine, BoutenStyle::DoubleUnderLine),
        ];
        for (kind, expected) in cases {
            assert_eq!(bouten_style(kind), expected);
        }
    }

    #[test]
    fn bouten_position_covers_left_and_right() {
        assert_eq!(
            bouten_position(AozoraBoutenPosition::Right),
            BoutenPosition::Right
        );
        assert_eq!(
            bouten_position(AozoraBoutenPosition::Left),
            BoutenPosition::Left
        );
    }

    #[test]
    fn section_subtype_covers_every_upstream_variant() {
        assert_eq!(section_subtype(SectionKind::Choho), SectionSubtype::Choho);
        assert_eq!(section_subtype(SectionKind::Dan), SectionSubtype::Dan);
        assert_eq!(section_subtype(SectionKind::Spread), SectionSubtype::Spread);
    }

    #[test]
    fn container_subtype_and_indent_level_round_trip_each_variant() {
        let indent = ContainerKind::Indent { amount: 3 };
        assert_eq!(container_subtype(indent), ContainerSubtype::Indent);
        assert_eq!(container_indent_level(indent), Some(3));

        let align = ContainerKind::AlignEnd {
            offset: AlignEnd { offset: 1 }.offset,
        };
        assert_eq!(container_subtype(align), ContainerSubtype::AlignEnd);
        assert_eq!(container_indent_level(align), Some(1));

        assert_eq!(
            container_subtype(ContainerKind::Warichu),
            ContainerSubtype::Warichu
        );
        assert!(container_indent_level(ContainerKind::Warichu).is_none());
        assert_eq!(
            container_subtype(ContainerKind::Keigakomi),
            ContainerSubtype::Keigakomi
        );
        assert!(container_indent_level(ContainerKind::Keigakomi).is_none());
    }

    #[test]
    fn annotation_resolved_covers_every_named_variant() {
        // Upstream `Unknown` is the classifier's "tried, gave up" outcome;
        // we surface it as `Some(AnnotationKind::Unknown)` so consumers
        // distinguish it from a future-variant hit (`None`).
        assert_eq!(
            annotation_resolved(AozoraAnnotationKind::Unknown),
            Some(AnnotationKind::Unknown)
        );
        assert_eq!(
            annotation_resolved(AozoraAnnotationKind::AsIs),
            Some(AnnotationKind::AsIs)
        );
        assert_eq!(
            annotation_resolved(AozoraAnnotationKind::TextualNote),
            Some(AnnotationKind::TextualNote)
        );
        assert_eq!(
            annotation_resolved(AozoraAnnotationKind::InvalidRubySpan),
            Some(AnnotationKind::InvalidRubySpan)
        );
        assert_eq!(
            annotation_resolved(AozoraAnnotationKind::WarichuOpen),
            Some(AnnotationKind::WarichuOpen)
        );
        assert_eq!(
            annotation_resolved(AozoraAnnotationKind::WarichuClose),
            Some(AnnotationKind::WarichuClose)
        );
    }

    #[test]
    fn resolved_to_string_handles_char_and_multi() {
        assert_eq!(resolved_to_string(Resolved::Char('a')), "a");
        assert_eq!(resolved_to_string(Resolved::Multi("か゚")), "か゚");
    }

    #[test]
    fn project_content_inlines_covers_plain_segments_and_empty() {
        assert!(project_content_inlines(Content::Plain("")).is_empty());
        let plain = project_content_inlines(Content::Plain("hi"));
        assert!(matches!(
            plain.as_slice(),
            [IrInline::Text { value, .. }] if value == "hi"
        ));

        let segs: &[Segment<'_>] = &[Segment::Text("a"), Segment::Text("")];
        let segs_out = project_content_inlines(Content::Segments(segs));
        // Empty Text drops; non-empty survives.
        assert_eq!(segs_out.len(), 1);
    }

    #[test]
    fn content_to_string_concatenates_segment_text_only() {
        assert_eq!(content_to_string(Content::Plain("xyz")), "xyz");
        let segs: &[Segment<'_>] = &[Segment::Text("a"), Segment::Text("b")];
        assert_eq!(content_to_string(Content::Segments(segs)), "ab");
    }

    #[test]
    fn table_align_maps_every_alignment() {
        assert!(matches!(
            table_align(TableAlignment::Left),
            IrTableAlign::Left
        ));
        assert!(matches!(
            table_align(TableAlignment::Center),
            IrTableAlign::Center
        ));
        assert!(matches!(
            table_align(TableAlignment::Right),
            IrTableAlign::Right
        ));
        assert!(matches!(
            table_align(TableAlignment::None),
            IrTableAlign::Default
        ));
    }

    #[test]
    fn sourcepos_to_range_returns_some_for_well_ordered_positions() {
        let pos = Sourcepos {
            start: LineColumn { line: 1, column: 1 },
            end: LineColumn { line: 1, column: 5 },
        };
        let range = sourcepos_to_range(&pos).expect("forward range");
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.column, 1);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.column, 5);
        assert!(range.start <= range.end);
    }

    #[test]
    fn sourcepos_to_range_returns_none_for_inverted_positions() {
        // Constructed (impossible) inverted sourcepos: start later
        // than end. The helper guards against negative ranges by
        // returning `None`, which keeps the IR robust under malformed
        // upstream output.
        let pos = Sourcepos {
            start: LineColumn { line: 5, column: 5 },
            end: LineColumn { line: 1, column: 1 },
        };
        assert!(sourcepos_to_range(&pos).is_none());
    }

    #[test]
    fn sourcepos_to_range_preserves_multiline_extent() {
        // Comrak emits ranges that span multiple source lines for
        // block constructs (a fenced code block, a multi-line list
        // item, …). The IR must preserve the line / column pair so
        // editor surfaces can map back to the right slice without
        // doing pseudo-byte arithmetic.
        let pos = Sourcepos {
            start: LineColumn { line: 3, column: 1 },
            end: LineColumn {
                line: 7,
                column: 12,
            },
        };
        let range = sourcepos_to_range(&pos).expect("forward range");
        assert_eq!(range.start.line, 3);
        assert_eq!(range.end.line, 7);
        assert_eq!(range.end.column, 12);
    }
}
