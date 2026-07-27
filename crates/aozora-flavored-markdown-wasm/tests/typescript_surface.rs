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
//!
//! Host-only, and not merely by habit: it reads its own source off the disk,
//! and on `wasm32-unknown-unknown` there is no disk to read it from. The wasm
//! half of this crate's suite is `tests/wasm.rs`.
//!
//! The last section reads the same source for a second census. The `.d.ts`
//! answers what the ABI *declares*; `tests/wasm.rs` answers what the ABI is
//! *run* through, and until `just test-wasm` existed the answer was four of
//! fourteen. Both questions are about a list nobody maintains by hand, and
//! both are answerable only from the source text — so they are asked in one
//! place, off one read.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

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

// ---------------------------------------------------------------------------
// every export the ABI declares is run on the target it crosses to
// ---------------------------------------------------------------------------
//
// The census above is of names. This one is of the functions behind them, and
// it is the check the coverage exclusion has always assumed: this crate is out
// of the llvm-cov denominator because `just test-wasm` runs it on wasm32
// instead, so "runs it" has to be a fact about the file rather than about the
// sentence. It was not, twice over — the step did not exist, and ten of the
// fourteen exports were named by no test on any target.
//
// A count would not have caught that and does not catch the next one: exports
// are added one at a time, and the failure is always the one nobody listed.
// The list is therefore taken from `src/lib.rs` itself, for the reason the
// class census is: `#[wasm_bindgen]` and its `js_name` live in an attribute,
// and an attribute is not something a `#[test]` can observe at runtime.

/// One `#[wasm_bindgen]` export: the Rust item a test calls, the name it
/// crosses the ABI under, and — for a method — the type it hangs off.
#[derive(Debug)]
struct Export {
    rust: String,
    js: String,
    owner: Option<String>,
    constructor: bool,
}

/// Every function this crate exports. A `pub fn` is one when it carries its
/// own `#[wasm_bindgen]`, and *also* when it merely sits in a `#[wasm_bindgen]
/// impl` — wasm-bindgen exports those under their Rust name whether or not
/// anybody wrote an attribute, which is exactly the export a census of
/// attributes would miss.
fn wasm_bindgen_exports(src: &str) -> Vec<Export> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut owner: Option<String> = None;
    for (idx, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix("impl ") {
            owner = attributes_above(&lines, idx)
                .iter()
                .any(|a| a.starts_with("#[wasm_bindgen"))
                .then(|| ident_at(rest));
            continue;
        }
        if *line == "}" {
            owner = None;
            continue;
        }
        let Some(tail) = line.trim_start().strip_prefix("pub fn ") else {
            continue;
        };
        let attributes = attributes_above(&lines, idx);
        let tagged = attributes.iter().any(|a| a.starts_with("#[wasm_bindgen"));
        if !tagged && owner.is_none() {
            continue;
        }
        let rust = ident_at(tail);
        let js = js_name_in(&attributes).unwrap_or_else(|| rust.clone());
        let constructor = attributes.iter().any(|a| a.contains("constructor"));
        out.push(Export {
            rust,
            js,
            owner: owner.clone(),
            constructor,
        });
    }
    out
}

fn wasm_harness() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wasm.rs");
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e}. This is the file `just test-wasm` runs and the file the coverage \
             exclusion defers to; without it the exclusion is a sentence again",
            path.display()
        )
    })
}

/// Everything the `#[wasm_bindgen_test]` functions execute. Only those: a call
/// from a helper nothing invokes, an export named in a doc comment, or a
/// plain `#[test]` the wasm runner does not collect are all mentions rather
/// than runs, and a census that counted them would re-open the hole it closes.
fn wasm_test_bodies(src: &str) -> String {
    let mut out = String::new();
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "#[wasm_bindgen_test]" {
            continue;
        }
        for body in lines.by_ref() {
            if body == "}" {
                break;
            }
            // The signature carries the test's own name, which is prose about
            // an export rather than a call to one.
            if !body.trim_start().starts_with("fn ") {
                out.push_str(without_line_comment(body));
                out.push('\n');
            }
        }
    }
    out
}

fn without_line_comment(line: &str) -> &str {
    line.split_once("//").map_or(line, |(before, _)| before)
}

/// Does this text call the export? A method is reached through a receiver and
/// a constructor through its type, because the bare identifier is not
/// distinctive: `new(` occurs in `Vec::new()`, and matching it would let the
/// one export nothing constructs pass on somebody else's allocation.
fn reached(text: &str, export: &Export) -> bool {
    let call = format!("{}(", export.rust);
    text.match_indices(&call).any(|(at, _)| {
        let before = &text[..at];
        match (&export.owner, export.constructor) {
            (Some(owner), true) => before.ends_with(&format!("{owner}::")),
            (Some(_), false) => before.ends_with('.'),
            (None, _) => before
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_'),
        }
    })
}

#[test]
fn every_export_this_crate_declares_is_called_by_a_wasm_test() {
    let exports = wasm_bindgen_exports(&wasm_source());
    assert!(
        exports.len() >= 14,
        "only {} exports found; the reader must be retargeted, not deleted. Found: {:?}",
        exports.len(),
        exports.iter().map(|e| &e.js).collect::<Vec<_>>()
    );

    let harness = wasm_harness();
    assert!(
        harness.contains("#![cfg(target_arch = \"wasm32\")]"),
        "`tests/wasm.rs` is the wasm half of this suite and has to be gated to wasm32; a \
         `#[wasm_bindgen_test]` compiled for the host is collected by nothing"
    );
    let bodies = wasm_test_bodies(&harness);

    let missed: Vec<&str> = exports
        .iter()
        .filter(|export| !reached(&bodies, export))
        .map(|export| export.js.as_str())
        .collect();
    assert!(
        missed.is_empty(),
        "{missed:?} cross the ABI and no `#[wasm_bindgen_test]` calls them. This crate is out of \
         the coverage denominator (`_COV_IGNORE`) precisely because `just test-wasm` is supposed \
         to be reaching it, so an export missing here is covered by nothing anywhere — which is \
         the state this file was written to end. Add a case to `tests/wasm.rs`"
    );
}

/// Every integration test this crate ships, with its source.
fn test_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let src = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            (name, src)
        })
        .collect()
}

#[test]
fn every_test_file_in_this_crate_declares_which_target_it_runs_on() {
    // This crate's suite is compiled twice — once by `just test`, once by
    // `just test-wasm` — and the two runners collect different attributes.
    // A `#[test]` built for wasm32 is a function wasm-bindgen's runner never
    // calls, and a `#[wasm_bindgen_test]` built for the host is the same
    // silence from the other side. Neither reports anything: the file is
    // compiled, the run is green, and nothing in it executed. So each file
    // says which half it belongs to, and this asserts that it said so.
    let files = test_files();
    assert!(
        files.len() >= 3,
        "only {} test file(s) found; the reader is looking in the wrong directory",
        files.len()
    );
    for (name, src) in files {
        let wasm = src.contains("#![cfg(target_arch = \"wasm32\")]");
        let host = src.contains("#![cfg(not(target_arch = \"wasm32\"))]");
        assert!(
            wasm ^ host,
            "`tests/{name}` declares neither target gate, or both. Exactly one of \
             `#![cfg(target_arch = \"wasm32\")]` and `#![cfg(not(target_arch = \"wasm32\"))]` \
             has to open the file, or its tests are collected by one runner and silently \
             dropped by the other"
        );
    }
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn a_method_with_no_attribute_of_its_own_is_still_an_export() {
    // The shape a census of `#[wasm_bindgen]` attributes would walk past:
    // inside a `#[wasm_bindgen] impl`, the attribute on the block is what
    // exports the method, and wasm-bindgen needs nothing on the method itself.
    let src = concat!(
        "#[wasm_bindgen]\n",
        "impl Handle {\n",
        "    pub fn bare(&self) -> u32 {\n",
        "        0\n",
        "    }\n",
        "}\n",
        "\n",
        "impl Handle {\n",
        "    pub fn internal(&self) -> u32 {\n",
        "        0\n",
        "    }\n",
        "}\n",
    );
    let exports = wasm_bindgen_exports(src);
    assert_eq!(
        exports.iter().map(|e| e.js.as_str()).collect::<Vec<_>>(),
        vec!["bare"],
        "an un-attributed method in a `#[wasm_bindgen] impl` is exported and one in a plain \
         `impl` is not"
    );
}

#[test]
fn a_mention_outside_a_wasm_test_is_not_a_call() {
    // Each of these is how the pre-`test-wasm` crate would have looked to a
    // laxer reader: the export appears in the file, and nothing runs it.
    let export = Export {
        rust: "slugs_json".to_owned(),
        js: "slugsJson".to_owned(),
        owner: None,
        constructor: false,
    };
    let harness = concat!(
        "#[test]\n",
        "fn a_plain_test_the_wasm_runner_never_collects() {\n",
        "    slugs_json();\n",
        "}\n",
        "\n",
        "#[wasm_bindgen_test]\n",
        "fn only_talks_about_it() {\n",
        "    // slugs_json() is what the completion menu reads.\n",
        "    assert!(true);\n",
        "}\n",
    );
    assert!(!reached(&wasm_test_bodies(harness), &export));

    let calling = concat!(
        "#[wasm_bindgen_test]\n",
        "fn calls_it() {\n",
        "    assert!(!slugs_json().is_empty());\n",
        "}\n",
    );
    assert!(reached(&wasm_test_bodies(calling), &export));
}

#[test]
fn a_constructor_is_not_reached_by_somebody_elses_new() {
    let export = Export {
        rust: "new".to_owned(),
        js: "new".to_owned(),
        owner: Some("AozoraDocument".to_owned()),
        constructor: true,
    };
    assert!(!reached("    let v: Vec<u8> = Vec::new();\n", &export));
    assert!(reached(
        "    let doc = AozoraDocument::new(src);\n",
        &export
    ));

    // A method is reached through a receiver, not by the bare name.
    let method = Export {
        rust: "pairs_json".to_owned(),
        js: "pairsJson".to_owned(),
        owner: Some("AozoraDocument".to_owned()),
        constructor: false,
    };
    assert!(!reached("    let pairs_json = 1;\n", &method));
    assert!(reached("    let out = doc.pairs_json();\n", &method));
}

#[test]
fn a_free_function_is_not_matched_inside_a_longer_name() {
    let export = Export {
        rust: "render".to_owned(),
        js: "render".to_owned(),
        owner: None,
        constructor: false,
    };
    assert!(!reached(
        "    let b = render_blocks(SOURCE, None);\n",
        &export
    ));
    assert!(!reached("    let b = pre_render(SOURCE);\n", &export));
    assert!(reached("    let r = render(SOURCE, None);\n", &export));
}
