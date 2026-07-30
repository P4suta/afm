import {
  CompletionContext,
  type CompletionResult,
} from '@codemirror/autocomplete';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import * as wasm from 'aozora-flavored-markdown-wasm';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { setEditorLocale } from '../i18n';
import { initializeWasm } from '../wasm-loader';
import { aozoraMdCompletionSource } from './completion';

async function complete(
  source: string,
  explicit: boolean,
): Promise<CompletionResult | null> {
  const state = EditorState.create({ doc: source });
  return await aozoraMdCompletionSource(
    new CompletionContext(state, state.doc.length, explicit),
  );
}

async function applyPageBreak(
  opener: string,
  closing: ']' | '］',
): Promise<string> {
  const source = `${opener}改${closing}`;
  const to = opener.length + 1;
  const state = EditorState.create({
    doc: source,
    selection: { anchor: to },
  });
  const completionResult = await aozoraMdCompletionSource(
    new CompletionContext(state, to, false),
  );
  if (!completionResult) {
    throw new TypeError('Expected slug completions');
  }
  const completion = completionResult.options.find(
    (option) => option.label === '改ページ',
  );
  if (!completion || typeof completion.apply !== 'function') {
    throw new TypeError('Expected a callable slug completion');
  }

  const view = new EditorView({ state });
  completion.apply(
    view,
    completion,
    completionResult.from,
    completionResult.to ?? to,
  );
  const output = view.state.doc.toString();
  view.destroy();
  return output;
}

describe('Aozora completion activation', () => {
  beforeAll(initializeWasm);
  afterEach(() => setEditorLocale('ja'));

  it.each([
    ['#', '＃ Annotation'],
    ['|', '｜ Ruby (explicit)'],
  ])(
    'offers the ASCII %s snippet only after an explicit request',
    async (trigger, label) => {
      setEditorLocale('en');
      expect(await complete(trigger, false)).toBeNull();

      const result = await complete(trigger, true);
      expect(result?.options).toHaveLength(1);
      expect(result?.options[0]?.label).toBe(label);
    },
  );

  it.each(['＃', '｜'])(
    'continues to offer the full-width %s snippet while typing',
    async (trigger) => {
      expect((await complete(trigger, false))?.options).toHaveLength(1);
    },
  );
});

describe('Aozora slug completion application', () => {
  it.each([
    ['［＃', '］'],
    ['［＃', ']'],
    ['［#', '］'],
    ['［#', ']'],
    ['[＃', '］'],
    ['[＃', ']'],
    ['[#', '］'],
    ['[#', ']'],
  ] as const)(
    'canonicalizes the %s opener and %s closer',
    async (opener, closing) => {
      const source = await applyPageBreak(opener, closing);
      expect(source).toBe('［＃改ページ］');

      const rendered = wasm.render(source);
      expect(rendered.diagnostics).toStrictEqual([]);
      expect(rendered.html).toContain(
        '<div class="aozora-md-page-break"></div>',
      );
    },
  );
});
