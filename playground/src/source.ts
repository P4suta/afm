/**
 * Match CodeMirror's document model at every engine-facing source boundary.
 *
 * Shared URLs and persisted drafts keep their original text until the author
 * edits it, but editor selections and analysis ranges always use LF offsets.
 */
export function normalizeSourceLineEndings(source: string): string {
  return source.includes('\r') ? source.replace(/\r\n?/g, '\n') : source;
}
