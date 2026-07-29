# Contributing to aozora-flavored-markdown

This repository is the Markdown ↔ Aozora composition layer. New 青空文庫
notation starts in the sibling
[`P4suta/aozora`](https://github.com/P4suta/aozora) parser; a dependency
update then brings it here.

## Development environment

Install [mise](https://mise.jdx.dev/), then use the checked-in resolution:

```sh
mise trust
mise install --locked
just test
```

`rust-toolchain.toml`, `Cargo.lock` and `playground/bun.lock` remain the
language-specific authorities. Fuzzing alone uses the date-pinned nightly
declared in `mise.toml`; all production crates use Rust 1.96.0.

The same fixed entry points are used locally and by GitHub Actions:

```sh
just ci-rust      # fmt, clippy, coverage, doctests, public rustdoc
just ci-web       # wasm tests/package and Playground lint/test/build
just ci-repo      # cargo-deny, typos, actionlint, zizmor
just ci-release   # generated assets, packages, dist, release mutation
just ci-fuzz      # build every fuzz target
just ci           # all five suites
```

Biome's formatter and linter are enabled explicitly by the command line in
`just playground-lint`. Coverage over published Rust sources must stay at or
above the floor in `Justfile`.

The important guarantees in this repository are executable: compiler lints,
official tool checks, package construction, and product build/test runs.
Do not add a parser that tries to prove that YAML, TOML, Rust source and the
Justfile agree with each other.

## Where a change lands

- Splice edge cases belong in
  `crates/aozora-flavored-markdown/src/ast_splice.rs`, with a test beside
  the code.
- Construct substitution belongs in `src/constructs.rs`; byte-range changes
  need coverage in `tests/construct_spans.rs`.
- IR projection belongs in `src/ir/`. An IR schema change is semver-major
  because the WASM bridge and downstream JavaScript consumers validate it.
- Renderer-emitted CSS classes must be styled in both files under
  `crates/aozora-flavored-markdown/theme/`.
- CommonMark/GFM source fixtures remain under `spec/sources/`; generated JSON
  fixtures consumed by unit tests live under the core crate's `spec/`.

`comrak` is an ordinary crates.io dependency and is never patched locally
(ADR-0024).

## Pull requests

Use Conventional Commits and keep each commit logically focused. PR titles
follow the same convention; use `Closes #N` when appropriate. Add or update a
product test for behaviour changes and update `[Unreleased]` in
`CHANGELOG.md` when users need to know about the change.

Run `just ci` before requesting review. A reasoned compiler-lint exception is
acceptable only at the narrow production site that needs it; the default is
to remove the panic, output, or warning-producing path.

Architectural decisions use MADR files under `docs/adr/`:

```sh
just adr 'my new decision'
```

## Releasing

The normal release preparation is:

```sh
just release-smoke              # mutation and packaging in an isolated repo
just release minor              # inspect cargo-release's proposed changes
just release minor --execute    # rewrite this worktree
git commit -am 'chore(release): v0.6.0'
git tag -a v0.6.0 -m 'v0.6.0'
git push --follow-tags
```

`publish-crates.yml` publishes the four public crates in workspace dependency
order. A live publish is protected by the `release` environment and uses a
short-lived crates.io OIDC credential. Its preflight checks the changelog,
package tarballs and public semver baseline. Cargo-dist builds the tagged CLI
binaries on native platform runners.

The release commit and tag remain explicit, signed developer actions.

## Security and licence

Report vulnerabilities through `SECURITY.md`, never a public issue. By
contributing, you agree that your work is dual-licensed under Apache-2.0 OR
MIT.
