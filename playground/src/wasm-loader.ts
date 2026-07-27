// Async entry point for the aozora-flavored-markdown-wasm bundle.
//
// `wasm-pack build --target bundler` ships an ES module that lazily
// instantiates the .wasm on first call. We import everything as a
// namespace so the wasm-bindgen bootstrap runs once at module-eval time.

import * as aozoraMdWasm from 'aozora-flavored-markdown-wasm';

// The raw 青空文庫 AozoraDocument handle + slug catalogue, re-exported for
// the editor-assist layer (completion / hover / inlay / outline / fold /
// linter / structural highlight). These talk to the Aozora parser
// directly — a separate path from `render` (which goes through comrak
// and loses source offsets). See `crates/aozora-flavored-markdown-wasm/src/lib.rs`.
//
// `AozoraDocument` is re-exported as both a value (the constructor) and a
// type via this single named re-export — the bundler-target pkg exports it
// as a class. `slugsJson` is wrapped so the panic hook is installed first.
export { AozoraDocument } from 'aozora-flavored-markdown-wasm';

export function slugsJson(): string {
  ensureInit();
  return aozoraMdWasm.slugsJson();
}

// Wire types come straight from the wasm-pack `.d.ts`, which `tsify`
// derives from the Rust IR + envelope types (ADR-0017) — so the `ir`
// field below is the real IR tree rather than `unknown`, with no separate
// codegen step that could drift. Re-exported here because this module is the
// one edge to the wasm package; the IR types are consumed from here directly
// (outline.ts, App.tsx) and the diagnostic ones through `diagnostics.ts`,
// which `diagnostics.test.ts` holds every consumer to.
//
// The IR types are aliased back to their `Ir*` spelling on the way out:
// unprefixed is right inside the Rust `ir` module, which supplies the
// context, but a browser module has no such module scope and `Document`
// there is the DOM's. The alias is TypeScript's problem to solve, so it
// is solved in TypeScript.
//
// `Options` is aliased for the same reason: it is the renderer's option set
// named from inside the Rust crate that owns it, and a browser module has no
// such context. Every field is optional, so `{ sourceLineAnchors: true }` is
// a complete argument.
import type {
  Options as RenderOptions,
  RenderResult,
} from 'aozora-flavored-markdown-wasm';

export type {
  Block as IrBlock,
  Diagnostic,
  DiagnosticSource,
  Document as IrDocument,
  Inline as IrInline,
  Severity,
} from 'aozora-flavored-markdown-wasm';
export type { RenderOptions, RenderResult };

let initialised = false;

// Block the UI thread on first render until the wasm has booted.
// `--target bundler` arranges synchronous lazy init the first time an
// export is touched, but we still call `initPanicHook` so any panic
// inside the renderer lands in the browser console with a readable
// trace instead of "unreachable executed".
function ensureInit(): void {
  if (initialised) return;
  aozoraMdWasm.initPanicHook();
  initialised = true;
}

export function render(source: string, options?: RenderOptions): RenderResult {
  ensureInit();
  return aozoraMdWasm.render(source, options);
}

export function hashSource(source: string): bigint {
  ensureInit();
  return aozoraMdWasm.hashSource(source);
}
