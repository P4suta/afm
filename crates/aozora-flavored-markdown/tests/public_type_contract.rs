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
//!
//! The same source-text reading answers a second question the compiler will
//! not: whether a public signature names a type belonging to `comrak` or to
//! the sibling `aozora` parser. Both are pre-1.0 and neither is re-exported,
//! so such a signature makes their minor bumps this crate's breaking changes
//! — which is exactly what `diagnostics.rs` claims the boundary prevents.

use core::error::Error as StdError;
use core::fmt::{Debug, Display};
use core::hash::Hash;
use core::iter::once;
use core::ops::Range as ByteRange;
use std::collections::BTreeSet;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};

use aozora_flavored_markdown::ir::{Block, Document, Inline, Position, Range, TableAlign};
use aozora_flavored_markdown::{
    Diagnostic, DiagnosticSource, Options, Rendered, RenderedBlock, RenderedBlocks, RenderedIr,
    Severity, Span, render, render_blocks, render_to_ir,
};
use aozora_flavored_markdown_test_support::config;
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

/// The half the derive table above does not cover.
///
/// A `Diagnostic` is the one value this crate hands back in an *error*
/// position, so a host has to be able to print it and to put it in a
/// `Box<dyn Error>` chain without an adapter of its own. Nothing here asked
/// until now, and the CLI paid for it: `CliDiagnostic`, a shadow struct whose
/// entire content was `#[error("{message}")]` plus a hand-written
/// `impl miette::Diagnostic`, existed because the type it copied could not be
/// printed.
fn behaves_as_a_reportable_error<T: Debug + Clone + Display + StdError + Send + Sync + 'static>(
    value: &T,
    message: &str,
) {
    assert_eq!(
        value.to_string(),
        message,
        "`Display` must be the message alone — it is what a host prints under the header: \
         {value:?}"
    );
    let reported: &dyn StdError = value;
    assert!(
        reported.source().is_none(),
        "nothing here wraps a lower-level failure, so the cause chain must stay empty: {value:?}"
    );
    // The owned trait object is the point of `impl Error`, and the bounds are
    // not decoration: `anyhow::Error`, `Box<dyn Error + Send + Sync>` and
    // `miette::Report::new` all demand `Send + Sync + 'static`, which is
    // exactly what a `code` borrowed from the caller's text would cost. The
    // wire round-trip this type is heading for makes that a live temptation.
    let owned: Box<dyn StdError + Send + Sync> = Box::new(value.clone());
    assert_eq!(
        owned.to_string(),
        message,
        "boxing must not change what the host prints: {value:?}"
    );
}

/// Every diagnostic the malformed pool produces.
///
/// Reached through the API rather than constructed: `Diagnostic` is sealed
/// and has no public constructor, so a consumer only ever meets one a render
/// handed back.
fn diagnostics_from_the_malformed_pool() -> Vec<Diagnostic> {
    let diagnostics: Vec<Diagnostic> = MALFORMED
        .iter()
        .flat_map(|src| render(src, &Options::default()).diagnostics)
        .collect();
    assert!(
        !diagnostics.is_empty(),
        "no malformed sample produced a diagnostic; the sample pool is stale"
    );
    diagnostics
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

fn visit_blocks(blocks: &[Block], seen: &mut Seen) {
    for block in blocks {
        behaves_as_a_hashable_value(block);
        seen.blocks += 1;
        visit_children(block, seen);
    }
}

fn visit_children(block: &Block, seen: &mut Seen) {
    match block {
        Block::Paragraph { children, .. } | Block::Heading { children, .. } => {
            visit_inlines(children, seen);
        }
        Block::Blockquote { children, .. } => visit_blocks(children, seen),
        Block::List { items, .. } => {
            for item in items {
                behaves_as_a_hashable_value(item);
                seen.items += 1;
                visit_blocks(&item.children, seen);
            }
        }
        Block::Table {
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

fn visit_inlines(inlines: &[Inline], seen: &mut Seen) {
    for inline in inlines {
        behaves_as_a_hashable_value(inline);
        seen.inlines += 1;
        match inline {
            Inline::Strong { children, .. }
            | Inline::Emphasis { children, .. }
            | Inline::Link { children, .. } => visit_inlines(children, seen),
            Inline::Image { alt, .. } => visit_inlines(alt, seen),
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
    // The whole envelope, not only its parts: `render_blocks` used to hand
    // back a tuple, which is `Clone`/`Eq` for free and says nothing about
    // whether the type this crate owns is.
    behaves_as_a_value(&render_blocks(SAMPLE, &Options::default()));
    let RenderedBlocks {
        blocks,
        diagnostics,
        ..
    } = render_blocks(SAMPLE, &Options::default());
    assert!(!blocks.is_empty(), "the sample must produce blocks");
    for block in &blocks {
        behaves_as_a_value(block);
    }
    for diagnostic in &diagnostics {
        behaves_as_a_hashable_value(diagnostic);
    }
}

#[test]
fn options_is_a_hashable_value_whichever_path_built_it() {
    // Hiding comrak is what makes `Options` derivable at all, and the derives
    // are load-bearing: a host memoises a render on the pair it was given.
    // Two chains that describe the same dialect must therefore be one key,
    // and knob order must not matter.
    behaves_as_a_hashable_value(&Options::default());
    assert_eq!(
        Options::new(),
        Options::default(),
        "`new` and `default` must be the same dialect"
    );
    let by_chain = Options::commonmark()
        .with_tables(true)
        .with_strikethrough(true)
        .with_autolinks(true)
        .with_task_lists(true);
    behaves_as_a_hashable_value(&by_chain);
    assert_eq!(by_chain, Options::gfm(), "two paths, one configuration");
    assert_eq!(
        hash_of(&by_chain),
        hash_of(&Options::gfm()),
        "equal options must hash equal, or a memo keyed on them splits"
    );
    let one_way = Options::new().with_hardbreaks(false).with_tables(false);
    let other_way = Options::new().with_tables(false).with_hardbreaks(false);
    assert_eq!(one_way, other_way, "knob order must not matter");
    assert_ne!(
        one_way,
        Options::new(),
        "a knob that changed nothing would make every configuration equal"
    );
}

#[test]
fn a_diagnostic_and_its_two_enums_compare_and_hash() {
    for diagnostic in &diagnostics_from_the_malformed_pool() {
        behaves_as_a_hashable_value(diagnostic);
        behaves_as_a_hashable_value(&diagnostic.severity());
        behaves_as_a_hashable_value(&diagnostic.source());
        behaves_as_a_hashable_value(&diagnostic.span());
    }
    behaves_as_a_hashable_value(&Severity::Error);
    behaves_as_a_hashable_value(&DiagnosticSource::Internal);
}

#[test]
fn a_diagnostic_prints_and_chains_as_an_error_without_a_shadow_type() {
    for diagnostic in &diagnostics_from_the_malformed_pool() {
        behaves_as_a_reportable_error(diagnostic, diagnostic.message());
        assert!(
            !diagnostic.to_string().contains(diagnostic.code()),
            "`Display` must not repeat the code: a reporter prints the code in the header, so \
             a message carrying it too prints it twice ({diagnostic:?})"
        );
        // `Error::source` and `Diagnostic::source` are two questions wearing
        // one name — the cause chain, and which side the diagnostic blames.
        // The inherent method wins unqualified resolution, so this is what a
        // consumer writing `d.source()` gets. That is the whole content of
        // the `#[expect(clippy::same_name_method)]` the crate carries, and
        // nothing but a call site can say which way it actually resolved.
        let origin: DiagnosticSource = diagnostic.source();
        assert!(
            StdError::source(diagnostic).is_none(),
            "the cause chain must stay empty; `{origin:?}` answers the other question"
        );
    }
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
        Document::default().blocks.is_empty(),
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
    let blocks = RenderedBlocks::default();
    assert!(
        blocks.blocks.is_empty() && blocks.diagnostics.is_empty(),
        "the default streaming render is the empty one: {blocks:?}"
    );
}

#[test]
fn the_default_alignment_is_the_one_a_marker_less_column_projects() {
    // `#[default]` on the wrong variant would compile, serialise and pass
    // every other test — this is what says which variant it belongs on.
    let src = "| a | b | c |\n|---|:--:|--:|\n| 1 | 2 | 3 |\n";
    let rendered = render_to_ir(src, &Options::default());
    let Block::Table { align, .. } = &rendered.ir.blocks[0] else {
        panic!("expected a table, got {:?}", rendered.ir.blocks[0]);
    };
    assert_eq!(
        align[0],
        TableAlign::default(),
        "a column with no alignment marker must project the default variant"
    );
    assert_ne!(
        align[1],
        TableAlign::default(),
        "a centred column must not project the default variant"
    );
}

proptest! {
    #![proptest_config(config::default())]

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

/// The six the rule had missed, plus `Error`. Named individually so a
/// regression reports the type rather than a count — and so deleting one
/// fails here rather than passing by simply not being found.
///
/// `Error` earns its place for a reason the others do not have: the two
/// failures it names today are the two the canonicaliser can reach today, and
/// a third — a pass budget, an upstream `ParseError` variant — is exactly the
/// kind of thing that arrives in a minor release.
const SEALED_BY_RULE: &[&str] = &[
    "Severity",
    "DiagnosticSource",
    "TableAlign",
    "Document",
    "TableRow",
    "ListItem",
    "Error",
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
    sources()
        .iter()
        .flat_map(|(path, src)| decls_in(src, path))
        .collect()
}

/// Every `.rs` file under this crate's own `src/`, in path order.
fn sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    collect_rust_sources(&root, &mut paths);
    assert!(
        !paths.is_empty(),
        "no source found under {}",
        root.display()
    );
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let src = fs::read_to_string(&path).expect("source must be readable");
            (path, src)
        })
        .collect()
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

// ---------------------------------------------------------------------------
// the private-field half — a reader per wire key
// ---------------------------------------------------------------------------
//
// Making a field private takes it out of reach of every other rule in this
// file. `every_public_type_a_consumer_can_construct_is_sealed` *skips* a
// struct with no public field — correctly, since one cannot be constructed
// from outside — and the foreign-type and laziness readers only look at
// declarations a consumer can name. So `Diagnostic`'s five field names, which
// are the `aozora-md.diagnostics.v1` wire names (ADR-0012) and the field names
// of the emitted `.d.ts`, now live where nothing but serde's derive reads
// them: rename one and every gate in the workspace stays green while the
// envelope changes shape.
//
// These two are what `pub` used to give for free. Whatever goes on the wire
// comes back through a reader of the same name, and answers the same value.

/// The fields a struct declaration lists, private ones included, each paired
/// with whether a consumer can name it. Comment and attribute lines are
/// skipped; a field is `name: Type,` at one indent level.
fn fields_of(lines: &[&str], decl: usize) -> Vec<(String, bool)> {
    lines[decl + 1..]
        .iter()
        .take_while(|line| **line != "}")
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                return None;
            }
            let head = trimmed.split_once(':')?.0;
            let public = head.starts_with("pub ");
            let name = head.strip_prefix("pub ").unwrap_or(head).trim();
            (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .then(|| (name.to_owned(), public))
        })
        .collect()
}

/// Whether the attribute block above a declaration puts it on the wire. Only
/// a `#[derive(…)]` counts: the `#[cfg_attr(feature = "tsify", …)]` beside it
/// names `Tsify`, which follows serde rather than deciding anything.
fn derives_serialize(lines: &[&str], decl: usize) -> bool {
    attributes_above(lines, decl)
        .iter()
        .any(|line| line.starts_with("#[derive(") && line.contains("Serialize"))
}

/// Whether the file declares a public reader of that name, `const` or not.
fn declares_reader(src: &str, field: &str) -> bool {
    [
        format!("pub fn {field}(&self)"),
        format!("pub const fn {field}(&self)"),
    ]
    .iter()
    .any(|signature| src.contains(signature))
}

#[test]
fn every_wire_field_a_consumer_cannot_name_has_a_reader_of_the_same_name() {
    let mut checked: Vec<(String, usize)> = Vec::new();
    for (path, src) in sources() {
        let lines: Vec<&str> = src.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let Some((is_enum, name)) = declared_type(line) else {
                continue;
            };
            if is_enum || !derives_serialize(&lines, idx) {
                continue;
            }
            let fields = fields_of(&lines, idx);
            if fields.is_empty() || fields.iter().any(|(_, public)| *public) {
                continue;
            }
            for (field, _) in &fields {
                assert!(
                    declares_reader(&src, field),
                    "{}: `{name}.{field}` is serialised but private, and nothing reads it back. \
                     A consumer who can see a key in the JSON must be able to reach it from the \
                     type, by the same name",
                    path.display()
                );
            }
            checked.push((name, fields.len()));
        }
    }
    assert_eq!(
        checked,
        vec![("Diagnostic".to_owned(), 5)],
        "the wire types with private fields changed; retarget this rule deliberately rather \
         than letting it pass by finding nothing"
    );
}

/// The JSON one public value serialises to.
fn wire<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).expect("a public value must serialise")
}

#[test]
fn the_wire_keys_of_a_diagnostic_are_exactly_what_its_readers_answer() {
    for diagnostic in &diagnostics_from_the_malformed_pool() {
        let json = wire(diagnostic);
        let object = json
            .as_object()
            .unwrap_or_else(|| panic!("a diagnostic serialises as an object, got {json}"));
        let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["code", "message", "severity", "source", "span"]
                .into_iter()
                .collect::<BTreeSet<&str>>(),
            "the `aozora-md.diagnostics.v1` key set changed (ADR-0012): {json}"
        );
        // Same name, same value. A reader that drifted from the field it
        // reads would still serialise, and the CLI's envelope — built out of
        // the readers — would then disagree with the wasm bridge's, built out
        // of this derive, under one schema name.
        assert_eq!(object["code"], wire(diagnostic.code()), "code: {json}");
        assert_eq!(
            object["message"],
            wire(diagnostic.message()),
            "message: {json}"
        );
        assert_eq!(
            object["severity"],
            wire(diagnostic.severity()),
            "severity: {json}"
        );
        assert_eq!(
            object["source"],
            wire(diagnostic.source()),
            "source: {json}"
        );
        assert_eq!(object["span"], wire(diagnostic.span()), "span: {json}");
    }
}

// ---------------------------------------------------------------------------
// the fallibility boundary — one public function returns a `Result`
// ---------------------------------------------------------------------------

/// The entry points that must stay infallible, and the reason the rule below
/// is a rule rather than a sentence in `canonicalize`'s rustdoc.
///
/// CommonMark is a total grammar — pulldown-cmark, comrak and markdown-rs all
/// render infallibly — so what the lexer saw comes back as a [`Diagnostic`]
/// beside a rendered document, at a rustc warning's standing. Giving one of
/// these a `Result` would be a breaking change made in the name of tidiness,
/// and it is the *symmetry* with `canonicalize` that would motivate it. Read
/// off the source rather than asserted per function, so an entry point added
/// later is covered without editing this.
const INFALLIBLE_BY_DESIGN: &[&str] = &["render", "render_to_ir", "render_blocks", "to_html"];

/// The name a `pub fn` declares, `const` and `async` spellings included.
fn public_fn_name(decl: &str) -> Option<String> {
    let mut rest = decl.trim().strip_prefix("pub ")?.trim_start();
    for qualifier in ["const ", "async ", "extern "] {
        if let Some(tail) = rest.strip_prefix(qualifier) {
            rest = tail.trim_start();
        }
    }
    let name: String = rest
        .strip_prefix("fn ")?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

#[test]
fn the_canonicaliser_is_the_only_public_function_that_can_fail() {
    let publics: Vec<String> = public_type_decls().into_iter().map(|d| d.name).collect();
    let mut fallible: Vec<String> = Vec::new();
    let mut infallible_seen: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0usize;
    for (path, src) in sources() {
        let lines: Vec<&str> = src.lines().collect();
        for start in 0..lines.len() {
            if !opens_public_surface(lines[start], &publics) {
                continue;
            }
            let decl = declaration_text(&lines, start);
            let Some(name) = public_fn_name(&decl) else {
                continue;
            };
            checked += 1;
            if INFALLIBLE_BY_DESIGN.contains(&name.as_str()) {
                infallible_seen.insert(name.clone());
            }
            if decl.contains("-> Result") {
                fallible.push(format!("{}:{}: {name}", path.display(), start + 1));
            }
        }
    }
    assert!(
        checked > 15,
        "only {checked} public functions found; the reader must be retargeted, not deleted"
    );
    assert_eq!(
        infallible_seen,
        INFALLIBLE_BY_DESIGN
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<String>>(),
        "an entry point named here is gone or renamed; retarget the rule rather than \
         letting it pass by finding nothing"
    );
    assert_eq!(
        fallible.len(),
        1,
        "exactly one public function may return a `Result` — the canonicaliser, which \
         has a source it can be handed too much of. Found: {fallible:?}"
    );
    assert!(
        fallible[0].ends_with(": canonicalize"),
        "the fallible one must be `canonicalize`, not {:?}",
        fallible[0]
    );
}

/// The return type a `pub fn` names: the text after the first arrow outside
/// any parameter list, with a `where` clause cut off first. No public function
/// in this crate carries an `Fn`-bounded generic — a second arrow — and the
/// entry-point round-trip below is what would report it if one arrived.
fn return_type(decl: &str) -> Option<&str> {
    let head = decl.split(" where ").next().unwrap_or(decl);
    let bytes = head.as_bytes();
    let mut depth = 0i32;
    for (idx, &byte) in bytes.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'-' if depth == 0 && bytes.get(idx + 1) == Some(&b'>') => {
                return Some(head[idx + 2..].trim());
            }
            _ => {}
        }
    }
    None
}

/// The leading identifier of a return type — `RenderedBlocks` of
/// `RenderedBlocks`, `Result` of `Result<String, Error>`.
fn head_of(ty: &str) -> String {
    idents(ty).first().cloned().unwrap_or_default()
}

#[test]
fn no_public_function_returns_an_anonymous_tuple() {
    // The shape this rule exists for shipped: `render_blocks_to_ir` handed
    // back `(Vec<RenderedBlock>, Vec<Diagnostic>)`. A tuple cannot carry
    // `#[non_exhaustive]`, cannot name its own members, and cannot grow one
    // without breaking every destructuring at every call site — so the
    // sealing rule two tests up had nothing to bind to. Every other public
    // output is a named type; the ones the entry points return must further
    // be *this crate's* named types, sealed.
    let publics: Vec<String> = public_type_decls().into_iter().map(|d| d.name).collect();
    let sealed: BTreeSet<String> = public_type_decls()
        .into_iter()
        .filter(|d| d.sealed)
        .map(|d| d.name)
        .collect();
    let mut entry_points: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0usize;
    for (path, src) in sources() {
        let lines: Vec<&str> = src.lines().collect();
        for start in 0..lines.len() {
            if !opens_public_surface(lines[start], &publics) {
                continue;
            }
            let decl = declaration_text(&lines, start);
            let Some(name) = public_fn_name(&decl) else {
                continue;
            };
            let Some(ty) = return_type(&decl) else {
                continue;
            };
            checked += 1;
            assert!(
                !ty.starts_with('('),
                "{}:{}: `{name}` returns the anonymous tuple `{ty}`; a tuple cannot be \
                 `#[non_exhaustive]`, so growing it is a breaking release with no way to \
                 stage it (ADR-0013)",
                path.display(),
                start + 1,
            );
            if !INFALLIBLE_BY_DESIGN.contains(&name.as_str()) {
                continue;
            }
            entry_points.insert(name.clone());
            let head = head_of(ty);
            assert!(
                head == "String" || sealed.contains(&head),
                "{}:{}: entry point `{name}` returns `{ty}`, which is neither a `String` \
                 nor a sealed public type of this crate",
                path.display(),
                start + 1,
            );
        }
    }
    assert!(
        checked > 15,
        "only {checked} returning public functions found; the reader must be retargeted"
    );
    assert_eq!(
        entry_points,
        INFALLIBLE_BY_DESIGN
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<String>>(),
        "an entry point is gone or renamed; retarget the rule rather than letting it pass \
         by finding nothing"
    );
    // The reader itself, on the signature that used to be here and on the one
    // shape it must not misread.
    assert_eq!(
        return_type(
            "pub fn render_blocks_to_ir(i: &str, o: &Options) -> (Vec<RenderedBlock>, Vec<Diagnostic>) "
        ),
        Some("(Vec<RenderedBlock>, Vec<Diagnostic>)"),
        "the reader must see the retired tuple return"
    );
    assert_eq!(
        return_type("pub fn f(cb: fn(u8) -> u8) -> String "),
        Some("String"),
        "an arrow inside the parameter list is not the return type"
    );
}

/// Types whose whole job is *when* a value is built, not what it is. A public
/// signature naming one publishes an implementation detail a consumer cannot
/// act on and this crate cannot change without a breaking release.
const LAZINESS_WRAPPERS: &[&str] = &["LazyLock", "OnceLock", "LazyCell", "OnceCell", "Lazy"];

fn names_a_laziness_wrapper(decl: &str) -> Option<&'static str> {
    let words = idents(decl);
    LAZINESS_WRAPPERS
        .iter()
        .copied()
        .find(|wrapper| words.iter().any(|word| word == wrapper))
}

#[test]
fn no_public_declaration_names_a_laziness_wrapper() {
    // The sibling rule below catches a *foreign crate's* type on the surface.
    // This one catches the std wrapper that was on it: `AOZORA_MD_CLASSES`
    // was a `pub static … : LazyLock<Vec<String>>`, so every consumer read
    // the interning strategy out of the signature and would have seen a
    // breaking change had it moved to a `OnceLock` or a plain `const`.
    // Laziness is the module's business; `&'static [&'static str]` is the
    // contract.
    let publics: Vec<String> = public_type_decls().into_iter().map(|d| d.name).collect();
    let mut checked = 0usize;
    for (path, src) in sources() {
        let lines: Vec<&str> = src.lines().collect();
        for start in 0..lines.len() {
            if !opens_public_surface(lines[start], &publics) {
                continue;
            }
            checked += 1;
            let decl = declaration_text(&lines, start);
            assert!(
                names_a_laziness_wrapper(&decl).is_none(),
                "{}:{}: `{}` names a laziness wrapper; hand out the value it interns \
                 (`&'static [&'static str]`) and keep the wrapper private",
                path.display(),
                start + 1,
                decl.trim()
            );
        }
    }
    assert!(
        checked > 40,
        "only {checked} public declarations found; the reader must be retargeted, not deleted"
    );
    assert_eq!(
        names_a_laziness_wrapper(
            "pub static AOZORA_MD_CLASSES: LazyLock<Vec<String>> = LazyLock::new(|| "
        ),
        Some("LazyLock"),
        "the rule must catch the surface this crate used to ship, or it can pass by \
         finding nothing"
    );
    assert_eq!(
        names_a_laziness_wrapper("pub fn all() -> &'static [&'static str] "),
        None,
        "the replacement surface must stay clean"
    );
}

/// The modules a consumer is meant to read, and therefore the only ones that
/// may appear in the rendered docs. Everything else this crate makes `pub` is
/// reachable for a mechanical reason — the leak checks read `sentinels::ALL`
/// — and carries `#[doc(hidden)]` to say so.
const DOCUMENTED_MODULES: &[&str] = &["classes", "diagnostics", "ir", "theme"];

/// The attributes attached to the declaration at `decl`: the unbroken run of
/// `#[…]` lines directly above it. Narrower than [`attributes_above`], which
/// also reads the comment block, so a `#[doc(hidden)]` on a *neighbouring*
/// item in the same paragraph-less run cannot be mistaken for this one's.
fn attributes_directly_above<'a>(lines: &[&'a str], decl: usize) -> Vec<&'a str> {
    lines[..decl]
        .iter()
        .rev()
        .take_while(|line| line.trim_start().starts_with("#["))
        .map(|line| line.trim())
        .collect()
}

/// The name a column-0 `pub mod` declares, inline (`pub mod x {`) and file
/// (`pub mod x;`) spellings alike.
fn declared_module(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("pub mod ")?;
    let name = rest.split([';', ' ', '{']).next()?;
    (!name.is_empty()).then_some(name)
}

#[test]
fn every_public_module_is_documented_surface_or_hidden() {
    // A rule over the module list rather than a note on the one module that
    // needed hiding: `pub mod sentinels` published the PUA representation to
    // rustdoc for as long as nothing said it must not, and `pub mod html`
    // published a second entry point for the same render. Adding a module to
    // `DOCUMENTED_MODULES` is the deliberate act; making one `pub` is not.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (path, src) in sources() {
        let lines: Vec<&str> = src.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let Some(name) = declared_module(line) else {
                continue;
            };
            seen.insert(name.to_owned());
            let hidden = attributes_directly_above(&lines, idx).contains(&"#[doc(hidden)]");
            assert!(
                hidden || DOCUMENTED_MODULES.contains(&name),
                "{}:{}: `pub mod {name}` is neither documented surface nor `#[doc(hidden)]`; \
                 a module a consumer cannot use is one rustdoc should not show them",
                path.display(),
                idx + 1
            );
        }
    }
    // The rule above passes vacuously on a crate with no `pub mod` at all,
    // so the modules it is written about are named.
    for name in DOCUMENTED_MODULES.iter().chain(once(&"sentinels")) {
        assert!(
            seen.contains(*name),
            "`pub mod {name}` is gone or renamed; retarget this rule rather than \
             letting it pass by finding nothing. Found: {seen:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// the boundary — no public signature names a foreign type
// ---------------------------------------------------------------------------

/// The two crates whose types must not reach a signature a consumer can see.
/// Both are pre-1.0 and neither is re-exported, so naming one in public makes
/// its next minor bump a breaking release here.
const FOREIGN_CRATES: [&str; 2] = ["comrak", "aozora"];

/// The names a foreign type can go by in one file: the qualified prefixes,
/// plus whatever that file imported from those crates. The import is the only
/// way a bare `AstNode` gets into scope, so reading the `use` lines is what
/// turns this from a grep for `comrak::` — which `Options::comrak_mut` would
/// have failed but `StreamingIrBuilder::walk_block` would not — into a rule.
fn foreign_names(src: &str) -> Vec<String> {
    let mut names: Vec<String> = FOREIGN_CRATES
        .iter()
        .map(|krate| format!("{krate}::"))
        .collect();
    let mut pending = String::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if pending.is_empty()
            && !FOREIGN_CRATES
                .iter()
                .any(|krate| trimmed.starts_with(&format!("use {krate}::")))
        {
            continue;
        }
        pending.push_str(trimmed);
        if !pending.contains(';') {
            continue;
        }
        // Type imports only: a lowercase tail is a module segment, and a
        // free function imported from either crate is caught by the prefix
        // where it is called.
        names.extend(
            idents(&pending)
                .into_iter()
                .filter(|ident| ident.starts_with(char::is_uppercase)),
        );
        pending.clear();
    }
    names
}

fn idents(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// A line that opens something a consumer can name: a `pub ` item or field —
/// never `pub(crate)`, which is why the prefix carries its space — or a trait
/// impl on one of this crate's public types.
fn opens_public_surface(line: &str, publics: &[String]) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("pub ") {
        return true;
    }
    let Some(rest) = trimmed.strip_prefix("impl ").or_else(|| {
        trimmed
            .strip_prefix("impl<")
            .and_then(|r| r.split_once('>').map(|(_, tail)| tail))
    }) else {
        return false;
    };
    rest.rsplit(" for ")
        .next()
        .map(|target| idents(target).first().cloned().unwrap_or_default())
        .is_some_and(|target| publics.contains(&target))
}

/// The declaration at `start`, joined across the lines a signature may wrap
/// onto and cut at the body it introduces. Comments are dropped first: a
/// parameter documented as coming from upstream is prose, not surface.
fn declaration_text(lines: &[&str], start: usize) -> String {
    let mut text = String::new();
    let mut depth = 0isize;
    for line in lines.iter().skip(start).take(16) {
        let code = line.split_once("//").map_or(*line, |(before, _)| before);
        text.push_str(code.trim());
        text.push(' ');
        depth += isize::try_from(code.matches('(').count()).unwrap_or(0)
            - isize::try_from(code.matches(')').count()).unwrap_or(0);
        let ends = code.trim_end();
        if depth <= 0 && (code.contains('{') || ends.ends_with(';') || ends.ends_with(',')) {
            break;
        }
    }
    text.find('{')
        .map_or_else(|| text.clone(), |body| text[..body].to_owned())
}

fn names_a_foreign_type(decl: &str, foreign: &[String]) -> Option<String> {
    let words = idents(decl);
    foreign
        .iter()
        .find(|name| {
            name.strip_suffix("::").map_or_else(
                || words.contains(name),
                |krate| decl.contains(&format!("{krate}::")),
            )
        })
        .cloned()
}

#[test]
fn no_public_declaration_names_a_comrak_or_aozora_type() {
    let publics: Vec<String> = public_type_decls().into_iter().map(|d| d.name).collect();
    let mut checked = 0usize;
    for (path, src) in sources() {
        let foreign = foreign_names(&src);
        let lines: Vec<&str> = src.lines().collect();
        for start in 0..lines.len() {
            if !opens_public_surface(lines[start], &publics) {
                continue;
            }
            checked += 1;
            let decl = declaration_text(&lines, start);
            assert!(
                names_a_foreign_type(&decl, &foreign).is_none(),
                "{}:{}: `{}` names a type from an unexported pre-1.0 dependency; a minor \
                 bump of it would be a breaking release of this crate (ADR-0021)",
                path.display(),
                start + 1,
                decl.trim()
            );
        }
    }
    assert!(
        checked > 40,
        "only {checked} public declarations found; the reader must be retargeted, not deleted"
    );
}

#[test]
fn the_foreign_type_rule_rejects_the_surface_this_crate_used_to_ship() {
    // The three shapes that were public before comrak and aozora were hidden.
    // Without this, the rule above could pass by finding nothing at all.
    let file = "\
use comrak::nodes::AstNode;

impl From<&aozora::Diagnostic> for Diagnostic {
    fn from(d: &aozora::Diagnostic) -> Self {
        Self
    }
}

impl Options {
    pub fn comrak_mut(&mut self) -> &mut comrak::Options<'static> {
        &mut self.comrak
    }
}

impl StreamingIrBuilder {
    pub fn walk_block<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<Block> {
        Vec::new()
    }
}
";
    let publics = vec!["Diagnostic".to_owned()];
    let foreign = foreign_names(file);
    let lines: Vec<&str> = file.lines().collect();
    let caught: Vec<usize> = (0..lines.len())
        .filter(|&start| opens_public_surface(lines[start], &publics))
        .filter(|&start| names_a_foreign_type(&declaration_text(&lines, start), &foreign).is_some())
        .collect();
    assert_eq!(
        caught.len(),
        3,
        "the rule must catch the `From` impl, the comrak escape hatch and the bare \
         `AstNode` parameter; caught lines {caught:?}"
    );
}
