import { playgroundAdapterContract } from '@aozora/playground-ui/testing';
import { describe, expect, it } from 'vitest';

import { afmPlaygroundAdapter } from './adapter';

playgroundAdapterContract('Aozora Flavored Markdown', afmPlaygroundAdapter);

describe('AFM playground adapter contract', () => {
  it('initializes the real WASM and returns author-facing analysis', async () => {
    await afmPlaygroundAdapter.initialize();
    const source = '# 章\n\n吾輩《わがはい》は猫である。';
    const analysis = await afmPlaygroundAdapter.analyze(source, {
      revision: 1,
      signal: new AbortController().signal,
    });

    expect(analysis.html).toContain('<h1');
    expect(analysis.html).toContain('<ruby');
    expect(analysis.outline).toEqual([
      {
        level: 1,
        text: '章',
        range: { start: 0, end: 3 },
      },
    ]);
    expect(analysis.diagnostics).toEqual([]);
  });

  it('owns the preview innerHTML boundary and writing-mode theme link', async () => {
    await afmPlaygroundAdapter.initialize();
    const link = document.createElement('link');
    link.id = 'aozora-md-theme';
    document.head.append(link);
    const host = document.createElement('div');
    const preview = afmPlaygroundAdapter.createPreview(host);

    preview.update('<p>本文</p>', 'vertical');
    const paragraph = host.querySelector('.aozora-md-root p');
    expect(paragraph?.innerHTML).toBe('本文');
    expect(document.documentElement.dataset.aozoraMdTheme).toBe('vertical');
    expect(link.hasAttribute('href')).toBe(true);

    preview.update('<p>本文</p>', 'horizontal');
    expect(host.querySelector('.aozora-md-root p')).toBe(paragraph);

    preview.destroy();
    expect(host.childElementCount).toBe(0);
    link.remove();
  });

  it('normalizes byte diagnostics to browser UTF-16 ranges', async () => {
    await afmPlaygroundAdapter.initialize();
    const analysis = await afmPlaygroundAdapter.analyze('😀》', {
      revision: 4,
      signal: new AbortController().signal,
    });
    expect(analysis.diagnostics).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          severity: 'error',
          range: { start: 2, end: 3 },
        }),
      ]),
    );
  });

  it('aligns second-line analysis ranges with a CRLF-normalized editor', async () => {
    await afmPlaygroundAdapter.initialize();
    const analysis = await afmPlaygroundAdapter.analyze(
      '# First\r\n# Second\r\n》',
      {
        revision: 5,
        signal: new AbortController().signal,
      },
    );

    expect(analysis.outline[1]).toMatchObject({
      text: 'Second',
      range: { start: 8, end: 16 },
    });
    expect(analysis.diagnostics).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          range: { start: 17, end: 18 },
        }),
      ]),
    );
  });
});
