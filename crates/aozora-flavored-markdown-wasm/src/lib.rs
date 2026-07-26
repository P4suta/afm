//! WebAssembly bindings for aozora-flavored-markdown.
//!
//! `#[wasm_bindgen]` exports that aozora-flavored-markdown-obsidian and other
//! browser hosts call across the WASM boundary. `tsify` derives the `.d.ts`
//! from these types, so the TS shape cannot drift from the Rust one.
//!
//! Exports are version-pinned to the workspace version: a bump here implies
//! an obsidian recompilation against the new IR shape.

#![forbid(unsafe_code)]

use aozora::json;
use aozora_flavored_markdown::ir::{Block, Document};
// Aliased because the `renderBlocks` export below is itself a `render_blocks`
// in Rust, and the ABI's name is the one that cannot move.
use aozora_flavored_markdown::{
    Diagnostic, Options, RenderedBlocks, render_blocks as render_blocks_core, render_to_ir,
};
use serde::Serialize;
use tsify::Tsify;
use twox_hash::XxHash3_64;
use wasm_bindgen::prelude::*;

/// Install a `console.error` panic hook for friendlier debugging.
/// No-op when compiled without the `panic-hook` feature.
#[wasm_bindgen(js_name = initPanicHook)]
pub fn init_panic_hook() {
    #[cfg(feature = "panic-hook")]
    {
        console_error_panic_hook::set_once();
    }
}

/// Result envelope returned to JS.
#[derive(Debug, Serialize, Tsify)]
#[tsify(into_wasm_abi)]
pub struct RenderResult {
    ir: Document,
    /// A debug / fallback surface: hosts that ship a JS renderer should
    /// render from `ir` instead.
    html: String,
    diagnostics: Vec<Diagnostic>,
}

/// Missing fields fall back to `Options::default()`.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize, Tsify)]
#[tsify(from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct RenderOptions {
    aozora_enabled: Option<bool>,
    source_line_anchors: Option<bool>,
}

fn build_options(opts: RenderOptions) -> Options {
    let mut base = Options::default();
    if let Some(v) = opts.aozora_enabled {
        base = base.with_aozora(v);
    }
    if let Some(v) = opts.source_line_anchors {
        base = base.with_source_line_anchors(v);
    }
    base
}

/// The parser core's span offsets are `u32`, so a longer source trips an
/// assert that would abort the whole Wasm instance under `panic = "abort"`.
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// Takes the length rather than the string, so the boundary is testable
/// without allocating a 4 GiB buffer.
///
/// # Errors
///
/// When `byte_len > u32::MAX`.
const fn source_len_within_span_limit(byte_len: usize) -> Result<(), &'static str> {
    if byte_len > MAX_SOURCE_BYTES {
        return Err("source exceeds 4 GiB (u32::MAX) span limit");
    }
    Ok(())
}

/// Checking `source.len()` is exact even though the pipeline masks
/// code-block triggers first: masking is a 1:1 substitution between
/// equal-width UTF-8 characters.
///
/// # Errors
///
/// When `source.len()` exceeds [`u32::MAX`].
fn guard_source_len(source: &str) -> Result<(), JsValue> {
    source_len_within_span_limit(source.len()).map_err(JsValue::from_str)
}

/// Render aozora-flavored-markdown source to IR + HTML + diagnostics.
///
/// # Errors
///
/// When `source` exceeds the parser core's ~4 GiB span limit. A malformed
/// `options` surfaces as a wasm-bindgen `TypeError` instead, since `tsify`
/// owns the ABI decode.
#[wasm_bindgen(js_name = render)]
pub fn render(source: &str, options: Option<RenderOptions>) -> Result<RenderResult, JsValue> {
    guard_source_len(source)?;
    let resolved = build_options(options.unwrap_or_default());
    let rendered = render_to_ir(source, &resolved);
    Ok(RenderResult {
        ir: rendered.ir,
        html: rendered.html,
        diagnostics: rendered.diagnostics,
    })
}

/// Aozora-only inline mode for the obsidian inline post-processor.
///
/// Deliberately a thin wrapper over [`render`]: the notation boundary lives
/// in the sibling repo (ADR-0010) and this crate composes — never extends —
/// its public API (ADR-0021). The separate name lets callers target the mode
/// without committing to the `render` shape.
///
/// # Errors
///
/// As [`render`].
#[wasm_bindgen(js_name = renderAozoraOnly)]
pub fn render_aozora_only(text: &str) -> Result<RenderResult, JsValue> {
    render(text, None)
}

/// xxh3-64 cache key; JS receives a `bigint`.
#[must_use]
#[wasm_bindgen(js_name = hashSource)]
pub fn hash_source(source: &str) -> u64 {
    XxHash3_64::oneshot_with_seed(0, source.as_bytes())
}

#[derive(Debug, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct BlockResult {
    /// Usually one entry; empty for comrak constructs with no IR
    /// projection, several for a paired-container drain.
    ir: Vec<Block>,
    html: String,
    /// 1-based source line (serialised as `sourceLine`).
    source_line: u32,
}

#[derive(Debug, Serialize, Tsify)]
#[tsify(into_wasm_abi)]
pub struct BlocksResult {
    blocks: Vec<BlockResult>,
    diagnostics: Vec<Diagnostic>,
}

/// One entry per top-level comrak block, so the obsidian bridge can check
/// its `AbortSignal` between them (ADR-0009 chunked cancellation).
///
/// # Errors
///
/// As [`render`].
#[wasm_bindgen(js_name = renderBlocks)]
pub fn render_blocks(
    source: &str,
    options: Option<RenderOptions>,
) -> Result<BlocksResult, JsValue> {
    guard_source_len(source)?;
    let resolved = build_options(options.unwrap_or_default());
    let RenderedBlocks {
        blocks,
        diagnostics,
        ..
    } = render_blocks_core(source, &resolved);
    Ok(BlocksResult {
        blocks: blocks
            .into_iter()
            .map(|b| BlockResult {
                ir: b.ir,
                html: b.html,
                source_line: b.source_line,
            })
            .collect(),
        diagnostics,
    })
}

// =====================================================================
// Editor-assist surface
//
// Everything below is for the playground's *editor*, not its renderer.
// `render` (above) is the full aozora-flavored-markdown pipeline: source →
// constructs → comrak → IR → HTML. That path is correct for output but
// drops the source byte offsets the editor needs for hover / inlay /
// fold / structural-highlight.
//
// So the editor talks to the 青空文庫 parser *directly* through the
// document handle re-exposed here. It sees only the Aozora notation spans
// (ruby / bouten / gaiji / containers …) in source coordinates — Markdown
// constructs are simply not Aozora nodes, so they don't appear, which is
// exactly right for these assists. Every envelope is the parser's own
// (`{ schemaVersion, data }`), so the playground's TS editor layer is a
// near-verbatim port of the sibling parser's.
// =====================================================================

/// Canonical 青空文庫 annotation slugs, so the editor's `［＃...］`
/// completion menu drives a catalogue without re-implementing the table.
#[must_use]
#[wasm_bindgen(js_name = slugsJson)]
pub fn slugs_json() -> String {
    json::slugs()
}

/// `std::time::Instant` panics on `wasm32-unknown-unknown`, so this reads
/// `performance.now()` there. Host builds only need to compile, and read 0.
#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    0.0
}

/// JS-facing handle to a 青空文庫-parsed document (editor assists only).
///
/// The raw 青空文庫 parser, **not** the aozora-flavored-markdown pipeline, so
/// its spans are in source coordinates.
// Named for the parser it wraps rather than `Document`, because the ABI is
// where the disambiguation belongs: `tsify` derives a TS name from the Rust
// one, so `ir::Document` already claims `Document` in the emitted `.d.ts` —
// and so does the DOM. Two `Document` declarations in one `.d.ts` merge
// silently instead of failing, which is the worse of the two outcomes.
#[derive(Debug)]
#[wasm_bindgen]
pub struct AozoraDocument {
    inner: aozora::Document,
}

#[wasm_bindgen]
impl AozoraDocument {
    /// A source past the ~4 GiB span budget — which no JS string reaches —
    /// yields an empty document rather than throwing, so the editor's
    /// per-keystroke path has no failure mode to handle.
    ///
    /// # Panics
    ///
    /// Never in practice: the fallback parses the empty string.
    #[must_use]
    #[wasm_bindgen(constructor)]
    pub fn new(source: String) -> Self {
        Self {
            inner: aozora::parse(source)
                .or_else(|_| aozora::parse(""))
                .expect("an empty source is always within the span budget"),
        }
    }

    /// Node spans sorted by `span.start`, driving structural highlight,
    /// outline and fold.
    #[must_use]
    #[wasm_bindgen(js_name = nodesJson)]
    pub fn nodes_json(&self) -> String {
        json::nodes(&self.inner.snapshot())
    }

    /// Matched open/close pairs, driving linked-range editing and folds.
    #[must_use]
    #[wasm_bindgen(js_name = pairsJson)]
    pub fn pairs_json(&self) -> String {
        json::pairs(&self.inner.snapshot())
    }

    /// Drives the in-editor squiggle linter.
    #[must_use]
    #[wasm_bindgen(js_name = diagnosticsJson)]
    pub fn diagnostics_json(&self) -> String {
        json::diagnostics(self.inner.snapshot().diagnostics())
    }

    /// UTF-8 byte length, for the offset tables and the profile badge.
    #[must_use]
    #[wasm_bindgen(js_name = sourceByteLen)]
    pub fn source_byte_len(&self) -> usize {
        self.inner.source().len()
    }

    /// The literal string `"null"` when `byte_offset` is not inside a
    /// `※［＃…］` span. Editors call this on every cursor move; the parser
    /// answers from a projection it already built, so it costs a lookup
    /// rather than a scan.
    #[must_use]
    #[wasm_bindgen(js_name = resolveGaijiAt)]
    pub fn resolve_gaiji_at(&self, byte_offset: usize) -> String {
        json::gaiji_entry_at(&self.inner.snapshot(), byte_offset)
            .and_then(|entry| serde_json::to_string(&entry).ok())
            .unwrap_or_else(|| "null".to_owned())
    }

    /// Powers inlay hints (`→GLYPH` after every `※［＃…］`).
    #[must_use]
    #[wasm_bindgen(js_name = gaijiResolutionsJson)]
    pub fn gaiji_resolutions_json(&self) -> String {
        json::gaiji(&self.inner.snapshot())
    }

    /// Per-method timings for the editor's perf badge.
    #[must_use]
    #[wasm_bindgen(js_name = profileJson)]
    pub fn profile_json(&self) -> String {
        let p0 = now_ms();
        let snapshot = self.inner.snapshot();
        let p1 = now_ms();

        let d0 = now_ms();
        let _diag = json::diagnostics(snapshot.diagnostics());
        let d1 = now_ms();

        let n0 = now_ms();
        let _nodes = json::nodes(&snapshot);
        let n1 = now_ms();

        let pa0 = now_ms();
        let _pairs = json::pairs(&snapshot);
        let pa1 = now_ms();

        let g0 = now_ms();
        let _gaiji = json::gaiji(&snapshot);
        let g1 = now_ms();

        let entries = serde_json::json!([
            { "name": "parse",             "duration_ms": p1  - p0  },
            { "name": "diagnostics_json",  "duration_ms": d1  - d0  },
            { "name": "nodes_json",        "duration_ms": n1  - n0  },
            { "name": "pairs_json",        "duration_ms": pa1 - pa0 },
            { "name": "gaiji_resolutions", "duration_ms": g1  - g0  },
        ]);
        serde_json::json!({
            "schemaVersion": json::SCHEMA_VERSION,
            "byte_len": self.inner.source().len(),
            "data": entries,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{guard_source_len, source_len_within_span_limit};

    /// `u32::MAX` is the inclusive upper bound, matching the parser core's
    /// own assert. Every render entry point calls the guard so an oversize
    /// source surfaces as `Err` instead of tearing down the Wasm instance.
    #[test]
    fn source_len_guard_matches_u32_span_boundary() {
        source_len_within_span_limit(0).expect("empty source is in range");
        source_len_within_span_limit(4096).expect("4 KiB source is in range");
        source_len_within_span_limit(u32::MAX as usize)
            .expect("u32::MAX bytes is the inclusive upper bound");
        let err = source_len_within_span_limit(u32::MAX as usize + 1)
            .expect_err("u32::MAX + 1 bytes must be rejected");
        assert!(err.contains("u32::MAX"), "error mentions the limit: {err}");
    }

    /// `.expect()` rather than `assert!(….is_ok())` to satisfy clippy's
    /// `assertions_on_result_states`.
    #[test]
    fn guard_accepts_typical_source() {
        guard_source_len("").expect("empty source must be accepted");
        guard_source_len("｜漢字《かんじ》 and **markdown**")
            .expect("typical mixed source must be accepted");
    }
}
