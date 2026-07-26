//! The public-type contract, checked from the only place it is real: another
//! crate.
//!
//! An integration test compiles as its own crate, so `#[non_exhaustive]` binds
//! here exactly as it binds a consumer from crates.io. Two halves follow from
//! that:
//!
//! * The **open** half is checked by compiling. Every literal construction,
//!   functional record update and exhaustive destructuring of `Span`,
//!   `Position` and `Range` below stops compiling the day one of them is
//!   sealed — which is the whole content of the decision not to seal them.
//! * The **sealed** half cannot be checked that way: sealing turns *downstream*
//!   code into a compile error, and a compile error is not something a
//!   `#[test]` can observe. It is read off the source text instead, as a rule
//!   over every public type rather than a list of the ones somebody
//!   remembered — ADR-0013 was a rule, and it was applied to two of the eight
//!   types it covered.
//!
//! The derives are exercised on values the API actually hands back, not merely
//! asserted as bounds: a `Hash` that disagreed with `Eq` would satisfy every
//! bound and still corrupt the memo table the derive exists for.

use core::fmt::Debug;
use core::hash::Hash;
use core::iter::once;
use core::ops::Range as ByteRange;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};

use aozora_flavored_markdown::ir::{IrBlock, IrDocument, IrInline, IrTableAlign, Position, Range};
use aozora_flavored_markdown::{
    Diagnostic, DiagnosticSource, Options, Rendered, RenderedBlock, RenderedIr, Severity, Span,
    render, render_blocks_to_ir, render_to_ir,
};
use aozora_flavored_markdown_test_support::config::default_config;
use proptest::prelude::*;

/// One document reaching every container in the IR: heading, paragraph,
/// aozora inline, list, table, blockquote, fence.
const SAMPLE: &str = "# ｜見出し《みだし》\n\n本文と｜青梅《おうめ》\n\n- item\n- ｜漢字《かんじ》\n\n> quoted\n\n| a | b | c |\n|---|:--:|--:|\n| 1 | 2 | 3 |\n\n```\ncode\n```\n";

// ---------------------------------------------------------------------------
// the derives, exercised on real values
// ---------------------------------------------------------------------------

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// The bounds are the derive table; the body is the agreement a memo keyed on
/// the value needs from them.
fn behaves_as_a_hashable_value<T: Debug + Clone + PartialEq + Eq + Hash>(value: &T) {
    let clone = value.clone();
    assert_eq!(&clone, value, "a clone must equal its source: {value:?}");
    assert_eq!(
        hash_of(&clone),
        hash_of(value),
        "equal values must hash equal: {value:?}"
    );
}

/// The output types are comparable and cloneable but not hashable — they
/// carry the rendered HTML, which is what hashing them would be for.
fn behaves_as_a_value<T: Debug + Clone + PartialEq + Eq>(value: &T) {
    let clone = value.clone();
    assert_eq!(&clone, value, "a clone must equal its source: {value:?}");
}

/// Guards the walk below against passing because it never reached a type.
#[derive(Debug, Default, Clone, Copy)]
struct Seen {
    blocks: usize,
    inlines: usize,
    items: usize,
    rows: usize,
    aligns: usize,
}

fn visit_blocks(blocks: &[IrBlock], seen: &mut Seen) {
    for block in blocks {
        behaves_as_a_hashable_value(block);
        seen.blocks += 1;
        visit_children(block, seen);
    }
}

fn visit_children(block: &IrBlock, seen: &mut Seen) {
    match block {
        IrBlock::Paragraph { children, .. } | IrBlock::Heading { children, .. } => {
            visit_inlines(children, seen);
        }
        IrBlock::Blockquote { children, .. } => visit_blocks(children, seen),
        IrBlock::List { items, .. } => {
            for item in items {
                behaves_as_a_hashable_value(item);
                seen.items += 1;
                visit_blocks(&item.children, seen);
            }
        }
        IrBlock::Table {
            header,
            rows,
            align,
            ..
        } => {
            for row in once(header).chain(rows) {
                behaves_as_a_hashable_value(row);
                seen.rows += 1;
                for cell in &row.cells {
                    visit_inlines(cell, seen);
                }
            }
            for column in align {
                behaves_as_a_hashable_value(column);
                seen.aligns += 1;
            }
        }
        _ => {}
    }
}

fn visit_inlines(inlines: &[IrInline], seen: &mut Seen) {
    for inline in inlines {
        behaves_as_a_hashable_value(inline);
        seen.inlines += 1;
        match inline {
            IrInline::Strong { children, .. }
            | IrInline::Emphasis { children, .. }
            | IrInline::Link { children, .. } => visit_inlines(children, seen),
            IrInline::Image { alt, .. } => visit_inlines(alt, seen),
            _ => {}
        }
    }
}

#[test]
fn every_ir_value_the_api_hands_back_clones_compares_and_hashes() {
    let rendered = render_to_ir(SAMPLE, &Options::default());
    behaves_as_a_value(&rendered);
    behaves_as_a_hashable_value(&rendered.ir);

    let mut seen = Seen::default();
    visit_blocks(&rendered.ir.blocks, &mut seen);
    assert!(
        seen.blocks > 0 && seen.inlines > 0 && seen.items > 0 && seen.rows > 0 && seen.aligns > 0,
        "the sample must reach every nested IR type, got {seen:?}"
    );
}

#[test]
fn every_render_output_type_clones_and_compares() {
    behaves_as_a_value(&render(SAMPLE, &Options::default()));
    let (blocks, diagnostics) = render_blocks_to_ir(SAMPLE, &Options::default());
    assert!(!blocks.is_empty(), "the sample must produce blocks");
    for block in &blocks {
        behaves_as_a_value(block);
    }
    for diagnostic in &diagnostics {
        behaves_as_a_hashable_value(diagnostic);
    }
}

#[test]
fn a_diagnostic_and_its_two_enums_compare_and_hash() {
    // Reached through the API rather than constructed: `Diagnostic` is sealed,
    // so a consumer only ever meets one a render handed back.
    let diagnostics: Vec<Diagnostic> = MALFORMED
        .iter()
        .flat_map(|src| render(src, &Options::default()).diagnostics)
        .collect();
    assert!(
        !diagnostics.is_empty(),
        "no malformed sample produced a diagnostic; the sample pool is stale"
    );
    for diagnostic in &diagnostics {
        behaves_as_a_hashable_value(diagnostic);
        behaves_as_a_hashable_value(&diagnostic.severity);
        behaves_as_a_hashable_value(&diagnostic.source);
        behaves_as_a_hashable_value(&diagnostic.span);
    }
    behaves_as_a_hashable_value(&Severity::Error);
    behaves_as_a_hashable_value(&DiagnosticSource::Internal);
}

/// Shapes the lexer reports on. Any one of them producing a diagnostic is
/// enough; the assertion is on the pool, not on a particular member.
const MALFORMED: &[&str] = &[
    "｜青梅《",
    "［＃",
    "［＃ここから字下げ］",
    "［＃ここで字下げ終わり］",
    "※［＃",
    "《》",
];

// ---------------------------------------------------------------------------
// the open half — checked by compiling
// ---------------------------------------------------------------------------

#[test]
fn the_geometric_types_stay_constructible_from_a_consumer_crate() {
    // Every line here is a compile error the day one of these three is
    // sealed. `#[non_exhaustive]` forbids all three shapes from outside the
    // defining crate: the literal, the functional record update, and the
    // exhaustive destructuring.
    let span = Span { start: 3, end: 21 };
    let widened = Span { end: 34, ..span };
    let Span { start, end } = widened;
    assert_eq!(
        (start, end),
        (3, 34),
        "a functional record update must reach every field"
    );
    assert_eq!(span, Span::new(3, 21), "the literal and `new` must agree");

    let position = Position { line: 2, column: 5 };
    let Position { line, column } = position;
    assert_eq!(
        (line, column),
        (2, 5),
        "destructuring must reach every field"
    );
    assert_eq!(
        position,
        Position::new(2, 5),
        "the literal and `new` must agree"
    );

    let range = Range {
        start: position,
        end: Position::new(2, 9),
    };
    let moved = Range {
        end: Position::new(3, 1),
        ..range
    };
    let Range { start, end } = moved;
    assert_eq!(
        (start, end),
        (Position::new(2, 5), Position::new(3, 1)),
        "a functional record update must reach every field"
    );
    assert_eq!(
        range,
        Range::new(position, Position::new(2, 9)),
        "the literal and `new` must agree"
    );
}

#[test]
fn ordering_on_the_geometric_types_is_lexicographic_in_field_order() {
    // The derive reads field order, so this is what pins the field order to
    // the meaning: a span sorts by where it starts, a position by line.
    assert!(
        Span::new(3, 9) < Span::new(4, 0),
        "a span must sort on `start` first"
    );
    assert!(
        Span::new(3, 9) < Span::new(3, 10),
        "equal starts must sort on `end`"
    );
    assert!(
        Position::new(2, 40) < Position::new(3, 1),
        "a position must sort on `line` first"
    );
    assert!(
        Range::new(Position::new(1, 1), Position::new(9, 9))
            < Range::new(Position::new(1, 2), Position::new(1, 3)),
        "a range must sort on `start` first"
    );
}

#[test]
fn the_default_of_every_defaulted_type_is_its_empty_value() {
    assert_eq!(
        Span::default(),
        Span::new(0, 0),
        "the default span is the empty one a document-scoped diagnostic carries"
    );
    assert!(Span::default().is_empty(), "the default span is empty");
    assert_eq!(
        Position::default(),
        Position::new(0, 0),
        "the default position is the origin"
    );
    assert_eq!(
        Range::default(),
        Range::new(Position::default(), Position::default()),
        "the default range is empty at the origin"
    );
    assert!(
        IrDocument::default().blocks.is_empty(),
        "the default document has no blocks"
    );

    let rendered = Rendered::default();
    assert!(
        rendered.html.is_empty() && rendered.diagnostics.is_empty(),
        "the default render is the empty one: {rendered:?}"
    );
    let rendered_ir = RenderedIr::default();
    assert!(
        rendered_ir.ir.blocks.is_empty()
            && rendered_ir.html.is_empty()
            && rendered_ir.diagnostics.is_empty(),
        "the default IR render is the empty one: {rendered_ir:?}"
    );
    let block = RenderedBlock::default();
    assert!(
        block.ir.is_empty() && block.html.is_empty() && block.source_line == 0,
        "the default block is the empty one: {block:?}"
    );
}

#[test]
fn the_default_alignment_is_the_one_a_marker_less_column_projects() {
    // `#[default]` on the wrong variant would compile, serialise and pass
    // every other test — this is what says which variant it belongs on.
    let src = "| a | b | c |\n|---|:--:|--:|\n| 1 | 2 | 3 |\n";
    let rendered = render_to_ir(src, &Options::default());
    let IrBlock::Table { align, .. } = &rendered.ir.blocks[0] else {
        panic!("expected a table, got {:?}", rendered.ir.blocks[0]);
    };
    assert_eq!(
        align[0],
        IrTableAlign::default(),
        "a column with no alignment marker must project the default variant"
    );
    assert_ne!(
        align[1],
        IrTableAlign::default(),
        "a centred column must not project the default variant"
    );
}

proptest! {
    #![proptest_config(default_config())]

    /// `len` / `is_empty` / `From<Span>` agree with each other and with the
    /// `Range<usize>` a caller would otherwise hand-write — over the whole
    /// `u32` square, reversed and overflowing pairs included.
    #[test]
    fn span_measurements_agree_with_the_byte_range_they_convert_to(start: u32, end: u32) {
        let span = Span::new(start, end);
        prop_assert_eq!(span.start, start, "`new` must keep `start`");
        prop_assert_eq!(span.end, end, "`new` must keep `end`");
        // Bound before comparing: clippy reads `span.len() == 0` as a spelling
        // of `is_empty`, which is the equivalence under test rather than
        // something to assume.
        let width = span.len();
        prop_assert_eq!(
            width,
            end.saturating_sub(start),
            "`len` must saturate rather than wrap on a reversed span"
        );
        prop_assert_eq!(
            span.is_empty(),
            width == 0,
            "`is_empty` must mean exactly a zero `len`"
        );

        let bytes = ByteRange::<usize>::from(span);
        prop_assert_eq!(bytes.start, start as usize, "the range must keep `start`");
        prop_assert_eq!(bytes.end, end as usize, "the range must keep `end`");
        prop_assert_eq!(
            bytes.len(),
            width as usize,
            "the range and the span must measure the same width"
        );
    }
}

// ---------------------------------------------------------------------------
// the sealed half — read off the source
// ---------------------------------------------------------------------------

/// Geometrically closed, so deliberately left open to literal construction.
/// The compile-time half of this lives in
/// `the_geometric_types_stay_constructible_from_a_consumer_crate`.
const OPEN_BY_DESIGN: &[&str] = &["Span", "Position", "Range"];

/// The six the rule had missed. Named individually so a regression reports
/// the type rather than a count.
const SEALED_BY_RULE: &[&str] = &[
    "Severity",
    "DiagnosticSource",
    "IrTableAlign",
    "IrDocument",
    "IrTableRow",
    "IrListItem",
];

#[derive(Debug)]
struct Decl {
    name: String,
    is_enum: bool,
    sealed: bool,
    has_public_field: bool,
    file: PathBuf,
}

/// Every public type this crate declares under its own `src/`.
///
/// Found by walking the tree rather than by naming files, so a type moved
/// into a new module stays in scope.
fn public_type_decls() -> Vec<Decl> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    collect_rust_sources(&root, &mut sources);
    assert!(
        !sources.is_empty(),
        "no source found under {}",
        root.display()
    );
    sources.sort();

    let mut decls = Vec::new();
    for path in sources {
        let src = fs::read_to_string(&path).expect("source must be readable");
        decls.extend(decls_in(&src, &path));
    }
    decls
}

fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src/ must be readable") {
        let path = entry.expect("a directory entry must be readable").path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn decls_in(src: &str, path: &Path) -> Vec<Decl> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let Some((is_enum, name)) = declared_type(line) else {
            continue;
        };
        out.push(Decl {
            name,
            is_enum,
            sealed: attributes_above(&lines, idx).contains(&"#[non_exhaustive]"),
            has_public_field: has_public_field(&lines, idx),
            file: path.to_path_buf(),
        });
    }
    out
}

/// Only column-0 declarations: a type nested inside a function or a private
/// module is not public surface whatever its own `pub` says.
fn declared_type(line: &str) -> Option<(bool, String)> {
    let (is_enum, rest) = line
        .strip_prefix("pub enum ")
        .map(|rest| (true, rest))
        .or_else(|| line.strip_prefix("pub struct ").map(|rest| (false, rest)))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some((is_enum, name))
}

/// The attribute and comment block directly above a declaration, which
/// rustfmt keeps unbroken and this crate separates from the item before it
/// with a blank line.
fn attributes_above<'a>(lines: &[&'a str], decl: usize) -> Vec<&'a str> {
    lines[..decl]
        .iter()
        .rev()
        .take_while(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .collect()
}

/// A field a consumer can name is a field a consumer can construct with, so
/// it is the presence of one that makes sealing load-bearing.
fn has_public_field(lines: &[&str], decl: usize) -> bool {
    lines[decl + 1..]
        .iter()
        .take_while(|line| **line != "}")
        .any(|line| line.trim_start().starts_with("pub "))
}

#[test]
fn every_public_type_a_consumer_can_construct_is_sealed() {
    for decl in public_type_decls() {
        if OPEN_BY_DESIGN.contains(&decl.name.as_str()) {
            continue;
        }
        // A struct whose fields are all private is already unconstructible
        // from outside; the attribute would say nothing.
        if !decl.is_enum && !decl.has_public_field {
            continue;
        }
        assert!(
            decl.sealed,
            "{} in {} is public and constructible but not `#[non_exhaustive]`; \
             adding a variant or a field to it would be a breaking release (ADR-0013)",
            decl.name,
            decl.file.display()
        );
    }
}

#[test]
fn the_types_the_rule_had_missed_are_sealed() {
    let decls = public_type_decls();
    for name in SEALED_BY_RULE {
        let decl = decls
            .iter()
            .find(|d| d.name == *name)
            .unwrap_or_else(|| panic!("{name} is gone; the guard must be retargeted, not deleted"));
        assert!(
            decl.sealed,
            "{name} must stay `#[non_exhaustive]`: upstream adding one level, origin or \
             alignment would otherwise force a breaking release"
        );
    }
}

#[test]
fn the_geometric_types_are_left_unsealed_on_purpose() {
    let decls = public_type_decls();
    for name in OPEN_BY_DESIGN {
        let decl = decls
            .iter()
            .find(|d| d.name == *name)
            .unwrap_or_else(|| panic!("{name} is gone; the guard must be retargeted, not deleted"));
        assert!(
            !decl.sealed,
            "{name} is geometrically closed — sealing it costs every consumer literal \
             construction and functional record update for a field set that cannot grow"
        );
    }
}
