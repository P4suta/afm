//! Regression tests for deeply-nested input (stack-overflow / `DoS`).
//!
//! comrak builds an arbitrarily deep AST from a *small* input — nested
//! blockquotes carry no depth cap (`handle_blockquote` in the vendored
//! tree), unlike list nesting which it caps at 100 — and aozora-flavored-markdown walks that
//! AST to splice Aozora spans and to project the IR. Before the splice
//! walk (`ast_splice`), the inline descent (`constructs`), and the
//! IR builder were made iterative / depth-bounded, a deeply nested
//! document overflowed the call stack. Under the release profile's
//! `panic = "abort"` that is a hard process abort — a crash on untrusted
//! input, which both repos' `SECURITY.md` scope IN as a vulnerability
//! (and which is fatal for any server-side library embedder).
//!
//! These tests pin that the public entry points *return* on pathological
//! nesting instead of crashing. Reaching the assertions at all (no abort)
//! is the core guarantee; the assertions additionally pin that the
//! innermost content still renders.

use aozora_flavored_markdown::{Options, render, render_blocks_to_ir, render_to_ir, serialize};

/// ~100k nested blockquotes on a single line, wrapping a leaf paragraph.
/// Pre-fix this overflowed `ast_splice::walk`'s recursion; the input is
/// only ~200 KB so parsing and the now-iterative walk stay fast.
fn deeply_nested_blockquotes() -> String {
    format!("{}deep", "> ".repeat(100_000))
}

#[test]
fn render_survives_deep_blockquote_nesting() {
    let out = render(&deeply_nested_blockquotes(), &Options::default());
    assert!(
        out.html.contains("deep"),
        "innermost content should still render"
    );
}

#[test]
fn render_to_ir_survives_deep_blockquote_nesting() {
    // Exercises the IR builder's depth guard (the `collect_blocks`
    // recursion over nested blockquotes).
    let rendered = render_to_ir(&deeply_nested_blockquotes(), &Options::default());
    assert!(rendered.html.contains("deep"));
}

#[test]
fn render_blocks_to_ir_survives_deep_blockquote_nesting() {
    let (blocks, _diagnostics) =
        render_blocks_to_ir(&deeply_nested_blockquotes(), &Options::default());
    assert!(!blocks.is_empty(), "the document should yield blocks");
}

#[test]
fn serialize_survives_deep_nesting() {
    // `serialize` runs the aozora linear serializer (not comrak), but
    // pin it too so the whole public surface is covered.
    let serialized = serialize(&deeply_nested_blockquotes());
    assert!(serialized.contains("deep"));
}

#[test]
fn deep_nesting_with_aozora_annotation_holds_tier_a() {
    // A page-break annotation buried under deep nesting must still not
    // leak a bare ［＃ into the output (Tier-A), and no PUA sentinel
    // (U+E001..E004) may survive into the HTML.
    let input = format!("{}［＃改ページ］", "> ".repeat(50_000));
    let out = render(&input, &Options::default());
    assert!(
        !out.html.contains('\u{E001}')
            && !out.html.contains('\u{E002}')
            && !out.html.contains('\u{E003}')
            && !out.html.contains('\u{E004}'),
        "a PUA sentinel leaked into HTML under deep nesting"
    );
}

#[test]
fn deeply_nested_lists_survive() {
    // List nesting is capped at 100 inside comrak, so this never reaches
    // the old overflow, but it exercises the `collect_list_items` ->
    // `collect_blocks` recursion path under the depth guard.
    let mut input = String::new();
    for i in 0..2_000_usize {
        for _ in 0..i.min(120) {
            input.push_str("  ");
        }
        input.push_str("- item\n");
    }
    let rendered = render(&input, &Options::default());
    let ir = render_to_ir(&input, &Options::default());
    assert!(rendered.html.contains("item"), "list items should render");
    assert!(!ir.html.is_empty(), "IR render should produce HTML");
}
