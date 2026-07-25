# 0020. Canonicalise the aozora-md CSS at the next aozora bump

- Status: accepted; executed 2026-07-25 at aozora 0.5.0
- Date: 2026-06-22
- Deciders: @P4suta
- Tags: architecture, epub, css, deferred

## Context

The EPUB generator vendored its own byte copy of the two `aozora-md-*` themes
alongside a near-identical canonical pair at the repo root. The right owner
is the crate that emits the classes, consumed downstream through a normal
crate dependency — `include_bytes!` reaching outside a crate's own directory
is not bundled by `cargo publish`.

Two things blocked doing it against the pin of the day: `AOZORA_MD_CLASSES`
lived in a `publish = false` dev crate rather than the library's public API,
and `aozora` had an unreleased `aozora-double-ruby` → `aozora-angle-quote`
rename in flight. Pinning canonical CSS against the old name would have been
redone at the bump.

## Decision

Defer to the next `aozora` bump, and land all four steps together there:
promote the class contract to public API derived from the parser's own list;
add a default-off `theme` feature publishing the stylesheets as `pub const`;
have the EPUB crate enable it and delete its vendored copy; keep one
canonical pair, inside the library crate so `cargo publish` bundles it.

## Execution (2026-07-25)

The bump was 0.5.0, where `AOZORA_CLASSES` grew from 19 to 91 entries and
carried the anticipated rename. All four steps landed. The drift gate is now
two-directional — `tests/css_class_contract.rs` fails both on a class no
theme styles and on a theme rule for a class the renderer cannot emit — so
the interim `UNSTYLED_CLASSES` backlog was emptied and deleted.

## Consequences

The class contract is public API: adding, renaming or removing an
`aozora-md-*` class is a semver-visible change, answered by CSS from the same
crate.

**The gate binds a lockfile, not a version range.** `AOZORA_MD_CLASSES` is
derived at compile time from whichever `aozora` cargo resolves, and the
dependency is a caret range. A semver-compatible upstream release that adds a
class therefore reaches a downstream build's HTML while the bundled themes
have no rule for it, and that class ships unstyled; this repo's CI stays
green until a maintainer updates the lock. That is the deal being accepted,
not an oversight — pinning `aozora` exactly under the `theme` feature would
force every consumer onto one patch release of the parser to get its
stylesheets. The mitigation is release sequencing (same author on both
crates), and `theme`'s module documentation says so where a downstream reader
will meet it.

## Alternatives considered

- **Do it against 0.4.1** — pins CSS to soon-to-be-renamed classes; redone at
  the bump.
- **Ship CSS from `aozora`** — it publishes the class *contract* only and
  leaves CSS to consumers, and this crate renames classes anyway (ADR-0011).

## References

- ADR-0011 (brand boundary), ADR-0018 (interim CSS handling), ADR-0021
  (the boundary these sit on)
