// Light/dark colour-scheme preference plumbing.
//
// This is a *separate axis* from `theme-toggle.ts`: that one swaps the
// preview's writing mode (vertical / horizontal). This one controls the
// chrome's light/dark palette.
//
//   - 'auto'        : follow the OS `prefers-color-scheme`, reacting live
//   - 'light'/'dark': force, ignoring the OS
//
// Single source of truth is `<html data-color-scheme="light" | "dark">`.
// CSS variables switch only through that attribute, so the JS state and
// the painted DOM never drift. (Mirrors the sibling aozora playground's
// `theme.ts`; aozora-md keeps the writing-mode toggle on a distinct attribute,
// `data-aozora-md-theme`, so the two axes never collide.)

import { type Accessor, createEffect, createSignal } from 'solid-js';

const STORAGE_KEY = 'aozora-md-playground:color-scheme';
const CYCLE = ['auto', 'light', 'dark'] as const;

export type ColorSchemePref = (typeof CYCLE)[number];

function loadPref(): ColorSchemePref {
  try {
    const v = globalThis.localStorage?.getItem(STORAGE_KEY);
    return v === 'light' || v === 'dark' || v === 'auto' ? v : 'auto';
  } catch {
    return 'auto';
  }
}

function savePref(pref: ColorSchemePref): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, pref);
  } catch {
    // localStorage may be unavailable (private mode / strict CSP).
  }
}

function osPrefersDark(): boolean {
  return (
    globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false
  );
}

function effective(pref: ColorSchemePref): 'light' | 'dark' {
  return pref === 'auto' ? (osPrefersDark() ? 'dark' : 'light') : pref;
}

function paint(pref: ColorSchemePref): void {
  document.documentElement.dataset.colorScheme = effective(pref);
}

/**
 * Call once at the very top of `main.tsx`, before Solid renders, to avoid
 * a flash of the wrong scheme. Paints the saved preference and subscribes
 * to OS changes while in 'auto'.
 */
export function bootstrapColorScheme(): void {
  paint(loadPref());
  const mql = globalThis.matchMedia?.('(prefers-color-scheme: dark)');
  mql?.addEventListener('change', () => {
    if (loadPref() === 'auto') paint('auto');
  });
}

export interface ColorSchemeApi {
  readonly pref: Accessor<ColorSchemePref>;
  cyclePref(): void;
}

/** Solid hook driving the toolbar toggle (auto -> light -> dark -> auto). */
export function createColorScheme(): ColorSchemeApi {
  const [pref, setPref] = createSignal<ColorSchemePref>(loadPref());

  createEffect(() => {
    const p = pref();
    paint(p);
    savePref(p);
  });

  return {
    pref,
    cyclePref() {
      setPref((p) => CYCLE[(CYCLE.indexOf(p) + 1) % CYCLE.length] ?? 'auto');
    },
  };
}
