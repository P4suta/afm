# 0022. Collapse the Aozora half of the IR to `{kind, span, html}`

- Status: accepted
- Date: 2026-07-25
- Deciders: @P4suta
- Tags: ir, api, boundary, typescript, breaking

## Context

The IR had two halves. The Markdown half — paragraph, heading, list, table,
code, link, image — is this crate's own vocabulary. The Aozora half was a
second copy of the sibling parser's: `Ruby` carried `base` / `reading` /
`explicit`, `Bouten` carried a style and a position enum, `Container` carried
a subtype and an indent level, and four classification enums existed only to
restate upstream's own.

Roughly 480 lines of projection code existed to translate one vocabulary into
the other, and every notation upstream added meant a new variant, a new enum
arm, and a new `.d.ts` union member here. The mirror was also lossy in the
other direction: constructs with no hand-written projection (a leaf indent
marker, 返り点, an illustration) rendered into the HTML but silently vanished
from the IR.

[ADR-0021](0021-aozora-boundary-is-the-public-surface.md) settled where the
boundary sits. This is the same rule applied to the IR: if the notation is
upstream's to define, its vocabulary cannot be re-declared here.

## Decision

Every 青空文庫 construct projects to **one** variant per level —
`IrBlock::Aozora` and `IrInline::Aozora` — carrying three things:

- `kind` — an opaque notation tag (`"ruby"`, `"bouten"`, `"containerOpen"`,
  …), serialised as `aozoraKind` because `kind` is already the union's serde
  discriminant. Consumers treat it as an open string set.
- `span` — the byte range the notation occupied in the source, so a consumer
  can slice back to what the author typed. **Optional, and reported only
  when it is true.** The parser measures spans against its own normalised
  text, and normalisation moves bytes (BOM strip, `\r\n` → `\n`, accent
  decomposition inside `〔…〕`, decorative-rule isolation). On such an input
  the offsets address a different — possibly mid-codepoint — position in the
  caller's source, so the projection withholds the span instead of
  publishing coordinates no consumer holds. Moving the whole pipeline to
  source coordinates is the follow-up that makes this unconditional.
- `html` — the rendered fragment, produced by the same renderer the HTML
  splice uses, already rebranded to `aozora-md-*` (ADR-0011).

The Markdown half is untouched and stays typed.

Three consequences are deliberate:

1. **Paired containers do not nest in the IR.** Their open and close markers
   are two sibling `Aozora` blocks in document order, each with its own span
   and its own half of the HTML. A single `span` field cannot describe a
   construct with two markers, and the nesting is recoverable from the
   ordering — so the IR reports what the source says instead of re-deriving a
   tree.
2. **Every construct reaches the IR.** Since projection no longer requires a
   hand-written variant, the constructs that used to drop out now appear.
3. **Context beats vocabulary.** `html` promises byte-identity with what the
   same notation contributes to the rendered document, and that is a claim
   about *whether* it renders as much as about what it renders to. The
   splice suppresses an annotation inside a heading body (Tier C) and
   promotes a hint-bearing paragraph to a heading wherever it sits; the
   projection makes the same two calls, so no consumer can render markup
   from the IR that `render` does not emit.

`#[non_exhaustive]` stays on both enums (ADR-0013), but it now covers only
Markdown growth: a new notation is a new `kind` string, not a new variant.

## Consequences

- `ir/projection.rs` is deleted, and the `ContainerSubtype` / `SectionSubtype`
  / `BoutenStyle` / `BoutenPosition` / `AnnotationKind` enums with it. A
  notation added upstream needs no change here to render, and none to reach
  the IR.
- **Breaking for Rust and TypeScript consumers.** The `IrInline::Ruby` /
  `Bouten` / `Tcy` / `Gaiji` / `Annotation` / `DoubleRuby` and `IrBlock::
  PageBreak` / `SectionBreak` / `Container` variants are gone; the
  tsify-derived `.d.ts` changes shape to match.
  `aozora-flavored-markdown-obsidian` is an archived snapshot (last push
  2026-05-23) and has no follow-up to make; the playground's outline reads
  heading text out of the `html` fragment instead of a typed node.
- A consumer that wants ruby *readings* — as data, not markup — no longer
  gets them handed over. It reads them from the fragment, or slices the span
  and asks the parser. That is the cost of not owning the notation twice.
- The IR's `html` and the document's HTML now come from one call, so they
  cannot disagree about what a notation renders to. `tests/ir_aozora.rs`
  looks for every projected fragment in the rendered document — over heading
  and nested-hint input as well as paragraph input, since those are where
  the context rules above decide the answer.
- The per-block entry point (`render_blocks_to_ir`) drains the container
  stack at the end, so a container the source never closed produces its
  synthesised close block there, matching the closing tag the HTML side
  appends. Both outputs of one call therefore balance. `StreamingIrBuilder`
  exposes the same drain as `finish()` for callers driving it themselves.
- Spans are absent on input the parser had to normalise (see the Decision).
  A consumer that needs them for CRLF or BOM-prefixed source normalises the
  text before handing it over, or waits for the source-coordinate pipeline.

## Alternatives considered

**Keep the typed variants and bump them per release.** What we had. It costs a
variant, an enum arm, a `.d.ts` member and a test per notation, forever, and
it made the IR quietly incomplete whenever that work lagged.

**Collapse but keep container children.** `Aozora { kind, span, html,
children }` would preserve nesting. Rejected: a container has two markers and
two spans, so one span/html pair cannot describe it honestly, and the walker
would keep a tree-assembly stack whose only job is re-deriving what the marker
order already states.

**Build a typed projection from public projections after the 0.5.0 bump.**
Rejected for the reason ADR-0021 gives: the published node kinds are family
tags, so recovering bouten kind, heading level or kaeriten would mean
re-parsing the notation here.

## References

- [ADR-0021](0021-aozora-boundary-is-the-public-surface.md) — the boundary rule this applies.
- [ADR-0013](0013-public-ir-enums-non-exhaustive.md) — amended: `#[non_exhaustive]` now covers Markdown growth only.
- [ADR-0017](0017-derive-typescript-types-with-tsify.md) — amended: the derived union changes shape.
- [ADR-0011](0011-brand-boundary-css-class-rewrite.md) — the `aozora-md-*` rebrand the fragments carry.
