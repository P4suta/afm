import type {
  LocalizedText,
  PlaygroundAdapter,
  PlaygroundGuide,
  PlaygroundSample,
} from '@aozora/playground-ui';

import wasmPackage from '../../crates/aozora-flavored-markdown-wasm/pkg/package.json';
import type { EngineAwareEditorController } from './editor';
import type { EngineFeatureFactory } from './editor/compartments';
import { loadExamples } from './examples';
import { setEditorLocale } from './i18n';
import { normalizeSourceLineEndings } from './source';

type EngineModule = typeof import('./adapter-engine');
type EditorModule = typeof import('./editor');
type EngineFeatureModule = typeof import('./editor/engineFeatures');

let enginePromise: Promise<EngineModule> | null = null;
let engineModule: EngineModule | null = null;
let editorPromise: Promise<EditorModule> | null = null;
let engineFeaturePromise: Promise<EngineFeatureModule> | null = null;
let engineFeatureFactory: EngineFeatureFactory | null = null;
let initializationPromise: Promise<void> | null = null;
let engineReady = false;
const activeEditors = new Set<EngineAwareEditorController>();

async function loadEngine(): Promise<EngineModule> {
  enginePromise ??= import('./adapter-engine').catch((error: unknown) => {
    enginePromise = null;
    throw error;
  });
  return enginePromise;
}

function readyEngine(): EngineModule {
  if (!engineReady || !engineModule) {
    throw new Error('playground engine used before initialization');
  }
  return engineModule;
}

function loadEditor(): Promise<EditorModule> {
  editorPromise ??= import('./editor');
  return editorPromise;
}

function loadEngineFeatures(): Promise<EngineFeatureModule> {
  engineFeaturePromise ??= import('./editor/engineFeatures').catch(
    (error: unknown) => {
      engineFeaturePromise = null;
      throw error;
    },
  );
  return engineFeaturePromise;
}

async function initializeAdapterEngine(): Promise<void> {
  if (engineReady) return;
  initializationPromise ??= (async () => {
    const engine = await loadEngine();
    await engine.initializeEngine();
    const features = await loadEngineFeatures();
    engineModule = engine;
    engineFeatureFactory = features.createEngineFeatures;
    for (const editor of activeEditors) {
      editor.enableEngineFeatures(engineFeatureFactory);
    }
    engineReady = true;
  })().catch((error: unknown) => {
    initializationPromise = null;
    throw error;
  });
  await initializationPromise;
}

const commandLabels = {
  'aozora-md.wrap.ruby': { ja: 'ルビ', en: 'Ruby' },
  'aozora-md.wrap.angleQuote': {
    ja: '二重山括弧',
    en: 'Double angle brackets',
  },
  'aozora-md.wrap.bouten': { ja: '傍点', en: 'Emphasis dots' },
  'aozora-md.wrap.kagikakko': { ja: '鉤括弧で囲む', en: 'Wrap in 「」' },
  'aozora-md.wrap.kikkou': { ja: '亀甲括弧で囲む', en: 'Wrap in 〔〕' },
  'aozora-md.wrap.chuki': { ja: '注記で囲む', en: 'Wrap in ［＃］' },
} as const satisfies Readonly<Record<string, LocalizedText>>;

type CommandId = keyof typeof commandLabels;

const shortcuts: Partial<Readonly<Record<CommandId, string>>> = {
  'aozora-md.wrap.ruby': 'Ctrl/⌘+Alt+R',
  'aozora-md.wrap.angleQuote': 'Ctrl/⌘+Alt+Shift+R',
  'aozora-md.wrap.bouten': 'Ctrl/⌘+Alt+B',
};

const commandIds: readonly CommandId[] = [
  'aozora-md.wrap.ruby',
  'aozora-md.wrap.angleQuote',
  'aozora-md.wrap.bouten',
  'aozora-md.wrap.kagikakko',
  'aozora-md.wrap.kikkou',
  'aozora-md.wrap.chuki',
] as const;

const guide: PlaygroundGuide = {
  title: {
    ja: 'aozora-md 記法ガイド',
    en: 'aozora-md notation guide',
  },
  introduction: {
    ja: 'CommonMark と GFM に青空文庫記法を重ね、Markdown の中でルビ、傍点、縦中横などを利用できます。',
    en: 'aozora-md layers Aozora Bunko notation over CommonMark and GFM, adding Japanese typography such as ruby, emphasis dots, and tate-chu-yoko.',
  },
  sections: [
    {
      id: 'ruby',
      title: { ja: 'ルビ', en: 'Ruby' },
      body: {
        ja: '漢字の直後に読みを置くか、縦線で対象範囲を明示します。',
        en: 'Place a reading after kanji, or use a vertical bar to mark an explicit base range.',
      },
      example: '吾輩《わがはい》は｜青梅《おうめ》に行った。',
    },
    {
      id: 'mixed',
      title: { ja: 'Markdown との混在', en: 'Mixing with Markdown' },
      body: {
        ja: '見出し、強調、表、タスクリストなどの CommonMark/GFM 記法と同じ文書で使用できます。コード内では青空文庫記法を解釈しません。',
        en: 'Use headings, emphasis, tables, and task lists in the same document. Aozora notation is not interpreted inside code.',
      },
      example:
        '# 第一章\n\n彼は｜青梅《おうめ》に行った。\n\n| 表記 | 読み |\n|---|---|\n| 漢字《かんじ》 | ruby |',
    },
    {
      id: 'commands',
      title: { ja: '編集支援', en: 'Editing assistance' },
      body: {
        ja: 'テキストを選択して記法コマンドを実行すると、ルビや傍点などで囲めます。Ctrl/⌘+Shift+P でコマンドパレットを開きます。',
        en: 'Select text and run a notation command to wrap it in ruby, emphasis dots, and more. Open the command palette with Ctrl/⌘+Shift+P.',
      },
    },
    {
      id: 'specification',
      title: { ja: '仕様', en: 'Specifications' },
      body: {
        ja: '青空文庫記法の詳細は仕様書を参照してください。',
        en: 'See the specification for full Aozora notation details.',
      },
      href: 'https://p4suta.github.io/aozora-notation-spec/',
    },
  ],
};

function samples(): readonly PlaygroundSample[] {
  return loadExamples().map((example) => ({
    id: example.slug,
    title: example.label,
    source: example.source,
  }));
}

const playgroundSamples = samples();

export const afmPlaygroundAdapter: PlaygroundAdapter = {
  product: {
    id: 'aozora-flavored-markdown',
    name: 'Aozora Flavored Markdown',
    shortName: 'aozora-md',
    description: {
      ja: 'CommonMark + GFM + 青空文庫記法',
      en: 'CommonMark + GFM + Aozora Bunko notation',
    },
    repositoryUrl: 'https://github.com/P4suta/aozora-flavored-markdown',
    engineVersion: wasmPackage.version,
  },
  samples: playgroundSamples,
  guide,
  commands: commandIds.map((id) => ({
    id,
    label: commandLabels[id],
    ...(shortcuts[id] ? { shortcut: shortcuts[id] } : {}),
  })),
  settings: [
    {
      id: 'structureHighlight',
      label: { ja: '構造ハイライト', en: 'Structure highlighting' },
      description: {
        ja: '見出し・ルビ・傍点・注記をエディタ上で識別します。',
        en: 'Identify headings, ruby, emphasis dots, and annotations in the editor.',
      },
      defaultValue: true,
    },
    {
      id: 'gaijiInlayHints',
      label: { ja: '外字インレイヒント', en: 'Gaiji inlay hints' },
      description: {
        ja: '外字注記の後ろに解決された文字を表示します。',
        en: 'Show resolved characters after gaiji annotations.',
      },
      defaultValue: true,
    },
  ],
  createEditorDuringInitialization: true,
  setLocale: setEditorLocale,
  async initialize() {
    await initializeAdapterEngine();
  },
  async analyze(source, context) {
    return readyEngine().analyze(normalizeSourceLineEndings(source), context);
  },
  async createEditor(parent, initialValue, onChange) {
    const editor = await loadEditor();
    const controller = editor.createEditor(
      parent,
      normalizeSourceLineEndings(initialValue),
      onChange,
      engineReady ? engineFeatureFactory : null,
    );
    activeEditors.add(controller);
    let destroyed = false;
    return {
      ...controller,
      setValue(value) {
        controller.setValue(normalizeSourceLineEndings(value));
      },
      destroy() {
        if (destroyed) return;
        destroyed = true;
        activeEditors.delete(controller);
        controller.destroy();
      },
    };
  },
  createPreview(parent) {
    return readyEngine().createPreview(parent);
  },
};
