# 0002. Every dev operation runs inside Docker

- Status: accepted
- Date: 2026-04-23
- Tags: infra, dev-env

## Context

Rust/Node toolchain version drift between contributors and CI is a recurring source
of "works on my machine" failures. This project is Japanese-typography-sensitive —
encoding, Unicode version, OS locales all affect results — and the team is small. We
want the cost of environment setup to fall on the Docker image, not on each human.

## Decision

Every development operation runs inside a container defined by `/Dockerfile`.
The entry point is `/Justfile`: a recipe reaches its tool through the image,
and CI runs the same recipes, so the CI environment is structurally identical
to every developer's environment. Reaching for a host-level `cargo` or `bun`
instead is what this forbids — a second way to run the same step is the drift
this decision exists to remove.

**Scope exception: a check whose subject IS the host toolchain.** Where the
question being asked is about something the image cannot hold — that the
declared MSRV compiles on a clean install of it, that a commit range CI can see
lints clean, that a release binary matches its runner's OS, that a publish
reaches crates.io with only cargo and rustc — the step runs natively and says
so at the point it runs. Wrapping those in the image would answer a different
question. Everything else stays inside.

## Consequences

Easier:
- Toolchain bumps are one-point (Dockerfile) and propagate everywhere.
- Reproducing a CI failure locally is literally `just ci`.
- Onboarding is `docker compose build dev && just test`.

Harder:
- First-time build of the image takes minutes (mitigated by sccache, mold, multi-stage
  caching, and Dependabot keeping image deps fresh).
- Interactive debuggers (rust-gdb) require devcontainer attach — documented in the
  dev-env section of the README.

## Alternatives considered

- **Host toolchain + rust-toolchain.toml**: insufficient because non-Rust tooling
  (bun, sccache, the lint/coverage binaries) still drifts.
- **Nix flake**: powerful but the team hasn't invested in Nix knowledge, and Nix on
  WSL/Windows has friction.

## References

- [Justfile](../../Justfile)
- [Dockerfile](../../Dockerfile)
