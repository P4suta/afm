import type { EditorController, TextRange } from '@aozora/playground-ui';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { markdown } from '@codemirror/lang-markdown';
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
} from '@codemirror/language';
import { forceLinting } from '@codemirror/lint';
import { searchKeymap } from '@codemirror/search';
import {
  Annotation,
  Compartment,
  EditorState,
  Transaction,
} from '@codemirror/state';
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  placeholder,
} from '@codemirror/view';

import {
  type EngineFeatureFactory,
  type EngineFeatures,
  engineFeaturesCompartment,
  inlayHintsCompartment,
  structureHighlightCompartment,
} from './editor/compartments';
import {
  aozoraMdWrapKeymap,
  WRAP_SHAPES,
  wrapCommand,
} from './editor/wrapCommands';
import { t } from './i18n';

export interface EngineAwareEditorController extends EditorController {
  enableEngineFeatures(factory: EngineFeatureFactory): void;
  refreshLocale(): void;
}

const externalUpdate = Annotation.define<true>();
const localeCompartment = new Compartment();

function localizedEditorExtensions() {
  return [
    EditorView.contentAttributes.of({
      'aria-label': t('editorPaneTitle'),
    }),
    placeholder(t('editorPlaceholder')),
  ];
}

export function createEditor(
  parent: HTMLElement,
  initialValue: string,
  onChange: (value: string) => void,
  engineFeatureFactory: EngineFeatureFactory | null = null,
): EngineAwareEditorController {
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
        localeCompartment.of(localizedEditorExtensions()),
        keymap.of([
          ...aozoraMdWrapKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          ...searchKeymap,
          ...foldKeymap,
        ]),
        markdown(),
        engineFeaturesCompartment.of([]),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return;
          if (
            update.transactions.some((transaction) =>
              transaction.annotation(externalUpdate),
            )
          )
            return;
          onChange(update.state.doc.toString());
        }),
      ],
    }),
  });
  let destroyed = false;
  let engineFeaturesEnabled = false;
  let engineFeatures: EngineFeatures | null = null;
  let structureHighlightEnabled = true;
  let gaijiInlayHintsEnabled = true;

  const enableEngineFeatures = (factory: EngineFeatureFactory): void => {
    if (destroyed || engineFeaturesEnabled) return;
    engineFeatures = factory({
      structureHighlight: structureHighlightEnabled,
      gaijiInlayHints: gaijiInlayHintsEnabled,
    });
    view.dispatch({
      effects: engineFeaturesCompartment.reconfigure(engineFeatures.extension),
    });
    engineFeaturesEnabled = true;
  };
  if (engineFeatureFactory) enableEngineFeatures(engineFeatureFactory);

  return {
    enableEngineFeatures,
    refreshLocale: () => {
      if (destroyed) return;
      view.dispatch({
        effects: localeCompartment.reconfigure(localizedEditorExtensions()),
      });
      forceLinting(view);
    },
    setValue: (value: string) => {
      if (destroyed) return;
      if (view.state.doc.toString() === value) return;
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: value },
        annotations: [
          externalUpdate.of(true),
          Transaction.addToHistory.of(false),
        ],
      });
    },
    focus: () => {
      if (!destroyed) view.focus();
    },
    revealRange: (range: TextRange) => {
      if (destroyed) return;
      const from = Math.max(0, Math.min(range.start, view.state.doc.length));
      const to = Math.max(from, Math.min(range.end, view.state.doc.length));
      view.dispatch({
        selection: { anchor: from, head: to },
        effects: EditorView.scrollIntoView(from, {
          y: 'center',
        }),
      });
    },
    runCommand: (commandId: string) => {
      if (destroyed) return false;
      const shape = WRAP_SHAPES.find((candidate) => candidate.id === commandId);
      return shape ? wrapCommand(shape)(view) : false;
    },
    setSetting: (settingId: string, enabled: boolean) => {
      if (settingId === 'structureHighlight') {
        structureHighlightEnabled = enabled;
        if (destroyed || !engineFeaturesEnabled) return;
        view.dispatch({
          effects: structureHighlightCompartment.reconfigure(
            enabled ? (engineFeatures?.structureHighlight ?? []) : [],
          ),
        });
      } else if (settingId === 'gaijiInlayHints') {
        gaijiInlayHintsEnabled = enabled;
        if (destroyed || !engineFeaturesEnabled) return;
        view.dispatch({
          effects: inlayHintsCompartment.reconfigure(
            enabled ? (engineFeatures?.gaijiInlayHints ?? []) : [],
          ),
        });
      }
    },
    destroy: () => {
      if (destroyed) return;
      destroyed = true;
      view.destroy();
    },
  };
}
