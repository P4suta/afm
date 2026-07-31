import { type EditorView, ViewPlugin } from '@codemirror/view';

import {
  type EngineFeatureFactory,
  inlayHintsCompartment,
  structureHighlightCompartment,
} from './compartments';
import { aozoraMdCompletion } from './completion';
import { aozoraDecorations } from './decorations';
import { aozoraFolding } from './folding';
import { aozoraMdHover } from './hover';
import { aozoraInlayHints } from './inlayHints';
import { linkedRangesFilter } from './linkedRanges';
import { aozoraMdLinter, aozoraMdLintGutter } from './linter';
import { parserStateField } from './parserState';

const parserDocumentOwner = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {}

    destroy(): void {
      this.view.state.field(parserStateField, false)?.doc?.free();
    }
  },
);

export const createEngineFeatures: EngineFeatureFactory = (settings) => ({
  extension: [
    parserStateField,
    parserDocumentOwner,
    structureHighlightCompartment.of(
      settings.structureHighlight ? aozoraDecorations : [],
    ),
    aozoraMdLinter,
    aozoraMdLintGutter,
    aozoraMdCompletion,
    aozoraMdHover,
    aozoraFolding,
    linkedRangesFilter,
    inlayHintsCompartment.of(settings.gaijiInlayHints ? aozoraInlayHints : []),
  ],
  structureHighlight: aozoraDecorations,
  gaijiInlayHints: aozoraInlayHints,
});
