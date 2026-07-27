import { expect, test } from 'vitest';

import * as diagnostics from './diagnostics';

// `diagnostics.ts` is a re-export and nothing else, so there is no behaviour
// under it to exercise. What there is, is a claim: the module says it exists
// "so callers can `import type { Diagnostic } from './diagnostics'` without
// reaching into the wasm bridge module name". A one-line indirection whose
// whole value is that someone goes through it — and, until this file, with
// nothing checking that anyone did. It had zero importers: its own named
// consumer, `components/DiagnosticsDrawer.tsx`, took the type from
// `../wasm-loader` and type-checked perfectly, which is what a dead
// indirection looks like from inside `tsc`.
//
// So the tests here are about the module's edges rather than its body.

const sources = import.meta.glob<string>('./**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
});

const SELF = './diagnostics.ts';
const BRIDGE = './wasm-loader.ts';

interface BridgeImport {
  /** Names taken with `import type` / an inline `type` marker. */
  readonly types: string[];
  /** Names taken as values — a real module edge at run time. */
  readonly values: string[];
}

/** What one module takes out of `wasm-loader`, split by what it costs. */
function fromBridge(source: string): BridgeImport {
  const pattern =
    /(import|export)(\s+type)?\s+\{([^}]*)\}\s*from\s*'[^']*wasm-loader'/g;
  const types: string[] = [];
  const values: string[] = [];
  for (const [, , typeOnly, braced] of source.matchAll(pattern)) {
    for (const raw of (braced ?? '').split(',')) {
      const name = raw.trim();
      if (name.length === 0) continue;
      const inlineType = name.startsWith('type ');
      const bare = inlineType ? name.slice('type '.length).trim() : name;
      if (typeOnly !== undefined || inlineType) types.push(bare);
      else values.push(bare);
    }
  }
  return { types, values };
}

/** What this module forwards, read off it rather than restated here. */
function forwarded(): Set<string> {
  const self = sources[SELF];
  if (self === undefined) {
    throw new Error(`the glob no longer resolves ${SELF}; the reader is blind`);
  }
  const { types } = fromBridge(self);
  expect(types, 'diagnostics.ts forwards nothing any more').not.toStrictEqual(
    [],
  );
  return new Set(types);
}

test('the module contributes nothing to the bundle', () => {
  // `export type` is erased, so the namespace is empty at run time. Not a
  // formality: a value export here would give every `import type` consumer a
  // real module edge into `wasm-loader`, which instantiates the wasm when it
  // evaluates — a presentational component that wanted a type would boot the
  // renderer.
  expect(Object.keys({ ...diagnostics })).toStrictEqual([]);
});

test('something imports it', () => {
  // The state it was found in. An indirection with no importers is not a
  // seam, it is a signpost pointing at nothing — and it reads to the next
  // person as the canonical place to import from, which it was not.
  const importers = Object.entries(sources)
    .filter(([path]) => path !== SELF)
    .filter(([, source]) => /from\s*'[^']*\/diagnostics'/.test(source))
    .map(([path]) => path);
  expect(importers, 'nothing imports diagnostics.ts').not.toStrictEqual([]);
});

test('a module that wants only the type does not reach past it', () => {
  // Stated as a property rather than as a list of blessed files. A module
  // that also calls `render` or `hashSource` already has the bridge in its
  // import graph and buys nothing by splitting the type off (`App.tsx`); a
  // module that takes types alone is exactly the caller `diagnostics.ts` was
  // written for. The guarded set is read from that module's own re-export
  // list, so a fourth type added there is covered without anybody
  // remembering to widen this.
  const guarded = forwarded();
  const offenders: string[] = [];
  for (const [path, source] of Object.entries(sources)) {
    if (path === SELF || path === BRIDGE) continue;
    const { types, values } = fromBridge(source);
    if (values.length > 0) continue;
    for (const name of types) {
      if (guarded.has(name)) offenders.push(`${path} imports \`${name}\``);
    }
  }
  expect(
    offenders,
    'these take a diagnostic type straight from the wasm bridge and take \
nothing else from it; `diagnostics.ts` is the module they are meant to take it \
from, and it exists for no other reason',
  ).toStrictEqual([]);
});

test('the reader finds the tree it is scanning', () => {
  // Every assertion above is satisfied by an empty glob.
  expect(Object.keys(sources).length).toBeGreaterThan(10);
});
