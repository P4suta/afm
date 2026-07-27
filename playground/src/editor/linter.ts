// Ported from aozora's playground/src/editor/linter.ts.
//
// In-editor diagnostics for the aozora-md playground. The parserStateField
// owns the single aozora-flavored-markdown-wasm AozoraDocument and exposes the parsed
// `diagnostics_json` payload as `ps.diagnostics` (DiagnosticEntry[],
// spans in UTF-8 byte offsets). This module translates those entries
// into CodeMirror lint Diagnostics so they render as squigglies plus
// a gutter marker.
//
// aozora-md is "Aozora Flavored Markdown" and embeds the aozora parser, so
// the diagnostic `kind`s arriving over the wire are aozora-notation
// diagnostics. The wording below is tuned for casual playground use
// rather than typesetter precision.
import { type Diagnostic, linter, lintGutter } from '@codemirror/lint';
import type { EditorView } from '@codemirror/view';
import {
  byteToUtf16,
  type DiagnosticEntry,
  type ParserState,
  parserStateField,
} from './parserState';

/**
 * Map a raw diagnostic `kind` from aozora-flavored-markdown-wasm to a CM6 severity + a
 * Japanese message. The kinds originate from the embedded aozora
 * parser's `diagnostics_json`. Unknown kinds fall through as `info`
 * with the raw kind name displayed so nothing is silently dropped.
 */
function classify(entry: DiagnosticEntry): {
  severity: Diagnostic['severity'];
  message: string;
} {
  switch (entry.kind) {
    case 'unclosed_bracket':
      return { severity: 'error', message: '括弧が閉じられていません' };
    case 'unmatched_close':
      return { severity: 'error', message: '対応する開き括弧がありません' };
    case 'source_contains_pua': {
      const hex = entry.codepoint
        ? `U+${entry.codepoint.toString(16).toUpperCase().padStart(4, '0')}`
        : '不明';
      return {
        severity: 'warning',
        message: `Private Use Area の文字が含まれています (${hex})`,
      };
    }
    case 'residual_annotation_marker':
      return { severity: 'warning', message: '注記マーカーが残存しています' };
    default:
      return { severity: 'info', message: entry.kind };
  }
}

function lintSource(view: EditorView): readonly Diagnostic[] {
  const ps: ParserState = view.state.field(parserStateField);
  const entries = ps.diagnostics;
  if (entries.length === 0) return [];
  const docLen = view.state.doc.length;
  const out: Diagnostic[] = [];
  for (const entry of entries) {
    const { severity, message } = classify(entry);
    let from = byteToUtf16(ps, entry.span.start);
    let to = byteToUtf16(ps, entry.span.end);
    // Clamp to doc bounds; widen 0-width diagnostics by 1 so they
    // are visible in the gutter / underline.
    if (from < 0) from = 0;
    if (to > docLen) to = docLen;
    if (from === to) {
      if (to < docLen) to = from + 1;
      else if (from > 0) from = to - 1;
    }
    if (from > to) continue;
    out.push({ from, to, severity, message, source: 'aozora-md' });
  }
  return out;
}

export const aozoraMdLinter = linter(lintSource, {
  // The parse runs synchronously in parserStateField. We do not want
  // to throttle squigglies; latency stays sub-millisecond on typical
  // playground documents. Override the default 750ms.
  delay: 50,
});

export const aozoraMdLintGutter = lintGutter();
