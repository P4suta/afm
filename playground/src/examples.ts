// Starter snippet catalogue.
//
// Files are loaded eagerly via Vite's `import.meta.glob('?raw')` so the
// production bundle ships their content inline (no extra fetch on
// dropdown change).

import type { LocalizedText } from '@aozora/playground-ui';

const rawModules = import.meta.glob<string>('../examples/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
});

interface ExampleLabelEntry {
  readonly slug: string;
  readonly label: LocalizedText;
}

const ORDERED_LABELS: readonly ExampleLabelEntry[] = [
  {
    slug: '01-welcome',
    label: {
      ja: 'はじめに ― aozora-md へようこそ',
      en: 'Welcome to aozora-md',
    },
  },
  {
    slug: '02-ruby-furigana',
    label: { ja: 'ルビ (｜ … 《 … 》)', en: 'Ruby and furigana' },
  },
  {
    slug: '03-bouten',
    label: {
      ja: '傍点 (［＃「…」に傍点］)',
      en: 'Emphasis dots',
    },
  },
  {
    slug: '04-tate-chu-yoko',
    label: { ja: '縦中横', en: 'Tate-chu-yoko' },
  },
  {
    slug: '05-breaks-and-indent',
    label: {
      ja: '改ページ・字下げ・段組',
      en: 'Page breaks and indentation',
    },
  },
  {
    slug: '06-paired-containers',
    label: { ja: '罫囲み・割注などの対構造', en: 'Paired containers' },
  },
  {
    slug: '07-gfm-mixed',
    label: {
      ja: 'GFM × 青空文庫 (表・タスクリスト)',
      en: 'GFM with Aozora notation',
    },
  },
];

export interface Example {
  readonly slug: string;
  readonly label: LocalizedText;
  readonly source: string;
}

export function loadExamples(): readonly Example[] {
  const bySlug = new Map<string, string>();
  for (const [path, source] of Object.entries(rawModules)) {
    const m = /\/(\d+-[a-z-]+)\.md$/.exec(path);
    if (m && m[1] !== undefined) bySlug.set(m[1], source);
  }
  return ORDERED_LABELS.flatMap(({ slug, label }) => {
    const source = bySlug.get(slug);
    return source !== undefined ? [{ slug, label, source }] : [];
  });
}
