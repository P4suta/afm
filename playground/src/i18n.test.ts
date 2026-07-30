import { afterEach, describe, expect, it } from 'vitest';

import {
  CATALOG,
  diagnosticText,
  setEditorLocale,
  slugDocumentation,
  t,
  tf,
} from './i18n';

describe('editor catalog', () => {
  afterEach(() => setEditorLocale('ja'));

  it('keeps the Japanese and English key sets aligned', () => {
    expect(Object.keys(CATALOG.ja).sort()).toEqual(
      Object.keys(CATALOG.en).sort(),
    );
  });

  it('switches locale without coupling editor features to the app shell', () => {
    setEditorLocale('en');
    expect(t('outlineUntitled')).toBe('(untitled)');
    expect(tf('lintPua', { hex: 'U+E000' })).toContain('U+E000');

    setEditorLocale('ja');
    expect(t('outlineUntitled')).toBe('（無題）');
    expect(tf('lintPua', { hex: 'U+E000' })).toContain('U+E000');
  });

  it('keeps Japanese-only slug documentation behind the locale boundary', () => {
    setEditorLocale('en');
    expect(slugDocumentation('日本語だけの説明')).toBe('Aozora notation');

    setEditorLocale('ja');
    expect(slugDocumentation('日本語だけの説明')).toBe('日本語だけの説明');
  });

  it('localizes stable diagnostics without exposing English parser prose', () => {
    expect(
      diagnosticText(
        'aozora::lex::unmatched_close',
        'error',
        'unmatched close',
      ),
    ).toEqual({
      ja: '対応する開き括弧がありません。',
      en: 'unmatched close',
    });
    expect(
      diagnosticText('future::warning', 'warning', 'future warning'),
    ).toEqual({
      ja: '確認が必要な記法があります。',
      en: 'future warning',
    });
  });
});
