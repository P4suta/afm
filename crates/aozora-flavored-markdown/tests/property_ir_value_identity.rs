//! Value-level properties of a render — the half that needed `PartialEq`.
//!
//! Determinism was already a property here, but only of the *rendered string*
//! (`post_process_invariants::render_determinism`), and cross-path agreement
//! was pinned on one hand-written document, again as HTML
//! (`streaming_blocks::concatenated_block_html_matches_the_document_render`).
//! Both stopped at the HTML because HTML was the only output that could be
//! compared: `MarkdownDocument`, `RenderedIr`, `RenderedBlock` and `Diagnostic` had
//! no `PartialEq`, so an IR that differed run to run — or between the document
//! and the streaming path — had nothing to fail.
//!
//! The same gap covered the ergonomics. A `span` was only ever sliced by hand
//! at a call site, so "a `Some` span slices the source the caller passed in"
//! was asserted for 青空文庫 constructs over a fixture corpus
//! (`construct_spans`) and nowhere else. Here it is asserted for *every* span
//! and every range the IR carries, over generated input, through the
//! `From<ByteSpan> for SourceRange<usize>` a consumer is meant to use.
//!
//! The coordinate walk goes through the serialised form on purpose: a new
//! `Block` / `Inline` variant joins these properties without anyone
//! remembering to add an arm, which a typed walker with a `_ => {}` cannot
//! promise.

// That walk is the whole file, so the file follows the feature that puts the
// IR on the wire. Every gate builds the workspace, where the CLI and the wasm
// crate turn `serde` on, so this compiles out only for a lone
// `cargo test -p aozora-flavored-markdown --no-default-features`.
#![cfg(feature = "serde")]

use core::hash::Hash;
use core::ops::Range as ByteRange;
use std::hash::{DefaultHasher, Hasher};

use aozora_flavored_markdown::ir::{
    Block, ByteSpan, Inline, MarkdownDocument, SourcePosition, SourceRange,
};
use aozora_flavored_markdown::{
    Diagnostic, Options, RenderedBlocks, render, render_blocks, render_to_ir,
};
use aozora_flavored_markdown_test_support::config;
use aozora_flavored_markdown_test_support::generators::{
    aozora_fragment, commonmark_adversarial, pathological_aozora,
};
use proptest::prelude::*;
use serde_json::{Value, json};

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
    spans: Vec<ByteSpan>,
    ranges: Vec<SourceRange>,
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
fn span_at(value: &Value) -> Option<ByteSpan> {
    let start = coordinate(value.get("start")?)?;
    let end = coordinate(value.get("end")?)?;
    Some(ByteSpan::new(start, end))
}

fn range_at(value: &Value) -> Option<SourceRange> {
    let start = position_at(value.get("start")?)?;
    let end = position_at(value.get("end")?)?;
    Some(SourceRange::new(start, end))
}

fn position_at(value: &Value) -> Option<SourcePosition> {
    let line = coordinate(value.get("line")?)?;
    let column = coordinate(value.get("column")?)?;
    Some(SourcePosition::new(line, column))
}

fn coordinate(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok()
}

fn serialised(src: &str) -> Value {
    let document = render_to_ir(src, &Options::default()).ir;
    serde_json::to_value(document).expect("the IR must serialise")
}

// ---------------------------------------------------------------------------
// the wire round trip — the direction nothing in the workspace could state
// ---------------------------------------------------------------------------
//
// Every serde assertion in this suite ran one way: `serialised` above walks
// the JSON, `public_type_contract` compares a diagnostic's keys against its
// readers, and the wasm bridge and the CLI both only ever write. So the
// promise ADR-0012 and ADR-0017 actually make — that these are wire formats,
// which is to say that what one process writes another can read — was
// asserted by nothing, and could not be: no type here implemented
// `Deserialize`, so a test that tried would not have compiled.

/// The IR and the diagnostics of one render, read back from their own JSON.
///
/// Three assertions, because each covers the others' blind spot:
///
/// * The **value** comes back equal, and hashes equal with it — a document
///   read off the wire has to be usable as the key a rendered one is.
/// * The **JSON** of the value read back is the JSON that produced it. A key
///   `skip_serializing_if` dropped must still be missing after the reader has
///   supplied something for it; when it is not, the pair is a round trip in
///   neither direction, and re-serialising is the only thing that says so.
/// * `Diagnostic` is asserted beside the IR rather than separately. It is a
///   second derive over a second envelope (`aozora-md.diagnostics.v1`), it
///   travels with every render, and a property that stopped at `MarkdownDocument`
///   would leave the type this crate hands back in an *error* position as the
///   only public value nobody could read back.
fn assert_the_wire_round_trips(src: &str) {
    let rendered = render_to_ir(src, &Options::default());

    let json = serde_json::to_value(&rendered.ir).expect("the IR must serialise");
    let document: MarkdownDocument = serde_json::from_value(json.clone()).unwrap_or_else(|e| {
        panic!("the IR did not read back for src={src:?}: {e}\n  json = {json}")
    });
    assert_eq!(
        document, rendered.ir,
        "the IR read back is not the IR written for src={src:?}"
    );
    assert_eq!(
        hash_of(&document),
        hash_of(&rendered.ir),
        "an IR read back must hash as the one it equals: src={src:?}"
    );
    assert_eq!(
        serde_json::to_value(&document).expect("a document read back must serialise"),
        json,
        "the IR read back does not write the JSON it was read from: src={src:?}"
    );

    let envelope = serde_json::to_value(&rendered.diagnostics).expect("diagnostics must serialise");
    let diagnostics: Vec<Diagnostic> =
        serde_json::from_value(envelope.clone()).unwrap_or_else(|e| {
            panic!("the diagnostics did not read back for src={src:?}: {e}\n  json = {envelope}")
        });
    assert_eq!(
        diagnostics, rendered.diagnostics,
        "the diagnostics read back are not the ones written for src={src:?}"
    );
    assert_eq!(
        serde_json::to_value(&diagnostics).expect("diagnostics read back must serialise"),
        envelope,
        "the diagnostics read back do not write the JSON they were read from: src={src:?}"
    );
}

// ---------------------------------------------------------------------------
// the properties
// ---------------------------------------------------------------------------

/// The two render paths hand back the same document.
fn assert_the_streaming_path_agrees(src: &str) {
    let options = Options::default();
    let document = render_to_ir(src, &options);
    let RenderedBlocks {
        blocks,
        diagnostics,
        ..
    } = render_blocks(src, &options);
    let streamed: Vec<Block> = blocks.iter().flat_map(|block| block.ir.clone()).collect();
    assert_eq!(
        streamed, document.ir.blocks,
        "the streamed IR is not the document's for src={src:?}"
    );
    assert_eq!(
        diagnostics, document.diagnostics,
        "the two paths disagreed on diagnostics for src={src:?}"
    );
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
        let line_count = src
            .bytes()
            .filter(|&byte| matches!(byte, b'\n' | b'\r'))
            .count()
            .saturating_add(1);
        assert!(
            range.start.line as usize <= line_count && range.end.line as usize <= line_count,
            "range {range:?} names a line outside its source ({line_count} lines): src={src:?}"
        );
    }
}

proptest! {
    #![proptest_config(config::default())]

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

        let blocks = render_blocks(&src, &options);
        prop_assert_eq!(
            &blocks,
            &render_blocks(&src, &options),
            "two streaming renders of {:?} disagreed",
            src
        );
    }

    /// The streaming path and the document path describe the same document.
    /// They share a construct table and an AST by construction, so any
    /// difference is the per-block walker's own — which is exactly the walker
    /// that carries state (an open-container stack, a cursor) across blocks.
    ///
    #[test]
    fn the_streaming_path_projects_the_document_path_s_ir(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        assert_the_streaming_path_agrees(&src);
    }

    /// A `Some` span slices the caller's own source, and the width it reports
    /// is the width of that slice. Asserted through `SourceRange::from`, the
    /// conversion a consumer is pointed at, so the cast lives in one place
    /// instead of at every call site.
    #[test]
    fn every_projected_coordinate_addresses_the_source(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        assert_coordinates_address_the_source(&src);
    }

    /// The IR and the diagnostics survive their own wire format. No
    /// carve-out: every source these generators produce round-trips, which is
    /// what makes ADR-0012's and ADR-0017's "stable wire format" a claim a
    /// gate can check rather than a sentence in a document.
    #[test]
    fn the_wire_format_reads_back_what_it_wrote(
        src in prop_oneof![aozora_fragment(12), pathological_aozora(6), commonmark_adversarial()]
    ) {
        assert_the_wire_round_trips(&src);
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
    "｜\r------------",
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

#[test]
fn the_hard_documents_read_back_off_the_wire_too() {
    for src in HARD_SOURCES {
        assert_the_wire_round_trips(src);
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

/// A `［＃…終わり］` with no open contributes no block of its own. Omitting
/// that empty projection is what keeps the following HTML child zipped to
/// its own IR rather than to the orphan's slot.
#[test]
fn an_orphan_container_close_must_not_cost_the_next_block_its_ir() {
    for src in [
        "一\n\n［＃ここで字下げ終わり］\n\n二\n",
        "［＃ここで字下げ終わり］\n\n一\n\n二\n",
        "［＃ここで罫囲み終わり］本文",
    ] {
        assert_the_streaming_path_agrees(src);
    }
}

// ---------------------------------------------------------------------------
// the wire shape itself — the half a round trip is structurally blind to
// ---------------------------------------------------------------------------
//
// A round trip through one crate's own derive is symmetric: rename a key on
// both sides, or drop a `#[serde(rename)]` that both sides then stop
// applying, and every property above still holds. It is the same blind spot
// I3 had for fence bodies — a consistently wrong rewrite is still a fixed
// point — and the fix is the same one `property_canonicalize_fidelity` uses:
// assert against bytes this file wrote, not against what the library wrote a
// moment ago.
//
// The shapes below are the ones ADR-0012 / ADR-0017 publish and the ones the
// `.d.ts` and the playground read. Each is also a case the derive could
// plausibly get wrong on its own: `aozoraKind` sits in the same map as the
// `kind` discriminant that selects the variant, `codeBlock` is a tag that
// matches no variant name, and an absent optional key is a document with a
// hole in it that a reader still has to accept.

#[test]
fn an_aozora_block_reads_back_with_its_kind_beside_the_discriminant() {
    // `kind` selects the variant and `aozoraKind` is one of that variant's
    // own fields, in the same map. serde strips the discriminant before the
    // variant sees the rest, so the two coexist — the thing the internally
    // tagged representation was least obviously going to survive.
    //
    // Dropping the rename outright is now a compile error (serde refuses a
    // field named after the tag once `Deserialize` is derived; with
    // `Serialize` alone it compiled and emitted a duplicate JSON key). What
    // this pins is the rename's *target*: renaming it to anything else would
    // round-trip symmetrically and change the published key under every
    // consumer.
    let json = json!({
        "kind": "aozora",
        "aozoraKind": "ruby",
        "span": { "start": 0, "end": 12 },
        "html": "<ruby>青梅<rt>おうめ</rt></ruby>",
        "sourceLine": 3,
    });
    let block: Block = serde_json::from_value(json).expect("the published block shape must read");
    let Block::Aozora {
        kind,
        span,
        html,
        source_line,
    } = block
    else {
        panic!("`kind: \"aozora\"` must select `Block::Aozora`");
    };
    assert_eq!(
        kind, "ruby",
        "the notation tag is read off `aozoraKind`, not off the discriminant"
    );
    assert_eq!(
        span,
        Some(ByteSpan::new(0, 12)),
        "a byte span is two numbers"
    );
    assert!(html.starts_with("<ruby>"), "html: {html}");
    assert_eq!(
        source_line,
        Some(3),
        "`sourceLine` is camelCase on the wire"
    );
}

#[test]
fn every_optional_wire_key_is_absent_rather_than_null_and_reads_back_as_none() {
    // Absence, at the value level: what `skip_serializing_if` leaves out, the
    // reader must supply. Both shapes below are documents this crate writes —
    // a container close synthesised at end of input has no span to report,
    // and an inline outside any anchored block has no range — so a reader
    // that rejected them could not read this crate's own output.
    let block: Block = serde_json::from_value(json!({
        "kind": "aozora",
        "aozoraKind": "containerClose",
        "html": "</div>",
    }))
    .expect("a synthesised container close carries neither span nor sourceLine");
    let Block::Aozora {
        span, source_line, ..
    } = block
    else {
        panic!("`kind: \"aozora\"` must select `Block::Aozora`");
    };
    assert_eq!(span, None, "an absent `span` is `None`");
    assert_eq!(source_line, None, "an absent `sourceLine` is `None`");

    let inline: Inline = serde_json::from_value(json!({ "kind": "text", "value": "本文" }))
        .expect("an inline with no range must read");
    let Inline::Text { value, range } = inline else {
        panic!("`kind: \"text\"` must select `Inline::Text`");
    };
    assert_eq!(value, "本文", "the text is the text");
    assert_eq!(range, None, "an absent `range` is `None`");
}

#[test]
fn the_code_block_tag_is_the_published_one_and_not_the_variant_name() {
    let block: Block = serde_json::from_value(json!({
        "kind": "codeBlock",
        "value": "fn main() {}\n",
    }))
    .expect("`codeBlock` is the published tag for `Block::Code`");
    let Block::Code { lang, value, .. } = block else {
        panic!("`kind: \"codeBlock\"` must select `Block::Code`");
    };
    assert_eq!(lang, None, "an absent `lang` is `None`");
    assert_eq!(value, "fn main() {}\n", "the fence body is the value");

    // The other half of the rename: `code` is an *inline* tag, and a reader
    // that accepted it as a block would mean the rename had quietly lapsed.
    assert!(
        serde_json::from_value::<Block>(json!({ "kind": "code", "value": "x" })).is_err(),
        "`code` names no block; it is `Inline::Code`, and the two share a union in the `.d.ts`"
    );
    let inline: Inline = serde_json::from_value(json!({ "kind": "code", "value": "x" }))
        .expect("`code` is the inline tag");
    assert!(
        matches!(inline, Inline::Code { .. }),
        "`kind: \"code\"` must select `Inline::Code`, got {inline:?}"
    );
}

#[test]
fn a_document_this_test_wrote_round_trips_to_itself() {
    // The outermost shape — `{ "blocks": [ … ] }`, what the wasm bridge posts
    // and what the CLI's `--format json` writes — starting from bytes rather
    // than from a render. Read then written, it must come back unchanged:
    // that is the two directions agreeing about *absence*, which the property
    // above can only assert for keys a render happened to omit.
    let json = json!({
        "blocks": [{
            "kind": "paragraph",
            "children": [{ "kind": "text", "value": "本文" }],
        }],
    });
    let document: MarkdownDocument =
        serde_json::from_value(json.clone()).expect("the published document shape must read");
    assert_eq!(
        serde_json::to_value(&document).expect("a hand-written document serialises"),
        json,
        "a `#[serde(default)]` that supplied a value, or a `skip_serializing_if` that stopped \
         skipping, would show up here as a key this test never wrote"
    );
}
