# Architecture Decision Records

This directory holds [MADR 4.0](https://adr.github.io/madr/) Architecture
Decision Records — one decision per file.

Numbers 0003–0008 are retired and never reused: 0003 and 0005 were superseded
and removed; 0004 / 0006 / 0007 / 0008 moved to the sibling
[`P4suta/aozora`](https://github.com/P4suta/aozora) repo when the parser core
was extracted (ADR-0010).

| ADR | Title | Status |
| --- | ----- | ------ |
| [0001](./adr/0001-fork-comrak-vendor-in-tree.md) | Fork comrak and vendor it in-tree (0-line diff budget) | accepted |
| [0002](./adr/0002-docker-only-execution.md) | Every dev operation runs inside Docker | accepted |
| [0009](./adr/0009-authoring-tools-live-in-sibling-repositories.md) | Authoring tools live in sibling repositories | accepted (reversed in practice by 0018) |
| [0010](./adr/0010-extract-aozora-core.md) | Extract aozora parser core into sibling repository `aozora` | accepted |
| [0011](./adr/0011-brand-boundary-css-class-rewrite.md) | Brand boundary: HTML class rewrite at the aozora-flavored-markdown side | accepted (rewrite site restated by 0021) |
| [0012](./adr/0012-diagnostic-json-output-schema-and-stability.md) | Diagnostic JSON output schema and stability (`aozora-md.diagnostics.v1`) | accepted |
| [0013](./adr/0013-public-ir-enums-non-exhaustive.md) | Public IR enums are `#[non_exhaustive]` | accepted (narrowed by 0022) |
| [0014](./adr/0014-comrak-vendoring-upgrade-policy.md) | comrak vendoring upgrade & follow policy | accepted |
| [0015](./adr/0015-crates-io-publication-and-semver.md) | crates.io publication and semver policy | accepted |
| [0016](./adr/0016-rebrand-to-aozora-flavored-markdown.md) | Rebrand `afm` → `aozora-flavored-markdown` | accepted |
| [0017](./adr/0017-derive-typescript-types-with-tsify.md) | Derive the TypeScript `.d.ts` with `tsify` | accepted (union re-stated by 0022) |
| [0018](./adr/0018-consolidate-the-epub-generator-into-this-workspace.md) | Consolidate the EPUB generator into this workspace | accepted |
| [0019](./adr/0019-epub-generation-is-hand-rolled-not-via-pandoc.md) | EPUB generation is hand-rolled, not via pandoc | accepted |
| [0020](./adr/0020-canonicalise-aozora-md-css-at-the-next-aozora-bump.md) | Canonicalise the `aozora-md-*` CSS at the next aozora bump | accepted (executed at 0.5.0) |
| [0021](./adr/0021-aozora-boundary-is-the-public-surface.md) | The aozora boundary is aozora's public surface only | accepted |
| [0022](./adr/0022-collapse-the-aozora-half-of-the-ir.md) | Collapse the Aozora half of the IR to `{kind, span, html}` | accepted |
| [0023](./adr/0023-substitute-constructs-in-source-coordinates.md) | Substitute constructs in source coordinates | accepted |

## Authoring a new ADR

1. Scaffold with `cargo xtask new-adr 'my new decision'` (copies
   `adr/0000-template.md` to the next sequential number).
2. Fill in the sections; keep them short and action-oriented.
3. Add a row to the table above.
