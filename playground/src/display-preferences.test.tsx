import { fireEvent, render, screen } from '@solidjs/testing-library';
import type { Component } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { bootstrapColorScheme, createColorScheme } from './color-scheme';
import { THEME_URLS } from './styles/theme-urls';
import { createTheme } from './theme-toggle';

const ThemeHarness: Component = () => {
  const theme = createTheme();
  return (
    <button type="button" onClick={() => theme.setMode('horizontal')}>
      {theme.mode()}
    </button>
  );
};

const ColourHarness: Component = () => {
  const colour = createColorScheme();
  return (
    <button type="button" onClick={() => colour.cyclePref()}>
      {colour.pref()}
    </button>
  );
};

describe('display preferences', () => {
  beforeEach(() => {
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      get length() {
        return values.size;
      },
      clear: () => values.clear(),
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      removeItem: (key: string) => values.delete(key),
      setItem: (key: string, value: string) => values.set(key, value),
    } satisfies Storage);
    localStorage.clear();
    document.documentElement.removeAttribute('data-aozora-md-theme');
    document.documentElement.removeAttribute('data-color-scheme');
    document.head.innerHTML =
      '<link id="aozora-md-theme" rel="stylesheet" href="/initial.css">';
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('switches the preview stylesheet and persists writing mode from a click', () => {
    render(() => <ThemeHarness />);

    expect(document.documentElement.dataset.aozoraMdTheme).toBe('vertical');
    expect(
      document.querySelector<HTMLLinkElement>('#aozora-md-theme')?.href,
    ).toContain(THEME_URLS.vertical);

    fireEvent.click(screen.getByRole('button', { name: 'vertical' }));

    expect(screen.getByRole('button', { name: 'horizontal' })).toBeTruthy();
    expect(document.documentElement.dataset.aozoraMdTheme).toBe('horizontal');
    expect(
      document.querySelector<HTMLLinkElement>('#aozora-md-theme')?.href,
    ).toContain(THEME_URLS.horizontal);
    expect(localStorage.getItem('aozora-md-playground:theme-mode')).toBe(
      'horizontal',
    );
  });

  it('cycles auto, light, and dark while painting and persisting the DOM', () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn(() => ({
        matches: true,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    bootstrapColorScheme();
    expect(document.documentElement.dataset.colorScheme).toBe('dark');

    render(() => <ColourHarness />);
    fireEvent.click(screen.getByRole('button', { name: 'auto' }));
    expect(document.documentElement.dataset.colorScheme).toBe('light');
    expect(localStorage.getItem('aozora-md-playground:color-scheme')).toBe(
      'light',
    );

    fireEvent.click(screen.getByRole('button', { name: 'light' }));
    expect(document.documentElement.dataset.colorScheme).toBe('dark');

    fireEvent.click(screen.getByRole('button', { name: 'dark' }));
    expect(document.documentElement.dataset.colorScheme).toBe('dark');
    expect(screen.getByRole('button', { name: 'auto' })).toBeTruthy();
  });
});
