import { readdir, readFile } from 'node:fs/promises';
import { extname, join, relative, resolve } from 'node:path';
import { gzipSync } from 'node:zlib';

const root = resolve('dist');
const limits = {
  '.css': 24 * 1024,
  '.html': 8 * 1024,
  '.js': 440 * 1024,
  '.wasm': 480 * 1024,
} as const;

const totals = new Map<string, number>();
const files: string[] = [];
const failures: string[] = [];

async function walk(directory: string): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await walk(path);
    } else {
      files.push(path);
    }
  }
}

await walk(root);
for (const file of files) {
  if (file.endsWith('.map')) {
    failures.push(
      `${relative(root, file)}: production source maps must not be published`,
    );
    continue;
  }
  const extension = extname(file);
  if (!(extension in limits)) continue;
  const bytes = gzipSync(await readFile(file)).byteLength;
  totals.set(extension, (totals.get(extension) ?? 0) + bytes);
}

for (const [extension, limit] of Object.entries(limits)) {
  const total = totals.get(extension) ?? 0;
  const formatted = `${(total / 1024).toFixed(1)} KiB / ${(limit / 1024).toFixed(0)} KiB`;
  if (total > limit) failures.push(`${extension}: ${formatted}`);
  else process.stdout.write(`${extension}: ${formatted}\n`);
}

for (const file of files) {
  if (!/\.(?:css|html|js)$/.test(file) || file.endsWith('.map')) continue;
  const contents = await readFile(file, 'utf8');
  const forbiddenBuildStrings = [
    ['use.typekit.net', 'external Typekit reference'],
    ['The style macro must be imported', 'untransformed Spectrum style macro'],
    ['fileURLToPath', 'Node-only Spectrum macro runtime'],
  ] as const;
  for (const [pattern, reason] of forbiddenBuildStrings) {
    if (contents.includes(pattern)) {
      failures.push(`${relative(root, file)}: ${reason}`);
    }
  }
}

// CodeMirror, the renderer bridge, and WASM are intentionally loaded after
// first paint. Accidentally turning any of them into an entry dependency adds
// hundreds of compressed KiB to the critical path even though the total
// bundle budget still passes.
const indexHtml = await readFile(join(root, 'index.html'), 'utf8');
for (const deferredChunk of [
  'vendor-codemirror',
  'adapter-engine',
  'engineFeatures',
  'wasm-loader',
  '.wasm',
]) {
  const preload = new RegExp(
    `<link[^>]+rel=["']modulepreload["'][^>]+${deferredChunk}`,
    'i',
  );
  if (preload.test(indexHtml)) {
    failures.push(
      `index.html: ${deferredChunk} must remain off the preload path`,
    );
  }
}

if (failures.length > 0) {
  throw new Error(
    `Production bundle budget exceeded in ${relative(process.cwd(), root)}:\n${failures.join('\n')}`,
  );
}
