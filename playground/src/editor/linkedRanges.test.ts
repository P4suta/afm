import { EditorState } from '@codemirror/state';
import { describe, expect, it } from 'vitest';
import 'aozora-flavored-markdown-wasm';
import { linkedRangesFilter } from './linkedRanges';
import { parserStateField } from './parserState';

function deleteText(source: string, deletedText: string): string {
  const from = source.indexOf(deletedText);
  if (from < 0) {
    throw new Error(`Expected source to contain ${deletedText}`);
  }

  const state = EditorState.create({
    doc: source,
    extensions: [parserStateField, linkedRangesFilter],
  });
  const transaction = state.update({
    changes: { from, to: from + deletedText.length },
  });

  try {
    return transaction.state.doc.toString();
  } finally {
    transaction.state.field(parserStateField).doc?.free();
  }
}

describe('linked range deletion', () => {
  it('deletes the closer at its mapped offset when the opener is deleted', () => {
    expect(deleteText('前｜漢字《かんじ》後', '《')).toBe('前｜漢字かんじ後');
  });

  it('deletes the opener when the closer is deleted', () => {
    expect(deleteText('前｜漢字《かんじ》後', '》')).toBe('前｜漢字かんじ後');
  });

  it('uses UTF-16 offsets around astral characters', () => {
    expect(deleteText('𠮷｜漢𠮷《かん𠮷じ》終', '《')).toBe(
      '𠮷｜漢𠮷かん𠮷じ終',
    );
  });
});
