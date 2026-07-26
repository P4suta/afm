//! The generated `.d.ts`, checked as a namespace rather than as a compile.
//!
//! `tsify` hands wasm-pack one declaration per type through
//! [`tsify::Tsify::DECL`], and wasm-bindgen hands it one per exported class;
//! wasm-pack concatenates them into a single `.d.ts` module. Nothing before
//! this file read the result:
//!
//! * `just wasm-build` only has to produce a file.
//! * `just playground-build` runs `tsc --noEmit` over it, and TypeScript
//!   *merges* an `interface` into a `class` of the same name rather than
//!   rejecting it. Two declarations of one name therefore type-check, and the
//!   consumer silently gets the union of two unrelated shapes.
//! * `just ci` is the only gate that runs wasm-pack at all, and it runs it
//!   last.
//!
//! So the collision this crate had to navigate — `ir::Document` and the
//! `Document` handle both claiming `Document` — is invisible to every gate
//! that exists. It is checked here instead, natively: `DECL` is an ordinary
//! `const &'static str`, so the declarations are readable from a plain
//! `cargo test` without wasm-pack, a browser or a `pkg/` directory.
//!
//! The class names come off this crate's own source text for the same reason
//! the sealed half of `public_type_contract` does: `#[wasm_bindgen]` renames
//! happen in an attribute, and an attribute is not something a `#[test]` can
//! observe at runtime.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use aozora_flavored_markdown::ir::{
    Block, Document, Inline, ListItem, Position, Range, Span, TableAlign, TableRow,
};
use aozora_flavored_markdown::{Diagnostic, DiagnosticSource, Options, Severity, render_to_ir};
use aozora_flavored_markdown_wasm::{BlockResult, BlocksResult, RenderResult};
use serde_json::Value;
use tsify::Tsify;

// ---------------------------------------------------------------------------
// the declarations wasm-pack concatenates
// ---------------------------------------------------------------------------

/// Every `Tsify` declaration that reaches the emitted `.d.ts`.
///
/// A hand-written list, but not a list anyone has to remember to extend:
/// `every_type_a_declaration_names_is_declared_beside_it` fails the moment a
/// declaration references a type this list does not carry, and a type only
/// reaches the `.d.ts` by being referenced.
fn tsify_declarations() -> Vec<&'static str> {
    vec![
        <RenderResult as Tsify>::DECL,
        <Options as Tsify>::DECL,
        <BlockResult as Tsify>::DECL,
        <BlocksResult as Tsify>::DECL,
        <Document as Tsify>::DECL,
        <Block as Tsify>::DECL,
        <Inline as Tsify>::DECL,
        <ListItem as Tsify>::DECL,
        <TableRow as Tsify>::DECL,
        <TableAlign as Tsify>::DECL,
        <Range as Tsify>::DECL,
        <Position as Tsify>::DECL,
        <Span as Tsify>::DECL,
        <Diagnostic as Tsify>::DECL,
        <Severity as Tsify>::DECL,
        <DiagnosticSource as Tsify>::DECL,
    ]
}

/// Rustdoc travels into `DECL` as a `/** … */` block, and prose is full of
/// capitalised words that are not types.
fn without_comments(decl: &str) -> String {
    let mut out = String::new();
    let mut rest = decl;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        match after.find("*/") {
            Some(close) => rest = &after[close + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The identifier a declaration claims in the module's type namespace.
fn declared_name(decl: &str) -> String {
    let body = without_comments(decl);
    let after_export = body
        .split_once("export ")
        .unwrap_or_else(|| panic!("a tsify declaration must export something: {body}"))
        .1;
    let tail = after_export
        .strip_prefix("interface ")
        .or_else(|| after_export.strip_prefix("type "))
        .unwrap_or_else(|| panic!("unrecognised declaration form: {after_export}"));
    ident_at(tail)
}

fn ident_at(text: &str) -> String {
    text.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Every capitalised identifier a declaration body names, which for TypeScript
/// is every type it depends on.
fn referenced_names(decl: &str) -> BTreeSet<String> {
    let body = without_quoted(&without_comments(decl));
    body.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| word.starts_with(char::is_uppercase))
        .map(ToOwned::to_owned)
        .collect()
}

/// String literals are the union discriminants, not type references.
fn without_quoted(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in text.chars() {
        if ch == '"' {
            inside = !inside;
        } else if !inside {
            out.push(ch);
        }
    }
    out
}

/// The `kind: "…"` discriminants of an internally tagged union declaration.
fn union_tags(decl: &str) -> BTreeSet<String> {
    let body = without_comments(decl);
    body.split("kind: \"")
        .skip(1)
        .filter_map(|tail| tail.split_once('"'))
        .map(|(tag, _)| tag.to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// the classes wasm-bindgen declares alongside them
// ---------------------------------------------------------------------------

fn wasm_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    fs::read_to_string(&path).expect("this crate's own source must be readable")
}

/// A `#[wasm_bindgen]` type becomes an `export class`, which occupies the
/// *type* namespace as well as the value one — so it collides with an
/// `interface` of the same name instead of coexisting with it.
fn wasm_bindgen_class_names(src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let Some(rest) = line
            .strip_prefix("pub struct ")
            .or_else(|| line.strip_prefix("pub enum "))
        else {
            continue;
        };
        let attributes = attributes_above(&lines, idx);
        if !attributes.iter().any(|a| a.starts_with("#[wasm_bindgen")) {
            continue;
        }
        out.push(js_name_in(&attributes).unwrap_or_else(|| ident_at(rest)));
    }
    out
}

/// The attribute block directly above a declaration, which rustfmt keeps
/// unbroken and this workspace separates from the item before it with a blank
/// line.
fn attributes_above<'a>(lines: &[&'a str], decl: usize) -> Vec<&'a str> {
    lines[..decl]
        .iter()
        .rev()
        .take_while(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .collect()
}

/// `#[wasm_bindgen(js_name = Foo)]` renames the emitted class, so the Rust
/// identifier is not always the one in the `.d.ts`.
fn js_name_in(attributes: &[&str]) -> Option<String> {
    attributes
        .iter()
        .find_map(|a| a.split_once("js_name = "))
        .map(|(_, tail)| ident_at(tail))
}

/// Everything the emitted module declares in its type namespace.
fn declared_type_names() -> Vec<String> {
    let src = wasm_source();
    let mut names: Vec<String> = tsify_declarations()
        .iter()
        .map(|decl| declared_name(decl))
        .collect();
    names.extend(wasm_bindgen_class_names(&src));
    names
}

// ---------------------------------------------------------------------------
// the checks
// ---------------------------------------------------------------------------

#[test]
fn no_two_declarations_in_the_generated_dts_share_a_name() {
    let names = declared_type_names();
    assert!(
        names.len() > 15,
        "only {} declarations found; the reader must be retargeted, not deleted",
        names.len()
    );
    let mut sorted = names.clone();
    sorted.sort();
    let duplicates: Vec<&String> = sorted
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| &w[0])
        .collect();
    assert!(
        duplicates.is_empty(),
        "the emitted `.d.ts` declares {duplicates:?} twice. TypeScript merges a duplicate \
         `interface` into a `class` instead of rejecting it, so `tsc` stays green and the \
         consumer gets the union of two unrelated shapes. Rename on the wasm side — the ABI \
         layer — rather than letting TypeScript pick a Rust name. Declared: {names:?}"
    );
}

/// Names `lib.dom.d.ts` also declares. A module-scoped declaration shadows the
/// global for the whole file, so anything wasm-bindgen later emits meaning the
/// DOM type binds to ours instead — silently, because the shapes are unrelated
/// but the name resolves.
const DOM_GLOBALS: &[&str] = &[
    "Document",
    "Range",
    "Text",
    "Comment",
    "Node",
    "Element",
    "Event",
    "Image",
    "Selection",
    "Location",
    "History",
    "Screen",
    "Storage",
    "Option",
    "Attr",
];

/// The shadows this package ships today, both of them older than the IR
/// rename: `Range` has been a `tsify` declaration since the IR carried source
/// positions, and `Document` was an `export class` before it was the IR root.
/// Held as a list so a *third* one is a decision somebody makes on purpose.
const ACCEPTED_DOM_SHADOWS: &[&str] = &["Document", "Range"];

#[test]
fn the_dts_shadows_no_dom_global_beyond_the_two_already_accepted() {
    let shadows: BTreeSet<String> = declared_type_names()
        .into_iter()
        .filter(|name| DOM_GLOBALS.contains(&name.as_str()))
        .collect();
    let accepted: BTreeSet<String> = ACCEPTED_DOM_SHADOWS.iter().map(|&s| s.to_owned()).collect();
    assert_eq!(
        shadows, accepted,
        "the emitted `.d.ts` shadows a DOM global that has not been accepted. A consumer can \
         alias on import, but a signature wasm-bindgen emits inside this same file cannot — it \
         would bind to ours"
    );
}

#[test]
fn every_type_a_declaration_names_is_declared_beside_it() {
    let declared: BTreeSet<String> = declared_type_names().into_iter().collect();
    for decl in tsify_declarations() {
        for referenced in referenced_names(decl) {
            assert!(
                declared.contains(&referenced),
                "`{referenced}` is named by a declaration but declared by none of them, so it \
                 reaches the `.d.ts` unchecked by the two tests above. Add it to \
                 `tsify_declarations`. Declaration: {decl}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// the TypeScript union and the JSON must describe one wire format
// ---------------------------------------------------------------------------

/// Reaches every `Block` and `Inline` variant: heading, paragraph, the four
/// inline emphasis shapes, an image, a soft break, a list, a table, a
/// blockquote, a fence, a rule, and 青空文庫 notation at both levels.
const EVERY_VARIANT: &str = concat!(
    "# ｜見出し《みだし》\n\n",
    "本文 **強調** *斜体* `code` [link](https://example.com \"t\") ",
    "![alt](https://example.com/a.png \"u\")\n続き\n\n",
    "- item\n- ｜漢字《かんじ》\n\n",
    "> quoted\n\n",
    "| a | b | c |\n|---|:--:|--:|\n| 1 | 2 | 3 |\n\n",
    "---\n\n",
    "```rust\nfn main() {}\n```\n\n",
    "［＃ここから字下げ］\n字下げ本文\n［＃ここで字下げ終わり］\n",
);

fn tags_in(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(tag)) = map.get("kind") {
                out.insert(tag.clone());
            }
            for child in map.values() {
                tags_in(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                tags_in(child, out);
            }
        }
        _ => {}
    }
}

#[test]
fn the_ts_union_members_are_exactly_the_tags_the_renderer_emits() {
    let ir = render_to_ir(EVERY_VARIANT, &Options::default()).ir;
    let json = serde_json::to_value(ir).expect("the IR must serialise");
    let mut emitted = BTreeSet::new();
    tags_in(&json, &mut emitted);

    let mut declared = union_tags(<Block as Tsify>::DECL);
    declared.extend(union_tags(<Inline as Tsify>::DECL));

    assert_eq!(
        emitted, declared,
        "the `kind` values `render_to_ir` emits and the union members `tsify` declares are not \
         the same alphabet. `serde` and `tsify` read the same attributes down different code \
         paths — `tsify` ignores a container-level `#[serde(rename)]`, for one — so a variant \
         renamed on one side and not the other type-checks and then misses every `switch` arm \
         at runtime. If a variant was added, extend `EVERY_VARIANT` until it appears"
    );
}
