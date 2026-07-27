// URL-shareable state.
//
// Encodes the editor source into `#src=<lz-string>` so a copy-paste of
// the current URL re-opens with the same content. lz-string adds ~3 KB
// gzipped and gives 50–70% compression on typical CJK text — well
// inside the practical URL-length limit for a "share a snippet" use case.

import LZString from 'lz-string';

const HASH_KEY = 'src';

export function encodeSourceToHash(source: string): string {
  const encoded = LZString.compressToEncodedURIComponent(source);
  return `#${HASH_KEY}=${encoded}`;
}

export function decodeSourceFromHash(hash: string): string | null {
  // location.hash includes the leading '#'.
  const stripped = hash.startsWith('#') ? hash.slice(1) : hash;
  if (stripped.length === 0) return null;
  for (const part of stripped.split('&')) {
    const eq = part.indexOf('=');
    if (eq < 0) continue;
    const k = part.slice(0, eq);
    if (k !== HASH_KEY) continue;
    const v = part.slice(eq + 1);
    // The TS declaration is `string`, but the runtime returns null on
    // malformed input. Treat both null and "" as "no source" so a
    // share-link copy/paste that got mangled never crashes the boot.
    const decoded = LZString.decompressFromEncodedURIComponent(v) as
      | string
      | null;
    if (decoded === null || decoded.length === 0) return null;
    return decoded;
  }
  return null;
}

export async function copyShareLink(source: string): Promise<void> {
  const url = new URL(globalThis.location.href);
  url.hash = encodeSourceToHash(source);
  await navigator.clipboard.writeText(url.toString());
}
