# 0014. comrak vendoring upgrade & follow policy

- Status: superseded by ADR-0024
- Date: 2026-06-20
- Deciders: @P4suta
- Tags: parser, fork, supply-chain, maintenance

## Context

ADR-0001 vendors comrak verbatim under `upstream/comrak/` with a 0-line-diff
budget; `just upstream-diff` enforces it and `just upstream-sync <tag>` is a
pure tree replace (aozora-flavored-markdown carries no comrak patches). What ADR-0001 did *not*
pin down is *when* we follow a new comrak release, how a sync interacts with
releases, and — now that aozora-flavored-markdown is published (ADR-0015) — how the
vendored tree relates to the comrak version aozora-flavored-markdown depends on.

The published dependency is `comrak = { version = "0.52.0", path =
"upstream/comrak" }`: local builds use the vendored path, but `cargo publish`
emits the registry `version`. The two are byte-identical only as long as the
vendored tree equals the registry release of that version.

## Decision

**Trigger.** Sync comrak only when (a) `just audit-comrak` reports a
`RUSTSEC-*` advisory against the vendored version, or (b) a comrak release
carries a CommonMark/GFM conformance fix aozora-flavored-markdown's spec suite would benefit from.
Routine non-security comrak releases are **not** auto-followed (matching the
no-cron, per-PR maintainer cadence in SECURITY.md and Dependabot's ignore of
the vendored tree).

**Process.** `just upstream-sync <tag>` → run `just spec-commonmark`,
`just spec-gfm`, and full `just ci` → update the pinned version in
`[workspace.dependencies] comrak` and `upstream/comrak/Cargo.toml` to the new
tag → add a CHANGELOG `Changed` entry. The 0-line-diff gate guarantees the
vendored tree still equals the registry crate of the new version, so the
published-vs-local dependency stays identical.

**Semver impact.** A comrak sync that changes rendered HTML for *any*
CommonMark/GFM input is a breaking change for aozora-flavored-markdown consumers and bumps the
breaking axis per ADR-0015.

**Published-dep coupling.** The registry `comrak` version aozora-flavored-markdown
publishes against MUST equal the vendored `upstream/comrak` tag. CI's
`upstream-diff` + `audit-comrak` jobs keep them locked together.

## Consequences

- The "when do we upgrade comrak" question has a written, auditable answer;
  routine churn is ignored, security/conformance changes are actioned.
- Publishing against the registry comrak (rather than shipping a fork) keeps
  aozora-flavored-markdown a normal crates.io citizen while preserving the vendored tree
  for offline dev, CI, and the audit shim.
- A future intentional comrak *patch* (breaking the 0-line budget) would need
  its own ADR and a real fork-publish strategy; this policy assumes the
  budget stays at 0.

## Alternatives considered

**Auto-follow every comrak release via Dependabot.** Rejected: the vendored
tree is a deliberate pin (ADR-0001) and each bump must re-verify the 0-line
diff + spec suites; automating it would churn the conformance fixtures and the
published-dep coupling for little benefit.

**Publish a comrak fork crate.** Rejected while the diff budget is 0 — there
is nothing to fork. Depending on the upstream registry crate is simpler and
honest.

## References

- ADR-0001 (fork comrak and vendor it in-tree)
- ADR-0015 (crates.io publication & semver policy)
- [ADR-0024](0024-depend-on-crates-io-comrak.md) — supersedes this ADR; the
  trigger and semver-impact rules below are carried over verbatim.
