//! Compile-time and wire-level checks from the same external-crate boundary a
//! published consumer sees. This deliberately imports and uses the API instead
//! of inferring it by parsing Rust source text.

use core::ops::Range as StdRange;

use aozora_flavored_markdown::ir::{MarkdownDocument, SourcePosition, SourceRange, TableAlign};
use aozora_flavored_markdown::{
    ByteSpan, CanonicalizeError, Options, Rendered, RenderedBlocks, RenderedIr, canonicalize,
    render, render_blocks, render_to_ir,
};

#[test]
fn renamed_geometry_is_constructible_without_dom_shadow_names() {
    let span = ByteSpan::new(3, 9);
    let bytes: StdRange<usize> = span.into();
    assert_eq!(bytes, 3..9);

    let start = SourcePosition::new(2, 5);
    let range = SourceRange::new(start, SourcePosition::new(3, 1));
    assert_eq!(range.start, start);
    assert!(range < SourceRange::new(SourcePosition::new(3, 1), SourcePosition::new(3, 2)));

    let document = MarkdownDocument::default();
    assert!(document.blocks.is_empty());
    assert_eq!(TableAlign::default(), TableAlign::Default);
}

#[test]
fn every_entry_point_has_the_published_result_shape() {
    let options = Options::default();
    let rendered: Rendered = render("text", &options);
    let rendered_ir: RenderedIr = render_to_ir("text", &options);
    let streamed: RenderedBlocks = render_blocks("text", &options);
    let canonical: Result<String, CanonicalizeError> = canonicalize("text");

    assert!(!rendered.html.is_empty());
    assert!(!rendered_ir.ir.blocks.is_empty());
    assert!(!streamed.blocks.is_empty());
    assert_eq!(canonical, Ok("text".to_owned()));
}

#[cfg(feature = "serde")]
#[test]
fn renamed_types_round_trip_without_changing_wire_keys_or_tags() {
    let rendered = render_to_ir("# heading", &Options::default());
    let json = serde_json::to_value(&rendered.ir).expect("IR serialises");
    assert!(json.get("blocks").is_some(), "document key changed: {json}");
    assert_eq!(json["blocks"][0]["kind"], "heading");
    assert!(json["blocks"][0].get("range").is_some());

    let round_trip: MarkdownDocument =
        serde_json::from_value(json.clone()).expect("IR reads its own wire form");
    assert_eq!(round_trip, rendered.ir);
    assert_eq!(
        serde_json::to_value(round_trip).expect("round-tripped IR serialises"),
        json
    );
}
