import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

import {
  compareFileSet,
  listFiles,
  PLAYGROUND_UI_FILES,
} from './playground-ui-files';

interface VendorLock {
  readonly schemaVersion: number;
  readonly state: 'bootstrap' | 'locked';
  readonly upstreamRepository: string;
  readonly upstreamCommit: string;
  readonly upstreamRepositoryTree: string;
  readonly upstreamPackagePath: string;
  readonly upstreamPackageTree: string | null;
  readonly snapshotSha256: string;
  readonly files: readonly string[];
}

function git(directory: string, ...arguments_: string[]): string {
  const process = Bun.spawnSync(['git', '-C', directory, ...arguments_]);
  if (process.exitCode !== 0) {
    throw new Error(process.stderr.toString().trim());
  }
  return process.stdout.toString().trim();
}

async function digest(root: string): Promise<string> {
  const hash = createHash('sha256');
  for (const file of PLAYGROUND_UI_FILES) {
    hash.update(file);
    hash.update('\0');
    hash.update(await readFile(join(root, file)));
    hash.update('\0');
  }
  return hash.digest('hex');
}

const vendorRoot = resolve('vendor/playground-ui');
const lock = JSON.parse(
  await readFile(resolve('vendor/playground-ui.lock.json'), 'utf8'),
) as VendorLock;

if (lock.schemaVersion !== 1) throw new Error('unsupported vendor lock schema');
if (JSON.stringify(lock.files) !== JSON.stringify(PLAYGROUND_UI_FILES)) {
  throw new Error('vendor lock allowlist differs from the sync allowlist');
}
const { extra, missing } = compareFileSet(await listFiles(vendorRoot));
if (extra.length > 0 || missing.length > 0) {
  throw new Error(
    `vendored playground-ui file set differs (extra: ${extra.join(', ') || 'none'}; missing: ${missing.join(', ') || 'none'})`,
  );
}
const vendorDigest = await digest(vendorRoot);
if (vendorDigest !== lock.snapshotSha256) {
  throw new Error(
    `vendored playground-ui digest differs: ${vendorDigest} != ${lock.snapshotSha256}`,
  );
}

const upstreamArgument = process.argv[2];
if (upstreamArgument) {
  if (lock.state !== 'locked' || lock.upstreamPackageTree === null) {
    throw new Error(
      'bootstrap snapshot has no canonical package tree to compare',
    );
  }
  const upstreamRoot = resolve(upstreamArgument);
  if (git(upstreamRoot, 'rev-parse', 'HEAD^{commit}') !== lock.upstreamCommit) {
    throw new Error('upstream checkout is not at the locked commit');
  }
  if (
    git(upstreamRoot, 'rev-parse', 'HEAD^{tree}') !==
    lock.upstreamRepositoryTree
  ) {
    throw new Error('upstream repository tree differs from the lock');
  }
  if (
    git(upstreamRoot, 'rev-parse', `HEAD:${lock.upstreamPackagePath}`) !==
    lock.upstreamPackageTree
  ) {
    throw new Error('upstream package tree differs from the lock');
  }
  for (const file of PLAYGROUND_UI_FILES) {
    const upstream = await readFile(
      join(upstreamRoot, lock.upstreamPackagePath, file),
    );
    const vendored = await readFile(join(vendorRoot, file));
    if (!upstream.equals(vendored)) {
      throw new Error(`${file} differs from the locked upstream byte stream`);
    }
  }
}

process.stdout.write(
  `playground-ui ${lock.state} snapshot verified (${vendorDigest})\n`,
);
