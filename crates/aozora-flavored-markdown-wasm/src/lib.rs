//! WebAssembly bindings for aozora-flavored-markdown.
//!
//! Exposes a thin set of `#[wasm_bindgen]` exports that
//! aozora-flavored-markdown-obsidian (and other browser hosts) call across the WASM
//! boundary. The IR shape returned by `render` and
//! `render_aozora_only` mirrors the TS `IRDocument` defined in
//! `aozora-flavored-markdown-obsidian/src/ir/types.ts` and is validated on the JS side
//! by `from-wasm.ts`.
//!
//! # Stability
//!
//! Public exports here are version-pinned to aozora-flavored-markdown's
//! workspace version. A bump on this crate implies an aozora-flavored-markdown-obsidian
//! recompilation against the new IR shape.
//!
//! # Surface
//!
//! - [`init_panic_hook`] — opt-in panic forwarder (debug builds).
//! - [`render`] — full aozora-flavored-markdown pipeline (CommonMark + GFM + aozora).
//! - [`render_aozora_only`] — aozora-only inline mode (used by
//!   aozora-flavored-markdown-obsidian's inline post-processor; bypasses comrak).
//! - [`hash_source`] — xxh3-64 over the source, returned as `u64`
//!   for cache-key construction on the JS side.

#![forbid(unsafe_code)]

use aozora::{Document as AozoraDoc, SLUGS, SlugFamily, encoding::gaiji, wire};
use aozora_flavored_markdown::ir::{IrBlock, IrDocument};
use aozora_flavored_markdown::{Diagnostic, Options, render_blocks_to_ir, render_to_ir};
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
///
/// Matches the shape consumed by
/// `aozora-flavored-markdown-obsidian/src/ir/from-wasm.ts`. `tsify` derives
/// its `.d.ts` straight from this struct, so the TS shape can't drift.
#[derive(Debug, Serialize, Tsify)]
#[tsify(into_wasm_abi)]
pub struct RenderResult {
    /// Structured IR — see `aozora_flavored_markdown::ir` for the type tree.
    /// Mirrors the TS `IRDocument` (camelCase fields, discriminated
    /// unions on `kind`).
    ir: IrDocument,
    /// Reference HTML (post-aozora-splice + source-line anchored).
    /// Consumers may render straight from the IR via the JS
    /// renderers; this string is a debug / fallback surface and a
    /// lifeline for hosts that don't ship a JS renderer.
    html: String,
    diagnostics: Vec<Diagnostic>,
}

/// Optional render configuration accepted from JS. All fields are
/// optional; missing fields fall back to `Options::default()`
/// (aozora on, anchors off).
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
        base = base.with_aozora_enabled(v);
    }
    if let Some(v) = opts.source_line_anchors {
        base = base.with_source_line_anchors(v);
    }
    base
}

/// Largest input the aozora parser core accepts, in bytes. Its span
/// offsets are `u32`, so a longer source trips a `u32::MAX` assert inside
/// the lexer. Under `panic = "abort"` that assert would abort the whole
/// Wasm instance.
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// `Ok(())` iff a source of `byte_len` UTF-8 bytes is within the parser
/// core's `u32` span-offset limit. Pure (takes the length, not the
/// string) so the boundary is unit-testable without allocating a 4 GiB
/// buffer.
///
/// # Errors
///
/// `Err(&'static str)` when `byte_len > u32::MAX`.
const fn source_len_within_span_limit(byte_len: usize) -> Result<(), &'static str> {
    if byte_len > MAX_SOURCE_BYTES {
        return Err("source exceeds 4 GiB (u32::MAX) span limit");
    }
    Ok(())
}

/// Reject sources larger than the parser core's `u32` span limit before
/// any parsing starts, returning a catchable `Err(JsValue)`.
///
/// `aozora-flavored-markdown` masks code-block triggers before lexing, but masking is a 1:1
/// character substitution (`｜`/`《`/… → U+E000, both 3-byte UTF-8), so
/// the masked source is byte-for-byte the same length as `source` —
/// checking `source.len()` here is exact.
///
/// # Errors
///
/// `Err(JsValue)` when `source.len()` (UTF-8 bytes) exceeds
/// [`u32::MAX`].
fn guard_source_len(source: &str) -> Result<(), JsValue> {
    source_len_within_span_limit(source.len()).map_err(JsValue::from_str)
}

/// Render aozora-flavored-markdown source to IR + HTML + diagnostics.
///
/// `options` is decoded as `{ aozoraEnabled?: boolean,
/// sourceLineAnchors?: boolean }`. Both default to the values from
/// `Options::default()` (aozora on, anchors off).
///
/// # Errors
///
/// Returns `Err(JsValue::String)` when `source` exceeds the parser core's
/// `u32` span limit (~4 GiB). `options` decoding and `RenderResult`
/// encoding are handled by `tsify`'s wasm ABI (a malformed `options`
/// surfaces as a wasm-bindgen `TypeError`, not an `Err`).
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

/// Render aozora-only inline text (no markdown re-parse).
///
/// Routes through the full aozora-flavored-markdown pipeline with default
/// options. The naming preserves an entry point that callers can target
/// without committing to the `render` shape; the implementation is
/// intentionally a thin wrapper because the notation boundary lives in the
/// sibling repo (ADR-0010) and this crate composes — never extends — its
/// public API (ADR-0021).
///
/// # Errors
///
/// Returns `Err(JsValue::String)` when `text` exceeds the parser core's
/// `u32` span limit (~4 GiB; delegated to [`render`]).
#[wasm_bindgen(js_name = renderAozoraOnly)]
pub fn render_aozora_only(text: &str) -> Result<RenderResult, JsValue> {
    render(text, None)
}

/// xxh3-64 over the source, returned as a `u64` (JS receives a
/// `bigint`). Used for cache keys.
#[must_use]
#[wasm_bindgen(js_name = hashSource)]
pub fn hash_source(source: &str) -> u64 {
    XxHash3_64::oneshot_with_seed(0, source.as_bytes())
}

#[derive(Debug, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct BlockResult {
    /// IR blocks for this comrak top-level child. Usually one entry;
    /// may be empty (comrak constructs without an IR projection) or
    /// multiple (paired-container drain at the call boundary).
    ir: Vec<IrBlock>,
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

/// Per-block streaming render.
///
/// Returns one `{ir, html, sourceLine}` entry per top-level comrak
/// block. The aozora-flavored-markdown-obsidian bridge iterates the array and checks its
/// `AbortSignal` between blocks (ADR-0009 chunked-cancellation
/// strategy).
///
/// # Errors
///
/// Returns `Err(JsValue::String)` when `source` exceeds the parser core's
/// `u32` span limit (~4 GiB). `options` decoding and `BlocksResult`
/// encoding are handled by `tsify`'s wasm ABI.
#[wasm_bindgen(js_name = renderBlocks)]
pub fn render_blocks(
    source: &str,
    options: Option<RenderOptions>,
) -> Result<BlocksResult, JsValue> {
    guard_source_len(source)?;
    let resolved = build_options(options.unwrap_or_default());
    let (blocks, diagnostics) = render_blocks_to_ir(source, &resolved);
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
// `render` (above) is the full aozora-flavored-markdown pipeline: source → aozora
// normalize → comrak → IR → HTML. That path is correct for output but
// drops the source byte offsets the editor needs for hover / inlay /
// fold / structural-highlight.
//
// So the editor talks to the 青空文庫 parser *directly* through the
// raw `aozora::Document` re-exposed here. It sees only the Aozora
// notation spans (ruby / bouten / gaiji / containers …) in source
// coordinates — Markdown constructs are simply not Aozora nodes, so
// they don't appear, which is exactly right for these assists. The
// wire format is byte-identical to the sibling aozora-wasm
// (`{ schema_version, data }`), so the playground's TS editor layer is
// a near-verbatim port of aozora's.
// =====================================================================

/// All canonical 青空文庫 slugs from the spec, in the standard wire
/// envelope, so the editor's `［＃...］` completion menu can drive a
/// catalogue without re-implementing the table.
///
/// Each `data[]` entry: `{ canonical, family, accepts_param, doc, partner }`.
/// `family` is the camelCase form of the Rust enum variant.
#[must_use]
#[wasm_bindgen(js_name = slugsJson)]
pub fn slugs_json() -> String {
    let entries: Vec<serde_json::Value> = SLUGS
        .iter()
        .map(|s| {
            let family = match s.family {
                SlugFamily::PageBreak => "pageBreak",
                SlugFamily::Section => "section",
                SlugFamily::BlockContainerOpen => "blockContainerOpen",
                SlugFamily::BlockContainerClose => "blockContainerClose",
                SlugFamily::LeafAlign => "leafAlign",
                SlugFamily::Bouten => "bouten",
                SlugFamily::Sashie => "sashie",
                SlugFamily::Keigakomi => "keigakomi",
                SlugFamily::Warichu => "warichu",
                SlugFamily::TateChuYoko => "tateChuYoko",
                SlugFamily::KaeritenSingle => "kaeritenSingle",
                SlugFamily::KaeritenCompound => "kaeritenCompound",
                // `SlugFamily` is `#[non_exhaustive]`: a family added in a
                // newer spec surfaces as "unknown" so JS can ignore it.
                _ => "unknown",
            };
            serde_json::json!({
                "canonical": s.canonical,
                "family": family,
                "accepts_param": s.accepts_param,
                "doc": s.doc,
                "partner": s.partner,
            })
        })
        .collect();
    serde_json::json!({ "schema_version": 1, "data": entries }).to_string()
}

const GAIJI_OPEN: &str = "※［＃";
const GAIJI_CLOSE: &str = "］";
// Bounded window for the cursor-pinned hover variant: a real ※［＃…］
// span is at most a few hundred bytes, so capping the search keeps
// per-keystroke resolution O(window) rather than O(doc).
const MAX_GAIJI_SPAN_LEN: usize = 512;

/// High-resolution wall-clock in milliseconds. On wasm32 it reads the
/// browser `performance.now()` (`std::time::Instant` panics on
/// `wasm32-unknown-unknown`); on host builds — where this code only
/// needs to compile for clippy / tests — it returns 0.0, so the
/// profile deltas read as constant 0 off the browser.
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
/// Wraps an [`aozora::Document`], which owns both the source and the
/// bumpalo arena backing the borrowed AST. This is the raw Aozora
/// parser — NOT the aozora-flavored-markdown pipeline — so its spans are in source
/// coordinates. Drop is automatic when the JS handle is GC'd (or via
/// the generated `free()`).
#[derive(Debug)]
#[wasm_bindgen]
pub struct Document {
    inner: AozoraDoc,
}

#[wasm_bindgen]
impl Document {
    /// Construct from a UTF-16 JS string (copied once into the
    /// Document's internal `Box<str>`; later queries reuse the arena).
    #[must_use]
    #[wasm_bindgen(constructor)]
    pub fn new(source: String) -> Self {
        Self {
            inner: AozoraDoc::new(source),
        }
    }

    /// Aozora-node spans as JSON: `{ kind, span: { start, end } }`,
    /// source bytes, sorted by `span.start`. Drives structural
    /// highlight / outline / fold.
    #[must_use]
    #[wasm_bindgen(js_name = nodesJson)]
    pub fn nodes_json(&self) -> String {
        wire::serialize_nodes(&self.inner.parse())
    }

    /// Matched open/close pair links as JSON:
    /// `{ kind, open: { start, end }, close: { start, end } }`. Drives
    /// linked-range editing and fold ranges.
    #[must_use]
    #[wasm_bindgen(js_name = pairsJson)]
    pub fn pairs_json(&self) -> String {
        wire::serialize_pairs(&self.inner.parse())
    }

    /// Diagnostics as JSON in the standard envelope. Drives the
    /// in-editor squiggle linter.
    #[must_use]
    #[wasm_bindgen(js_name = diagnosticsJson)]
    pub fn diagnostics_json(&self) -> String {
        wire::serialize_diagnostics(self.inner.parse().diagnostics())
    }

    /// Source byte length (UTF-8). Used by the offset tables / profile.
    #[must_use]
    #[wasm_bindgen(js_name = sourceByteLen)]
    pub fn source_byte_len(&self) -> usize {
        self.inner.source().len()
    }

    /// Resolve the gaiji reference at `byte_offset`, or the literal
    /// string `"null"` if the offset is not inside a `※［＃…］` span.
    /// Bounded to a 512-byte window, so cost is independent of document
    /// size — editors call this on every cursor move.
    ///
    /// On hit:
    /// `{ span, description, mencode?, codepoint?, resolved? }`.
    #[must_use]
    #[wasm_bindgen(js_name = resolveGaijiAt)]
    pub fn resolve_gaiji_at(&self, byte_offset: usize) -> String {
        let source = self.inner.source();
        find_gaiji_span_local(source, byte_offset)
            .and_then(|span| build_resolution_value(source, span.0, span.1))
            .map_or_else(|| "null".to_owned(), |v| v.to_string())
    }

    /// All gaiji resolutions in the document, in the standard envelope.
    /// Powers inlay hints (`→GLYPH` after every `※［＃…］`). Walks the
    /// source once, `O(source)`.
    #[must_use]
    #[wasm_bindgen(js_name = gaijiResolutionsJson)]
    pub fn gaiji_resolutions_json(&self) -> String {
        let source = self.inner.source();
        let mut entries: Vec<serde_json::Value> = Vec::new();
        let mut cursor = 0usize;
        while let Some(rel) = source[cursor..].find(GAIJI_OPEN) {
            let span_start = cursor + rel;
            let body_start = span_start + GAIJI_OPEN.len();
            let Some(close_rel) = source[body_start..].find(GAIJI_CLOSE) else {
                break;
            };
            let span_end = body_start + close_rel + GAIJI_CLOSE.len();
            if let Some(val) = build_resolution_value(source, span_start, span_end) {
                entries.push(val);
            }
            cursor = span_end;
        }
        serde_json::json!({ "schema_version": 1, "data": entries }).to_string()
    }

    /// Per-method timing snapshot (`{ name, duration_ms }[]`) plus
    /// `byte_len`, for the editor's perf badge. Wall-clock via
    /// `performance.now()` (host builds read 0.0 — see `now_ms`).
    #[must_use]
    #[wasm_bindgen(js_name = profileJson)]
    pub fn profile_json(&self) -> String {
        let p0 = now_ms();
        let tree = self.inner.parse();
        let p1 = now_ms();

        let d0 = now_ms();
        let _diag = wire::serialize_diagnostics(tree.diagnostics());
        let d1 = now_ms();

        let n0 = now_ms();
        let _nodes = wire::serialize_nodes(&tree);
        let n1 = now_ms();

        let pa0 = now_ms();
        let _pairs = wire::serialize_pairs(&tree);
        let pa1 = now_ms();

        let g0 = now_ms();
        let _gaiji = self.gaiji_resolutions_json();
        let g1 = now_ms();

        let entries = serde_json::json!([
            { "name": "parse",             "duration_ms": p1  - p0  },
            { "name": "diagnostics_json",  "duration_ms": d1  - d0  },
            { "name": "nodes_json",        "duration_ms": n1  - n0  },
            { "name": "pairs_json",        "duration_ms": pa1 - pa0 },
            { "name": "gaiji_resolutions", "duration_ms": g1  - g0  },
        ]);
        serde_json::json!({
            "schema_version": 1,
            "byte_len": self.inner.source().len(),
            "data": entries,
        })
        .to_string()
    }
}

/// Byte-range of the `※［＃…］` span containing `byte_offset`, scanned
/// only within a bounded window around the cursor.
fn find_gaiji_span_local(source: &str, byte_offset: usize) -> Option<(usize, usize)> {
    if source.is_empty() {
        return None;
    }
    let win_start =
        snap_to_char_boundary_left(source, byte_offset.saturating_sub(MAX_GAIJI_SPAN_LEN));
    let win_end = snap_to_char_boundary_right(
        source,
        byte_offset
            .saturating_add(MAX_GAIJI_SPAN_LEN)
            .min(source.len()),
    );
    let window = &source[win_start..win_end];
    let win_offset = byte_offset.saturating_sub(win_start);

    for (start_in_win, _) in window.match_indices(GAIJI_OPEN) {
        let after_open = start_in_win + GAIJI_OPEN.len();
        let Some(end_rel) = window.get(after_open..).and_then(|s| s.find(GAIJI_CLOSE)) else {
            continue;
        };
        let end_in_win = after_open + end_rel + GAIJI_CLOSE.len();
        if (start_in_win..end_in_win).contains(&win_offset) {
            return Some((win_start + start_in_win, win_start + end_in_win));
        }
    }
    None
}

const fn snap_to_char_boundary_left(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

const fn snap_to_char_boundary_right(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    while idx < len && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Split a gaiji body (`「description」、mencode[、page-line]`) into
/// `(description, mencode?)`. Tail fields (page-line refs) are dropped.
fn parse_gaiji_body(body: &str) -> (String, Option<String>) {
    let body = body.trim();
    let (description, rest) = body.find('「').map_or_else(
        || (body.to_owned(), ""),
        |open_idx| {
            let after_open = &body[open_idx + '「'.len_utf8()..];
            after_open.find('」').map_or_else(
                || (body.to_owned(), ""),
                |close_rel| {
                    let desc = after_open[..close_rel].to_owned();
                    let rest = &after_open[close_rel + '」'.len_utf8()..];
                    (desc, rest)
                },
            )
        },
    );
    let rest = rest.trim_start_matches('、').trim();
    let mencode = rest
        .split('、')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    (description, mencode)
}

/// JSON resolution object for a `※［＃…］` span at `[start..end)`, or
/// `None` if the body cannot be parsed.
fn build_resolution_value(source: &str, start: usize, end: usize) -> Option<serde_json::Value> {
    let body_start = start.checked_add(GAIJI_OPEN.len())?;
    let body_end = end.checked_sub(GAIJI_CLOSE.len())?;
    if body_end <= body_start || body_end > source.len() {
        return None;
    }
    let body = source.get(body_start..body_end)?;
    let (description, mencode) = parse_gaiji_body(body);
    let resolved = gaiji::lookup(None, mencode.as_deref(), &description);
    let (resolved_str, codepoint) = resolved.map_or((None, None), |r| {
        let mut s = String::new();
        _ = r.write_to(&mut s);
        let cp = r.as_char().map(|c| c as u32);
        (Some(s), cp)
    });
    Some(serde_json::json!({
        "span": { "start": start, "end": end },
        "description": description,
        "mencode": mencode,
        "codepoint": codepoint,
        "resolved": resolved_str,
    }))
}

#[cfg(test)]
mod tests {
    use super::{guard_source_len, source_len_within_span_limit};

    /// The boundary guard accepts in-range lengths (including the
    /// inclusive `u32::MAX` upper bound) and rejects anything larger,
    /// matching the `u32::MAX` assert the aozora parser core enforces in
    /// `tokenize_in`. The Wasm render entry points (`render`,
    /// `renderBlocks`, and `renderAozoraOnly` via `render`) call the
    /// guard so an oversize source surfaces as `Err(JsValue)` instead of
    /// a `panic = "abort"` teardown of the Wasm instance.
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

    /// Typical inputs pass the `&str` wrapper unharmed. Uses `.expect()`
    /// (not `assert!(… .is_ok())`) to satisfy clippy's
    /// `assertions_on_result_states`; `JsValue` implements `Debug` on
    /// all targets, so this compiles on the host test build.
    #[test]
    fn guard_accepts_typical_source() {
        guard_source_len("").expect("empty source must be accepted");
        guard_source_len("｜漢字《かんじ》 and **markdown**")
            .expect("typical mixed source must be accepted");
    }
}
