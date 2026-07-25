# 0013. Public IR enums are `#[non_exhaustive]`

- Status: accepted; scope narrowed by ADR-0022
- Date: 2026-06-20
- Deciders: @P4suta
- Tags: api, ir, stability, semver

## Context

`ir::{IrBlock, IrInline}` are public and grow over time. With the crate on
crates.io (ADR-0015), an exhaustive enum would make every added variant a
**breaking** change for any downstream `match` — a major bump for what is
conceptually additive.

## Decision

Mark `IrBlock` and `IrInline` `#[non_exhaustive]`. Serde output is unchanged,
so the JSON / TypeScript contract is unaffected — TS consumers already
tolerate an unknown `kind` under the ADR-0012 rule.

The IR **structs**, plus `IrTableAlign`, `Range` and `Position`, stay
exhaustive: `IrTableAlign` is a closed GFM set, `Range` / `Position` are a
stable coordinate contract, and the structs already evolve additively through
`skip_serializing_if` optional fields.

## Consequences

Downstream `match`es need a wildcard arm. Adding a variant is a minor
release.

## Superseded mechanisms

- **ADR-0017 (2026-06-21)** removed the hand-written TypeScript union and the
  `assert_*_variants` completeness witnesses this ADR relocated in-crate, when
  the IR↔TS codegen moved to `tsify` derives.
- **ADR-0022 (2026-07-25)** collapsed the Aozora half of the IR to one
  `Aozora { kind, span, html }` variant per level, so a new notation is a new
  `kind` string rather than a new variant. `#[non_exhaustive]` now keeps
  **Markdown** growth additive; the Aozora-side motivation is historical.

## References

- ADR-0012 (diagnostic JSON schema & stability — additive-only precedent)
- ADR-0015 (crates.io publication & semver policy)
- ADR-0017, ADR-0022
