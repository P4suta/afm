// Minimal CodeMirror theme tying the editor surface to the playground
// chrome tokens, so the editor follows light/dark (`<html
// data-color-scheme>`) together with the rest of the shell. CodeMirror's
// default theme hardcodes a light surface; without this the editor stays
// white in dark mode.
//
// We deliberately style only the *chrome* (surface / text / gutters /
// cursor / selection). Syntax-token colours for the markdown + aozora
// overlay are left to CodeMirror's defaults, which read fine in both
// schemes against these surfaces. Property-name style mirrors the sibling
// aozora playground's editor/theme.ts (proven to compile against the
// style-mod StyleSpec).

import { EditorView } from '@codemirror/view';

export const aozoraMdEditorTheme = EditorView.theme({
  '&': {
    background: 'var(--aozora-md-pg-bg-elevated)',
    color: 'var(--aozora-md-pg-text)',
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-content': { caretColor: 'var(--aozora-md-pg-accent)' },
  '.cm-cursor, .cm-dropCursor': {
    borderLeftColor: 'var(--aozora-md-pg-accent)',
  },
  '.cm-gutters': {
    background: 'var(--aozora-md-pg-bg)',
    color: 'var(--aozora-md-pg-text-soft)',
    borderRight: '1px solid var(--aozora-md-pg-border)',
  },
  '.cm-activeLine': { background: 'var(--aozora-md-pg-accent-soft)' },
  '.cm-activeLineGutter': { background: 'var(--aozora-md-pg-accent-soft)' },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground, .cm-content ::selection':
    {
      background: 'var(--aozora-md-pg-accent-soft)',
    },
  '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': {
    background: 'var(--aozora-md-pg-accent-soft)',
    outline: '1px solid var(--aozora-md-pg-border)',
  },
});
