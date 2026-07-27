// The editor's parse-state backbone, ported from the sibling aozora
// playground's editor/parserState.ts.
//
// A single CodeMirror StateField owns one `AozoraDocument` (the raw 青空文庫
// parser handle from aozora-flavored-markdown-wasm) per source revision, runs every wire
// query once, pre-parses the JSON, and builds the UTF-16 <-> UTF-8
// offset tables. Every other editor assist (decorations / linter /
// hover / inlay / fold / linked-ranges) reads from
// `view.state.field(parserStateField)` instead of touching the handle.
//
// This handle is the Aozora parser directly, NOT the aozora-md pipeline, so
// its spans are in source coordinates — which is what the assists need.

import type { EditorState, Transaction } from '@codemirror/state';
import { StateField } from '@codemirror/state';

import { AozoraDocument } from '../wasm-loader';

/** A single container fold range, both endpoints in UTF-16 code units. */
export interface ContainerFold {
  openLineEnd: number;
  closeStart: number;
}

/** A `nodesJson` entry, post-parse. Spans are UTF-8 byte offsets. */
export interface NodeEntry {
  kind: string;
  span: { start: number; end: number };
}

/** A `diagnosticsJson` entry. */
export interface DiagnosticEntry {
  kind: string;
  span: { start: number; end: number };
  codepoint?: number;
}

/** A `pairsJson` entry — matched open/close bracket pair. */
export interface PairEntry {
  kind: string;
  open: { start: number; end: number };
  close: { start: number; end: number };
}

/** A `gaijiResolutionsJson` entry. */
export interface GaijiResolutionEntry {
  span: { start: number; end: number };
  description: string;
  mencode: string | null;
  codepoint: number | null;
  resolved: string | null;
}

/** One row of `profileJson`. */
export interface ProfilePhaseEntry {
  name: string;
  duration_ms: number;
}

export interface ParserState {
  doc: AozoraDocument | null;
  source: string;
  nodesJson: string;
  diagJson: string;
  pairsJson: string;
  gaijiResJson: string;
  nodes: NodeEntry[];
  diagnostics: DiagnosticEntry[];
  pairs: PairEntry[];
  gaijiResolutions: GaijiResolutionEntry[];
  parseDurationMs: number;
  byteLen: number;
  u2b: Uint32Array;
  b2u: Uint32Array;
  containerFolds: ContainerFold[];
  profile: ProfilePhaseEntry[];
}

const EMPTY_ENVELOPE = '{"schemaVersion":3,"data":[]}';

const EMPTY_PARSER_STATE: ParserState = {
  doc: null,
  source: '',
  nodesJson: EMPTY_ENVELOPE,
  diagJson: EMPTY_ENVELOPE,
  pairsJson: EMPTY_ENVELOPE,
  gaijiResJson: EMPTY_ENVELOPE,
  nodes: [],
  diagnostics: [],
  pairs: [],
  gaijiResolutions: [],
  parseDurationMs: 0,
  byteLen: 0,
  u2b: new Uint32Array(1),
  b2u: new Uint32Array(1),
  containerFolds: [],
  profile: [],
};

/**
 * Build UTF-16 <-> UTF-8 offset translation tables for `source`.
 * `u2b[i]` is the UTF-8 byte offset where the i-th UTF-16 code unit
 * starts; `b2u[j]` is the UTF-16 code unit index for the character that
 * contains byte j. The high surrogate of an astral character owns all 4
 * bytes; the low surrogate contributes 0, keeping `u2b` monotonic.
 */
export function buildOffsetTables(source: string): {
  u2b: Uint32Array;
  b2u: Uint32Array;
  byteLen: number;
} {
  const len = source.length;
  const u2b = new Uint32Array(len + 1);
  let byte = 0;
  for (let i = 0; i < len; i++) {
    u2b[i] = byte;
    const code = source.charCodeAt(i);
    if (code < 0x80) byte += 1;
    else if (code < 0x800) byte += 2;
    else if (code >= 0xd800 && code < 0xdc00) byte += 4;
    else if (code >= 0xdc00 && code < 0xe000) byte += 0;
    else byte += 3;
  }
  u2b[len] = byte;
  const b2u = new Uint32Array(byte + 1);
  let utf16 = 0;
  for (let bi = 0; bi <= byte; bi++) {
    while (utf16 < len && (u2b[utf16 + 1] ?? byte) <= bi) {
      utf16++;
    }
    b2u[bi] = utf16;
  }
  return { u2b, b2u, byteLen: byte };
}

export interface ParseCallbacks {
  onParse?: (payload: ParserState) => void;
}

let callbacks: ParseCallbacks = {};

export function setParseCallbacks(cb: ParseCallbacks): void {
  callbacks = cb;
}

function safeParseData<T>(json: string): T[] {
  try {
    const env = JSON.parse(json) as { data?: T[] };
    return env.data ?? [];
  } catch {
    return [];
  }
}

function utf16AtByte(b2u: Uint32Array, byteOffset: number): number {
  if (byteOffset < 0) return 0;
  if (byteOffset >= b2u.length) return b2u[b2u.length - 1] ?? 0;
  return b2u[byteOffset] ?? 0;
}

/**
 * Derive container fold ranges from the node stream. A `containerOpen`
 * is matched with its balancing `containerClose`; the fold runs from
 * the end of the open marker's line to the start of the close marker.
 */
function deriveContainerFolds(
  source: string,
  nodes: NodeEntry[],
  b2u: Uint32Array,
): ContainerFold[] {
  const folds: ContainerFold[] = [];
  const stack: NodeEntry[] = [];
  for (const entry of nodes) {
    if (entry.kind === 'containerOpen') {
      stack.push(entry);
    } else if (entry.kind === 'containerClose') {
      const opened = stack.pop();
      if (opened === undefined) continue;
      const openEndU16 = utf16AtByte(b2u, opened.span.end);
      const closeStartU16 = utf16AtByte(b2u, entry.span.start);
      const nlIdx = source.indexOf('\n', openEndU16);
      const lineEnd = nlIdx === -1 ? openEndU16 : nlIdx;
      if (closeStartU16 > lineEnd) {
        folds.push({ openLineEnd: lineEnd, closeStart: closeStartU16 });
      }
    }
  }
  return folds;
}

function computeParserState(
  prev: ParserState | null,
  source: string,
): ParserState {
  prev?.doc?.free();
  if (source === '') {
    const tables = buildOffsetTables('');
    const ps: ParserState = {
      ...EMPTY_PARSER_STATE,
      source: '',
      u2b: tables.u2b,
      b2u: tables.b2u,
    };
    callbacks.onParse?.(ps);
    return ps;
  }
  const t0 = performance.now();
  const doc = new AozoraDocument(source);
  const nodesJson = doc.nodesJson();
  const parseDurationMs = performance.now() - t0;
  const diagJson = doc.diagnosticsJson();
  const pairsJson = doc.pairsJson();
  const gaijiResJson = doc.gaijiResolutionsJson();
  const byteLen = doc.sourceByteLen();
  const tables = buildOffsetTables(source);

  const nodes = safeParseData<NodeEntry>(nodesJson);
  const diagnostics = safeParseData<DiagnosticEntry>(diagJson);
  const pairs = safeParseData<PairEntry>(pairsJson);
  const gaijiResolutions = safeParseData<GaijiResolutionEntry>(gaijiResJson);
  const profile = safeParseData<ProfilePhaseEntry>(doc.profileJson());

  const containerFolds = deriveContainerFolds(source, nodes, tables.b2u);
  const ps: ParserState = {
    doc,
    source,
    nodesJson,
    diagJson,
    pairsJson,
    gaijiResJson,
    nodes,
    diagnostics,
    pairs,
    gaijiResolutions,
    parseDurationMs,
    byteLen,
    u2b: tables.u2b,
    b2u: tables.b2u,
    containerFolds,
    profile,
  };
  callbacks.onParse?.(ps);
  return ps;
}

/**
 * The single AozoraDocument owner. Every editor assist reads parsed data from
 * `view.state.field(parserStateField)`; the previous handle is
 * `.free()`-ed inside `computeParserState` to keep the WASM heap stable.
 */
export const parserStateField = StateField.define<ParserState>({
  create(state: EditorState): ParserState {
    return computeParserState(null, state.doc.toString());
  },
  update(value: ParserState, tr: Transaction): ParserState {
    if (!tr.docChanged) return value;
    return computeParserState(value, tr.newDoc.toString());
  },
});

/** UTF-16 code unit offset -> UTF-8 byte offset (clamped). */
export function utf16ToByte(ps: ParserState, u16: number): number {
  if (u16 < 0) return 0;
  if (u16 >= ps.u2b.length) return ps.u2b[ps.u2b.length - 1] ?? 0;
  return ps.u2b[u16] ?? 0;
}

/** UTF-8 byte offset -> UTF-16 code unit offset (clamped). */
export function byteToUtf16(ps: ParserState, byte: number): number {
  return utf16AtByte(ps.b2u, byte);
}
