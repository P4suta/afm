import { createHash } from 'node:crypto';
import { copyFile, mkdir, readFile, unlink, writeFile } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';

import {
  compareFileSet,
  listFiles,
  PLAYGROUND_UI_FILES,
} from './playground-ui-files';
import {
  assertCanonicalRepository,
  UPSTREAM_REPOSITORY,
} from './vendor-repository';

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

const sourceArgument = process.argv[2];
if (!sourceArgument) {
  throw new Error(
    'usage: bun scripts/sync-playground-ui.ts PATH_TO_PLAYGROUND_UI',
  );
}

const sourceRoot = resolve(sourceArgument);
const destinationRoot = resolve('vendor/playground-ui');
const repositoryRoot = git(sourceRoot, 'rev-parse', '--show-toplevel');
const packagePath = relative(repositoryRoot, sourceRoot);
const status = git(repositoryRoot, 'status', '--porcelain', '--', packagePath);
if (status !== '') {
  throw new Error(
    'canonical playground-ui must be committed before it can be vendored',
  );
}
if (packagePath !== 'playground-ui') {
  throw new Error(
    `canonical playground-ui package path differs: ${packagePath}`,
  );
}
const upstreamCommit = git(repositoryRoot, 'rev-parse', 'HEAD^{commit}');
const upstreamRepositoryTree = git(repositoryRoot, 'rev-parse', 'HEAD^{tree}');
const upstreamPackageTree = git(
  repositoryRoot,
  'rev-parse',
  `HEAD:${packagePath}`,
);
const remote = git(repositoryRoot, 'remote', 'get-url', 'origin');
assertCanonicalRepository(remote);

await Promise.all(
  PLAYGROUND_UI_FILES.map((file) => readFile(join(sourceRoot, file))),
);
const destinationFiles = await listFiles(destinationRoot);
const { extra } = compareFileSet(destinationFiles);
for (const file of extra) {
  await unlink(join(destinationRoot, file));
}
for (const file of PLAYGROUND_UI_FILES) {
  const source = join(sourceRoot, file);
  const destination = join(destinationRoot, file);
  await mkdir(dirname(destination), { recursive: true });
  await copyFile(source, destination);
}

const lock = {
  schemaVersion: 1,
  state: 'locked',
  upstreamRepository: UPSTREAM_REPOSITORY,
  upstreamCommit,
  upstreamRepositoryTree,
  upstreamPackagePath: packagePath,
  upstreamPackageTree,
  snapshotSha256: await digest(destinationRoot),
  files: PLAYGROUND_UI_FILES,
};
await writeFile(
  resolve('vendor/playground-ui.lock.json'),
  `${JSON.stringify(lock, null, 2)}\n`,
);
