import { describe, expect, it } from 'vitest';

import {
  buildLineRanges,
  buildUtf8OffsetTable,
  normalizeDiagnostics,
  normalizeOutline,
} from './adapter-engine';
import type { IrBlock, IrDocument } from './wasm-loader';

describe('analysis source indexes', () => {
  it('maps UTF-8 bytes to browser UTF-16 offsets without per-character encoding', () => {
    expect(Array.from(buildUtf8OffsetTable('a青𠮷z'))).toEqual([
      0, 1, 1, 1, 2, 2, 2, 2, 4, 5,
    ]);
  });

  it('normalizes astral diagnostic ranges and keeps localized messages', () => {
    expect(
      normalizeDiagnostics('😀》', [
        {
          severity: 'error',
          source: 'source',
          code: 'aozora::lex::unmatched_close',
          message: 'unmatched close delimiter',
          span: { start: 4, end: 7 },
        },
      ]),
    ).toEqual([
      {
        severity: 'error',
        message: {
          ja: '対応する開き括弧がありません。',
          en: 'unmatched close delimiter',
        },
        range: { start: 2, end: 3 },
        code: 'aozora::lex::unmatched_close',
      },
    ]);
    expect(normalizeDiagnostics('x'.repeat(100_000), [])).toEqual([]);
  });

  it('indexes LF, CRLF, and lone CR lines once', () => {
    expect(buildLineRanges('one\r\ntwo\rthree\n')).toEqual([
      { start: 0, end: 3 },
      { start: 5, end: 8 },
      { start: 9, end: 14 },
      { start: 15, end: 15 },
    ]);
  });

  it('normalizes a heading-heavy CRLF outline with indexed line ranges', () => {
    const count = 2_000;
    const lines = Array.from({ length: count }, (_, index) => `# H${index}`);
    const blocks: IrBlock[] = lines.map((_, index) => ({
      kind: 'heading',
      level: 1,
      children: [{ kind: 'text', value: `H${index}` }],
      sourceLine: index + 1,
    }));
    const source = lines.join('\r\n');
    const outline = normalizeOutline(source, {
      blocks,
    } satisfies IrDocument);

    expect(outline).toHaveLength(count);
    const last = outline.at(-1);
    expect(last?.text).toBe(`H${count - 1}`);
    expect(source.slice(last?.range?.start, last?.range?.end)).toBe(
      `# H${count - 1}`,
    );
  });
});
