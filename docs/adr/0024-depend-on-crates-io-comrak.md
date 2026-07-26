# 0024. Depend on crates.io comrak, retire the vendored tree

- Status: accepted
- Date: 2026-07-26
- Deciders: @P4suta
- Tags: architecture, parser, supply-chain, build

## Context

ADR-0001 forked comrak at `v0.52.0` and vendored 139 files / 2.2 MB verbatim
under `upstream/comrak/`, under a 0-line diff budget. ADR-0014 then made the
published manifest depend on registry `comrak = "0.52.0"` while local builds
kept resolving the `path`. That arrangement has three problems.

**We shipped a build graph nobody built.** `cargo publish` strips `path`, so
every consumer on crates.io compiled against the registry crate — a graph that
had never been built locally or in CI. Local, CI and post-publish were three
different resolutions of the same dependency.

**ADR-0001's stated benefits do not hold.**

| ADR-0001 claim | Reality |
| -- | -- |
| CommonMark/GFM tests pass for free (vendored ships them) | False. `upstream/comrak` was `exclude`d from the workspace, so `cargo test` never compiled it. Conformance reads this repo's own `spec/*.json`. |
| Rich AST from day one | A property of depending on comrak, not of vendoring it. |
| No hooks, so a sync is a tree replace | A property of not patching, not of vendoring. |

**The gate that enforced the budget did not measure anything.** `xtask
upstream-diff` computed no diff: it read `upstream/comrak/COMRAK_SHA` and
grepped `upstream/comrak/UPSTREAM_DIFF.md` for `"0-line"` or `"0 lines"`. Any
byte of `upstream/comrak/**` could change and every gate stayed green.
(`"0-line"` also matched the file's own historical `"200-line diff budget"`
prose, so the needle was satisfied twice over by text about a budget that no
longer applied.)

## Decision

Delete `upstream/` and depend on the registry crate:

```toml
comrak = { version = "0.52.0", default-features = false }
```

`upstream-diff`, `upstream-sync` and `audit-comrak`, plus their xtask
implementations, CI jobs, lefthook hook, CodeQL config, typos/bacon/gitignore
/gitattributes/CODEOWNERS carve-outs and the workspace `exclude`, all go with
it. `upstream/comrak/UPSTREAM_DIFF.md` is deleted rather than corrected.

**No replacement gate is added.** `Cargo.lock` now carries comrak's `source`
and `checksum`, which cargo verifies on every build — strictly stronger than a
grep for a policy sentence. `cargo audit`, `cargo deny`, `dependency-review`
and Dependabot all see comrak directly for the first time; the synthetic
one-crate lockfile `just audit-comrak` fabricated was a workaround for a
problem vendoring created.

**Trigger (carried over verbatim from ADR-0014).** Upgrade comrak only when
(a) an advisory scan reports a `RUSTSEC-*` against the pinned version, or (b)
a comrak release carries a CommonMark/GFM conformance fix this workspace's
spec suite would benefit from. Routine non-security comrak releases are not
auto-followed.

**Semver impact (carried over verbatim from ADR-0014).** A comrak upgrade that
changes rendered HTML for *any* CommonMark/GFM input is a breaking change for
aozora-flavored-markdown consumers and bumps the breaking axis per ADR-0015.
This is now genuinely enforceable: the CommonMark and GFM conformance suites
run against the same crate consumers get, so a rendering change surfaces as a
red spec run instead of as a policy sentence.

## The one-time byte-identity check

Before deleting, the vendored tree was compared against the published
`comrak-0.52.0.crate`
(sha256 `aac0b255932a9cd52fbfd664b67957f9f2e095ae4711cb0e41b4e291edef94c2`).
The tarball's `.cargo_vcs_info.json` records git sha1
`60a4fae8babc3847089592868583be83d635ff1a`, exactly the sha pinned in
`upstream/comrak/COMRAK_SHA` — so both sides claim the same upstream commit.

**They were not identical. Two files carried local edits**, i.e. the 0-line
diff budget had been breached and nothing noticed:

- `src/parser/inlines.rs` — in `find_special_char`, the local `index` binding
  was renamed to `byte_index` (2 lines).
- `src/tests/sourcepos.rs` — an explanatory comment was added above the
  `NodeValueDiscriminants::VARIANTS` filter (1 line).

Both are behaviour-preserving: a local variable rename in a function whose
result is `self.scanner.pos + <position>` either way, and a comment inside a
`#[cfg(test)]` module this workspace never compiles. So the switch to the
registry crate changes no rendered output — but the fact that a hand edit sat
in a "verbatim" tree for months, past four gates, is the finding, and it is
the strongest argument for this ADR: an unenforced policy is not a policy.

Everything else that differed is packaging, not source:

- Only in the tarball: `Cargo.lock`, `Cargo.toml.orig`,
  `.cargo_vcs_info.json`, `.github/` (cargo metadata, and a directory the
  vendoring drop never carried).
- Only in the vendored tree: `COMRAK_SHA` and `UPSTREAM_DIFF.md` (ours), plus
  `hooks/`, `script/`, `Makefile`, `spec_out.txt` (comrak's own `exclude`
  list) and `fuzz/` (a nested package cargo does not package).
- `Cargo.toml` differs only by cargo's publish-time normalisation
  (dependency tables expanded, `[[example]]`/`[[bench]]` paths made explicit).

`src/`, `build.rs`, `benches/`, `examples/` and `www/` were otherwise
byte-for-byte equal.

## Consequences

Easier:

- One dependency graph everywhere; what CI builds is what a consumer gets.
- comrak enters every supply-chain tool this repo already runs, verified by
  checksum on every build.
- 2.2 MB / 139 files leave the tree, along with ~200 lines of xtask, a CI job,
  a CI matrix leg, a pre-commit hook and five config carve-outs.

Harder:

- No offline vendored copy. Cold builds need the registry; the docker volumes
  already cache it, and `Cargo.lock` pins the exact version.
- Reading comrak's source now means opening the registry checkout or GitHub
  instead of a path in this repo.
- A genuine need to patch comrak becomes a fork-and-publish decision needing
  its own ADR, rather than an edit that a 0-line budget was supposed to (but
  did not) prevent.

## Alternatives considered

**Keep vendoring and make `upstream-diff` real** (fetch the tarball, diff it,
fail on any delta). Rejected: it would have caught the two edits above, but it
buys a real gate only to preserve a tree whose stated benefits do not hold,
and it still leaves local and published builds resolving different sources.
The checksum in `Cargo.lock` is the same guarantee for free.

**Publish a comrak fork crate.** Rejected: there is nothing to fork. The only
deltas were a variable rename and a comment.

**Keep the tree for offline builds.** Rejected: the workspace `exclude`d it,
so it never participated in a build anyway — it was offline-available source
that nothing compiled.

## References

- ADR-0001 (fork comrak and vendor it in-tree) — superseded by this ADR.
- ADR-0014 (comrak vendoring upgrade & follow policy) — superseded by this
  ADR; its trigger and semver rules are carried over above.
- ADR-0015 (crates.io publication and semver policy) — its `comrak` source
  bullet is replaced by this ADR.
- [comrak on crates.io](https://crates.io/crates/comrak/0.52.0)
