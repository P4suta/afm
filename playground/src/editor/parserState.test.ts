import { EditorState } from '@codemirror/state';
import { describe, expect, it, vi } from 'vitest';
import 'aozora-flavored-markdown-wasm';
import { AozoraDocument } from '../wasm-loader';
import {
  buildOffsetTables,
  byteToUtf16,
  type ParserState,
  parserStateField,
  utf16ToByte,
} from './parserState';

function coordinateState(source: string): ParserState {
  const tables = buildOffsetTables(source);
  return {
    doc: null,
    source,
    nodes: [],
    diagnostics: [],
    pairs: [],
    gaijiResolutions: [],
    u2b: tables.u2b,
    b2u: tables.b2u,
    containerFolds: [],
  };
}

describe('UTF-8 / UTF-16 source coordinates', () => {
  it('maps ASCII, BMP Japanese, and astral surrogate pairs in both directions', () => {
    const state = coordinateState('a青𠮷z');
    expect(Array.from(state.u2b)).toEqual([0, 1, 4, 8, 8, 9]);
    expect([0, 1, 2, 3, 4, 5].map((at) => utf16ToByte(state, at))).toEqual([
      0, 1, 4, 8, 8, 9,
    ]);
    expect(
      [0, 1, 3, 4, 5, 7, 8, 9].map((at) => byteToUtf16(state, at)),
    ).toEqual([0, 1, 1, 2, 2, 2, 4, 5]);
  });

  it('clamps coordinates outside the document', () => {
    const state = coordinateState('青');
    expect(utf16ToByte(state, -1)).toBe(0);
    expect(utf16ToByte(state, 99)).toBe(3);
    expect(byteToUtf16(state, -1)).toBe(0);
    expect(byteToUtf16(state, 99)).toBe(1);
  });
});

describe('editor parser projections', () => {
  it('keeps every projection and offset table required by editor assists', () => {
    const source = [
      '# ｜見出し《みだし》',
      '',
      '本文 ※［＃二の字点、1-2-22］ です',
      '',
      '［＃ここから字下げ］',
      '字下げ本文',
      '［＃ここで字下げ終わり］',
      '',
      'orphan》close',
    ].join('\n');
    const state = EditorState.create({
      doc: source,
      extensions: [parserStateField],
    });
    const parserState = state.field(parserStateField);

    try {
      expect(parserState.source).toBe(source);
      expect(parserState.doc).toBeInstanceOf(AozoraDocument);
      expect(parserState.nodes.length).toBeGreaterThan(0);
      expect(parserState.diagnostics.length).toBeGreaterThan(0);
      expect(parserState.pairs.length).toBeGreaterThan(0);
      expect(parserState.gaijiResolutions.length).toBeGreaterThan(0);
      expect(parserState.containerFolds.length).toBeGreaterThan(0);
      expect(parserState.u2b).toHaveLength(source.length + 1);
      expect(parserState.b2u.length).toBeGreaterThan(source.length);
    } finally {
      parserState.doc?.free();
    }
  });

  it('queries only WASM projections consumed by editor assists', () => {
    const requiredMethods = [
      'nodesJson',
      'diagnosticsJson',
      'pairsJson',
      'gaijiResolutionsJson',
    ] as const;
    const requiredSpies = requiredMethods.map((method) =>
      vi.spyOn(AozoraDocument.prototype, method),
    );
    const sourceByteLen = vi.spyOn(AozoraDocument.prototype, 'sourceByteLen');
    const profileJson = vi.spyOn(AozoraDocument.prototype, 'profileJson');
    const state = EditorState.create({
      doc: '｜漢字《かんじ》',
      extensions: [parserStateField],
    });
    const parserState = state.field(parserStateField);

    try {
      for (const spy of requiredSpies) {
        expect(spy).toHaveBeenCalledOnce();
      }
      expect(sourceByteLen).not.toHaveBeenCalled();
      expect(profileJson).not.toHaveBeenCalled();
    } finally {
      parserState.doc?.free();
      vi.restoreAllMocks();
    }
  });
});
