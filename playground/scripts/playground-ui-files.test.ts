import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import { compareFileSet, listFiles } from './playground-ui-files';

const temporaryRoots: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryRoots.splice(0).map((root) => rm(root, { recursive: true })),
  );
});

describe('playground UI vendor file set', () => {
  it('lists nested files and reports both stale and missing entries', async () => {
    const root = await mkdtemp(join(tmpdir(), 'afm-playground-ui-'));
    temporaryRoots.push(root);
    await mkdir(join(root, 'src'), { recursive: true });
    await writeFile(join(root, 'package.json'), '{}');
    await writeFile(join(root, 'src', 'stale.ts'), '');

    const actual = await listFiles(root);
    expect(actual).toEqual(['package.json', 'src/stale.ts']);
    expect(compareFileSet(actual, ['package.json', 'src/index.ts'])).toEqual({
      extra: ['src/stale.ts'],
      missing: ['src/index.ts'],
    });
  });
});
