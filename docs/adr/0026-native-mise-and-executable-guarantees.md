# 0026. Use native mise and executable guarantees

- Status: accepted
- Date: 2026-07-29
- Deciders: @P4suta
- Tags: infra, dev-env, testing
- Supersedes: ADR-0002

## Context

The Docker development image and repository-policy tests attempted to prove
consistency by parsing Dockerfiles, workflows, TOML, Rust source and the
Justfile. They made configuration a second product: roughly 15,500 lines
tested how commands were wired without exercising the behaviours those
commands were meant to protect.

Rust, Bun and their package managers already have lockfiles, while GitHub
Actions can run the same public Just recipes as a developer.

## Decision

The supported environment is native mise. `mise.toml` declares exact tool
versions, `mise.lock` records mise's resolution, and `mise install --locked`
installs it. `rust-toolchain.toml`, `Cargo.lock` and `playground/bun.lock`
remain authoritative for their language ecosystems. Fuzzing uses only the
date-pinned nightly named by the fuzz recipes.

CI has five fixed jobs—Rust, web, repository, release and fuzz—and each invokes
the identically named `just ci-*` recipe. We do not add source or configuration
parsers to prove that these declarations correspond.

Important guarantees are expressed by the compiler, an official tool, or an
actual build/test:

- rustc and Clippy reject unsafe code and production panic/output shortcuts;
- product tests and coverage exercise Rust, WASM and the Playground;
- cargo-deny, typos, actionlint and zizmor inspect their own domains;
- package and release smoke tests construct the artifacts consumers receive;
- cargo-fuzz builds every target and the scheduled workflow executes them.

## Consequences

Docker, Compose, the devcontainer, the development-image workflow and their
setup actions are removed. Onboarding requires mise and a native compiler
environment. Platform differences are visible rather than hidden, and release
binaries continue to build on their target platform runners.

The repository has fewer policy tests and more direct failures: a missing
fixture fails the packaged crate test, broken browser code fails the web build,
and a broken fuzz target fails cargo-fuzz.

## References

- [mise.toml](../../mise.toml)
- [Justfile](../../Justfile)
- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
- Umbrella issue #292
