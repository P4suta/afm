// Code folding for container blocks.
//
// Ported from aozora's editor/folding.ts. Container open/close annotations
// (for example, an Aozora indent-open marker followed by its close marker)
// are foldable: the
// region between the end of the open marker's line and the start of the close
// marker collapses to a single placeholder.
//
// The actual fold ranges are pre-computed once per parse and live on
// parserStateField as `ps.containerFolds` (a ContainerFold[] derived in
// parserState). This module is just the foldService that maps each line to
// its fold range, so the per-keystroke cost here is a linear scan over the
// (typically tiny) fold list.

import { foldService } from '@codemirror/language';
import { parserStateField } from './parserState';

/**
 * Fold container open/close blocks. The pre-computed `containerFolds` array on
 * `parserStateField` carries (openLineEnd, closeStart) for every detected pair.
 *
 * `openLineEnd` and `closeStart` are both UTF-16 code unit offsets, so they map
 * directly onto CodeMirror document positions without further translation.
 */
export const aozoraFolding = foldService.of((state, lineStart, lineEnd) => {
  const ps = state.field(parserStateField);
  if (ps.containerFolds.length === 0) return null;
  for (const fold of ps.containerFolds) {
    if (fold.openLineEnd >= lineStart && fold.openLineEnd <= lineEnd) {
      return { from: fold.openLineEnd, to: fold.closeStart };
    }
  }
  return null;
});
