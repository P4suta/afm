import { expect, test } from 'vitest';

import { loadExamples } from './examples';

// `loadExamples` iterates its own hand-written label list and looks each slug
// up in the glob, so a file in `playground/examples/` that nobody labelled —
// or one whose name the slug regex does not match — is dropped without a
// word. That directory is not only the dropdown: `crates/xtask/tests/
// `just fuzz-seed` seeds every fuzz target's corpus from it, so a document can
// be feeding the fuzzer while being unreachable in the UI it was written for.
//
// The same glob the module uses, so the two cannot disagree about what is on
// disk — only about what reaches a reader.
const onDisk = import.meta.glob<string>('../examples/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
});

/** `../examples/03-bouten.md` → `03-bouten`. */
function slugOf(path: string): string {
  return path.replace(/^.*\//, '').replace(/\.md$/, '');
}

test('the catalogue reaches every document in the examples directory', () => {
  const shipped = loadExamples().map((example) => example.slug);
  const present = Object.keys(onDisk).map(slugOf).sort();
  expect(shipped).toStrictEqual(present);
});

test('the reader is finding the documents at all', () => {
  // A glob that resolved to nothing would make the assertion above pass by
  // comparing two empty lists.
  expect(Object.keys(onDisk).length).toBeGreaterThanOrEqual(7);
});

test('each entry carries the source of the file it is named for', () => {
  const sourceBySlug = new Map(
    Object.entries(onDisk).map(([path, source]) => [slugOf(path), source]),
  );
  for (const example of loadExamples()) {
    expect(example.source, `${example.slug} carries another file's text`).toBe(
      sourceBySlug.get(example.slug),
    );
    expect(example.source.length, `${example.slug} is empty`).toBeGreaterThan(
      0,
    );
  }
});

test('every entry has a distinct, non-empty label', () => {
  const labels = loadExamples().map((example) => example.label);
  expect(labels.filter((label) => label.trim().length === 0)).toStrictEqual([]);
  expect(new Set(labels).size).toBe(labels.length);
});
