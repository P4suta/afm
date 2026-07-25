# 0015. crates.io publication and semver policy

- Status: accepted
- Date: 2026-06-20
- Deciders: @P4suta
- Tags: release, crates.io, semver, supply-chain

## Context

aozora-flavored-markdown should be installable as a normal Rust crate (`cargo install aozora-flavored-markdown-cli`,
`cargo add aozora-flavored-markdown`) rather than only via `--git`. Two manifest facts
blocked that: `aozora-flavored-markdown` depended on `aozora` by **git rev** and on
`comrak` by **path** (the vendored tree), and crates.io rejects both git and
path sources. The sibling `aozora` crate is now published on crates.io
(v0.4.1), so the git pin can become a registry version. The vendored comrak
is byte-identical to registry `comrak` v0.52.0 (ADR-0001's 0-line-diff gate).

## Decision

**Dependency sources.**

- `aozora`: switch from `git + rev = a53c632…` to the registry version
  `"0.4.1"` (the published cut of that rev). Intentional syncs are now
  `cargo update -p aozora` + a version bump, replacing the rev-swap discipline
  of ADR-0010 (which is otherwise preserved).
- `comrak`: keep `{ version = "0.52.0", path = "upstream/comrak" }`. cargo uses
  the path locally and the registry `version` when publishing; the 0-line-diff
  gate (ADR-0014) keeps them identical, so published aozora-flavored-markdown builds
  against registry comrak 0.52.0.
- `aozora-flavored-markdown-test-support`: path-only (no version) so `cargo publish` strips
  it — it is `publish = false` and never on crates.io. The resulting `*` path
  requirement is allowed by `deny.toml`'s `allow-wildcard-paths`.

**Publishable set & order.** Only `aozora-flavored-markdown` then `aozora-flavored-markdown-cli` are published,
in that topological order. `aozora-flavored-markdown-wasm` (npm/wasm-pack), `aozora-flavored-markdown-test-support`
and `xtask` (dev-only) are `publish = false`.

**Automation.** `.github/workflows/publish-crates.yml` (manual
`workflow_dispatch`, `dry_run` default true, resumable + rate-limit aware)
runs the 2-crate ladder. The release.yml cargo-dist pipeline (binaries) is
untouched and runs off the same `v<semver>` tag; crates.io publish is a
separate, manually-triggered step. `cargo-dist` ladders are aozora's
13-crate machinery scaled down — aozora-flavored-markdown's single-library workspace needs only two
rungs.

**Semver policy (pre-1.0).** Under `0.y.z`, the **minor** position is the
breaking-change axis (cargo treats `0.y`→`0.(y+1)` as breaking). Breaking =
a change to rendered HTML for any CommonMark/GFM input, the
`aozora-md.diagnostics.v1` schema (ADR-0012), the public IR enums (ADR-0013), or
`Options::default`. Patch = additive features and fixes.
`cargo semver-checks` cannot run until a baseline exists on crates.io, so it
is wired into the `publish-crates.yml` preflight *after* the first publish,
not into per-PR CI.

## Consequences

- `cargo install aozora-flavored-markdown-cli` / `cargo add aozora-flavored-markdown` become real; docs.rs will
  host the API docs (the `docs.yml` "not on crates.io" note is updated).
- The `aozora` upgrade discipline shifts from rev pinning to registry version
  bumps — slightly looser, but `Cargo.lock` still pins the exact build and a
  bump is still one reviewed PR.
- A first publish is greenfield (both crates 404 today); the leaf
  `aozora-flavored-markdown` dry-run is the pre-flight gate (verified green), while
  `aozora-flavored-markdown-cli` can only be verified live after aozora-flavored-markdown lands.

## Alternatives considered

**Keep the `aozora` git pin and don't publish.** Rejected: it is the single
reason aozora-flavored-markdown can't go on crates.io, and aozora is already published, so
the registry version is a drop-in.

**Publish a vendored-comrak fork crate.** Rejected while the diff budget is 0
(ADR-0014) — depending on the registry crate is simpler and equivalent.

**`cargo publish --workspace`.** Rejected: on the pinned toolchain it does not
order interdependent first-publishes topologically (same finding as aozora's
ladder). The explicit two-rung loop is deterministic.

## References

- ADR-0001 (vendored comrak), ADR-0010 (aozora extraction), ADR-0012
  (diagnostics schema), ADR-0013 (IR `#[non_exhaustive]`), ADR-0014 (comrak
  upgrade policy)
- `.github/workflows/publish-crates.yml`, `deny.toml`
- Plan: `~/.claude/plans/aozora-dapper-hopper.md`
