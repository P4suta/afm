// The retired names must not reappear: in a browser-facing module they
// shadow lib.dom's Document and Range. `@ts-expect-error` makes their absence
// part of the real tsc build instead of a text scan over the generated file.
// biome-ignore assist/source/organizeImports: each error directive must stay attached to its failing import.
import type {
  ByteSpan,
  MarkdownDocument,
  Options,
  SourcePosition,
  SourceRange,
} from 'aozora-flavored-markdown-wasm';
// @ts-expect-error Document was renamed to MarkdownDocument.
import type { Document as LegacyDocument } from 'aozora-flavored-markdown-wasm';
// @ts-expect-error Range was renamed to SourceRange.
import type { Range as LegacyRange } from 'aozora-flavored-markdown-wasm';

export type WasmPackageTypeContract = {
  document: MarkdownDocument;
  range: SourceRange;
  position: SourcePosition;
  span: ByteSpan;
  options: Options;
};

// Prove that the DOM globals retain their ordinary meaning in the same
// compilation unit that imports the package declarations.
export function acceptsDomTypes(
  documentValue: Document,
  rangeValue: Range,
): [Document, Range] {
  return [documentValue, rangeValue];
}

// Excess-property checking catches the retired alias at authoring time, while
// serde's deny_unknown_fields catches it at the runtime boundary.
export const retiredOptionsAlias = {
  // @ts-expect-error aozoraEnabled was retired; use aozora.
  aozoraEnabled: false,
} satisfies Options;

export type RetiredNamesStayUnusable = [LegacyDocument, LegacyRange];
