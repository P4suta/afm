# 0011. Brand boundary: HTML class rewrite at the aozora-flavored-markdown side

- Status: accepted
- Date: 2026-05-04
- Tags: rendering, css, brand, sibling-repo

## Context

aozora-flavored-markdown composes two renderers: comrak (vanilla CommonMark/GFM HTML) and
the 青空文庫 renderer in the sibling `aozora` repo. The upstream renderer
predates aozora-flavored-markdown and brands its CSS classes `aozora-*`
(`aozora-ruby`, `aozora-bouten-goma`, …); `aozora-tools` and any standalone
consumer expect that prefix. aozora-flavored-markdown's own public surface uses
`aozora-md-*`, styled by the themes this crate ships
(`crates/aozora-flavored-markdown/theme/`, ADR-0020). The two prefixes must be
reconciled at the boundary.

## Decision

Reconcile on the aozora-flavored-markdown side. The AST splicer
(`crates/aozora-flavored-markdown/src/ast_splice.rs`) rewrites every `aozora-*` class token
in spliced aozora-render output to `aozora-md-*` in a single linear pass; data
attributes, link targets, and text bodies are untouched.

Not parameterised upstream (a `class_prefix` knob on `aozora-render`): aozora-flavored-markdown
depends on aozora, not the reverse, so aozora-render keeps its own `aozora-*`
brand for its other consumers, and the rewrite is cheap and idempotent.

## Consequences

- aozora-render stays `aozora-*` for every consumer; aozora-flavored-markdown's public HTML carries
  only `aozora-md-*`, pinned by the `AOZORA_MD_CLASSES` contract in `aozora-flavored-markdown-test-support`
  and the `property_html_shape` sweep.
- New `aozora-*` classes are picked up automatically as long as the prefix holds.
- This is the only aozora-md-side rewrite of upstream HTML (besides the Tier-A `［＃`
  wrapper for unclaimed annotations); adding more needs a follow-up ADR.
