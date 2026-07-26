// Structure highlighting for the aozora-md playground editor.
//
// Ported from aozora's playground/src/editor/decorations.ts. The structure is
// kept intact: a viewport-windowed ViewPlugin reads the parsed node list out of
// the shared ParserState (populated from AozoraDocument.nodesJson) and turns each
// NodeEntry into a Decoration.mark with a cm-aozora-* class. aozora-md reuses the
// cm-aozora-* class names verbatim because they are an editor-decoration
// detail, not a brand contract.
//
// It ships as an independent extension that consumes the parsed node spans
// (ps.nodes), so highlighting follows the parser rather than a regex guess.

import { RangeSetBuilder } from '@codemirror/state';
import {
  Decoration,
  EditorView,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from '@codemirror/view';
import {
  byteToUtf16,
  parserStateField,
  utf16ToByte,
  type NodeEntry,
  type ParserState,
} from './parserState';

/**
 * Map every `kind` returned by `AozoraDocument.nodesJson()` to a CSS class.
 * Kinds not in this table are skipped (they carry no visual styling),
 * which keeps this the single source of truth for what gets highlighted.
 *
 * Mapping note: the wire format uses camelCase ("headingHint"); we fold
 * the prefix here so the cm-aozora-* class names stay short and readable.
 */
const KIND_TO_CLASS: Record<string, string> = {
  ruby: 'cm-aozora-ruby',
  angleQuote: 'cm-aozora-angle-quote',
  bouten: 'cm-aozora-bouten',
  gaiji: 'cm-aozora-gaiji',
  combineUpright: 'cm-aozora-tcy',
  illustration: 'cm-aozora-sashie',
  kaeriten: 'cm-aozora-kaeriten',
  directive: 'cm-aozora-annotation',
  heading: 'cm-aozora-aozora-heading',
  headingHint: 'cm-aozora-heading-hint',
  sectionBreak: 'cm-aozora-section-break',
  bodyEnd: 'cm-aozora-section-break',
  pageBreak: 'cm-aozora-page-break',
  containerOpen: 'cm-aozora-container-marker',
  containerClose: 'cm-aozora-container-marker',
};

/**
 * Lower-bound binary search: index of the first entry whose `span.start`
 * is >= `target` byte offset; `entries.length` if all start before it.
 *
 * `nodesJson` entries are emitted in source order, so they are sorted by
 * `span.start` and a binary search is valid. Inlined here (rather than
 * pulled from a shared editor/utils) to keep this module self-contained;
 * if aozora-md later grows an editor/utils with the same helper, import it
 * instead.
 */
function lowerBoundByStart(entries: NodeEntry[], target: number): number {
  let lo = 0;
  let hi = entries.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    const entry = entries[mid]!;
    if (entry.span.start < target) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function buildDecorations(view: EditorView): DecorationSet {
  const ps: ParserState = view.state.field(parserStateField);
  if (!ps.source) return Decoration.none;

  const entries = ps.nodes;
  if (entries.length === 0) return Decoration.none;

  const viewport = view.viewport;
  const vpFromByte = utf16ToByte(ps, viewport.from);
  const vpToByte = utf16ToByte(ps, viewport.to);

  // Find the slice of entries that could overlap the viewport. Widen by
  // 32 entries on the leading edge because entries are sorted by start,
  // and an earlier-starting entry may still cover bytes inside our
  // viewport.
  const startIdx = Math.max(0, lowerBoundByStart(entries, vpFromByte) - 32);

  // Decorations must be added in increasing `from` order. Since entries
  // are sorted by span.start, and span.start in bytes maps monotonically
  // to UTF-16 positions, the resulting `from` values are also
  // non-decreasing.
  const builder = new RangeSetBuilder<Decoration>();
  for (let i = startIdx; i < entries.length; i++) {
    const entry = entries[i]!;
    if (entry.span.start > vpToByte) break;
    if (entry.span.end < vpFromByte) continue;
    const cls = KIND_TO_CLASS[entry.kind];
    if (!cls) continue;
    const from = byteToUtf16(ps, entry.span.start);
    const to = byteToUtf16(ps, entry.span.end);
    if (from >= to) continue;
    builder.add(from, to, Decoration.mark({ class: cls }));
  }
  return builder.finish();
}

export const aozoraDecorations = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }
    update(update: ViewUpdate) {
      // parserStateField recomputes on every docChange, so a doc edit
      // also means a fresh node list is in view.state. viewportChanged
      // covers scroll / resize re-windowing.
      if (update.docChanged || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
  },
);
