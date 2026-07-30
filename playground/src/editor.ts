// CodeMirror 6 wiring.
//
// Beyond the base editor (line numbers, history, bracket matching,
// folding), this assembles the 青空文庫 editor assists ported from the
// sibling aozora playground. They all hang off `parserStateField`, which
// owns one aozora-flavored-markdown-wasm `AozoraDocument` per source revision and exposes the parse
// results (nodes / pairs / diagnostics / gaiji) in source coordinates.
//
// Toggleable features (structural highlight, gaiji inlay hints) live in
// Compartments so the settings panel can flip them on a live view.

import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { markdown } from '@codemirror/lang-markdown';
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
} from '@codemirror/language';
import { searchKeymap } from '@codemirror/search';
import { Compartment, EditorState } from '@codemirror/state';
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  ViewPlugin,
} from '@codemirror/view';

import { aozoraMdEditorTheme } from './cm-theme';
import { aozoraMdCompletion } from './editor/completion';
import { aozoraDecorations } from './editor/decorations';
import { aozoraFolding } from './editor/folding';
import { aozoraMdHover } from './editor/hover';
import { aozoraInlayHints } from './editor/inlayHints';
import { linkedRangesFilter } from './editor/linkedRanges';
import { aozoraMdLinter, aozoraMdLintGutter } from './editor/linter';
import { parserStateField } from './editor/parserState';
import { aozoraMdWrapKeymap } from './editor/wrapCommands';

// StateField values have no disposal hook. Revisions free their predecessor
// in `computeParserState`; this view plugin releases the final WASM document
// when CodeMirror itself is destroyed.
const parserDocumentOwner = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {}

    destroy(): void {
      this.view.state.field(parserStateField).doc?.free();
    }
  },
);

// Toggleable features (flipped by the settings panel). Default ON so the
// editor's initial state matches the panel's initial signal values.
export const structureHighlightCompartment = new Compartment();
export const inlayHintsCompartment = new Compartment();

export interface EditorHandle {
  readonly view: EditorView;
  getValue(): string;
  setValue(value: string): void;
}

export function createEditor(
  parent: HTMLElement,
  initialValue: string,
  onChange: (value: string) => void,
): EditorHandle {
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: initialValue,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        highlightSpecialChars(),
        history(),
        drawSelection(),
        indentOnInput(),
        bracketMatching(),
        foldGutter(),
        EditorView.lineWrapping,
        keymap.of([
          ...aozoraMdWrapKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          ...searchKeymap,
          ...foldKeymap,
        ]),
        markdown(),
        aozoraMdEditorTheme,
        // Parser backbone — every assist below reads from this field.
        parserStateField,
        parserDocumentOwner,
        structureHighlightCompartment.of(aozoraDecorations),
        aozoraMdLinter,
        aozoraMdLintGutter,
        aozoraMdCompletion,
        aozoraMdHover,
        aozoraFolding,
        linkedRangesFilter,
        inlayHintsCompartment.of(aozoraInlayHints),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChange(update.state.doc.toString());
          }
        }),
      ],
    }),
  });

  return {
    view,
    getValue: () => view.state.doc.toString(),
    setValue: (value: string) => {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: value },
      });
    },
  };
}
