import type {
  AnalyzeContext,
  PlaygroundAnalysis,
  PlaygroundDiagnostic,
  PlaygroundOutlineEntry,
  PreviewController,
  TextRange,
  WritingDirection,
} from '@aozora/playground-ui';

import { diagnosticText } from './i18n';
import { outlineFromIr } from './outline';
import { THEME_URLS } from './styles/theme-urls';
import {
  type Diagnostic,
  type IrDocument,
  initializeWasm,
  render,
} from './wasm-loader';

function utf8Width(codePoint: number): number {
  if (codePoint <= 0x7f) return 1;
  if (codePoint <= 0x7ff) return 2;
  if (codePoint <= 0xffff) return 3;
  return 4;
}

export function buildUtf8OffsetTable(source: string): Uint32Array {
  let byteLength = 0;
  for (let utf16Offset = 0; utf16Offset < source.length; utf16Offset++) {
    const codePoint = source.codePointAt(utf16Offset);
    if (codePoint === undefined) break;
    byteLength += utf8Width(codePoint);
    if (codePoint > 0xffff) utf16Offset++;
  }
  const table = new Uint32Array(byteLength + 1);
  let byteOffset = 0;
  for (let utf16Offset = 0; utf16Offset < source.length; utf16Offset++) {
    const codePoint = source.codePointAt(utf16Offset);
    if (codePoint === undefined) break;
    const width = utf8Width(codePoint);
    for (let index = 0; index < width; index++) {
      table[byteOffset + index] = utf16Offset;
    }
    byteOffset += width;
    if (codePoint > 0xffff) utf16Offset++;
  }
  table[byteOffset] = source.length;
  return table;
}

function toUtf16Range(
  table: Uint32Array,
  span: { readonly start: number; readonly end: number },
): TextRange {
  const last = table.length - 1;
  const start = table[Math.min(span.start, last)] ?? 0;
  const end = table[Math.min(span.end, last)] ?? start;
  return { start, end: Math.max(start, end) };
}

export function normalizeDiagnostics(
  source: string,
  diagnostics: readonly Diagnostic[],
): readonly PlaygroundDiagnostic[] {
  if (diagnostics.length === 0) return [];
  const table = buildUtf8OffsetTable(source);
  return diagnostics.map((diagnostic) => ({
    severity: diagnostic.severity === 'note' ? 'info' : diagnostic.severity,
    message: diagnosticText(
      diagnostic.code,
      diagnostic.severity,
      diagnostic.message,
    ),
    range: toUtf16Range(table, diagnostic.span),
    code: diagnostic.code,
  }));
}

export function buildLineRanges(source: string): readonly TextRange[] {
  const ranges: TextRange[] = [];
  let start = 0;
  for (let index = 0; index < source.length; index++) {
    const character = source[index];
    if (character !== '\n' && character !== '\r') continue;
    ranges.push({ start, end: index });
    if (character === '\r' && source[index + 1] === '\n') index++;
    start = index + 1;
  }
  ranges.push({ start, end: source.length });
  return ranges;
}

export function normalizeOutline(
  source: string,
  document: IrDocument,
): readonly PlaygroundOutlineEntry[] {
  const entries = outlineFromIr(document);
  if (entries.length === 0) return [];
  const lines = buildLineRanges(source);
  const end = { start: source.length, end: source.length };
  return entries.map((entry) => ({
    level: entry.level,
    text: entry.text,
    range:
      entry.sourceLine === null ? null : (lines[entry.sourceLine - 1] ?? end),
  }));
}

export async function initializeEngine(): Promise<void> {
  await initializeWasm();
}

export async function analyze(
  source: string,
  context: AnalyzeContext,
): Promise<PlaygroundAnalysis> {
  if (context.signal.aborted) throw new DOMException('Aborted', 'AbortError');
  const result = render(source, { sourceLineAnchors: true });
  if (context.signal.aborted) throw new DOMException('Aborted', 'AbortError');
  return {
    html: result.html,
    diagnostics: normalizeDiagnostics(source, result.diagnostics),
    outline: normalizeOutline(source, result.ir),
  };
}

export function createPreview(parent: HTMLElement): PreviewController {
  const root = document.createElement('div');
  root.className = 'aozora-md-root';
  parent.replaceChildren(root);
  let previousHtml: string | null = null;
  return {
    update(html: string, direction: WritingDirection) {
      const theme = document.getElementById('aozora-md-theme');
      if (theme instanceof HTMLLinkElement) {
        theme.rel = 'stylesheet';
        theme.href = THEME_URLS[direction];
      }
      document.documentElement.dataset.aozoraMdTheme = direction;
      if (html !== previousHtml) {
        root.innerHTML = html;
        previousHtml = html;
      }
    },
    destroy() {
      root.remove();
    },
  };
}
