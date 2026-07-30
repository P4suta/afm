import * as wasm from 'aozora-flavored-markdown-wasm';
import { expect, test } from 'vitest';

const PUBLIC_EXPORTS = [
  'AozoraDocument',
  'hashSource',
  'initPanicHook',
  'render',
  'renderAozoraOnly',
  'renderBlocks',
  'slugsJson',
] as const;

test('the built package exports exactly the supported runtime surface', () => {
  expect(Object.keys(wasm).sort()).toStrictEqual([...PUBLIC_EXPORTS].sort());
});

test('the built package preserves JSON, HTML, IR, diagnostics, and editor assists', () => {
  const source = '# ｜青梅《おうめ》\n\n本文 ※［＃二の字点、1-2-22］';
  const rendered = wasm.render(source);

  expect(rendered.ir.blocks[0]?.kind).toBe('heading');
  expect(rendered.html).toContain('<ruby>');
  expect(rendered.diagnostics).toStrictEqual([]);
  expect(wasm.render(source, { aozora: false }).html).not.toContain('<ruby>');

  const blocks = wasm.renderBlocks(source);
  expect(blocks.blocks.length).toBeGreaterThan(1);
  expect(blocks.blocks.every((block) => block.sourceLine >= 1)).toBe(true);

  const documentHandle = new wasm.AozoraDocument(source);
  try {
    const nodes = JSON.parse(documentHandle.nodesJson()) as {
      schemaVersion: number;
      data: unknown[];
    };
    expect(nodes.schemaVersion).toBeGreaterThan(0);
    expect(nodes.data.length).toBeGreaterThan(0);
    expect(documentHandle.sourceByteLen()).toBe(
      new TextEncoder().encode(source).length,
    );
  } finally {
    documentHandle.free();
  }
});

test('unknown and retired option keys throw at the actual wasm ABI', () => {
  expect(() =>
    wasm.render('text', {
      aozoraEnabled: false,
    } as never),
  ).toThrow();
  expect(() =>
    wasm.render('text', {
      unknown: true,
    } as never),
  ).toThrow();
});
