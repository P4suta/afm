// Vite ?url imports resolve at build time to hashed asset URLs. The two
// theme files live in `crates/aozora-flavored-markdown/theme/` — the
// renderer owns the CSS for the classes it emits, and the same files are
// what its `theme` feature embeds (ADR-0020), so the playground reads the
// single source of truth rather than a copy. Swapping
// `<link id="aozora-md-theme">.href` between them flips the preview between
// vertical (tategaki) and horizontal layout without re-running the wasm
// pipeline.

import horizontalUrl from '../../../crates/aozora-flavored-markdown/theme/aozora-md-horizontal.css?url';
import verticalUrl from '../../../crates/aozora-flavored-markdown/theme/aozora-md-vertical.css?url';

export const THEME_URLS = {
  vertical: verticalUrl,
  horizontal: horizontalUrl,
} as const;

export type ThemeMode = keyof typeof THEME_URLS;
