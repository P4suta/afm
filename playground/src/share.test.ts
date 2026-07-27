import { expect, test } from 'vitest';

import { decodeSourceFromHash, encodeSourceToHash } from './share';

// The one module that runs on input the app did not produce: a share link is
// pasted, forwarded, truncated by a chat client, and then read at boot before
// anything is on screen. `decodeSourceFromHash` returning instead of throwing
// on every one of those shapes is what keeps a mangled link from being a
// blank page.

test('a link this module writes is one it reads back', () => {
  for (const source of [
    '# 見出し\n\n|青梅《おうめ》の実\n',
    'a'.repeat(20_000),
    '#&=%20+/\\',
    '\u{1f363}\u{10ffff}',
  ]) {
    expect(decodeSourceFromHash(encodeSourceToHash(source))).toBe(source);
  }
});

test('the hash it writes is one a URL can carry unescaped', () => {
  const hash = encodeSourceToHash('｜青梅《おうめ》');
  expect(hash.startsWith('#src=')).toBe(true);
  // `URL` re-encodes anything a fragment may not hold verbatim; a hash that
  // survives the round trip unchanged is one a copy-paste cannot corrupt.
  const url = new URL('https://example.invalid/');
  url.hash = hash;
  expect(url.hash).toBe(hash);
});

// Every shape a hash can arrive in, and what it must decode to. `null` means
// "no source in this link" — the boot path then keeps its default document
// instead of showing an empty editor.
const HASHES: readonly (readonly [string, string | null, string])[] = [
  ['', null, 'no fragment at all'],
  ['#', null, 'a bare `#`, which is what a stripped link leaves behind'],
  ['#other=x', null, 'a fragment this app does not own'],
  ['#src', null, 'the key with no `=` and no value'],
  ['#src=', null, 'the key with an empty value'],
  [
    '#src=!!!not-lz-string!!!',
    null,
    'a payload lz-string decompresses to null at run time — its own type \
declaration says `string`, so this branch is unreachable to the type checker \
and is exactly the one a truncated link takes',
  ],
];

test('a hash that carries no readable source decodes to null, never throws', () => {
  for (const [hash, expected, why] of HASHES) {
    expect(decodeSourceFromHash(hash), why).toBe(expected);
  }
});

test('the source is found beside other fragment keys, in either position', () => {
  const encoded = encodeSourceToHash('本文').slice('#src='.length);
  expect(decodeSourceFromHash(`#tab=preview&src=${encoded}`)).toBe('本文');
  expect(decodeSourceFromHash(`#src=${encoded}&tab=preview`)).toBe('本文');
});

test('a hash is read with or without its leading marker', () => {
  const hash = encodeSourceToHash('本文');
  expect(decodeSourceFromHash(hash.slice(1))).toBe('本文');
});

test('an empty document round-trips to null rather than to an empty string', () => {
  // Asymmetry worth pinning rather than fixing: `""` and "no source" are the
  // same answer here on purpose, so sharing a cleared editor re-opens on the
  // starter document instead of on a blank page.
  expect(decodeSourceFromHash(encodeSourceToHash(''))).toBeNull();
});
