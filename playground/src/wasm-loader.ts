import * as aozoraMdWasm from 'aozora-flavored-markdown-wasm/aozora_flavored_markdown_wasm_bg.js';
import wasmUrl from 'aozora-flavored-markdown-wasm/aozora_flavored_markdown_wasm_bg.wasm?url';

// The raw AozoraDocument handle and slug catalogue, re-exported for
// the editor-assist layer (completion / hover / inlay / outline / fold /
// linter / structural highlight). These talk to the Aozora parser
// directly — a separate path from `render` (which goes through comrak
// and loses source offsets). See `crates/aozora-flavored-markdown-wasm/src/lib.rs`.
//
// `AozoraDocument` is re-exported as both a value and a type. The playground
// imports the generated binding module directly so a failed WASM fetch does
// not poison the browser's ESM module cache and can be retried in place.
export {
  AozoraDocument,
  type Block as IrBlock,
  type Diagnostic,
  type DiagnosticSource,
  type Inline as IrInline,
  type MarkdownDocument as IrDocument,
  type Severity,
} from 'aozora-flavored-markdown-wasm/aozora_flavored_markdown_wasm_bg.js';

export function slugsJson(): string {
  ensureInit();
  return aozoraMdWasm.slugsJson();
}

// Wire types come straight from the wasm-pack `.d.ts`, which `tsify`
// derives from the Rust IR + envelope types (ADR-0017). The `ir` field is
// therefore the real IR tree rather than `unknown`, with no separate codegen
// step that could drift. This module is the browser's single typed edge to the
// WASM package; adapter-engine.ts and outline.ts consume the aliases below.
//
// The IR types are aliased back to their `Ir*` spelling on the way out:
// unprefixed is right inside the Rust `ir` module, which supplies the
// context, but a browser module has no such module scope. The local `Ir*`
// names also keep source code visually distinct from DOM types.
//
// `Options` is aliased for the same reason: it is the renderer's option set
// named from inside the Rust crate that owns it, and a browser module has no
// such context. Every field is optional, so `{ sourceLineAnchors: true }` is
// a complete argument.
import type {
  Options as RenderOptions,
  RenderResult,
} from 'aozora-flavored-markdown-wasm';

export type { RenderOptions, RenderResult };

let initialized = false;
let initializationPromise: Promise<void> | null = null;

function ensureInit(): void {
  if (initialized) return;
  throw new Error('WASM used before initialization');
}

async function instantiateWasm(): Promise<WebAssembly.Instance> {
  const response = await fetch(wasmUrl);
  if (!response.ok) {
    throw new Error(
      `WASM request failed with ${response.status} ${response.statusText}`,
    );
  }
  const imports: WebAssembly.Imports = {
    './aozora_flavored_markdown_wasm_bg.js':
      aozoraMdWasm as WebAssembly.ModuleImports,
  };
  const contentType = response.headers.get('content-type') ?? '';
  if (
    typeof WebAssembly.instantiateStreaming === 'function' &&
    contentType.includes('application/wasm')
  ) {
    const result = await WebAssembly.instantiateStreaming(response, imports);
    return result.instance;
  }
  const result = await WebAssembly.instantiate(
    await response.arrayBuffer(),
    imports,
  );
  return result.instance;
}

async function initialize(): Promise<void> {
  if (import.meta.env.MODE === 'test') {
    const testModule = await import('aozora-flavored-markdown-wasm');
    testModule.initPanicHook();
    testModule.hashSource('');
    initialized = true;
    return;
  }
  const instance = await instantiateWasm();
  aozoraMdWasm.__wbg_set_wasm(instance.exports);
  aozoraMdWasm.initPanicHook();
  aozoraMdWasm.hashSource('');
  initialized = true;
}

export async function initializeWasm(): Promise<void> {
  if (initialized) return;
  initializationPromise ??= initialize().catch((error: unknown) => {
    initializationPromise = null;
    throw error;
  });
  await initializationPromise;
}

export function render(source: string, options?: RenderOptions): RenderResult {
  ensureInit();
  return aozoraMdWasm.render(source, options);
}

export function hashSource(source: string): bigint {
  ensureInit();
  return aozoraMdWasm.hashSource(source);
}
