import { fireEvent, render, screen } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { describe, expect, it, vi } from 'vitest';

import Toolbar from './Toolbar';

describe('Toolbar', () => {
  it('drives writing mode, examples, guide, colour scheme, and sharing through DOM events', () => {
    const [themeMode, setThemeMode] = createSignal<'vertical' | 'horizontal'>(
      'vertical',
    );
    const [colour, setColour] = createSignal<'auto' | 'light' | 'dark'>('auto');
    const loadExample = vi.fn();
    const showGuide = vi.fn();
    const share = vi.fn();

    render(() => (
      <Toolbar
        themeMode={themeMode}
        onThemeChange={setThemeMode}
        colorSchemePref={colour}
        onCycleColorScheme={() =>
          setColour((value) => (value === 'auto' ? 'light' : 'auto'))
        }
        examples={[{ slug: 'welcome', label: 'ようこそ', source: '# 本文' }]}
        onLoadExample={loadExample}
        onShare={share}
        editorView={() => null}
        onShowGuide={showGuide}
      />
    ));

    const vertical = screen.getByRole('button', { name: '縦書き' });
    const horizontal = screen.getByRole('button', { name: '横書き' });
    expect(vertical.getAttribute('aria-pressed')).toBe('true');
    expect(horizontal.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(horizontal);
    expect(themeMode()).toBe('horizontal');
    expect(vertical.getAttribute('aria-pressed')).toBe('false');
    expect(horizontal.getAttribute('aria-pressed')).toBe('true');

    fireEvent.change(screen.getByLabelText('例文'), {
      target: { value: 'welcome' },
    });
    expect(loadExample).toHaveBeenCalledWith('welcome');

    fireEvent.click(screen.getByRole('button', { name: '📖 記法' }));
    expect(showGuide).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole('button', { name: /配色:/ }));
    expect(colour()).toBe('light');
    expect(screen.getByRole('button', { name: /配色:.*ライト/ })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '共有リンクをコピー' }));
    expect(share).toHaveBeenCalledOnce();
  });
});
