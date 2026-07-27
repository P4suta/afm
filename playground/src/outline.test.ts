import { expect, test } from 'vitest';

// The IR type declarations wasm-pack generated, as text — the same file
// `tsc --noEmit` checks this tree against, reached through the `?raw` import
// `vite.config.ts`'s `server.fs.allow` already opens the crates tree for.
// `just playground-test` pulls `wasm-build` in before it runs, so this is the
// current build rather than a copy of one.
import WIRE_TYPES from '../../crates/aozora-flavored-markdown-wasm/pkg/aozora_flavored_markdown_wasm.d.ts?raw';

import { outlineFromIr } from './outline';
import type { IrBlock, IrDocument, IrInline } from './wasm-loader';

// `outlineFromIr` is two `switch` statements over the published IR, and both
// end in `default: break`. That is the right default for a projection — most
// variants contribute no heading — but it means a variant nobody enumerated
// is dropped in silence rather than reported. The wasm surface is versioned
// separately from this consumer, so "a variant was added and the outline
// stopped showing a heading" is a regression with no failing build anywhere.
//
// So the two tables below are the whole point of this file: each names every
// variant the IR can produce and what the outline does with it, and the last
// two tests hold the tables to the union types the wasm package actually
// declares. Adding a variant upstream fails here until someone decides.

const document = (...blocks: IrBlock[]): IrDocument => ({ blocks });

const text = (value: string): IrInline => ({ kind: 'text', value });

const heading = (children: IrInline[], level = 1): IrBlock => ({
  kind: 'heading',
  level,
  children,
});

/**
 * The `kind` tags of one discriminated-union type alias in the `.d.ts`.
 *
 * The declaration runs to the `;` that ends a line, not to the first `;` in
 * it: every variant is an object type, so its own members are `;`-separated
 * and a reader that stopped at one would see the first variant and report the
 * table as complete.
 */
function declaredKinds(alias: string): Set<string> {
  const declaration = new RegExp(
    `^export type ${alias} = ([\\s\\S]*?);$`,
    'm',
  ).exec(WIRE_TYPES);
  if (declaration === null || declaration[1] === undefined) {
    throw new Error(
      `the wasm package declares no \`export type ${alias}\` union any more; ` +
        'the outline reads its shape, so the change has to reach this file',
    );
  }
  const kinds = new Set<string>();
  for (const match of declaration[1].matchAll(/kind: "([A-Za-z]+)"/g)) {
    const kind = match[1];
    if (kind !== undefined) kinds.add(kind);
  }
  return kinds;
}

/**
 * Every inline variant, the node the test builds for it, and the heading text
 * it must contribute. An empty contribution is a decision and carries its
 * reason.
 */
const INLINE_CONTRIBUTION: Record<
  string,
  { readonly node: IrInline; readonly text: string; readonly why?: string }
> = {
  text: { node: text('梅'), text: '梅' },
  code: { node: { kind: 'code', value: 'render()' }, text: 'render()' },
  strong: {
    node: { kind: 'strong', children: [text('太')] },
    text: '太',
  },
  emphasis: {
    node: { kind: 'emphasis', children: [text('斜')] },
    text: '斜',
  },
  link: {
    node: { kind: 'link', href: '#a', children: [text('章')] },
    text: '章',
  },
  image: {
    // The alt text is the only part of an image a reader of an outline can
    // read, so it is what the entry carries.
    node: { kind: 'image', url: 'a.png', alt: [text('図')] },
    text: '図',
  },
  lineBreak: {
    node: { kind: 'lineBreak', hard: true },
    text: '',
    why: 'a break is layout, not text; a heading that wraps reads the same in a flat outline',
  },
  aozora: {
    // Ruby: the reading is in `<rt>`, the fallback parens in `<rp>`, and an
    // outline shows neither.
    node: {
      kind: 'aozora',
      aozoraKind: 'ruby',
      html: '<ruby>青梅<rp>（</rp><rt>おうめ</rt><rp>）</rp></ruby>',
    },
    text: '青梅',
  },
};

/**
 * Every block variant and how the walk treats it: the heading it becomes, the
 * children it descends into, or nothing — with the reason nothing is right.
 */
const BLOCK_TRAVERSAL: Record<
  string,
  {
    readonly block: IrBlock;
    readonly headings: string[];
    readonly why?: string;
  }
> = {
  heading: { block: heading([text('見出し')]), headings: ['見出し'] },
  blockquote: {
    block: { kind: 'blockquote', children: [heading([text('引用中')], 2)] },
    headings: ['引用中'],
  },
  list: {
    block: {
      kind: 'list',
      ordered: false,
      items: [{ children: [heading([text('項目中')], 3)] }],
    },
    headings: ['項目中'],
  },
  paragraph: {
    block: { kind: 'paragraph', children: [text('本文')] },
    headings: [],
    why: 'body text holds no heading',
  },
  codeBlock: {
    block: { kind: 'codeBlock', value: '# not a heading\n' },
    headings: [],
    why: 'a `#` inside a fence is code, and comrak has already decided that',
  },
  thematicBreak: {
    block: { kind: 'thematicBreak' },
    headings: [],
    why: 'no children and no text',
  },
  table: {
    block: {
      kind: 'table',
      header: { cells: [[text('列')]] },
      rows: [{ cells: [[text('値')]] }],
      align: ['default'],
    },
    headings: [],
    why: 'a cell holds inlines, so no heading can be inside one',
  },
  aozora: {
    block: {
      kind: 'aozora',
      aozoraKind: 'oomidashi',
      html: '<h2>大見出し</h2>',
    },
    headings: [],
    why: 'the IR walker promotes a 青空文庫 heading hint to a `heading` block \
(crates/aozora-flavored-markdown/src/ir/mod.rs), so what is left in this \
variant is the notations that are not headings',
  },
};

test('every inline variant contributes the text the table says it does', () => {
  for (const [kind, { node, text: expected }] of Object.entries(
    INLINE_CONTRIBUTION,
  )) {
    const outline = outlineFromIr(document(heading([text('前'), node])));
    expect(
      outline,
      `inline \`${kind}\` produced no outline entry`,
    ).toHaveLength(1);
    expect(
      outline[0]?.text,
      `inline \`${kind}\` contributed the wrong heading text`,
    ).toBe(`前${expected}`);
  }
});

test('every block variant is walked the way the table says it is', () => {
  for (const [kind, { block, headings }] of Object.entries(BLOCK_TRAVERSAL)) {
    const outline = outlineFromIr(document(block));
    expect(
      outline.map((entry) => entry.text),
      `block \`${kind}\` was not walked as declared`,
    ).toStrictEqual(headings);
  }
});

test('the inline table names every variant the wasm package can emit', () => {
  // The check the two tables exist for. `default: break` cannot fail, so this
  // is the only place a new variant is noticed before a user notices it.
  expect(new Set(Object.keys(INLINE_CONTRIBUTION))).toStrictEqual(
    declaredKinds('Inline'),
  );
});

test('the block table names every variant the wasm package can emit', () => {
  expect(new Set(Object.keys(BLOCK_TRAVERSAL))).toStrictEqual(
    declaredKinds('Block'),
  );
});

test('headings come out in document order, across nesting', () => {
  const outline = outlineFromIr(
    document(
      heading([text('一')], 1),
      { kind: 'paragraph', children: [text('本文')] },
      { kind: 'blockquote', children: [heading([text('二')], 2)] },
      {
        kind: 'list',
        ordered: true,
        items: [
          { children: [heading([text('三')], 3)] },
          { children: [heading([text('四')], 4)] },
        ],
      },
    ),
  );
  expect(outline.map((entry) => [entry.level, entry.text])).toStrictEqual([
    [1, '一'],
    [2, '二'],
    [3, '三'],
    [4, '四'],
  ]);
});

test('a source line is carried when the renderer attached one and is null otherwise', () => {
  const outline = outlineFromIr(
    document(
      { kind: 'heading', level: 1, children: [text('付き')], sourceLine: 12 },
      heading([text('無し')]),
    ),
  );
  expect(outline.map((entry) => entry.sourceLine)).toStrictEqual([12, null]);
});

test('a heading with no visible text falls back rather than vanishing', () => {
  // A heading whose only content is a ruby reading, or an empty `##`, still
  // occupies a line the reader can jump to — an empty label would make the
  // entry unclickable in the panel.
  const outline = outlineFromIr(
    document(heading([{ kind: 'lineBreak', hard: false }])),
  );
  expect(outline).toStrictEqual([
    { level: 1, text: '(無題)', sourceLine: null },
  ]);
});

test('a hidden directive wrapper is not heading text', () => {
  // `aozora-md-directive` is `hidden` in the stylesheet, so `textContent`
  // reports markup the reader never sees.
  const outline = outlineFromIr(
    document(
      heading([
        text('本題'),
        {
          kind: 'aozora',
          aozoraKind: 'kaipage',
          html: '<span class="aozora-md-directive">［＃改ページ］</span>',
        },
      ]),
    ),
  );
  expect(outline[0]?.text).toBe('本題');
});
