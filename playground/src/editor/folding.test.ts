import { foldable } from '@codemirror/language';
import { EditorState } from '@codemirror/state';
import { beforeAll, describe, expect, it } from 'vitest';

import { initializeWasm } from '../wasm-loader';
import { aozoraFolding } from './folding';
import { parserStateField } from './parserState';

function createState(source: string): EditorState {
  return EditorState.create({
    doc: source,
    extensions: [parserStateField, aozoraFolding],
  });
}

function freeState(state: EditorState): void {
  state.field(parserStateField).doc?.free();
}

function foldAtLine(state: EditorState, lineNumber: number) {
  const line = state.doc.line(lineNumber);
  return foldable(state, line.from, line.to);
}

describe('Aozora container folding', () => {
  beforeAll(initializeWasm);

  it('folds a real container from the opener line through its closer', () => {
    const source = [
      '😀前',
      '［＃ここから２字下げ］',
      '本文',
      '［＃ここで字下げ終わり］',
      '後',
    ].join('\n');
    const state = createState(source);

    try {
      const range = foldAtLine(state, 2);
      expect(range).not.toBeNull();
      if (!range) return;

      expect(range.from).toBe(state.doc.line(2).to);
      expect(range.to).toBe(state.doc.line(4).from);
      expect(state.doc.sliceString(range.from, range.to)).toBe('\n本文\n');
      expect(foldAtLine(state, 1)).toBeNull();
      expect(foldAtLine(state, 3)).toBeNull();
      expect(foldAtLine(state, 4)).toBeNull();
    } finally {
      freeState(state);
    }
  });

  it('returns the independently nested range for each opener', () => {
    const source = [
      '［＃ここから２字下げ］',
      '外側',
      '［＃罫囲み］',
      '内側',
      '［＃罫囲み終わり］',
      '外側',
      '［＃ここで字下げ終わり］',
    ].join('\n');
    const state = createState(source);

    try {
      const outer = foldAtLine(state, 1);
      const inner = foldAtLine(state, 3);
      expect(outer).toEqual({
        from: state.doc.line(1).to,
        to: state.doc.line(7).from,
      });
      expect(inner).toEqual({
        from: state.doc.line(3).to,
        to: state.doc.line(5).from,
      });
    } finally {
      freeState(state);
    }
  });

  it('offers no fold for plain text or an orphan closer', () => {
    for (const source of ['plain text', '［＃ここで字下げ終わり］', '']) {
      const state = createState(source);
      try {
        expect(foldAtLine(state, 1)).toBeNull();
      } finally {
        freeState(state);
      }
    }
  });
});
