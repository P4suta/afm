// Derive a heading outline from the aozora-md IR.
//
// aozora-md already ships heading positions in its IR — Markdown `#` and
// 青空文庫 `［＃大見出し］` both arrive as `heading` blocks carrying a
// `sourceLine` — so the outline needs no extra WASM call, unlike the sibling
// aozora playground, which reads a dedicated nodes_json. We walk the block
// tree (descending into blockquote / list) and flatten the headings in
// document order.

import type { IrBlock, IrDocument, IrInline } from './wasm-loader';

export interface OutlineEntry {
  readonly level: number;
  readonly text: string;
  /** 1-based source line, when the renderer attached one. */
  readonly sourceLine: number | null;
}

/**
 * Visible text of one 青空文庫 fragment, with ruby readings dropped.
 *
 * The IR hands every notation over as rendered HTML rather than as a typed
 * node, so the heading text is read back out of the fragment. Two kinds of
 * markup are stripped first, because `textContent` reports both and neither
 * is text the reader sees: `<rt>` / `<rp>` hold the ruby reading (`おうめ`),
 * and an `aozora-md-directive` wrapper is `hidden`.
 */
function aozoraText(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  for (const el of doc.querySelectorAll('rt, rp, .aozora-md-directive')) el.remove();
  return doc.body.textContent ?? '';
}

/** Flatten an inline run to its visible text (ruby readings excluded). */
function inlineText(nodes: readonly IrInline[]): string {
  let out = '';
  for (const node of nodes) {
    switch (node.kind) {
      case 'text':
      case 'code':
        out += node.value;
        break;
      case 'strong':
      case 'emphasis':
        out += inlineText(node.children);
        break;
      case 'link':
        out += inlineText(node.children);
        break;
      case 'image':
        out += inlineText(node.alt);
        break;
      case 'aozora':
        out += aozoraText(node.html);
        break;
      // lineBreak contributes no heading text.
      default:
        break;
    }
  }
  return out;
}

function collect(blocks: readonly IrBlock[], acc: OutlineEntry[]): void {
  for (const block of blocks) {
    switch (block.kind) {
      case 'heading':
        acc.push({
          level: block.level,
          text: inlineText(block.children).trim() || '(無題)',
          sourceLine: block.sourceLine ?? null,
        });
        break;
      case 'blockquote':
        collect(block.children, acc);
        break;
      case 'list':
        for (const item of block.items) collect(item.children, acc);
        break;
      default:
        break;
    }
  }
}

export function outlineFromIr(ir: IrDocument): OutlineEntry[] {
  const acc: OutlineEntry[] = [];
  collect(ir.blocks, acc);
  return acc;
}
