# 0011. Brand boundary: HTML class rewrite at the aozora-flavored-markdown side

- Status: accepted (rewrite site and contract owner restated 2026-07-25)
- Date: 2026-05-04
- Tags: rendering, css, brand, sibling-repo

> **Amendment (2026-07-25, aozora 0.5.0 follow-through):** the rewrite
> moved with the HTML it rewrites. A construct's fragment is produced in
> `crates/aozora-flavored-markdown/src/fragment.rs`
> ([ADR-0021](0021-aozora-boundary-is-the-public-surface.md)), and the
> rebrand happens there, scoped to `class="…"` attribute values — an
> author's own text may say `aozora-`, and used to be rewritten with it.
> The contract it produces, `AOZORA_MD_CLASSES`, is public API of this
> crate now and is *derived* from the parser's published class list rather
> than hand-kept ([ADR-0020](0020-canonicalise-aozora-md-css-at-the-next-aozora-bump.md)).
> The decision below — reconcile the two prefixes on this side, in one
> pass, touching nothing but class tokens — is unchanged.

## Context

aozora-flavored-markdown composes two renderers: comrak (vanilla CommonMark/GFM HTML) and
the 青空文庫 renderer in the sibling `aozora` repo. The upstream renderer
predates aozora-flavored-markdown and brands its CSS classes `aozora-*`
(`aozora-ruby`, `aozora-bouten-goma`, …); the parser's own CLI and any
standalone consumer expect that prefix. aozora-flavored-markdown's own public surface uses
`aozora-md-*`, styled by the themes this crate ships
(`crates/aozora-flavored-markdown/theme/`, ADR-0020). The two prefixes must be
reconciled at the boundary.

## Decision

Reconcile on the aozora-flavored-markdown side: every `aozora-*` class token in the HTML
this crate takes from the parser is rewritten to `aozora-md-*` in a single
linear pass; data attributes, link targets, and text bodies are untouched.

Not parameterised upstream (a `class_prefix` knob on the parser): aozora-flavored-markdown
depends on aozora, not the reverse, so the parser keeps its own `aozora-*`
brand for its other consumers, and the rewrite is cheap and idempotent.

## Consequences

- The parser stays `aozora-*` for every consumer; aozora-flavored-markdown's public HTML carries
  only `aozora-md-*`, pinned by the `AOZORA_MD_CLASSES` contract and the
  `property_html_shape` sweep.
- New `aozora-*` classes are picked up automatically as long as the prefix holds.
- The class rewrite is **not** the only place this crate depends on the shape of
  the HTML it takes from the parser, and this ADR must not be read as saying so.
  Three others are load-bearing today: the Tier-A `［＃` wrapper this crate emits
  for an unclaimed annotation (`src/ast_splice.rs`); `src/fragment.rs` dropping
  the `<p>` … `</p>` wrapper an inline construct's sub-document arrives in, and
  splitting a container marker's fragment at its own closing tag; and
  `src/constructs.rs` reading a heading hint's `data-level` / `data-target` off
  that fragment. Each follows from
  [ADR-0021](0021-aozora-boundary-is-the-public-surface.md)'s choice to render
  *through* the parser instead of re-implementing the notation here — reading
  its output is what that choice costs — and each fails loudly in
  `tests/aozora_parity.rs` if the shape moves. What still needs a follow-up ADR
  is a rewrite that changes what the markup *means*, rather than fitting a
  document-shaped answer into a fragment slot.
