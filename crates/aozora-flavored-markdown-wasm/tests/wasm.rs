//! Every export, run on the target it ships to.
//!
//! This is the step `_COV_IGNORE` has always deferred to. The exclusion that
//! keeps this crate out of the coverage denominator said its surface was
//! "exercised by `wasm-pack test`" — and no such step existed anywhere in the
//! repo, so ten of the fourteen exports were reached by nothing at all:
//! `initPanicHook`, `slugsJson`, and all eight of the `AozoraDocument`
//! editor-assist methods. `tests/native_smoke.rs` covers the four render
//! exports, and covers them well, but it builds for the host — where
//! `AozoraDocument::profileJson` reads the `0.0` stub instead of
//! `performance.now()`, and where the ABI these functions exist to cross is
//! not present to cross.
//!
//! Run by `just test-wasm` (`wasm-pack test --node`), a `[group('gate')]`
//! recipe, so CI expands it into a job like any other gate.
//!
//! Coverage here is semantic, not an assertion-count proxy: each export is
//! called with an input that distinguishes the value or side effect its host
//! relies on. `initPanicHook` is the one return-less exception. Its contract
//! is that installation, including a repeated call, does not trap; reaching
//! the next statement is therefore the observable assertion.
//!
//! The whole file is `#![cfg(target_arch = "wasm32")]`: `#[wasm_bindgen_test]`
//! is collected by wasm-bindgen's runner, not by libtest, so on the host these
//! would be tests nothing runs. The two files beside this one carry the
//! opposite gate for the same reason.

#![cfg(target_arch = "wasm32")]
#![forbid(unsafe_code)]

use aozora_flavored_markdown_wasm::{
    AozoraDocument, JsOptions, hash_source, init_panic_hook, render, render_aozora_only,
    render_blocks, slugs_json,
};
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

/// One document that reaches every export's interesting path: ruby (an Aozora
/// inline, so `render` has something only this dialect produces), a gaiji
/// reference the resolver has an answer for, a paired `［＃ここから…］`
/// container so `pairsJson` has a pair, and an unmatched `》` so the parser
/// has something to report.
const SOURCE: &str = concat!(
    "# ｜見出し《みだし》\n",
    "\n",
    "本文 ※［＃二の字点、1-2-22］ です\n",
    "\n",
    "［＃ここから字下げ］\n",
    "字下げ本文\n",
    "［＃ここで字下げ終わり］\n",
    "\n",
    "orphan》close\n",
);

/// The `Serialize`-only envelopes have private fields, so what a host reads is
/// what serde produces — which is what these assertions read too.
fn json_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("the envelopes serialise")
}

/// Every editor-assist export answers in the 青空文庫 parser's own
/// `{ schemaVersion, data }` envelope, which the playground's TS layer decodes
/// by that shape alone (`playground/src/editor/parserState.ts`).
fn envelope(label: &str, json: &str) -> Value {
    let value: Value =
        serde_json::from_str(json).unwrap_or_else(|e| panic!("{label} must return JSON: {e}"));
    assert!(
        value.get("schemaVersion").is_some_and(Value::is_number),
        "{label} must carry a numeric `schemaVersion`; got {value}"
    );
    assert!(
        value["data"].is_array(),
        "{label} must carry a `data` array; got {value}"
    );
    value
}

fn data_len(label: &str, json: &str) -> usize {
    envelope(label, json)["data"].as_array().map_or(0, Vec::len)
}

// ---------------------------------------------------------------------------
// the render surface
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn the_panic_hook_installs_and_a_second_call_is_still_safe() {
    // `wasm-loader.ts` calls this from `ensureInit`, which every export path in
    // the playground goes through, so it runs once per module load and more
    // than once across a hot reload. Nothing comes back to assert on: the
    // claim is that neither call unwinds, and `set_once` is what makes the
    // second one a no-op rather than a second registered hook.
    init_panic_hook();
    init_panic_hook();
}

#[wasm_bindgen_test]
fn hash_source_is_a_stable_key_that_separates_documents() {
    assert_eq!(
        hash_source(SOURCE),
        hash_source(SOURCE),
        "the cache key has to be stable, or the host re-renders on every keystroke"
    );
    assert_ne!(
        hash_source(SOURCE),
        hash_source(""),
        "two documents must not share a cache key"
    );
}

#[wasm_bindgen_test]
fn render_returns_the_ir_the_html_and_the_diagnostics_together() {
    let rendered = json_of(&render(SOURCE, None).expect("default options decode"));
    assert!(
        rendered["ir"]["blocks"]
            .as_array()
            .is_some_and(|blocks| !blocks.is_empty()),
        "`ir.blocks` is what a host renders from; got {}",
        rendered["ir"]
    );
    assert!(
        rendered["html"]
            .as_str()
            .is_some_and(|html| html.contains("<ruby>")),
        "the ruby in the source has to reach the html fallback; got {}",
        rendered["html"]
    );
    assert!(
        rendered["diagnostics"].is_array(),
        "the diagnostic channel is the reason this crate has no size guard of its own; got {}",
        rendered["diagnostics"]
    );
}

#[wasm_bindgen_test]
fn render_forwards_the_options_it_was_handed() {
    let value = tsify::serde_wasm_bindgen::to_value(&serde_json::json!({
        "aozora": false,
        "hardbreaks": false
    }))
    .expect("known options serialise");
    let options: JsOptions = value.unchecked_into();
    assert_ne!(
        json_of(&render(SOURCE, Some(options)).expect("known options decode"))["html"],
        json_of(&render(SOURCE, None).expect("default options decode"))["html"],
        "`commonmark()` runs no notation pass and `default()` does, so an export that dropped \
         its `options` argument would render these the same"
    );
}

#[wasm_bindgen_test]
fn render_aozora_only_is_render_with_the_default_options() {
    assert_eq!(
        json_of(&render_aozora_only(SOURCE)),
        json_of(&render(SOURCE, None).expect("default options decode")),
        "the aozora-only wrapper takes no options, so the only thing it can get wrong is \
         choosing a different default than `render` does"
    );
}

#[wasm_bindgen_test]
fn render_blocks_returns_every_block_with_the_line_it_started_on() {
    let bridged = json_of(&render_blocks(SOURCE, None).expect("default options decode"));
    let blocks = bridged["blocks"]
        .as_array()
        .unwrap_or_else(|| panic!("the envelope carries an array of blocks; got {bridged}"));
    assert!(
        !blocks.is_empty(),
        "the obsidian bridge checks its AbortSignal between blocks, so there have to be some"
    );
    for (index, block) in blocks.iter().enumerate() {
        assert!(
            block["ir"].is_array(),
            "block {index} must carry its ir projection; got {block}"
        );
        assert!(
            block["html"].is_string(),
            "block {index} must carry its html; got {block}"
        );
        // `sourceLine`, not `source_line`: the field is renamed on the way out
        // and a host reads the renamed one.
        assert!(
            block["sourceLine"].as_u64().is_some_and(|line| line >= 1),
            "block {index} must carry a 1-based source line; got {block}"
        );
    }
    assert!(
        bridged["diagnostics"].is_array(),
        "diagnostics are document-scoped, so they ride beside the blocks; got {bridged}"
    );
}

// ---------------------------------------------------------------------------
// the editor-assist surface
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn slugs_json_is_the_annotation_catalogue_the_completion_menu_reads() {
    assert!(
        data_len("slugsJson", &slugs_json()) > 0,
        "an empty catalogue would leave the `［＃...］` completion menu silent"
    );
}

#[wasm_bindgen_test]
fn the_document_handle_reports_the_byte_length_of_what_it_was_given() {
    let doc = AozoraDocument::new(SOURCE.to_owned());
    assert_eq!(
        doc.source_byte_len(),
        SOURCE.len(),
        "the offset tables are in UTF-8 bytes, so the length has to be too"
    );
}

#[wasm_bindgen_test]
fn the_document_handle_answers_every_projection_the_editor_asks_for() {
    let doc = AozoraDocument::new(SOURCE.to_owned());
    assert!(
        data_len("nodesJson", &doc.nodes_json()) > 0,
        "the fixture carries ruby, a gaiji reference and a container, so structural highlight, \
         outline and fold all have spans to be driven by"
    );
    assert!(
        data_len("pairsJson", &doc.pairs_json()) > 0,
        "the fixture's ［＃ここから字下げ］ / ［＃ここで字下げ終わり］ is a matched pair, and \
         linked-range editing is built on those"
    );
    assert!(
        data_len("gaijiResolutionsJson", &doc.gaiji_resolutions_json()) > 0,
        "the fixture's ※［＃二の字点、1-2-22］ resolves, and the inlay hints are those \
         resolutions"
    );
    let diagnostics = envelope("diagnosticsJson", &doc.diagnostics_json());
    let entries = diagnostics["data"]
        .as_array()
        .unwrap_or_else(|| panic!("`data` is an array; got {diagnostics}"));
    let close_start = SOURCE
        .rfind('》')
        .expect("the fixture ends with an unmatched ruby close");
    let unmatched = entries
        .iter()
        .find(|entry| {
            entry["kind"].as_str() == Some("unmatched_close")
                && entry["span"]["start"].as_u64() == u64::try_from(close_start).ok()
                && entry["span"]["end"].as_u64()
                    == u64::try_from(close_start + '》'.len_utf8()).ok()
        })
        .unwrap_or_else(|| {
            panic!(
                "diagnosticsJson must carry the fixture's unmatched `》` at bytes \
                 {close_start}..{}; got {diagnostics}",
                close_start + '》'.len_utf8()
            )
        });
    assert_eq!(
        unmatched["severity"].as_str(),
        Some("error"),
        "an unmatched close is an error; got {unmatched}"
    );
    assert_eq!(
        unmatched["source"].as_str(),
        Some("source"),
        "the unmatched close came from the source, not an internal check; got {unmatched}"
    );
}

#[wasm_bindgen_test]
fn resolve_gaiji_at_answers_inside_a_gaiji_span_and_null_outside_one() {
    let doc = AozoraDocument::new(SOURCE.to_owned());
    let at = SOURCE
        .find('※')
        .expect("the fixture carries a gaiji reference");

    let hit: Value = serde_json::from_str(&doc.resolve_gaiji_at(at))
        .expect("resolveGaijiAt must return JSON, not a bare string");
    assert!(
        hit["span"].is_object(),
        "the hover tooltip maps the returned span back to a CodeMirror range; got {hit}"
    );

    assert_eq!(
        doc.resolve_gaiji_at(0),
        "null",
        "offset 0 is the `#` of the heading. The literal string `\"null\"` is the contract — \
         `hover.ts` compares against it before parsing"
    );
}

#[wasm_bindgen_test]
fn profile_json_times_every_phase_off_the_clock_only_this_target_has() {
    let doc = AozoraDocument::new(SOURCE.to_owned());
    let value = envelope("profileJson", &doc.profile_json());

    assert_eq!(
        value["byte_len"]
            .as_u64()
            .and_then(|len| usize::try_from(len).ok()),
        Some(SOURCE.len()),
        "the perf badge prints throughput, so the size it divides by has to be the real one"
    );

    let entries = value["data"]
        .as_array()
        .unwrap_or_else(|| panic!("`data` is an array; got {value}"));

    let phases: Vec<&str> = entries
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        phases,
        [
            "parse",
            "diagnostics_json",
            "nodes_json",
            "pairs_json",
            "gaiji_resolutions"
        ],
        "the badge labels its rows from `name`, in order"
    );

    for entry in entries {
        let ms = entry["duration_ms"]
            .as_f64()
            .unwrap_or_else(|| panic!("every phase reports a `duration_ms`; got {entry}"));
        assert!(
            ms.is_finite() && ms >= 0.0,
            "`now_ms` reads `performance.now()` on this target and `0.0` on the host — either \
             way a duration is a finite, non-negative number of milliseconds; got {ms}"
        );
    }
}
