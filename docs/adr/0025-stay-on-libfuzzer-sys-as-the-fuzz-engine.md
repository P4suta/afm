# 0025. Stay on libfuzzer-sys as the fuzz engine

- Status: accepted
- Date: 2026-07-29
- Deciders: @P4suta
- Tags: testing, infra, dependencies

## Context

`crates/aozora-flavored-markdown/fuzz` depends on `libfuzzer-sys`, the Rust
bindings to LLVM's libFuzzer. Two facts put the choice up for review
(issue #197):

- libFuzzer has been in maintenance-only mode upstream since late 2022. Its
  original authors moved to Centipede. Important bugs still get fixed; new
  features do not land.
- It is the reason for a licence exemption. `libfuzzer-sys` declares
  `(MIT OR Apache-2.0) AND NCSA`, which requires a per-package entry in
  `.github/workflows/dependency-review.yml` (#195) and NCSA in `deny.toml`.

`libafl_libfuzzer` from the LibAFL project is documented as a drop-in
replacement for `libfuzzer-sys` under `cargo-fuzz`, and declares
`MIT OR Apache-2.0`. The comment #195 left in `dependency-review.yml` said
switching would retire the exemption as a side effect.

That sentence was wrong, and so was the premise it rested on. What follows is
measured against `libafl_libfuzzer` 0.15.4 (published 2025-11-12, the current
release) and against this repo's own toolchain and recipes.

## Decision

Stay on `libfuzzer-sys`. Keep both licence entries, and correct the comment in
`dependency-review.yml` that promised they were about to become removable.

Four measurements, in the order of how much each one costs.

### 1. `libafl_libfuzzer` does not replace `libfuzzer-sys`; it wraps it

`libafl_libfuzzer/src/lib.rs` line 116 is `pub use libfuzzer_sys::*;`, and its
manifest carries

```toml
[dependencies.libfuzzer-sys]
version = "0.4.7"
default-features = false
```

as a normal, non-optional dependency resolved from crates.io. The
`fuzz_target!` macro the harnesses use *is* `libfuzzer-sys`'s; what
`libafl_libfuzzer` substitutes is the runtime library the harness links
against, not the binding layer. `libfuzzer-sys` therefore stays in
`crates/aozora-flavored-markdown/fuzz/Cargo.lock`, stays in the dependency
graph GitHub's dependency-review reads, and the `(MIT OR Apache-2.0) AND NCSA`
declaration stays with it.

The one benefit that motivated the review is unavailable at any price. Every
remaining cost below is therefore paid for nothing.

### 2. It ignores `-max_total_time`, which every timed recipe passes

`libafl_libfuzzer`'s runtime parses 23 flags. `max_total_time` is absent, and
so is `max_len`. Its fallthrough arm pushes an unrecognised flag onto an
`unknown` list and carries on, so a run is handed the flag, discards it, and
searches until something else stops it.

Every timed recipe passes `-- -max_total_time={{SECONDS}}`: `fuzz-quick`,
`fuzz-deep`, and through them `fuzz-all-quick`, `fuzz-all-deep` and the
scheduled run in `.github/workflows/fuzz.yml`. Under the new engine each of those
would run past its budget into the `timeout --kill-after=10s` backstop, exit
124, fail the recipe under `set -e`, and take the sweep red on every pull
request — which is #224 exactly, fixed by #251 the day before this was written,
reappearing in a form no gate here would attribute to the engine.

`-max_len` is load-bearing separately: `canonicalize_round_trip.rs` argues its I9
totality invariant from inputs being bounded well under the `u32` span budget,
and that bound is libFuzzer's.

### 3. The runtime resolves a third dependency graph that this repo cannot pin

`libafl_libfuzzer`'s build script synthesises a manifest in `OUT_DIR` from
`runtime/Cargo.toml.template`, gives it a `[workspace]` table of its own, and
shells out to a nested `cargo build` against it. That graph is `libafl`,
`libafl_bolts`, `libafl_targets`, `mimalloc`, `bindgen`, `hashbrown`,
`env_logger` and their transitive closure, resolved fresh, bound by no
lockfile that can live in this repo.

The fuzz workspace's `Cargo.lock` is committed because `cargo fuzz` has no
`--locked`, and both workspaces are checked directly by cargo-deny. A second
resolution inside `OUT_DIR` would remain outside both lockfiles and both
official-tool scans.

### 4. It needs a larger nightly installation before it compiles once

The build script shells out to `llvm-nm` and `llvm-objcopy` out of the nightly
sysroot. This repo installs a date-pinned nightly with `rust-src` through mise;
the `llvm-tools` component is absent. Adopting the engine starts by adding that
component to the pinned toolchain. The runtime then builds at `lto = true`,
`codegen-units = 1`, `opt-level = 3` on top of the cold AddressSanitizer build
#251 measured at 64 s.

### What the switch was never going to fix

Stated here because it is the tempting wrong reason to revisit this: the
toolchain requirement is unaffected either way. It comes from `cargo-fuzz`
passing `-Zsanitizer`, so the fuzz crate would still sit outside the cargo
workspace, still be unreachable from `[lints] workspace = true`, and still go
uncompiled by `cargo check --workspace`. `just deny` and `just fuzz-build`
cover those two properties directly.

## Consequences

- The two licence entries stay: `pkg:cargo/libfuzzer-sys` in
  `dependency-review.yml` and NCSA in `deny.toml`. Cargo-deny checks the fuzz
  workspace directly.
- Maintenance-only mode remains the standing risk, and it is a small one here:
  five harnesses, a committed seed corpus `just fuzz-seed` regenerates from the
  playground examples and the spec sources, and a search whose findings are
  bug reports rather than blocked merges. The exposure is a libFuzzer bug going
  unfixed, and libFuzzer still takes fixes.
- Re-opening this is cheap and the trigger is concrete: `libafl_libfuzzer`
  dropping its `libfuzzer-sys` dependency, or implementing `max_total_time`.
  Either one changes the arithmetic; until one of them happens the answer is
  unchanged and this document is why.

## Alternatives considered

**Switch to `libafl_libfuzzer`.** Rejected on the four measurements above. The
headline benefit is unavailable because the crate depends on the package whose
licence the exemption is for; the headline cost is that every `just fuzz-*`
recipe stops respecting its time budget.

**Switch, and rewrite the recipes around `-runs=` instead of
`-max_total_time=`.** `-runs` is supported, so the recipes could be re-expressed
as an iteration count. Rejected: a run count is a different control from a time
budget. `fuzz.yml` bounds a job in wall-clock minutes, and #251's whole finding
was that the fuzzing budget and the build cost had been conflated — swapping the
budget for a quantity that varies with machine speed puts that back. It also
buys the change nothing, since #1 stands whatever the recipes say.

**Vendor `libafl_libfuzzer` and patch out the `libfuzzer-sys` re-export.**
Rejected on sight. ADR-0024 retired the last vendored tree in this repo and
recorded why; taking on a fuzzing engine as vendored source, to solve a licence
entry that is already correct, inverts that decision for less than it was worth
the first time.

**Drop fuzzing.** Not seriously entertained; recorded because it is the only
other way to retire the exemption. The five targets carry invariants nothing
else in the suite reaches over arbitrary bytes.

## References

- Issue #197 — the evaluation this records.
- #195 — the pull request that added the exemption, and wrote the sentence this
  ADR corrects.
- #224 / #251 — the sweep's time budget, broken and fixed. Section 2 is the
  measurement that says a swap would undo #251.
- The committed `fuzz/Cargo.lock` and the direct cargo-deny/fuzz-build checks
  described in section 3.
- ADR-0026 — the native mise environment used for the pinned nightly.
- ADR-0024 — the vendoring decision the third alternative would reverse.
- `libafl_libfuzzer` 0.15.4 on crates.io; measurements taken from the published
  `.crate` contents.
