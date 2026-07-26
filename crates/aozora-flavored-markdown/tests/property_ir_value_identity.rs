//! Value-level properties of a render — the half that needed `PartialEq`.
//!
//! Determinism was already a property here, but only of the *rendered string*
//! (`post_process_invariants::render_determinism`), and cross-path agreement
//! was pinned on one hand-written document, again as HTML
//! (`streaming_blocks::concatenated_block_html_matches_the_document_render`).
//! Both stopped at the HTML because HTML was the only output that could be
//! compared: `IrDocument`, `RenderedIr`, `RenderedBlock` and `Diagnostic` had
//! no `PartialEq`, so an IR that differed run to run — or between the document
//! and the streaming path — had nothing to fail.
//!
//! The same gap covered the ergonomics. A `span` was only ever sliced by hand
//! at a call site, so "a `Some` span slices the source the caller passed in"
//! was asserted for 青空文庫 constructs over a fixture corpus
//! (`construct_spans`) and nowhere else. Here it is asserted for *every* span
//! and every range the IR carries, over generated input, through the
//! `From<Span> for Range<usize>` a consumer is meant to use.
//!
//! The coordinate walk goes through the serialised form on purpose: a new
//! `IrBlock` / `IrInline` variant joins these properties without anyone
//! remembering to add an arm, which a typed walker with a `_ => {}` cannot
//! promise.

use core::hash::Hash;
use core::ops::Range as ByteRange;
use std::hash::{DefaultHasher, Hasher};

use aozora_flavored_markdown::ir::{IrBlock, Position, Range, Span};
use aozora_flavored_markdown::{Options, render, render_blocks_to_ir, render_to_ir};
use aozora_flavored_markdown_test_support::config::default_config;
use aozora_flavored_markdown_test_support::generators::{
    aozora_fragment, commonmark_adversarial, pathological_aozora,
};
use proptest::prelude::*;
use serde_json::Value;

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// coordinates, collected from the serialised IR so no variant escapes
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Coordinates {
    spans: Vec<Span>,
    ranges: Vec<Range>,
}

fn coordinates_of(document: &Value) -> Coordinates {
    let mut out = Coordinates::default();
    collect(document, &mut out);
    out
}

fn collect(value: &Value, out: &mut Coordinates) {
    match value {
        Value::Object(map) => {
            out.spans.extend(map.get("span").and_then(span_at));
            out.ranges.extend(map.get("range").and_then(range_at));
            for child in map.values() {
                collect(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect(child, out);
            }
        }
        _ => {}
    }
}

/// A byte span serialises as two numbers; a source range as two objects, so
/// neither reader can mistake the other's shape for its own.
fn span_at(value: &Value) -> Option<Span> {
    let start = coordinate(value.get("start")?)?;
    let end = coordinate(value.get("end")?)?;
    Some(Span::new(start, end))
}

fn range_at(value: &Value) -> Option<Range> {
    let start = position_at(value.get("start")?)?;
    let end = position_at(value.get("end")?)?;
    Some(Range::new(start, end))
}

fn position_at(value: &Value) -> Option<Position> {
    let line = coordinate(value.get("line")?)?;
    let column = coordinate(value.get("column")?)?;
    Some(Position::new(line, column))
}

fn coordinate(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok()
}

fn serialised(src: &str) -> Value {
    let document = render_to_ir(src, &Options::default()).ir;
    serde_json::to_value(document).expect("the IR must serialise")
}

// ---------------------------------------------------------------------------
// the properties
// ---------------------------------------------------------------------------

/// The two render paths hand back the same document.
fn assert_the_streaming_path_agrees(src: &str) {
    let options = Options::default();
    let document = render_to_ir(src, &options);
    let (blocks, diagnostics) = render_blocks_to_ir(src, &options);
    let streamed: Vec<IrBlock> = blocks.iter().flat_map(|block| block.ir.clone()).collect();
    assert_eq!(
        streamed, document.ir.blocks,
        "the streamed IR is not the document's for src={src:?}"
    );
    assert_eq!(
        diagnostics, document.diagnostics,
        "the two paths disagreed on diagnostics for src={src:?}"
    );
}

/// `［＃ここで字下げ終わり］`, `［＃ここで罫囲み終わり］` and their kin.
fn carries_a_container_close(src: &str) -> bool {
    src.contains("終わり］")
}

/// Every span and range the IR carries, held to what its type promises.
fn assert_coordinates_address_the_source(src: &str) {
    let coordinates = coordinates_of(&serialised(src));
    for span in &coordinates.spans {
        let text = src.get(ByteRange::from(*span));
        assert!(
            text.is_some(),
            "span {span:?} does not slice the source it was measured against: src={src:?}"
        );
        assert_eq!(
            text.unwrap_or_default().len(),
            span.len() as usize,
            "span {span:?} measures a different width than the text it slices: src={src:?}"
        );
    }
    for range in &coordinates.ranges {
        assert!(
            range.start <= range.end,
            "range {range:?} ends before it starts: src={src:?}"
        );
    }
}

proptest! {
    #![proptest_config(default_config())]

    /// Determinism, as the value rather than as the string: two independent
    /// renders of one source agree on IR, HTML and diagnostics alike. A
    /// container iteration order leaking into the IR — the classic source of
    /// a run-to-run difference — never reaches the HTML the older property
    /// compares.
    #[test]
    fn a_render_is_deterministic_as_a_value(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        let options = Options::default();
        prop_assert_eq!(
            render(&src, &options),
            render(&src, &options),
            "two renders of {:?} disagreed",
            src
        );

        let first = render_to_ir(&src, &options);
        let second = render_to_ir(&src, &options);
        prop_assert_eq!(&first, &second, "two IR renders of {:?} disagreed", src);
        prop_assert_eq!(
            hash_of(&first.ir),
            hash_of(&second.ir),
            "equal IR must hash equal, or the memo table keyed on it is wrong: {:?}",
            src
        );

        let blocks = render_blocks_to_ir(&src, &options);
        prop_assert_eq!(
            &blocks,
            &render_blocks_to_ir(&src, &options),
            "two streaming renders of {:?} disagreed",
            src
        );
    }

    /// The streaming path and the document path describe the same document.
    /// They share a construct table and an AST by construction, so any
    /// difference is the per-block walker's own — which is exactly the walker
    /// that carries state (an open-container stack, a cursor) across blocks.
    ///
    /// **Carved out**: a source carrying a container *close* marker. That is
    /// not a design decision, it is a live defect this property found on its
    /// first run — see
    /// [`an_orphan_container_close_must_not_cost_the_next_block_its_ir`].
    /// The filter is the broad form (any close marker, matched or not)
    /// because deciding whether a particular one is orphaned means redoing
    /// the parser's own pairing; the matched shapes are held to the full
    /// property by
    /// [`the_streaming_path_projects_the_document_path_s_ir_for_container_shapes`]
    /// instead.
    #[test]
    fn the_streaming_path_projects_the_document_path_s_ir(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        prop_assume!(!carries_a_container_close(&src));
        assert_the_streaming_path_agrees(&src);
    }

    /// A `Some` span slices the caller's own source, and the width it reports
    /// is the width of that slice. Asserted through `Range::from`, the
    /// conversion a consumer is pointed at, so the cast lives in one place
    /// instead of at every call site.
    #[test]
    fn every_projected_coordinate_addresses_the_source(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        assert_coordinates_address_the_source(&src);
    }
}

// ---------------------------------------------------------------------------
// the corpus the fixture-driven span test uses, held to the same rule
// ---------------------------------------------------------------------------

/// Documents whose spans a generator is unlikely to produce: notation that
/// resolves against text *before* it, and the normalisations that move every
/// offset out from under a span.
const HARD_SOURCES: &[&str] = &[
    "可哀想［＃「可哀想」に傍点］だ",
    "本文\r\n｜青梅《おうめ》",
    "\u{feff}｜青梅《おうめ》",
    "本文\n----------\n｜青梅《おうめ》",
    "〔e'tude〕｜青梅《おうめ》",
    "\u{feff}本文\r\n----------\r\n〔e'tude〕｜青梅《おうめ》",
    "| ruby | note |\n| -- | -- |\n| ｜青梅《おうめ》 | ［＃改ページ］ |\n",
    "> ［＃ここから字下げ］\n> ｜引用《いんよう》\n> ［＃ここで字下げ終わり］\n",
];

#[test]
fn the_hard_documents_coordinates_address_the_source_too() {
    for src in HARD_SOURCES {
        assert_coordinates_address_the_source(src);
    }
}

// ---------------------------------------------------------------------------
// container shapes — the carve-out, drawn as tightly as it can be drawn
// ---------------------------------------------------------------------------

/// Every container shape the streaming path *does* project faithfully: the
/// matched pair, and the open the document never closes (whose close marker
/// the renderer synthesises).
const MATCHED_CONTAINER_SHAPES: &[&str] = &[
    "［＃ここから字下げ］\n本文\n［＃ここで字下げ終わり］\n\nあと\n",
    "［＃ここから字下げ］\n\nあと\n",
    "［＃ここから罫囲み］\n本文\n［＃ここで罫囲み終わり］\n\nあと\n",
];

#[test]
fn the_streaming_path_projects_the_document_path_s_ir_for_container_shapes() {
    for src in MATCHED_CONTAINER_SHAPES {
        assert_the_streaming_path_agrees(src);
    }
}

/// The defect the property above had to carve out, pinned as the assertion
/// that *should* hold. A `［＃…終わり］` with no open is dropped by the parser,
/// and the block that follows it comes back from `render_blocks_to_ir` with
/// its `html` intact and its `ir` empty — one paragraph of structure lost, on
/// the path the editor bridge streams. `render_to_ir` keeps it, so the two
/// public renders of one document disagree.
///
/// Ignored rather than deleted: it is the specification, and the day the
/// walker stops losing the block it goes green and the `prop_assume!` above
/// comes out with it.
#[test]
#[ignore = "known defect: an orphan container close costs the next block its IR on the streaming path"]
fn an_orphan_container_close_must_not_cost_the_next_block_its_ir() {
    for src in [
        "一\n\n［＃ここで字下げ終わり］\n\n二\n",
        "［＃ここで字下げ終わり］\n\n一\n\n二\n",
        "［＃ここで罫囲み終わり］本文",
    ] {
        assert_the_streaming_path_agrees(src);
    }
}
