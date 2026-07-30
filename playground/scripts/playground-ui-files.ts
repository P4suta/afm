import { readdir } from 'node:fs/promises';
import { join } from 'node:path';

export const PLAYGROUND_UI_FILES = [
  'package.json',
  'src/PlaygroundApp.test.tsx',
  'src/PlaygroundApp.tsx',
  'src/catalog.ts',
  'src/index.ts',
  'src/share.test.ts',
  'src/share.ts',
  'src/storage.test.ts',
  'src/storage.ts',
  'src/testing/adapterContract.ts',
  'src/types.ts',
] as const;

export async function listFiles(root: string, prefix = ''): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(join(root, prefix), {
    withFileTypes: true,
  })) {
    const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      files.push(...(await listFiles(root, relativePath)));
    } else {
      files.push(relativePath);
    }
  }
  return files.sort();
}

export function compareFileSet(
  actualFiles: readonly string[],
  expectedFiles: readonly string[] = PLAYGROUND_UI_FILES,
): { readonly extra: readonly string[]; readonly missing: readonly string[] } {
  const actual = new Set(actualFiles);
  const expected = new Set(expectedFiles);
  return {
    extra: [...actual].filter((file) => !expected.has(file)).sort(),
    missing: [...expected].filter((file) => !actual.has(file)).sort(),
  };
}
