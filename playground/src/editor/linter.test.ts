import {
  type Diagnostic,
  diagnosticCount,
  forceLinting,
  forEachDiagnostic,
} from '@codemirror/lint';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { setEditorLocale } from '../i18n';
import { AozoraDocument, initializeWasm } from '../wasm-loader';
import { aozoraMdLinter } from './linter';
import { parserStateField } from './parserState';

interface LocatedDiagnostic extends Diagnostic {
  readonly mappedFrom: number;
  readonly mappedTo: number;
}

function createView(source: string): EditorView {
  const parent = document.createElement('div');
  document.body.append(parent);
  return new EditorView({
    parent,
    state: EditorState.create({
      doc: source,
      extensions: [parserStateField, aozoraMdLinter],
    }),
  });
}

function destroyView(view: EditorView): void {
  view.state.field(parserStateField).doc?.free();
  const parent = view.dom.parentElement;
  view.destroy();
  parent?.remove();
}

async function collectDiagnostics(
  view: EditorView,
  expectedCount: number,
): Promise<LocatedDiagnostic[]> {
  forceLinting(view);
  await vi.waitFor(() =>
    expect(diagnosticCount(view.state)).toBe(expectedCount),
  );
  const diagnostics: LocatedDiagnostic[] = [];
  forEachDiagnostic(view.state, (diagnostic, mappedFrom, mappedTo) => {
    diagnostics.push({ ...diagnostic, mappedFrom, mappedTo });
  });
  return diagnostics;
}

describe('Aozora source linter', () => {
  beforeAll(initializeWasm);

  afterEach(() => {
    setEditorLocale('ja');
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it('maps real UTF-8 parser diagnostics to UTF-16 editor ranges', async () => {
    setEditorLocale('en');
    const source = '😀》\u{e002}';
    const view = createView(source);

    try {
      const rawDiagnostics =
        view.state.field(parserStateField).diagnostics.length;
      expect(rawDiagnostics).toBeGreaterThanOrEqual(2);
      const diagnostics = await collectDiagnostics(view, rawDiagnostics);

      const unmatched = diagnostics.find(
        ({ message }) => message === 'No matching open bracket',
      );
      expect(unmatched).toMatchObject({
        severity: 'error',
        source: 'aozora-md',
        mappedFrom: 2,
        mappedTo: 3,
      });
      expect(source.slice(unmatched?.mappedFrom, unmatched?.mappedTo)).toBe(
        '》',
      );

      const privateUse = diagnostics.find(({ message }) =>
        message.includes('U+E002'),
      );
      expect(privateUse).toMatchObject({
        severity: 'warning',
        source: 'aozora-md',
        mappedFrom: 3,
        mappedTo: 4,
      });
      expect(source.slice(privateUse?.mappedFrom, privateUse?.mappedTo)).toBe(
        '\u{e002}',
      );
    } finally {
      destroyView(view);
    }
  });

  it('reports an unclosed bracket with the selected locale', async () => {
    const view = createView('前《');

    try {
      const rawDiagnostics =
        view.state.field(parserStateField).diagnostics.length;
      const diagnostics = await collectDiagnostics(view, rawDiagnostics);
      expect(diagnostics).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            severity: 'error',
            message: '括弧が閉じられていません',
            mappedFrom: 1,
            mappedTo: 2,
          }),
        ]),
      );
    } finally {
      destroyView(view);
    }
  });

  it('widens zero-width ABI spans and preserves unknown kinds as info', async () => {
    vi.spyOn(AozoraDocument.prototype, 'diagnosticsJson').mockReturnValue(
      JSON.stringify({
        data: [
          {
            kind: 'source_contains_pua',
            span: { start: 0, end: 0 },
          },
          {
            kind: 'residual_annotation_marker',
            span: { start: 3, end: 3 },
          },
          {
            kind: 'future_diagnostic',
            span: { start: 1, end: 2 },
          },
          {
            kind: 'unclosed_bracket',
            span: { start: 999, end: 999 },
          },
          {
            kind: 'reversed_span',
            span: { start: 3, end: 1 },
          },
        ],
      }),
    );
    const view = createView('abc');

    try {
      // CodeMirror groups the two end-of-document diagnostics into one
      // marked range while forEachDiagnostic still returns both entries.
      const diagnostics = await collectDiagnostics(view, 3);
      expect(diagnostics).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            severity: 'warning',
            message: '私用領域文字が含まれています（不明）',
            mappedFrom: 0,
            mappedTo: 1,
          }),
          expect.objectContaining({
            severity: 'warning',
            message: '分類できない注記が残っています',
            mappedFrom: 2,
            mappedTo: 3,
          }),
          expect.objectContaining({
            severity: 'info',
            message: 'future_diagnostic',
            mappedFrom: 1,
            mappedTo: 2,
          }),
          expect.objectContaining({
            severity: 'error',
            message: '括弧が閉じられていません',
            mappedFrom: 2,
            mappedTo: 3,
          }),
        ]),
      );
      expect(
        diagnostics.some(({ message }) => message === 'reversed_span'),
      ).toBe(false);
    } finally {
      destroyView(view);
    }
  });

  it('returns no diagnostics for an empty document', async () => {
    const view = createView('');

    try {
      expect(await collectDiagnostics(view, 0)).toEqual([]);
    } finally {
      destroyView(view);
    }
  });
});
