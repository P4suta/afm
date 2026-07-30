// Selection-wrap commands for Aozora notation, ported from the sibling
// aozora playground's `editor/wrapCommands.ts` (which in turn mirrors
// aozora-tools' VSCode `wrap.ts`). Every shape is plain aozora notation
// that aozora-md renders, and none collides with Markdown syntax, so the set
// carries over unchanged. Each wraps the selection in a snippet template
// so the `${0}` tabstop lands the cursor predictably.

import { snippet } from '@codemirror/autocomplete';
import type { Command, KeyBinding } from '@codemirror/view';

export interface WrapShape {
  /** Stable id (mirrors the aozora / aozora-tools command names). */
  id: string;
  /** Snippet template; `BASE` is the selection, `${0}` the final cursor. */
  template: string;
}

export const WRAP_SHAPES: readonly WrapShape[] = [
  {
    id: 'aozora-md.wrap.ruby',
    template: '｜BASE《${0}》',
  },
  {
    id: 'aozora-md.wrap.angleQuote',
    template: '≪BASE≫${0}',
  },
  {
    id: 'aozora-md.wrap.bouten',
    template: 'BASE［＃「BASE」に傍点］${0}',
  },
  {
    id: 'aozora-md.wrap.kagikakko',
    template: '「BASE」${0}',
  },
  {
    id: 'aozora-md.wrap.kikkou',
    template: '〔BASE〕${0}',
  },
  {
    id: 'aozora-md.wrap.chuki',
    template: '［＃BASE］${0}',
  },
] as const;

function escapeSnippet(text: string): string {
  return text
    .replace(/\\/g, '\\\\')
    .replace(/\$/g, '\\$')
    .replace(/\}/g, '\\}');
}

/**
 * Build a CM6 `Command` that wraps the current selection with the given
 * shape. An empty selection substitutes the empty string for `BASE`; the
 * `${0}` tabstop keeps cursor placement well-defined either way.
 */
export function wrapCommand(shape: WrapShape): Command {
  return (view) => {
    const sel = view.state.selection.main;
    const selected = view.state.sliceDoc(sel.from, sel.to);
    const body = shape.template.split('BASE').join(escapeSnippet(selected));
    // snippet()'s returned fn takes (view, completion, from, to); the
    // completion arg is only used for autocomplete provenance and the
    // expansion ignores it, so `null as never` is safe when invoked from
    // a keymap. (Same rationale as the aozora port.)
    snippet(body)(view, null as never, sel.from, sel.to);
    return true;
  };
}

const SHAPE_BY_ID: Record<string, WrapShape> = Object.fromEntries(
  WRAP_SHAPES.map((s) => [s.id, s]),
);

/**
 * Keybindings: Mod-Alt-R = ruby, Mod-Alt-Shift-R = double angle brackets,
 * Mod-Alt-B = bouten. Mirrors aozora-tools' bindings so muscle memory
 * carries across the VSCode extension and both playgrounds.
 */
export const aozoraMdWrapKeymap: readonly KeyBinding[] = [
  {
    key: 'Mod-Alt-r',
    run: wrapCommand(SHAPE_BY_ID['aozora-md.wrap.ruby']!),
    preventDefault: true,
  },
  {
    key: 'Mod-Alt-Shift-r',
    run: wrapCommand(SHAPE_BY_ID['aozora-md.wrap.angleQuote']!),
    preventDefault: true,
  },
  {
    key: 'Mod-Alt-b',
    run: wrapCommand(SHAPE_BY_ID['aozora-md.wrap.bouten']!),
    preventDefault: true,
  },
];
