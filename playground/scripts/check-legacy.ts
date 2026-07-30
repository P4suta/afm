import { readdir, readFile } from 'node:fs/promises';
import { extname, join, normalize, relative, resolve } from 'node:path';

const roots = ['src', 'vendor/playground-ui/src'];
const manifests = [
  'package.json',
  'bun.lock',
  'vendor/playground-ui/package.json',
];
const extensions = new Set(['.ts', '.tsx', '.css']);
const forbidden = [
  ['solid', '-js'].join(''),
  ['@solidjs', '/testing-library'].join(''),
  ['vite-plugin', '-solid'].join(''),
  ['UNSAFE', '_'].join(''),
  ['IR', ' JSON'].join(''),
  ['Nodes', ' JSON'].join(''),
  ['HTML', ' source'].join(''),
  ['Perf', 'Badge'].join(''),
  ['Code', 'View'].join(''),
];
const basicEditorEntry = resolve('src/editor.ts');
const engineOnlyModules = new Set(
  [
    'src/adapter-engine.ts',
    'src/wasm-loader.ts',
    'src/editor/completion.ts',
    'src/editor/decorations.ts',
    'src/editor/engineFeatures.ts',
    'src/editor/folding.ts',
    'src/editor/hover.ts',
    'src/editor/inlayHints.ts',
    'src/editor/linkedRanges.ts',
    'src/editor/linter.ts',
    'src/editor/parserState.ts',
  ].map((path) => resolve(path)),
);
const staticImportPattern =
  /\b(?:import|export)\s+(?!type\b)(?:[^'"]*?\sfrom\s*)?['"](\.[^'"]+)['"]/g;

async function filesUnder(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesUnder(path)));
    else if (extensions.has(extname(entry.name))) files.push(path);
  }
  return files;
}

async function resolveLocalModule(
  importer: string,
  specifier: string,
): Promise<string | null> {
  const base = resolve(importer, '..', specifier);
  const candidates = [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    join(base, 'index.ts'),
    join(base, 'index.tsx'),
  ];
  for (const candidate of candidates) {
    if (
      candidate === normalize(candidate) &&
      (await Bun.file(candidate).exists())
    ) {
      return candidate;
    }
  }
  return null;
}

async function staticDependencyGraph(entry: string): Promise<Set<string>> {
  const discovered = new Set<string>();
  const pending = [entry];
  while (pending.length > 0) {
    const path = pending.pop();
    if (!path || discovered.has(path)) continue;
    discovered.add(path);
    const source = await readFile(path, 'utf8');
    for (const match of source.matchAll(staticImportPattern)) {
      const specifier = match[1];
      if (!specifier) continue;
      const dependency = await resolveLocalModule(path, specifier);
      if (dependency && !discovered.has(dependency)) pending.push(dependency);
    }
  }
  return discovered;
}

const files = [
  ...manifests,
  ...(await Promise.all(roots.map(filesUnder))).flat(),
];
const violations: string[] = [];
for (const path of files) {
  const source = await readFile(path, 'utf8');
  for (const pattern of forbidden) {
    if (source.includes(pattern)) {
      violations.push(
        `${relative('.', path)} contains ${JSON.stringify(pattern)}`,
      );
    }
  }
}

const basicEditorDependencies = await staticDependencyGraph(basicEditorEntry);
for (const path of engineOnlyModules) {
  if (basicEditorDependencies.has(path)) {
    violations.push(
      `src/editor.ts has a static dependency path to ${relative('.', path)}`,
    );
  }
}

if (violations.length > 0) {
  throw new Error(
    `Legacy playground surfaces remain:\n${violations.join('\n')}`,
  );
}
