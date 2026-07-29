import { describe, expect, it } from 'vitest';

import {
  buildOffsetTables,
  byteToUtf16,
  type ParserState,
  utf16ToByte,
} from './parserState';

function coordinateState(source: string): ParserState {
  const tables = buildOffsetTables(source);
  return {
    doc: null,
    source,
    nodesJson: '',
    diagJson: '',
    pairsJson: '',
    gaijiResJson: '',
    nodes: [],
    diagnostics: [],
    pairs: [],
    gaijiResolutions: [],
    parseDurationMs: 0,
    byteLen: tables.byteLen,
    u2b: tables.u2b,
    b2u: tables.b2u,
    containerFolds: [],
    profile: [],
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
