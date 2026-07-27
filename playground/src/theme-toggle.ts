// Solid hook for the vertical/horizontal theme. Swaps `<link href>`
// on a `#aozora-md-theme` element (declared in `index.html`) and persists
// the user preference to localStorage so reloads are sticky.

import { type Accessor, createEffect, createSignal } from 'solid-js';

import { THEME_URLS, type ThemeMode } from './styles/theme-urls';

const STORAGE_KEY = 'aozora-md-playground:theme-mode';
const DEFAULT_MODE: ThemeMode = 'vertical';
const LINK_ID = 'aozora-md-theme';

function loadStored(): ThemeMode {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    return raw === 'horizontal' || raw === 'vertical' ? raw : DEFAULT_MODE;
  } catch {
    return DEFAULT_MODE;
  }
}

function persist(mode: ThemeMode): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, mode);
  } catch {
    // localStorage may be unavailable (private mode / strict CSP).
  }
}

export interface ThemeApi {
  readonly mode: Accessor<ThemeMode>;
  setMode(mode: ThemeMode): void;
}

export function createTheme(): ThemeApi {
  const [mode, setMode] = createSignal<ThemeMode>(loadStored());

  createEffect(() => {
    const m = mode();
    const link = document.getElementById(LINK_ID);
    if (link instanceof HTMLLinkElement) {
      link.href = THEME_URLS[m];
    }
    document.documentElement.dataset.aozoraMdTheme = m;
    persist(m);
  });

  return {
    mode,
    setMode(next: ThemeMode) {
      setMode(next);
    },
  };
}
