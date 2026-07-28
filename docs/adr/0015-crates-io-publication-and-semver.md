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

**Publishable set & order (amended).** All four non-dev members are published:
`aozora-flavored-markdown` and `aozora-flavored-markdown-cli` on the workspace
0.5.x line, `aozora-flavored-markdown-epub` and
`aozora-flavored-markdown-epub-cli` on their own 0.1.x line. This ADR first
named only the first two, because the EPUB pair was a sibling repository when
it was written; consolidating it into this workspace (ADR-0018) left nothing
holding them back, and a crate nobody can `cargo add` is the same problem this
ADR exists to fix. `aozora-flavored-markdown-wasm` (npm/wasm-pack),
`aozora-flavored-markdown-test-support` and `xtask` (dev-only) stay
`publish = false`, and `cargo publish --workspace` skips them on that fact
rather than on a list kept in step by hand.

The order is no longer written down anywhere. Cargo derives it from the
dependency graph and uploads a crate only once everything it depends on is
confirmed present in the index, so the ladder cannot be laddered wrong — a
stronger property than the loop it replaced had, since that loop's order was a
pair of literals in a shell `for`.

**Automation (amended).** `.github/workflows/publish-crates.yml` (manual
`workflow_dispatch`, `dry_run` default true) runs one command:
`cargo publish --workspace --locked`. It replaced a shell function that probed
crates.io per version over `curl`, slept and retried on HTTP 429, and walked a
two-name `for` loop. All of that restated what cargo already does, and its
version came from `grep`ping the first `version` out of the root `Cargo.toml`
— a single workspace version, which the 0.1.x EPUB crates make wrong the
moment they join the ladder. The release.yml cargo-dist pipeline (binaries) is
untouched and runs off the same `v<semver>` tag; crates.io publish is a
separate, manually-triggered step.

**Packaging is a per-PR gate (amended).** The dry run below is `just package`,
a `[group('gate')]` recipe holding `cargo publish --workspace --dry-run
--locked`, and `publish-crates.yml`'s preflight calls that recipe instead of
carrying a second copy of the command. Until this amendment that workflow was
the only thing in the repository that ever built the published form of these
crates, and it runs on `workflow_dispatch` — so the tarball a consumer receives
was verified when somebody decided to publish and at no other moment, and the
dependency graph a consumer resolves had never been built on `main` at all
while `comrak` was a path dependency (ADR-0024). Every pull request packages
and verify-builds the whole ladder now (DEV-224). The recipe adds
`--allow-dirty`, which suppresses one check — "is every file in this package
committed" — and changes nothing about what is packaged or built: `just ci` is
what a developer runs before the commit exists, and a gate that declines to
answer there is a gate that only ever runs on a runner. The live
`cargo publish` carries no such flag, so the tarball that is uploaded is still
required to correspond to a commit.

**Measured before the ladder was deleted.** On the pinned toolchain,
`cargo 1.96.0 (30a34c682 2026-05-25)`, inside the dev image (ADR-0002),
`cargo publish --workspace --dry-run --locked` selected exactly the four
publishable members, skipped the three `publish = false` ones unprompted, and
staged them in dependency order:

```text
   Packaging aozora-flavored-markdown v0.5.0
   Packaging aozora-flavored-markdown-cli v0.5.0
   Packaging aozora-flavored-markdown-epub v0.1.0
   Packaging aozora-flavored-markdown-epub-cli v0.1.0
   Verifying aozora-flavored-markdown v0.5.0
   Verifying aozora-flavored-markdown-cli v0.5.0
   Verifying aozora-flavored-markdown-epub v0.1.0
   Verifying aozora-flavored-markdown-epub-cli v0.1.0
   Uploading aozora-flavored-markdown v0.5.0
   Uploading aozora-flavored-markdown-cli v0.5.0
   Uploading aozora-flavored-markdown-epub v0.1.0
   Uploading aozora-flavored-markdown-epub-cli v0.1.0
warning: aborting upload due to dry run
```

Each `Verifying` step compiled against a temporary registry under
`target/package/` holding the crates packaged ahead of it — the epub-cli build
logged `Unpacking aozora-flavored-markdown-epub v0.1.0 (registry
.../tmp-registry)` — which is why the whole ladder dry-runs even though two of
these crates have never been on crates.io.

**Semver policy (pre-1.0).** Under `0.y.z`, the **minor** position is the
breaking-change axis (cargo treats `0.y`→`0.(y+1)` as breaking). Breaking =
a change to rendered HTML for any CommonMark/GFM input, the
`aozora-md.diagnostics.v1` schema (ADR-0012), the public IR enums (ADR-0013), or
`Options::default`. Patch = additive features and fixes.

**Semver enforcement (amended).** `cargo semver-checks` runs as `just semver`:
a `[group('gate')]` recipe, so a per-PR CI leg, and a step of the
`publish-crates.yml` preflight. This ADR first said the check "cannot run until
a baseline exists on crates.io" and deferred it past the first publish. That
was wrong, and the correction is the reason for this amendment:
`--baseline-rev <tag>` takes the baseline out of this repository's git
history and needs no registry presence at all. The check was available the
whole time the public surface was being rebuilt.

The baseline is `v0.4.1`, the newest tag, and the flag is scaffolding — after
the first publish it comes out and the registry version, cargo-semver-checks'
own default, is the baseline. Both callers check out with `fetch-depth: 0`,
since a depth-1 clone carries no tag to resolve.

Two limits, recorded rather than implied. The two epub crates are `--exclude`d:
they joined this workspace after `v0.4.1` (ADR-0018) on their own 0.1.x line,
so the baseline holds nothing to compare them against; they re-enter the check
at the first tag that contains them. And `cargo semver-checks` takes no
`--locked`, so the baseline build resolves its own dependency graph — the one
resolution in this repo that is not bound to a lockfile, with no in-repo
substitute (DEV-298).

While the current version is already a major bump ahead of the baseline
(`0.4.1`→`0.5.0` under the rule above), every lint is skipped: a major bump
permits any break, so the gate asserts only that the declared version covers
what changed. That is the correct reading for the 0.5.0 cycle, where the
breaking changes *are* the plan — and it is a vacuous pass, not a clean bill
of health. The gate starts reporting breakage the moment the baseline is a
version this workspace is merely a patch ahead of.

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
- Resumability is cargo's now, and it is weaker. The deleted loop skipped a
  version already on crates.io, so a re-dispatch continued a partial publish.
  Cargo instead checks every selected member up front and errors on the first
  one already there — before it packages or uploads anything — so a second
  dispatch stops on what did land rather than continuing with what did not.
  Finish a partial run by hand: one `cargo publish -p <crate> --locked` per
  crate still missing, in dependency order. The 429 back-off goes with it, so
  a rate-limited upload now fails the run instead of sleeping ten minutes
  inside it.
- The EPUB pair is not on crates.io, so neither crate can be given a Trusted
  Publishing configuration yet — crates.io has nothing to attach one to until
  the crate exists. One `cargo publish --workspace` carries one token, which
  makes that a fact about the whole ladder: the run that first uploads them
  goes through the legacy `CARGO_TOKEN` path (`-f use_oidc=false`), and OIDC
  resumes once all four are registered.

## Alternatives considered

**Keep the `aozora` git pin and don't publish.** Rejected: it is the single
reason aozora-flavored-markdown can't go on crates.io, and aozora is already published, so
the registry version is a drop-in.

**Publish a vendored-comrak fork crate.** Rejected while the diff budget is 0
(ADR-0014) — depending on the registry crate is simpler and equivalent.

**`cargo publish --workspace`.** Rejected here, and adopted a cycle later —
see the amended **Automation** above. The stated reason was that the pinned
toolchain does not order interdependent first-publishes topologically, carried
over from aozora's ladder. It was not re-measured here, and it was wrong when
written: `-Zpackage-workspace` stabilised in Cargo **1.89**
(`cargo::core::features`), while this repository pinned 1.95.0 and then 1.96.0
on the very day this ADR was accepted. So the finding never described a
toolchain this workspace ran on, and no version bump was needed to reverse it
— only the dry-run above, which is what the reversal now rests on. The
inherited half of a decision is the half worth re-measuring.

## References

- ADR-0001 (vendored comrak), ADR-0010 (aozora extraction), ADR-0012
  (diagnostics schema), ADR-0013 (IR `#[non_exhaustive]`), ADR-0014 (comrak
  upgrade policy), ADR-0018 (EPUB consolidation — why the ladder is four
  rungs, on two version lines)
- `.github/workflows/publish-crates.yml`, `deny.toml`
- Plan: `~/.claude/plans/aozora-dapper-hopper.md`
