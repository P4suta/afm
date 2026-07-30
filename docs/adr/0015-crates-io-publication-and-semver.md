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
path sources. The sibling `aozora` crate is now published on crates.io, so the
git pin can become a registry version.

## Decision

**Dependency sources.**

- `aozora`: switch from a git rev to the registry version. Intentional syncs
  are now `cargo update -p aozora` + a version bump, replacing the rev-swap
  discipline of ADR-0010 (which is otherwise preserved).
- `comrak`: superseded. This ADR kept the vendored tree behind a registry
  `version`; ADR-0024 retired the tree, and the manifest is now the only
  statement of where comrak comes from.
- `aozora-flavored-markdown-test-support`: path-only (no version) so `cargo publish` strips
  it — it is `publish = false` and never on crates.io. The resulting `*` path
  requirement is allowed by `deny.toml`'s `allow-wildcard-paths`.

**Publishable set & order (amended).** All four non-dev members are published:
`aozora-flavored-markdown`, `aozora-flavored-markdown-cli`,
`aozora-flavored-markdown-epub` and `aozora-flavored-markdown-epub-cli`. This
ADR first named only the first two, because the EPUB pair was a sibling
repository when it was written; consolidating it into this workspace (ADR-0018)
left nothing holding them back, and a crate nobody can `cargo add` is the same
problem this ADR exists to fix. `aozora-flavored-markdown-wasm` (npm/wasm-pack),
`aozora-flavored-markdown-test-support` and `xtask` (dev-only) stay
`publish = false`. Neither the set nor the order is written down anywhere else:
the manifests decide the one and the dependency graph decides the other, which
is a stronger property than the loop this replaced had — that loop's order was
a pair of literals in a shell `for`.

**Automation (amended).** Publication is one manually-dispatched workflow,
`.github/workflows/publish-crates.yml`, defaulting to a dry run. It replaced a
shell function that probed crates.io per version over `curl`, slept and retried
on HTTP 429, and walked a two-name `for` loop — all of which restated what
cargo already does, and all of which took its version from the first `version`
in the root `Cargo.toml`, a single workspace version that the 0.1.x EPUB line
makes wrong. The hand-maintained `release.yml` cargo-dist pipeline (binaries)
remains separate and runs off the same `v<semver>` tag; the crates.io upload
stays manually triggered, because it is the one operation nobody can take back.

**Packaging is a per-PR gate (amended).** `just package-smoke` runs
`cargo publish --workspace --dry-run --locked`, then extracts the core
`.crate` and runs its all-feature library unit tests from Cargo's normalized
manifest. Generated CommonMark/GFM JSON fixtures live under the core crate so
those tests exercise exactly the files a consumer receives. The
`publish-crates.yml` preflight calls the same recipe. On a partial-publish
retry, its `skip_crates` input is passed to both that dry run and the live
command through `scripts/publish-excludes.sh`; both log the selected package
set. Cargo's exact warnings for intentionally excluded test and benchmark
targets are filtered, while near matches, unrelated stderr and Cargo's exit
status remain observable.

**Semver policy (pre-1.0).** Under `0.y.z`, the **minor** position is the
breaking-change axis (cargo treats `0.y`→`0.(y+1)` as breaking). Breaking =
a change to rendered HTML for any CommonMark/GFM input, the
`aozora-md.diagnostics.v1` schema (ADR-0012), the public IR enums (ADR-0013), or
`Options::default`. Patch = additive features and fixes.

**Semver enforcement (amended).** Semver checks run in the tag/publish
preflight, where a real public baseline and the version about to be uploaded
exist. Before the first 0.5 publication, making this a pull-request gate would
compare against a baseline that does not describe the intended public 0.5
surface and provide an empty assurance. Issue #215 remains open until that
baseline exists.

One limit is recorded here rather than implied, because nothing in the tree can
hold it: `cargo semver-checks` takes no `--locked`, so the baseline build
resolves its own dependency graph. It is the one resolution in this repository
that is not bound to a lockfile, and there is no in-repo substitute (#215).

## Consequences

- `cargo install aozora-flavored-markdown-cli` / `cargo add aozora-flavored-markdown` become real; docs.rs will
  host the API docs (the `docs.yml` "not on crates.io" note is updated).
- The `aozora` upgrade discipline shifts from rev pinning to registry version
  bumps — slightly looser, but `Cargo.lock` still pins the exact build and a
  bump is still one reviewed PR.
- The whole ladder is pre-flightable, not just the leaf. This ADR said only
  `aozora-flavored-markdown` could be dry-run before anything was on the
  registry and that its CLI could only be verified live afterwards. That was a
  property of publishing one crate per command, not of cargo.
- Resumability is explicit. Cargo errors on the first member already present
  after a partial upload, so a re-dispatch lists those package names in
  `skip_crates`. Cargo metadata validates the names, and the preflight dry-run
  uses the same exclusion flags as the live publish. This preserves recovery
  without reviving the deleted loop's incorrect single-version assumption.
- The EPUB pair is not on crates.io, so neither crate can be given a Trusted
  Publishing configuration yet — crates.io has nothing to attach one to until
  the crate exists. One `cargo publish --workspace` carries one token, which
  makes that a fact about the whole ladder rather than about those two rungs:
  the run that first uploads them cannot use OIDC. `publish-crates.yml` carries
  that boundary directly: `use_oidc` defaults to false for the token-backed
  bootstrap and may be enabled only after every selected crate has a Trusted
  Publisher.

## Alternatives considered

**Keep the `aozora` git pin and don't publish.** Rejected: it is the single
reason aozora-flavored-markdown can't go on crates.io, and aozora is already published, so
the registry version is a drop-in.

**Publish a vendored-comrak fork crate.** Rejected while the diff budget was 0
(ADR-0014) — depending on the registry crate is simpler and equivalent, which
is where ADR-0024 ended up.

**`cargo publish --workspace`.** Rejected here, and adopted a cycle later —
see the amended **Automation** above. The stated reason was that the pinned
toolchain does not order interdependent first-publishes topologically, carried
over from aozora's ladder. It was not re-measured here, and it was wrong when
written: `-Zpackage-workspace` stabilised in Cargo **1.89**
(`cargo::core::features`), while this repository pinned 1.95.0 and then 1.96.0
on the very day this ADR was accepted. So the finding never described a
toolchain this workspace ran on, and no version bump was needed to reverse it —
only a dry run, which `just package-smoke` now performs on every pull request. The
inherited half of a decision is the half worth re-measuring.

## References

- ADR-0001 (vendored comrak), ADR-0010 (aozora extraction), ADR-0012
  (diagnostics schema), ADR-0013 (IR `#[non_exhaustive]`), ADR-0014 (comrak
  upgrade policy), ADR-0018 (EPUB consolidation — why the ladder is four
  rungs, on two version lines), ADR-0024 (comrak from the registry)
- `.github/workflows/publish-crates.yml`, `deny.toml`
- Plan: `~/.claude/plans/aozora-dapper-hopper.md`
