# 0018. Consolidate the EPUB generator into this workspace

- Status: accepted
- Date: 2026-06-22
- Deciders: @P4suta
- Tags: ecosystem, repo-layout, release-strategy

## Context

The EPUB3 generator (`aozora-flavored-markdown-epub`) turns an Aozora Flavored
Markdown manuscript into an EPUB 3.3 package. It began life as a sibling
repository (`P4suta/aozora-flavored-markdown-epub`), following the "downstream
tools live in sibling repos" principle of [ADR-0009](0009-authoring-tools-live-in-sibling-repositories.md).

That principle no longer reflects how the ecosystem actually evolves:

- The sibling `aozora` parser **reversed the same call**: it absorbed the
  editor/CLI tooling that used to live in `P4suta/aozora-tools` back into its
  own monorepo (aozora ADR-0016), and archived `aozora-tools` read-only.
- The EPUB generator is tightly coupled to this crate's *rendered output*: its
  vendored CSS themes must style exactly the `aozora-md-*` classes the renderer
  emits ([ADR-0011](0011-brand-boundary-css-class-rewrite.md)). A cross-repo
  boundary turned every renderer change into a coordinated tag bump + downstream
  PR — the precise cost ADR-0009 hoped would be "rare", paid on the most
  active surface.
- For ~1000 lines of code, a standalone repo carried a full second set of
  ceremony: its own cargo-dist release, docker-only CI, cargo-deny/audit, and
  ADR log — pure duplication of this workspace's.

The generator was never published to crates.io under its new name, so this is
the moment to land it directly rather than ship-then-archive.

## Decision

Fold the EPUB generator into this workspace as two crates:

- `crates/aozora-flavored-markdown-epub` — the library (discover → render →
  compose → package pipeline).
- `crates/aozora-flavored-markdown-epub-cli` — its `aozora-flavored-markdown-epub`
  binary.

The former `P4suta/aozora-flavored-markdown-epub` repository is archived
read-only with a pointer here. History is **not** rewritten into this tree (a
clean copy under one consolidation commit), mirroring how `aozora` absorbed
`aozora-tools`.

The EPUB crates are **independently versioned** (0.1.x), not pinned to the
workspace's unified 0.4.x line: the generator is young and its public surface
moves on a different cadence than the parser. Each EPUB crate therefore sets an
explicit `version` instead of inheriting `version.workspace`.

The pure parser/renderer crates stay free of the generator's I/O dependencies
(`zip`, `quick-xml`, `uuid`, `chrono`): those live only under the
`aozora-flavored-markdown-epub` crate, asserted by `cargo tree`. This is the
crate-sibling shape, not a library feature — adding EPUB output must never make
`aozora-flavored-markdown` (parser-only consumers) pull a ZIP writer.

This supersedes ADR-0009 for the EPUB generator. (ADR-0009 already stood
contradicted by the `aozora-tools` → `aozora` consolidation; this ADR records
the same reversal for this repo.)

## Consequences

- One repo, one CI, one release pipeline, one ADR log. Renderer + theme +
  generator move in lockstep in a single commit; no cross-repo tag dance.
- The EPUB CLI is **not** yet part of this workspace's cargo-dist release
  (`[package.metadata.dist] dist = false`): binary distribution + release
  cadence for an independently-versioned crate is a separate decision. The crate
  still publishes to crates.io as an ordinary library/binary.
- Mixed versions now live in one workspace (0.4.x parser line + 0.1.x EPUB
  line). Release tooling and tag schemes must account for per-package versions.
- The theme-coverage test consumes `AOZORA_MD_CLASSES` instead of a hand-copied
  list, so an upstream class change fails the EPUB build automatically. Full CSS
  canonicalisation followed in [ADR-0020](0020-canonicalise-aozora-md-css-at-the-next-aozora-bump.md):
  the generator no longer vendors the themes at all — it enables the renderer's
  `theme` feature and bundles the stylesheets that crate publishes.

## Alternatives considered

- **Keep the sibling repo, dedup only the CSS.** Rejected: leaves the
  release/CI/ADR ceremony duplicated and keeps the cross-repo version dance that
  motivated the move; inconsistent with the `aozora-tools` precedent.
- **Fold EPUB output into the `aozora-flavored-markdown` library behind a
  feature.** Rejected: pollutes a pure, I/O-free parser/renderer with `zip` /
  `quick-xml` / filesystem dependencies. Parser-only consumers (a static-site
  generator, a server) must not inherit a ZIP writer. The sibling-crate shape
  keeps the library lean while co-locating the code.
- **Adopt the `aozora` Pandoc bridge instead of the hand-rolled generator.**
  Decided separately in [ADR-0019](0019-epub-generation-is-hand-rolled-not-via-pandoc.md).

## References

- [ADR-0009](0009-authoring-tools-live-in-sibling-repositories.md) — superseded for EPUB.
- [ADR-0011](0011-brand-boundary-css-class-rewrite.md) — the `aozora-md-*` class contract the themes track.
- [ADR-0019](0019-epub-generation-is-hand-rolled-not-via-pandoc.md) — generation approach.
- [ADR-0020](0020-canonicalise-aozora-md-css-at-the-next-aozora-bump.md) — deferred CSS-ownership follow-up.
- aozora ADR-0016 — the parallel "consolidate tooling into the monorepo" decision in the sibling repo.
