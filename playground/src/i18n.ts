import type { Locale, LocalizedText } from '@aozora/playground-ui';

const japanese = {
  completionAnnotation: '＃ 注記',
  completionAnnotationDetail: '［＃...］ 一行注記のテンプレート',
  completionExplicitRuby: '｜ ルビ（明示）',
  completionExplicitRubyDetail: '｜base《reading》 で明示ルビ',
  completionGaiji: '※ 外字',
  completionGaijiDetail: '※［＃「desc」、mencode］',
  completionImplicitRuby: '《 ルビ（暗黙）',
  completionImplicitRubyDetail: '直前の漢字に読みを振る',
  completionSlugDetail: '青空文庫注記',
  diagnosticConstructsUnresolved:
    '出力に含められなかった青空文庫記法があります。',
  diagnosticGenericError: '原稿内に修正が必要な記法があります。',
  diagnosticGenericInfo: '記法に関する情報があります。',
  diagnosticGenericWarning: '確認が必要な記法があります。',
  diagnosticPrivateUse: '私用領域文字が含まれています。',
  diagnosticSourceTooLarge: '入力が大きすぎるため解析できません。',
  diagnosticUnclosed: '括弧が閉じられていません。',
  diagnosticUnmatchedClose: '対応する開き括弧がありません。',
  editorPaneTitle: 'Markdown ソース',
  editorPlaceholder: 'Markdown と青空文庫記法を入力…',
  hoverUnresolved: '（未解決）',
  lintPua: '私用領域文字が含まれています（{hex}）',
  lintStrayMarker: '分類できない注記が残っています',
  lintUnclosed: '括弧が閉じられていません',
  lintUnknownCodepoint: '不明',
  lintUnmatched: '対応する開き括弧がありません',
  outlineUntitled: '（無題）',
};

export type MessageKey = keyof typeof japanese;

const english: Record<MessageKey, string> = {
  completionAnnotation: '＃ Annotation',
  completionAnnotationDetail: '［＃...］ one-line annotation template',
  completionExplicitRuby: '｜ Ruby (explicit)',
  completionExplicitRubyDetail: 'Explicit ruby via ｜base《reading》',
  completionGaiji: '※ Gaiji',
  completionGaijiDetail: '※［＃「desc」、mencode］',
  completionImplicitRuby: '《 Ruby (implicit)',
  completionImplicitRubyDetail: 'Add a reading to the preceding kanji',
  completionSlugDetail: 'Aozora notation',
  diagnosticConstructsUnresolved:
    'Some Aozora constructs could not be included in the output.',
  diagnosticGenericError: 'The document contains notation that must be fixed.',
  diagnosticGenericInfo: 'The document contains notation information.',
  diagnosticGenericWarning: 'The document contains notation to review.',
  diagnosticPrivateUse: 'The document contains a private-use character.',
  diagnosticSourceTooLarge: 'The input is too large to analyze.',
  diagnosticUnclosed: 'Unclosed bracket',
  diagnosticUnmatchedClose: 'No matching open bracket',
  editorPaneTitle: 'Markdown source',
  editorPlaceholder: 'Type Markdown and Aozora notation…',
  hoverUnresolved: '(unresolved)',
  lintPua: 'Contains a private-use character ({hex})',
  lintStrayMarker: 'An annotation marker could not be classified',
  lintUnclosed: 'Unclosed bracket',
  lintUnknownCodepoint: 'unknown',
  lintUnmatched: 'No matching open bracket',
  outlineUntitled: '(untitled)',
};

export const CATALOG: Record<Locale, Record<MessageKey, string>> = {
  en: english,
  ja: japanese,
};

let currentLocale: Locale = 'ja';

export function setEditorLocale(locale: Locale): void {
  currentLocale = locale;
}

export function t(key: MessageKey): string {
  return CATALOG[currentLocale][key];
}

export function tf(
  key: MessageKey,
  parameters: Readonly<Record<string, string | number>>,
): string {
  return t(key).replace(/\{(\w+)\}/g, (match, name: string) =>
    name in parameters ? String(parameters[name]) : match,
  );
}

export function slugDocumentation(japaneseDocumentation: string): string {
  return currentLocale === 'ja'
    ? japaneseDocumentation
    : t('completionSlugDetail');
}

export function diagnosticText(
  code: string,
  severity: 'error' | 'note' | 'warning',
  englishMessage: string,
): LocalizedText {
  let key: MessageKey;
  if (code === 'aozora-md::source_too_large') {
    key = 'diagnosticSourceTooLarge';
  } else if (code === 'aozora-md::constructs_unresolved') {
    key = 'diagnosticConstructsUnresolved';
  } else if (/unmatched.*close|unmatched_close/.test(code)) {
    key = 'diagnosticUnmatchedClose';
  } else if (/unclosed/.test(code)) {
    key = 'diagnosticUnclosed';
  } else if (/private|pua/.test(code)) {
    key = 'diagnosticPrivateUse';
  } else {
    key =
      severity === 'error'
        ? 'diagnosticGenericError'
        : severity === 'warning'
          ? 'diagnosticGenericWarning'
          : 'diagnosticGenericInfo';
  }
  return { ja: CATALOG.ja[key], en: englishMessage };
}
