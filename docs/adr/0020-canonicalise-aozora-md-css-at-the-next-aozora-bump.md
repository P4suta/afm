# 0020. Canonicalise the aozora-md CSS at the next aozora bump

- Status: proposed (scheduled — executed in the aozora 0.5.0 follow-through, PR7)
- Date: 2026-06-22
- Deciders: @P4suta
- Tags: architecture, epub, css, deferred

> **Scheduling note (2026-07-25):** the "next aozora bump" is 0.5.0. Steps 1–3
> below land in PR7 of the 0.5.0 follow-through (see [ADR-0021](0021-aozora-boundary-is-the-public-surface.md));
> `AOZORA_CLASSES` there grew from 19 to 80+ entries and includes the
> `aozora-double-ruby` → `aozora-angle-quote` rename this ADR anticipated.
> Step 4 is already done: the mdbook site was retired and the canonical themes
> now live at the repo-root `theme/`.

## Context

The EPUB generator vendors its own copy of the two `aozora-md-*` themes
(`crates/aozora-flavored-markdown-epub/assets/aozora-md-{horizontal,vertical}.css`),
while the repo-root `theme/` ships a near-identical canonical
pair. That is byte duplication of CSS that must track the renderer's emitted
classes. The ideal is a single owner: the crate that emits the classes also owns
their stylesheet, and the EPUB generator consumes it through a normal crate
dependency (so `cargo publish` bundles it — an `include_bytes!` reaching outside
a crate's own directory would not).

This cannot be done cleanly *right now*, for two reasons tied to the current
pin:

1. **The class contract is not public.** `AOZORA_MD_CLASSES` lives in
   `aozora-flavored-markdown-test-support` (a `publish = false` dev-only crate),
   not in the library's public API. The principled design — mirroring `aozora`'s
   `aozora-render::AOZORA_CLASSES` + an auto-drift test that enumerates every
   emitted class — has to be promoted into the library first.
2. **The class names are mid-rename upstream.** `aozora` has already renamed
   `aozora-double-ruby` → `aozora-angle-quote` in its source (unreleased; the
   published 0.4.1 this repo depends on still uses the old name). Pinning a
   canonical stylesheet against today's `aozora-md-double-ruby` would be redone
   the moment this repo bumps its `aozora` dependency.

Both `aozora` and `aozora-flavored-markdown` are maintained by the same author,
so this is release sequencing within reach — not a cross-org coordination
problem, and not something to file upstream.

## Decision (proposed)

Defer CSS canonicalisation to the next `aozora` dependency bump. At that bump:

1. Promote the class contract into the `aozora-flavored-markdown` library as
   public API (`AOZORA_MD_CLASSES` + an auto-drift test that enumerates every
   emitted class, per the `aozora-render::AOZORA_CLASSES` pattern), so a renamed
   class can never silently ship.
2. Add a default-off `theme` feature to the library exposing the canonical
   horizontal/vertical CSS as `pub const` strings (pure data — no new heavy
   dependencies, no impact on parser-only consumers).
3. Have `aozora-flavored-markdown-epub` (and any future PDF crate) enable
   `features = ["theme"]` and embed the consts, deleting its vendored
   `assets/*.css` copy.
4. Keep the repo-root `theme/` as that same canonical source (done).

Until then (against published `aozora` 0.4.1), the EPUB crate keeps its vendored
CSS copy — necessary for publishability regardless — guarded by a theme-coverage
test that reads `AOZORA_MD_CLASSES` from `aozora-flavored-markdown-test-support`
([ADR-0018](0018-consolidate-the-epub-generator-into-this-workspace.md)). That
already catches a missing theme rule automatically; canonicalisation is a
maintenance-burden cleanup, not a correctness gap.

## Consequences

- One pre-existing CSS duplication remains until the bump, with no silent drift
  (the coverage test fires on a class the themes do not style).
- The bump becomes the single point where the rename, the public class contract,
  and the `theme` feature land together — no double work.

## Alternatives considered

- **Do it now against 0.4.1.** Rejected: pins canonical CSS to soon-to-be-renamed
  class names; the work would be redone at the bump.
- **Ship CSS from `aozora` and have everyone derive from it.** Rejected:
  `aozora` deliberately publishes the *class contract* only and leaves CSS to
  consumers; `aozora-flavored-markdown` renames classes to `aozora-md-*`
  (ADR-0011) and owns its own themes, so the library — not `aozora` — is the
  right owner.

## References

- [ADR-0018](0018-consolidate-the-epub-generator-into-this-workspace.md) — interim CSS handling.
- [ADR-0011](0011-brand-boundary-css-class-rewrite.md) — the `aozora-md-*` brand boundary.
- aozora `aozora-render::AOZORA_CLASSES` + its `class_list_matches_emitted` test — the contract pattern to adopt.
