// Gaiji inlay hints for the aozora-md playground editor.
//
// Ported verbatim-in-structure from the aozora playground
// (playground/src/editor/inlayHints.ts). A ViewPlugin reads the parser
// state's resolved gaiji entries and renders the substituted character
// inline (`→<resolved>`) just after each `※［＃...］` reference, using a
// Decoration.widget so the source text the user typed is never mutated.
//
// aozora-md adaptations:
//   - parser state is imported from the local aozora-md parserState module,
//   - the internal CSS class stays `cm-aozora-inlay` (editor implementation
//     detail, not an aozora-md brand surface) so the shared stylesheet matches;
//     its colours are wired to aozora-md chrome tokens (var(--aozora-md-pg-*)) in CSS.

import {
  Decoration,
  type DecorationSet,
  type EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import { byteToUtf16, type ParserState, parserStateField } from './parserState';

// Inline widget that renders `→<resolved>` after a gaiji reference. The
// codepoint and description feed the hover title; both are decorative and
// purely informational.
class GaijiInlayWidget extends WidgetType {
  constructor(
    readonly resolved: string,
    readonly codepoint: number | null,
    readonly description: string,
  ) {
    super();
  }

  override toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-aozora-inlay';
    span.textContent = `→${this.resolved}`;
    const cpHex =
      this.codepoint !== null
        ? `U+${this.codepoint.toString(16).toUpperCase().padStart(4, '0')}`
        : '';
    span.title = cpHex ? `${this.description} (${cpHex})` : this.description;
    return span;
  }

  override eq(other: GaijiInlayWidget): boolean {
    return (
      other.resolved === this.resolved &&
      other.codepoint === this.codepoint &&
      other.description === this.description
    );
  }

  override ignoreEvent(): boolean {
    return false;
  }
}

function buildInlays(view: EditorView): DecorationSet {
  const ps: ParserState = view.state.field(parserStateField);
  const entries = ps.gaijiResolutions;
  if (entries.length === 0) return Decoration.none;
  const docLen = view.state.doc.length;
  // Use a sparse array we can sort, since gaiji entries from the
  // WASM scan come in source order but Decoration.set wants the
  // ranges already ordered by `from`.
  const items: { pos: number; widget: GaijiInlayWidget }[] = [];
  for (const r of entries) {
    if (!r.resolved) continue;
    const pos = byteToUtf16(ps, r.span.end);
    if (pos < 0 || pos > docLen) continue;
    items.push({
      pos,
      widget: new GaijiInlayWidget(r.resolved, r.codepoint, r.description),
    });
  }
  items.sort((a, b) => a.pos - b.pos);
  const decos = items.map((it) =>
    Decoration.widget({ widget: it.widget, side: 1 }).range(it.pos),
  );
  return Decoration.set(decos, true);
}

export const aozoraInlayHints = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildInlays(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged) {
        this.decorations = buildInlays(update.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
  },
);
