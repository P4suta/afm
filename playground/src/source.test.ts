import { describe, expect, it } from 'vitest';

import { normalizeSourceLineEndings } from './source';

describe('external source normalization', () => {
  it('canonicalizes CRLF and lone CR without changing LF or astral text', () => {
    expect(normalizeSourceLineEndings('first\r\nsecond\rthird\n𠮷')).toBe(
      'first\nsecond\nthird\n𠮷',
    );
    expect(normalizeSourceLineEndings('already\ncanonical')).toBe(
      'already\ncanonical',
    );
  });
});
