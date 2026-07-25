# 0020. Canonicalise the aozora-md CSS at the next aozora bump

- Status: accepted (executed 2026-07-25 in the aozora 0.5.0 follow-through, PR7)
- Date: 2026-06-22
- Deciders: @P4suta
- Tags: architecture, epub, css, deferred

> **Execution note (2026-07-25):** the "next aozora bump" was 0.5.0, where
> `AOZORA_CLASSES` grew from 19 to 91 entries and carried the
> `aozora-double-ruby` → `aozora-angle-quote` rename this ADR anticipated.
> All four steps have landed (see
> [ADR-0021](0021-aozora-boundary-is-the-public-surface.md) for the boundary
> they sit on):
>
> 1. `AOZORA_MD_CLASSES` (+ `is_contract_class`) is public API of
>    `aozora-flavored-markdown`, derived from the parser's list.
> 2. The default-off `theme` feature publishes the two stylesheets as
>    `theme::HORIZONTAL_CSS` / `theme::VERTICAL_CSS`.
> 3. `aozora-flavored-markdown-epub` enables that feature and its vendored
>    `assets/*.css` are gone.
> 4. The canonical themes moved from the repo-root `theme/` into
>    `crates/aozora-flavored-markdown/theme/` — the same single canonical
>    pair, relocated because `include_str!` reaching outside a crate's own
>    directory is not bundled by `cargo publish` (the reason step 3 needs
>    them inside the library crate at all).
>
> The drift gate is now two-directional: `tests/css_class_contract.rs` fails
> both on a class no theme styles and on a theme rule for a class the
> renderer cannot emit, so the interim `UNSTYLED_CLASSES` backlog was
> emptied and deleted.

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
4. Keep one canonical pair of theme files as that source — living inside the
   library crate, so `cargo publish` bundles what the `theme` feature embeds.

Until then (against published `aozora` 0.4.1), the EPUB crate keeps its vendored
CSS copy — necessary for publishability regardless — guarded by a theme-coverage
test that reads `AOZORA_MD_CLASSES` from `aozora-flavored-markdown-test-support`
([ADR-0018](0018-consolidate-the-epub-generator-into-this-workspace.md)). That
already catches a missing theme rule automatically; canonicalisation is a
maintenance-burden cleanup, not a correctness gap.

## Consequences

- One pre-existing CSS duplication remained until the bump, with no silent drift
  (the coverage test fires on a class the themes do not style).
- The bump became the single point where the rename, the public class contract,
  and the `theme` feature landed together — no double work.
- The class contract is now public API: adding, renaming or removing an
  `aozora-md-*` class is a semver-visible change to this crate, and the CSS
  that answers it ships from the same crate.
- **The gate binds a lockfile, not a version range.** `AOZORA_MD_CLASSES` is
  derived at compile time from whichever `aozora` cargo resolves, and the
  dependency is a caret range (`aozora = "0.5.0"`). A semver-compatible
  upstream release that adds a class therefore reaches a downstream build's
  HTML — the contract, the rewrite and the renderer all follow it — while the
  bundled `theme::{HORIZONTAL_CSS, VERTICAL_CSS}` have no rule for it, so that
  class ships unstyled. This repo's CI stays green throughout, because its own
  lockfile still resolves the version the themes were written against; the
  sweep turns red only when a maintainer updates the lock. That is the deal
  being accepted, not an oversight: pinning `aozora` exactly under the `theme`
  feature would force every consumer of this crate onto one patch release of
  the parser to get its stylesheets, which costs more than an unstyled class
  in the window before the next bump. The mitigation is release sequencing —
  both crates have the same author, and an `AOZORA_CLASSES` addition upstream
  is the signal to bump here. `theme`'s module documentation says so where a
  downstream reader will meet it.

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
