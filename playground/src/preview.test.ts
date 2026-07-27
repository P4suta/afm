import { expect, test } from 'vitest';

import * as main from './main';
import * as preview from './preview';

// Two modules that are deliberately empty. `preview.ts` was the imperative
// render loop before it moved inside `App.tsx`; `main.ts` was the bootstrap
// before `main.tsx`. Each says, in its own header, that it is kept so a stale
// `import './preview'` does not "silently resolve to a stale imperative
// renderer" — i.e. the file is a guard, and the thing it guards against is
// content appearing in it again.
//
// Nothing enforced that. `tsc` is happy either way, and a second render loop
// in `preview.ts` would be a module that runs, subscribes and paints beside
// the one in `App.tsx` — two renderers over one preview element, which is the
// exact failure the tombstone was left behind to prevent.
//
// Table rather than a test each, so the next superseded module is one row.
const TOMBSTONES: readonly (readonly [
  string,
  Record<string, unknown>,
  string,
])[] = [
  ['./preview', preview, 'superseded by the render loop inside App.tsx'],
  ['./main', main, 'superseded by main.tsx, the Solid entry point'],
];

test('a superseded module stayed superseded', () => {
  for (const [name, namespace, why] of TOMBSTONES) {
    expect(
      Object.keys({ ...namespace }),
      `${name} has grown a runtime export again — it is ${why}, so whatever \
this is now runs alongside the thing that replaced it`,
    ).toStrictEqual([]);
  }
});

test('the tombstones are still modules an import resolves to', () => {
  // Deleting one is the other half of the failure: `import './preview'` would
  // then fall through to whatever a resolver finds next, which is what the
  // file was left behind to stop. Importing it above is the assertion; this
  // one only records that the list is not empty.
  expect(TOMBSTONES.length).toBeGreaterThan(0);
});
