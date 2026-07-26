//! Source-fidelity properties for `aozora_flavored_markdown::serialize`.
//!
//! The serializer had exactly one property: I3, the fixed point
//! (`serialize(serialize(x)) == serialize(x)`), asserted by the
//! `serialize_round_trip` fuzz target and by `tests/fuzz_regressions.rs`.
//! I3 relates the output to *itself*, never to the input, so a rewrite that
//! is consistently wrong satisfies it — which is how a `serialize` that never
//! called the code-block mask canonicalised `｜青梅《おうめ》` to
//! `青梅《おうめ》` inside a fence without any gate noticing.
//!
//! I5 is the missing half: what the input said inside a fence, the output
//! must still say. Three shapes assert it, because each covers the others'
//! blind spots:
//!
//! * `*_survives_verbatim` build the construct themselves, so its bytes are
//!   known to the test and the assertion cannot be skipped by a carve-out.
//! * `mixed_*` hand whole documents — including the shared
//!   `commonmark_adversarial` pool, whose atoms have carried a fenced
//!   `｜青梅《おうめ》` all along without any property ever serializing one —
//!   to [`check_fence_fidelity`], the same predicate the fuzz target runs.
//! * [`assert_code_survives_verbatim`] asks comrak where the code is and
//!   compares each node's source slice against the output. It is the only one
//!   of the three that reaches a fence behind a `> ` or a list marker, an
//!   inline span, or an indented block: `check_fence_fidelity`'s scanner is
//!   column-anchored on purpose, because deciding a container prefix needs
//!   the block parser it must not become.

use aozora_flavored_markdown::{sentinels, serialize};
use aozora_flavored_markdown_test_support::check_fence_fidelity;
use aozora_flavored_markdown_test_support::config;
use aozora_flavored_markdown_test_support::generators::{aozora_fragment, commonmark_adversarial};
use comrak::nodes::{NodeValue, Sourcepos};
use core::iter::once;
use core::ops::RangeInclusive;
use proptest::prelude::*;

/// I3, I5 as the fuzz target reads it, and I5 over every form of code.
fn assert_serialize_invariants(src: &str) {
    let first = serialize(src);
    let second = serialize(&first);
    assert_eq!(
        first, second,
        "I3 fixed-point broken for src={src:?}\n  first  = {first:?}\n  second = {second:?}"
    );
    check_fence_fidelity(src, &first)
        .unwrap_or_else(|e| panic!("I5 (fence fidelity) violated for src={src:?}: {e}"));
    assert_code_survives_verbatim(src, &first);
}

/// Every code node comrak finds in `src`, sliced out of `src` by that node's
/// own sourcepos, appears byte for byte in `out`.
///
/// Deliberately located by the parser rather than by a scanner of this test's
/// own: what a container prefix, a lazy continuation or an info string does
/// to a fence is exactly the question a second scanner would get wrong, and
/// getting it wrong here reads as a `serialize` bug. The arithmetic below is
/// this file's own, so a library that stopped protecting a region it still
/// locates correctly is still caught.
fn assert_code_survives_verbatim(src: &str, out: &str) {
    for code in code_node_sources(src) {
        assert!(
            out.contains(code),
            "code {code:?} did not survive serialize for src={src:?}\n  out = {out:?}",
        );
    }
}

fn code_node_sources(src: &str) -> Vec<&str> {
    // The dialect's comrak side, spelled out: the library no longer hands its
    // comrak options to anyone, and this file locates code nodes by asking
    // comrak directly.
    let mut options = comrak::Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.render.hardbreaks = true;
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, src, &options);
    let line_starts: Vec<usize> = once(0)
        .chain(src.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    root.descendants()
        .filter_map(|node| {
            let data = node.data.borrow();
            matches!(data.value, NodeValue::Code(_) | NodeValue::CodeBlock(_))
                .then(|| source_slice(src, &line_starts, data.sourcepos))
                .flatten()
        })
        .collect()
}

/// comrak reports 1-based lines and byte columns, the end column inclusive.
/// A pair that does not slice the source, or slices only a line break, says
/// nothing about fidelity and is dropped.
fn source_slice<'a>(src: &'a str, line_starts: &[usize], pos: Sourcepos) -> Option<&'a str> {
    let offset = |line: usize, column: usize| {
        line_starts
            .get(line.checked_sub(1)?)
            .copied()?
            .checked_add(column)
    };
    let start = offset(pos.start.line, pos.start.column.saturating_sub(1))?;
    let end = offset(pos.end.line, pos.end.column)?;
    let text = src.get(start..end)?.trim_end_matches(['\n', '\r']);
    (!text.is_empty()).then_some(text)
}

/// Notation the canonicaliser demonstrably rewrites when it is *not* lifted:
/// a ruby loses its explicit base marker, a block construct gains blank lines
/// around it, and an indent's full-width digit is normalised to ASCII — code
/// silently corrupted, not merely markup. One is planted in every payload, so
/// the property below cannot pass by drawing inert text.
fn canonicalising_notation() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("｜青梅《おうめ》".to_owned()),
        Just("｜漢字《かんじ》".to_owned()),
        Just("［＃改ページ］".to_owned()),
        Just("［＃ここから２字下げ］".to_owned()),
    ]
}

/// Line structure, the half of a payload a character mask cannot reach: a
/// decorative rule row (the canonicaliser isolates one with a blank line), a
/// run of three or more newlines (it collapses one to two), and a CRLF break
/// (it rewrites one to LF). Each was a `check_fence_fidelity` carve-out, and
/// each is now planted in every payload so no draw can miss all three.
fn line_structure() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("\n\n".to_owned()),
        Just("\r\n\r\n".to_owned()),
        Just("------------\n".to_owned()),
        Just("============\n".to_owned()),
        Just("____________\n".to_owned()),
    ]
}

/// A payload that can sit inside a fence without changing its shape. Only a
/// fence marker is excluded, because one would close the fence early; the
/// line structure is carried through by lifting the region out whole, and
/// drawing it is the point. It never ends on a blank line, which an indented
/// block — the same payload, four spaces in — would not count as its own.
fn multiline_payload() -> impl Strategy<Value = String> {
    (
        canonicalising_notation(),
        line_structure(),
        aozora_fragment(8),
        canonicalising_notation(),
    )
        .prop_map(|(planted, structure, drawn, tail)| {
            format!("{planted}\n{structure}{drawn}\n{tail}").replace(['`', '~'], "")
        })
}

/// One line, since a code span cannot hold a blank one.
fn inline_payload() -> impl Strategy<Value = String> {
    (canonicalising_notation(), aozora_fragment(4)).prop_map(|(planted, drawn)| {
        let mut out = format!("{planted}{drawn}");
        out.retain(|c| !matches!(c, '`' | '\r' | '\n'));
        out
    })
}

/// One draw of surrounding prose. Both grammars, because the bug only shows
/// where the two meet: notation the lexer *must* rewrite outside the fence and
/// must not rewrite inside it, in one document.
fn prose() -> impl Strategy<Value = String> {
    prop_oneof![aozora_fragment(6), commonmark_adversarial()]
}

/// `(first line, continuation)` prefixes. The empty pair keeps the top-level
/// case in the draw; the rest are the container depths the mask never saw,
/// because it matched a fence by column.
fn container() -> impl Strategy<Value = (&'static str, &'static str)> {
    prop::sample::select(vec![
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("1. ", "   "),
        ("> > ", "> > "),
    ])
}

fn prefix_lines(body: &str, first: &str, rest: &str) -> String {
    body.split('\n')
        .enumerate()
        .map(|(idx, line)| format!("{}{line}", if idx == 0 { first } else { rest }))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A document with one fence, at a drawn container depth, whose interior the
/// test knows exactly.
fn fenced_document() -> impl Strategy<Value = (String, String)> {
    (prose(), multiline_payload(), prose(), container()).prop_map(
        |(before, payload, after, (first, rest))| {
            let block = prefix_lines(&format!("```\n{payload}\n```"), first, rest);
            let doc = format!("{before}\n\n{block}\n\n{after}\n");
            (doc, prefix_lines(&payload, rest, rest))
        },
    )
}

/// The same, for an inline code span — which the mask never covered at all.
fn inline_code_document() -> impl Strategy<Value = (String, String)> {
    (prose(), inline_payload(), container()).prop_map(|(before, payload, (first, rest))| {
        let span = format!("前 `{payload}` 後");
        let doc = format!("{before}\n\n{}\n", prefix_lines(&span, first, rest));
        (doc, format!("`{payload}`"))
    })
}

/// The same, for an indented code block (CommonMark §4.4). Prose is drawn
/// from the Aozora pool only: a list or a blockquote in front would claim the
/// four spaces as its own continuation, and the block under test would not be
/// code at all.
fn indented_code_document() -> impl Strategy<Value = (String, String)> {
    (aozora_fragment(6), multiline_payload()).prop_map(|(before, payload)| {
        let block = prefix_lines(&payload, "    ", "    ");
        (format!("{before}\n\n{block}\n"), block)
    })
}

/// Block leaves that are CommonMark and *only* CommonMark, one per line-owning
/// construct, none of them carrying a blank line — so a concatenation of them
/// is a document `serialize` owes back byte for byte, with no run of blank
/// lines to collapse and no notation to canonicalise.
const PLAIN_BLOCKS: &[&str] = &[
    "aaa\n",
    "- item\n",
    "1. item\n",
    "> quote\n",
    "# heading\n",
    "| a | b |\n| - | - |\n| c | d |\n",
    "[a]: /url\n",
    "<div>\nraw\n</div>\n",
    "```\ncode\n```\n",
    "    indented\n",
    "***\n",
    "`span`\n",
];

fn plain_blocks(count: RangeInclusive<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PLAIN_BLOCKS), count)
        .prop_map(|blocks| blocks.concat())
}

/// A run of one `-`, `=` or `_`, at any width either grammar cares about and
/// several neither does. Deliberately unbounded by the sibling parser's
/// ten-character threshold: naming that constant here would pin this crate to
/// one that lives in the other parser, and the width is exactly what decides
/// whether the two grammars agree about the row.
fn rule_row() -> impl Strategy<Value = String> {
    (prop::sample::select(vec!['-', '=', '_']), 1usize..=40)
        .prop_map(|(rule, width)| String::from(rule).repeat(width))
}

/// One rule row, somewhere in a pure-CommonMark document, at a drawn
/// container depth. The row's neighbours are what decide which block owns it
/// — a paragraph above makes it a setext underline, a list marker makes it a
/// lazy continuation, a table above makes it a row — so drawing them is the
/// point.
fn rule_row_document() -> impl Strategy<Value = String> {
    (
        plain_blocks(0..=2),
        rule_row(),
        plain_blocks(0..=2),
        container(),
    )
        .prop_map(|(before, row, after, (first, rest))| {
            contained(&format!("{before}{row}\n{after}"), first, rest)
        })
}

/// `prefix_lines` over a document's own lines: the trailing break is put back
/// afterwards rather than prefixed, since a container prefix on the empty
/// line after it would be trailing whitespace the author never wrote.
fn contained(body: &str, first: &str, rest: &str) -> String {
    format!(
        "{}\n",
        prefix_lines(body.trim_end_matches('\n'), first, rest)
    )
}

/// One codepoint this crate reserves, in a pure-CommonMark document at a
/// drawn container depth. Four of the five are rewritten to `U+FFFD` by the
/// sibling parser on sight, so an unprotected one is the author's byte
/// destroyed rather than merely markup moved.
fn reserved_codepoint_document() -> impl Strategy<Value = String> {
    (
        plain_blocks(0..=2),
        prop::sample::select(sentinels::ALL.to_vec()),
        plain_blocks(0..=2),
        container(),
    )
        .prop_map(|(before, reserved, after, (first, rest))| {
            contained(&format!("{before}a{reserved}b\n{after}"), first, rest)
        })
}

/// The same codepoint amid prose of *both* grammars. Neither shared pool
/// contains one of its own — which is why the count invariant below was
/// vacuous for four of the five until this strategy planted them.
fn reserved_codepoint_in_prose() -> impl Strategy<Value = String> {
    (
        prose(),
        prop::sample::select(sentinels::ALL.to_vec()),
        prose(),
    )
        .prop_map(|(before, reserved, after)| format!("{before}\n{reserved}\n{after}"))
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

/// Every form of code CommonMark has, in one document, each carrying notation
/// the canonicaliser rewrites everywhere else. The indented block comes right
/// after a paragraph on purpose: behind a list marker the same four spaces are
/// the item's own continuation, and prose is what they would rightly be.
const CODE_IN_EVERY_FORM: &str = r"# 見出し

`｜青梅《おうめ》` in a span, ｜青梅《おうめ》 outside

```
｜青梅《おうめ》
------------


［＃改ページ］
```

段落

    ｜青梅《おうめ》
    ［＃改ページ］

> ```
> ｜青梅《おうめ》
> ```

- ```
  ｜青梅《おうめ》
  ```

~~~
｜青梅《おうめ》
~~~
";

#[test]
fn fenced_ruby_is_not_canonicalised() {
    // The acceptance case: the ruby's explicit base marker is dropped
    // everywhere else, and must survive here.
    let src = "```\n｜青梅《おうめ》\n```";
    assert_eq!(serialize(src), src);
    assert_serialize_invariants(src);
}

#[test]
fn the_same_notation_is_rewritten_outside_the_fence_and_kept_inside() {
    let src = "｜青梅《おうめ》\n\n```\n｜青梅《おうめ》\n```\n";
    assert_eq!(
        serialize(src),
        "青梅《おうめ》\n\n```\n｜青梅《おうめ》\n```\n"
    );
    assert_serialize_invariants(src);
}

#[test]
fn every_shape_of_canonicalisation_stops_at_the_fence() {
    // One case per rewrite an unmasked lexer applies to a fence body, since
    // the dropped ruby marker of the report is only the visible one: a block
    // construct also gets separated by blank lines, and a full-width digit
    // normalised to ASCII — a silent corruption of code, not just of markup.
    for src in [
        "```\n｜青梅《おうめ》\n```\n",
        "```\n［＃改ページ］\n```\n",
        "```\n［＃ここから２字下げ］\n```\n",
    ] {
        assert_eq!(serialize(src), src, "fence body rewritten: {src:?}");
    }
}

#[test]
fn tilde_and_wide_fences_are_masked_too() {
    for src in [
        "~~~\n｜青梅《おうめ》\n~~~\n",
        "````\n｜青梅《おうめ》\n````\n",
        "```rust\n// ｜青梅《おうめ》\n```\n",
        "  ```\n  ｜青梅《おうめ》\n  ```\n",
    ] {
        assert_eq!(serialize(src), src, "fence not masked: {src:?}");
        assert_serialize_invariants(src);
    }
}

#[test]
fn line_structure_inside_a_fence_survives() {
    // The two rewrites a character mask structurally cannot reach: a
    // decorative rule row gains a blank line ahead of it, and a run of three
    // or more newlines collapses to two. Both were `check_fence_fidelity`
    // carve-outs — a claim that this input was allowed to come back corrupted.
    for src in [
        "```\na\n------------\n```\n",
        "```\na\n============\n```\n",
        "```\na\n____________\n```\n",
        "```\na\n\n\nb\n```\n",
        "```\na\n\n\n\n\nb\n```\n",
    ] {
        assert_eq!(serialize(src), src, "fence body rewritten: {src:?}");
        assert_serialize_invariants(src);
    }
}

#[test]
fn a_crlf_fence_interior_survives_though_the_document_is_normalised_to_lf() {
    // The third carve-out. CRLF is normalised document-wide by the parser —
    // the closing fence's own break arrives as LF — but the interior is the
    // author's byte and comes back as written.
    let src = "```\r\n｜青梅《おうめ》\r\n```\r\n";
    assert_eq!(serialize(src), "```\r\n｜青梅《おうめ》\r\n```\n");
    assert_serialize_invariants(src);
}

#[test]
fn a_reserved_codepoint_in_the_source_does_not_expose_the_fence() {
    // The fourth carve-out: U+E000 is the codepoint the protection itself
    // uses. A source already carrying one used to abort masking outright,
    // leaving the fence to be canonicalised as prose.
    //
    // Widened from U+E000 alone to the whole reserved set, which is what
    // DEV-232 found the family to be: the other four are rewritten to U+FFFD
    // by the sibling parser on sight, so a source carrying one was not merely
    // losing its fence's protection, it was losing the codepoint. Read off
    // `sentinels::ALL` so one added later is covered without editing this.
    for reserved in sentinels::ALL {
        let src = format!("{reserved}\n```\n｜青梅《おうめ》\n```\n");
        assert_eq!(
            serialize(&src),
            src,
            "reserved codepoint or fence rewritten"
        );
        assert_serialize_invariants(&src);
    }
}

#[test]
fn a_container_nested_fence_survives_where_the_scanner_cannot_look() {
    // Bug 3: the mask matched a fence by column, so a `> ` or a list marker
    // in front of one hid it completely. `check_fence_fidelity` still cannot
    // read these — telling a container prefix from a lazy continuation needs
    // the block parser it must not become — so they are pinned by hand here,
    // and by `assert_code_survives_verbatim` for drawn documents.
    for src in [
        "> ```\n> ｜青梅《おうめ》\n> ```\n",
        "- ```\n  ｜青梅《おうめ》\n  ```\n",
        "1. ```\n   ｜青梅《おうめ》\n   ```\n",
        "> > ```\n> > ［＃改ページ］\n> > ```\n",
        "> ```\n> a\n> \n> \n> b\n> ------------\n> ```\n",
    ] {
        assert_eq!(serialize(src), src, "container-nested fence rewritten");
        assert_serialize_invariants(src);
    }
}

#[test]
fn an_inline_code_span_and_an_indented_block_survive() {
    // Neither was ever masked. The span case also pins the asymmetry: the
    // same notation outside the backticks *must* be rewritten.
    assert_eq!(
        serialize("`｜青梅《おうめ》` outside ｜青梅《おうめ》\n"),
        "`｜青梅《おうめ》` outside 青梅《おうめ》\n"
    );
    for src in [
        "text\n\n    ｜青梅《おうめ》\n    ［＃改ページ］\n",
        "text\n\n    a\n    ------------\n    \n    \n    b\n",
        "`｜青梅《おうめ》`\n",
        "> `｜青梅《おうめ》`\n",
    ] {
        assert_eq!(serialize(src), src, "code rewritten: {src:?}");
        assert_serialize_invariants(src);
    }
}

#[test]
fn code_in_every_form_survives_one_document() {
    // The equivalence stated directly: comrak locates the regions the
    // implementation protects, so every node it finds must reappear whole.
    let out = serialize(CODE_IN_EVERY_FORM);
    assert_code_survives_verbatim(CODE_IN_EVERY_FORM, &out);
    // …and again without comrak, so a walk that silently found nothing
    // cannot pass this test: the exact bytes, listed.
    for expected in [
        "`｜青梅《おうめ》`",
        "```\n｜青梅《おうめ》\n------------\n\n\n［＃改ページ］\n```",
        "> ```\n> ｜青梅《おうめ》\n> ```",
        "- ```\n  ｜青梅《おうめ》\n  ```",
        "    ｜青梅《おうめ》\n    ［＃改ページ］",
        "~~~\n｜青梅《おうめ》\n~~~",
    ] {
        assert!(out.contains(expected), "{expected:?} lost\n  out = {out:?}");
    }
    // The prose between them is still canonicalised — the point of the pass.
    assert!(
        out.contains("in a span, 青梅《おうめ》 outside"),
        "prose outside code must still be canonicalised\n  out = {out:?}"
    );
    assert_serialize_invariants(CODE_IN_EVERY_FORM);
}

#[test]
fn a_document_whose_block_structure_moves_between_passes_settles() {
    // A canonicalising pass inserts blank lines, which changes the block
    // structure the *next* pass would protect by. Left unchecked the two
    // passes disagree and I3 breaks; the rule row is protected, and the
    // passes are iterated until the text settles.
    for src in [
        "］ \n============\r\n<em>\r\n］``#",
        "a\n============\n    ｜青梅《おうめ》\n",
        "本文\n----------\n",
        "本文\r\n----------\r\n",
    ] {
        assert_serialize_invariants(src);
    }
    // A setext underline is the one rule row CommonMark owns, so it stays
    // where the author put it rather than being isolated by a blank line.
    assert_eq!(serialize("本文\n----------\n"), "本文\n----------\n");
    assert_eq!(
        serialize("a\n============\n    ｜青梅《おうめ》\n"),
        "a\n============\n    ｜青梅《おうめ》\n"
    );
}

#[test]
fn a_rule_row_is_held_wherever_commonmark_put_it_not_only_under_a_paragraph() {
    // A3 (#168) read the family as "the rule row comrak reports as a node",
    // which is a thematic break or a setext underline. DEV-232 measured that
    // it is not: the sibling parser isolates any row of `-`/`=`/`_`, and a
    // `=`-run is not a thematic break at all, so it arrives as a paragraph's
    // own text, a lazy continuation, or a table row. Every line below round-
    // tripped wrong before the fix, and all but the last changed the rendered
    // HTML outright — the list split, the blockquote split, the last row fell
    // out of the table, a paragraph became a paragraph plus an indented code
    // block.
    //
    // The exhaustive matrix (three rule characters × five widths × eighteen
    // block contexts) lives in `tests/serialize_commonmark_identity.rs`;
    // these are the shapes worth naming in the file that owns the invariant.
    for src in [
        "- aaa\n==========\n",
        "> aaa\n==========\n",
        "| a |\n| - |\n| b |\n==========\n",
        "aaa\n    ----------\n",
        "aaa\n    __________\n",
        "[a]: /url\n----------\n",
        "# h\n==========\n",
    ] {
        assert_eq!(serialize(src), src, "rule row rewritten: {src:?}");
        assert_serialize_invariants(src);
    }
}

#[test]
fn plain_commonmark_passes_through_verbatim() {
    // `serialize`'s rustdoc has claimed this all along, of neither the
    // pre-A2 nor the post-A2 behaviour — which is what makes it worth a test
    // rather than a sentence. Every document here is CommonMark and nothing
    // else, so the canonicaliser owes it back unchanged.
    for src in [
        "# heading\n\n- item\n  > quote in list\n    1. nested\n",
        "> outer\n> > inner\n> > > deepest\n",
        "- loose\n\n- items\n\n- here\n",
        "- tight\n- items\n- here\n",
        "\\*escaped\\* and \\[not a link\\]\n",
        "```rust\nlet x = 1;\n```\n",
        "```\ncode\n\n\nmore\n```\n",
        "> ```\n> code\n> ```\n",
        "| h1 | h2 |\n| -- | -- |\n| a  | b  |\n",
        "[link](url) and ![img](src)\n",
        "***\n\nthematic\n\n***\n",
        "  <em>inline HTML</em>\n",
        "<div>\nraw\n</div>\n",
        "trailing   spaces  \nhard break\n",
        "Heading\n============\n\nbody\n",
        "1. one\n2. two\n\n   para\n",
        "- item\n\n      indented code in a list\n",
        // DEV-232: a rule row CommonMark did *not* claim as a break, and a
        // codepoint this crate reserves. Both are plain CommonMark, and both
        // came back rewritten until the delegate was taught to lift them.
        "- aaa\n==========\n",
        "aaa\n    ----------\n",
        "a\u{E001}b\n",
        "> a\u{E004}b\n",
    ] {
        assert_eq!(serialize(src), src, "plain CommonMark rewritten: {src:?}");
    }
}

#[test]
fn the_rustdoc_names_the_only_two_normalisations() {
    // The bounded half of the same claim. Both are the sibling parser's line
    // form, applied document-wide, and CommonMark distinguishes neither — so
    // they are named in the doc instead of softening what it promises. Inside
    // code both are held byte for byte, which the fence cases above pin.
    assert_eq!(serialize("a\r\nb\n"), "a\nb\n");
    assert_eq!(serialize("a\n\n\n\nb\n"), "a\n\nb\n");
}

#[test]
fn a_fence_bearing_document_from_the_shared_pool_holds() {
    // The atom that has been in `commonmark_adversarial` since the pool was
    // written, never once handed to `serialize`.
    assert_serialize_invariants("```\n｜青梅《おうめ》\n［＃改ページ］\n```\n");
}

proptest! {
    #![proptest_config(config::default())]

    /// The interior the generator put in comes back out byte for byte. Known
    /// to the test rather than rediscovered by a scanner, so no carve-out can
    /// quietly turn this property into a no-op — and so a fence behind a
    /// container prefix is asserted as strictly as one in column 1.
    #[test]
    fn a_fenced_payload_survives_verbatim((src, payload) in fenced_document()) {
        let out = serialize(&src);
        prop_assert!(
            out.contains(&payload),
            "fenced payload {payload:?} lost from src={src:?}\n  out = {out:?}",
        );
    }

    /// …and the surrounding document still satisfies I3 and I5.
    #[test]
    fn a_fenced_document_satisfies_both_serialize_invariants((src, _) in fenced_document()) {
        assert_serialize_invariants(&src);
    }

    /// An inline code span, at every container depth.
    #[test]
    fn an_inline_code_span_survives_verbatim((src, payload) in inline_code_document()) {
        let out = serialize(&src);
        prop_assert!(
            out.contains(&payload),
            "code span {payload:?} lost from src={src:?}\n  out = {out:?}",
        );
        assert_serialize_invariants(&src);
    }

    /// An indented code block, whose boundaries the mask declined to compute.
    #[test]
    fn an_indented_block_survives_verbatim((src, payload) in indented_code_document()) {
        let out = serialize(&src);
        prop_assert!(
            out.contains(&payload),
            "indented block {payload:?} lost from src={src:?}\n  out = {out:?}",
        );
        assert_serialize_invariants(&src);
    }

    /// Whole documents from the shared pools, scanned for fences the way the
    /// fuzz target scans arbitrary bytes.
    #[test]
    fn mixed_documents_satisfy_both_serialize_invariants(src in prose()) {
        assert_serialize_invariants(&src);
    }

    /// I7 — a document that is CommonMark and nothing else comes back byte
    /// for byte, however the rule row inside it is read.
    ///
    /// The half of the README's superset claim no property had: I3 relates
    /// the output to itself and I5 only to the code regions, so a delegate
    /// that inserted a blank line in front of every `----------` satisfied
    /// both while splitting the list, blockquote or table that owned it.
    #[test]
    fn a_rule_row_bearing_commonmark_document_is_returned_verbatim(src in rule_row_document()) {
        prop_assert_eq!(&serialize(&src), &src, "rule row document rewritten");
        assert_serialize_invariants(&src);
    }

    /// The same, for the codepoints this crate reserves — which the sibling
    /// parser overwrites with `U+FFFD` rather than merely moving.
    #[test]
    fn a_reserved_codepoint_bearing_document_is_returned_verbatim(
        src in reserved_codepoint_document()
    ) {
        prop_assert_eq!(&serialize(&src), &src, "reserved codepoint document rewritten");
        assert_serialize_invariants(&src);
    }

    /// I8 — every reserved codepoint the author typed is still there, and no
    /// new one appeared. Stated over *any* draw rather than pure CommonMark
    /// only, because the count is preserved by lifting the codepoint out
    /// whole, which does not care what grammar surrounds it. This is the
    /// shape the fuzz target carries.
    #[test]
    fn the_reserved_codepoints_of_the_source_are_neither_lost_nor_invented(
        src in reserved_codepoint_in_prose()
    ) {
        let out = serialize(&src);
        for reserved in sentinels::ALL {
            prop_assert_eq!(
                src.matches(reserved).count(),
                out.matches(reserved).count(),
                "reserved U+{:04X} count moved for src={:?}\n  out = {:?}",
                reserved as u32, src, out,
            );
        }
    }
}
