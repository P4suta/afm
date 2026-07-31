// Slug completion + structured snippets for the aozora-md editor.
//
// Ported from aozora's editor/completion.ts (with editor/slugCatalog.ts
// inlined into this single file, as the aozora-md task requires). The structure
// is kept intact; only the project identity, the WASM import (aozora-md uses the
// camelCase `slugsJson()` re-export), and the warn() helper (aozora-md has no
// logger module, so we fall back to console.warn) are adapted to aozora-md.
//
// Two completion behaviours, exactly as in aozora:
//   1) Slug catalogue completion right after a ［＃ / [# annotation opener,
//      driven by `slugsJson()`. Accepting inserts the canonical slug body
//      (consuming any auto-inserted ］) as a snippet with tabstops.
//   2) Single-character structured snippets (＃ / ｜ / 《 / ※) that expand
//      into parameterised Aozora-notation templates the user can tab
//      through. The annotation notation itself is the shared Aozora-bunko
//      syntax that aozora-md's parser understands, so the snippet bodies carry
//      over verbatim.

import {
  autocompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
  snippet,
} from '@codemirror/autocomplete';
import type { EditorView } from '@codemirror/view';

import { type MessageKey, slugDocumentation, t } from '../i18n';
import { slugsJson } from '../wasm-loader';

// ---------------------------------------------------------------------------
// Slug catalogue (inlined from aozora's editor/slugCatalog.ts)
// ---------------------------------------------------------------------------

export interface SlugEntry {
  canonical: string;
  family: string;
  accepts_param: boolean;
  doc: string;
  partner: string | null;
}

let cache: SlugEntry[] | null = null;

/**
 * Load the slug catalogue from the WASM module. Idempotent: the first call
 * serialises via `aozora-flavored-markdown-wasm`'s `slugsJson()` and parses the shared envelope
 * (`{ schemaVersion, data }`); subsequent calls return the cached array.
 *
 * Must be called after the wasm bundle has booted (the editor is created
 * after wasm init, so this is always safe from the completion source).
 */
export function loadSlugCatalog(): SlugEntry[] {
  if (cache) return cache;
  try {
    const env = JSON.parse(slugsJson()) as {
      schemaVersion: number;
      data: SlugEntry[];
    };
    cache = env.data ?? [];
  } catch (err) {
    // aozora-md has no logger module; surface the failure on the console so an
    // empty catalogue is never silently swallowed.
    console.warn('Failed to load slug catalog from WASM:', err);
    cache = [];
  }
  return cache;
}

// ---------------------------------------------------------------------------
// Structured snippets
// ---------------------------------------------------------------------------

/**
 * Structured snippets — single-character triggers that immediately
 * expand into a parameterised template the user can tab through.
 *
 * Design notes:
 * - Use full-width Aozora notation characters throughout. Do not leave
 *   half-width notation in the expanded text.
 * - Keep the trigger character in the snippet. Prefixes such as `｜` and
 *   markers such as `※` carry semantic meaning, so accepting a completion
 *   must not discard the character the user entered.
 * - `${1:placeholder}` defines the initial selection and `${0}` defines the
 *   final cursor position.
 */
interface TriggerSnippet {
  trigger: string;
  snippet: string;
  label: MessageKey;
  detail: MessageKey;
  explicitOnly?: boolean;
}

const TRIGGER_SNIPPETS: TriggerSnippet[] = [
  // ＃ → ［＃...］: a single-line annotation. Slug catalog completion
  // handles the pair that onType inserts from `[`, so this is the fallback
  // for a standalone ＃.
  {
    trigger: '#',
    snippet: '［＃${1:body}］',
    label: 'completionAnnotation',
    detail: 'completionAnnotationDetail',
    explicitOnly: true,
  },
  {
    trigger: '＃',
    snippet: '［＃${1:body}］',
    label: 'completionAnnotation',
    detail: 'completionAnnotationDetail',
  },
  // ｜ → ｜${base}《${reading}》: explicit ruby. Keep the ｜ trigger,
  // select ${base} first, and advance to the reading with Tab.
  {
    trigger: '|',
    snippet: '｜${1:base}《${2:reading}》',
    label: 'completionExplicitRuby',
    detail: 'completionExplicitRubyDetail',
    explicitOnly: true,
  },
  {
    trigger: '｜',
    snippet: '｜${1:base}《${2:reading}》',
    label: 'completionExplicitRuby',
    detail: 'completionExplicitRubyDetail',
  },
  // 《 → 《${reading}》: implicit ruby for the preceding CJK characters.
  {
    trigger: '《',
    snippet: '《${1:reading}》',
    label: 'completionImplicitRuby',
    detail: 'completionImplicitRubyDetail',
  },
  // ※ → ※［＃「${description}」、${mencode}］: a gaiji template.
  {
    trigger: '※',
    snippet: '※［＃「${1:description}」、${2:mencode}］',
    label: 'completionGaiji',
    detail: 'completionGaijiDetail',
  },
];

/** Slug opener forms recognised both as full-width and half-width prefixes. */
const SLUG_OPENERS = ['［＃', '［#', '[＃', '[#'];

function familyToKind(family: string): string {
  switch (family) {
    case 'pageBreak':
    case 'section':
      return 'keyword';
    case 'blockContainerOpen':
    case 'blockContainerClose':
      return 'namespace';
    case 'leafAlign':
      return 'property';
    case 'bouten':
    case 'combineUpright':
    case 'warichu':
    case 'framed':
      return 'function';
    case 'illustration':
      return 'class';
    case 'kaeritenSingle':
    case 'kaeritenCompound':
      return 'enum';
    default:
      return 'text';
  }
}

/**
 * Build a slug completion that replaces every supported opener with the
 * canonical full-width form and consumes an existing closing bracket.
 */
function buildSlugCompletion(entry: SlugEntry, openerFrom: number): Completion {
  const body = entry.accepts_param
    ? entry.canonical.replace(/\{N\}/g, '${1:1}')
    : entry.canonical;

  // A block-container opener inserts its closing marker on a separate line
  // and leaves the final `${0}` cursor position inside the container.
  const template =
    entry.family === 'blockContainerOpen' && entry.partner
      ? `［＃${body}］\n\${0}\n［＃${entry.partner}］`
      : `［＃${body}］\${0}`;

  return {
    label: entry.canonical,
    type: familyToKind(entry.family),
    detail: slugDocumentation(entry.doc),
    apply: (
      view: EditorView,
      completion: Completion,
      _from: number,
      to: number,
    ) => {
      // Extending the replacement through either closing form avoids leaving
      // mixed or duplicate delimiters after canonicalization.
      const doc = view.state.doc;
      const after = doc.sliceString(to, Math.min(to + 1, doc.length));
      const hasClosing = after === '］' || after === ']';
      snippet(template)(view, completion, openerFrom, hasClosing ? to + 1 : to);
    },
  };
}

/**
 * Return one structured snippet completion. The snippet template includes
 * the trigger itself, so the replacement range covers that one trigger
 * character through `context.pos`.
 */
function buildSnippetCompletion(trig: TriggerSnippet): Completion {
  return {
    label: t(trig.label),
    type: 'snippet',
    detail: t(trig.detail),
    apply: snippet(trig.snippet),
  };
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * A structured snippet for the `＃` trigger is redundant immediately after
 * `［＃`, because the opener is already present. Skip it when the preceding
 * two characters are `［＃`.
 */
function isInsideSlugBody(context: CompletionContext): boolean {
  if (context.pos < 2) return false;
  const before = context.state.sliceDoc(context.pos - 2, context.pos);
  return before === '［＃';
}

export const aozoraMdCompletionSource: CompletionSource = (
  context: CompletionContext,
): CompletionResult | null => {
  // 1) Slug completion immediately after ［＃ or [#, while the cursor
  // remains in the annotation body.
  for (const opener of SLUG_OPENERS) {
    const slugMatch = context.matchBefore(
      new RegExp(`${escapeRegex(opener)}([^］\\]\\n]*)$`),
    );
    if (slugMatch) {
      const slugs = loadSlugCatalog();
      const bodyStart = slugMatch.from + opener.length;
      return {
        from: bodyStart,
        to: context.pos,
        options: slugs.map((entry) =>
          buildSlugCompletion(entry, slugMatch.from),
        ),
        validFor: /^[^］\]\n]*$/,
      };
    }
  }

  // 2) Structured snippets triggered by the preceding character.
  for (const trig of TRIGGER_SNIPPETS) {
    if (!context.matchBefore(new RegExp(`${escapeRegex(trig.trigger)}$`)))
      continue;
    if (trig.explicitOnly && !context.explicit) continue;
    // Prefer the slug catalog immediately after ［＃.
    if (
      (trig.trigger === '＃' || trig.trigger === '#') &&
      isInsideSlugBody(context)
    ) {
      continue;
    }
    return {
      from: context.pos - trig.trigger.length,
      to: context.pos,
      options: [buildSnippetCompletion(trig)],
      validFor: /^$/,
    };
  }

  return null;
};

export const aozoraMdCompletion = autocompletion({
  override: [aozoraMdCompletionSource],
  // Aozora notation has no whitespace-delimited words; the default
  // closeOnBlur=true is fine, but we make activate-on-typing snappy.
  activateOnTyping: true,
  defaultKeymap: true,
});
