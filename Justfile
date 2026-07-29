# aozora-flavored-markdown workspace task runner.
# The ONE entry point for every development operation. Every target runs inside Docker;
# never invoke cargo or bun on the host directly.

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

# --- internal helpers ---------------------------------------------------------

# `AOZORA_MD_IN_CONTAINER=1` is baked into the dev/fuzz/ci images (see Dockerfile). On
# the host it is unset, so every recipe wraps its tool in `docker compose run`
# (ADR-0002). Inside one of those images — a `just shell`, a devcontainer, or a
# Codespace — it is "1", so recipes run the tool DIRECTLY rather than nesting a
# second container (there is no Docker daemon in there). One Justfile, both
# worlds; no docker-in-docker.
#
# ci.yml's two `[group('native')]` jobs set it by hand on a bare runner, which
# is the same instruction — resolve the tool where you are — asked of the one
# place where "where you are" is deliberately not the dev image. It is what
# lets those jobs run the same recipe `just ci` does instead of a second copy
# of the command.
_in := env_var_or_default("AOZORA_MD_IN_CONTAINER", "0")

# Default run prefix for the interactive dev container (TTY attached)
_dev := if _in == "1" { "" } else { "docker compose run --rm dev" }
# Non-interactive variant for CI-like invocations (no TTY)
_ci  := if _in == "1" { "" } else { "docker compose run --rm --no-TTY ci" }
# Nightly-bearing variant. The `dev` image is stable-only after the Dockerfile
# fuzz-stage split; `_fuzz` is for recipes that need `cargo +nightly`
# (`udeps`, every `fuzz*` recipe, `coverage-branch`). Inside the `ci`/`fuzz`
# image nightly is present, so the direct form works there too.
_fuzz := if _in == "1" { "" } else { "docker compose run --rm fuzz" }

# Every cargo invocation below passes `--locked`. Cargo.lock is the resolution
# of record; a recipe that silently re-resolves lets a green local run, CI and
# the published crate each compile a different dependency graph, and the one
# that differs is discovered by a user. `--locked` rather than `--frozen`:
# `--frozen` also implies `--offline`, which breaks the first build in a cold
# container that still has to fetch the registry.
#
# The tools with no such flag, and why each is already covered:
#   cargo fmt, cargo insta review/accept — never resolve the graph.
#   cargo audit — Cargo.lock IS its input; it cannot read a different one.
#   cargo semver-checks — no `--locked` in its CLI.
#   cargo fuzz — cargo-fuzz 0.13.2 has no `--locked`, and no way to hand one
#     to the `cargo build` it shells out to (`cargo fuzz build --help`; the
#     trailing args of `run` go to libFuzzer, which has no such flag either).
#     The fuzz crate is its own workspace, so it resolves its own graph — and
#     until DEV-293 that resolution was thrown away and redone per build, i.e.
#     the targets could be fuzzing a different `aozora` / `comrak` than the
#     library ships. `fuzz/Cargo.lock` is committed instead, `just fuzz-build`
#     fails when a build rewrote it — which is what the flag would have
#     refused up front — and `just verify-version-pins` compares the versions
#     the two lockfiles resolve to.
#
# Two files spell cargo calls this one does not: `bacon.toml` (the jobs behind
# `just watch`) and the `Dockerfile` (tool installs). They carry the same flag,
# and `crates/xtask/tests/lock_binding.rs` reads all three plus the workflows
# so the policy is a test rather than a habit.

# --- metadata -----------------------------------------------------------------

# Default: show this help, recipes grouped by area
[group('meta')]
default:
    @just --list

# --- build/shell --------------------------------------------------------------

# Fastest possible "does it still compile" gate. Skips codegen and
# linking; runs in seconds on a warm cache. Use as the first thing you
# run after editing source — every other build/test recipe depends on
# this being green, so failing here surfaces the problem 10× sooner than
# waiting for `just test` to error out at the same site.
[group('build')]
check:
    {{_dev}} cargo check --locked --workspace --all-targets

# Build all workspace crates
[group('gate')]
[group('build')]
build:
    {{_dev}} cargo build --locked --workspace --all-targets

# rustdoc's lints are almost all warn-by-default — `broken_intra_doc_links` is
# the one that denies — so a `cargo doc` printing `private_intra_doc_links`,
# `invalid_html_tags` or `redundant_explicit_links` still exits 0. `-D warnings`
# is what makes the two recipes below gates instead of reports. It used to live
# in `docs.yml` alone, i.e. the earliest a doc regression could fail was the
# Pages deploy AFTER the merge, which is the one place the answer is too late.
#
# It is passed as an environment variable because it cannot be passed the usual
# way: `rustdocflags` belongs in `.cargo/config.toml`, and this repo cannot have
# that file — `.gitignore` excludes `/.cargo/` because CARGO_HOME resolves there
# inside the dev image, so the config would be untracked and the gate would
# exist only on the machine that wrote it.
#
# `--cfg docsrs` rides along because docs.rs passes it: every published crate's
# `[package.metadata.docs.rs]` sets `rustdoc-args = ["--cfg", "docsrs"]`, and a
# gate that omits it is building a different configuration from the one
# consumers read. Nothing in `src/` gates on it today, so it costs nothing —
# but the day an item takes a `#[cfg(docsrs)]` feature badge, the badge is
# already under a gate instead of being discovered on the published page.
_DOC_DENY := "-D warnings --cfg docsrs"

# The other warning a doc build can print, and the one `_DOC_DENY` cannot
# reach. `output filename collision` is CARGO's, not rustdoc's: two targets in
# one `--workspace` pass resolve to the same `target/doc/<name>/`, so the unit
# that finishes last overwrites the other and cargo says so and carries on
# exiting 0. RUSTDOCFLAGS is handed to rustdoc, which never sees this — the
# `-D warnings` that made the doc recipes gates could not have caught it, and
# did not: both CLI bins were overwriting their libraries' pages, and the API
# site `docs.yml` copies out of `target/doc` was serving a CLI page at the
# library's URL, nondeterministically, for as long as both crates have existed.
#
# Cargo has no `-D` of its own for it, so it is read back off the build's own
# output. Both recipes below `tee` into a log and end with this check, which
# expects `$log` (that output) and `$rc` (the build's exit status, captured
# under `pipefail`) in scope — one definition, because a check written twice is
# a check that gets fixed once.
#
# The clash itself is settled in the two CLI manifests (`doc = false` on each
# `[[bin]]`, with the reasoning); this is the part that fails if a third target
# ever resolves onto a second one's output path.
_NO_COLLISION := 'if grep -qF "output filename collision" "$log"; then printf "%s\n" "doc: two targets wrote one rustdoc output path (the collision warning above). The later unit overwrites the earlier, so what lands in target/doc — and on the Pages API site — is whichever finished last. Give the losing target doc = false in its manifest, or rename it." >&2; exit 1; fi; exit $rc'

# Build rustdoc for every crate, private items included — the wider of the two
# doc gates, and the only one that resolves a link written inside a private
# item. check / clippy run no rustdoc lint at all.
#
# `--all-features` for the same reason `doc-public` needs it (see below); this
# recipe is also what `docs.yml` deploys to Pages, and `[workspace.package]
# documentation` points a reader there.
[group('gate')]
[group('build')]
doc:
    {{_dev}} bash -c 'set -o pipefail; log=$(mktemp); RUSTDOCFLAGS="{{_DOC_DENY}}" cargo doc --locked --workspace --all-features --no-deps --document-private-items 2>&1 | tee "$log"; rc=$?; {{_NO_COLLISION}}'

# The build docs.rs performs: the public surface, no `--document-private-items`.
# Not a subset of `doc` — documenting private items also SILENCES
# `private_intra_doc_links`, so a public item linking into a private module
# passes `doc` and dangles for every reader of the published documentation.
# This recipe is the one kept equivalent to docs.rs's own invocation, so what a
# PR checks is what consumers will actually get.
#
# `--all-features` because every published crate's `[package.metadata.docs.rs]`
# says `all-features = true`, and the library has no default features at all:
# without it this gate builds `theme`, `serde`, `miette` and `tsify` — most of
# the documented surface — for nobody, and their rustdoc never runs until
# docs.rs runs it. `a_gate_builds_the_documentation_docs_rs_will_publish` reads
# the manifests and holds this line to them, so the pair cannot drift apart
# again by editing one side.
[group('gate')]
[group('build')]
doc-public:
    {{_dev}} bash -c 'set -o pipefail; log=$(mktemp); RUSTDOCFLAGS="{{_DOC_DENY}}" cargo doc --locked --workspace --all-features --no-deps 2>&1 | tee "$log"; rc=$?; {{_NO_COLLISION}}'

# Build release binaries
[group('build')]
build-release:
    {{_dev}} cargo build --locked --release --workspace

# Drop into an interactive dev shell
[group('build')]
shell:
    {{_dev}} bash

# Run the aozora-flavored-markdown CLI with arbitrary args (same as ./bin/aozora-flavored-markdown ARGS)
[group('build')]
run *ARGS:
    {{_dev}} cargo run --locked --package aozora-flavored-markdown-cli --quiet -- {{ARGS}}

# --- tests --------------------------------------------------------------------

# Run the full test suite (unit + integration + snapshot)
[group('gate')]
[group('test')]
test *ARGS:
    {{_dev}} cargo nextest run --locked --workspace --all-targets {{ARGS}}

# Run doctests (nextest skips these by design)
[group('gate')]
[group('test')]
test-doc:
    {{_dev}} cargo test --locked --workspace --doc

# `just test` (nextest) leaves `.snap.new` files but does not apply them.
# Review pending insta snapshot changes interactively (accept/reject each).
[group('test')]
snapshot-review:
    {{_dev}} cargo insta review

# Accept ALL pending insta snapshots without review (eyeball the diff first).
[group('test')]
snapshot-accept:
    {{_dev}} cargo insta accept

# Property-based tests. Default 128 cases per proptest block
# (AOZORA_PROPTEST_CASES override, read by the test-support crate's
# `config::default_config`). Fast enough to live in `just ci` — see
# `just prop-deep` for a stress run.
#
# `options_surface_contract` is named alongside the glob because it carries
# the one property quantified over the whole Options space; the rest of that
# binary is deterministic and costs milliseconds.
[group('gate')]
[group('test')]
prop:
    {{_dev}} cargo nextest run --locked --workspace --all-features --test 'property_*' --test options_surface_contract --run-ignored default

# Deep property sweep — 4096 cases per block, used before cutting a
# release to exercise invariants beyond the default CI budget.
[group('test')]
prop-deep:
    {{_dev}} bash -c 'AOZORA_PROPTEST_CASES=4096 cargo nextest run --locked --workspace --all-features --test "property_*" --test options_surface_contract --run-ignored default'

# Replay one proptest failure from its seed (printed on nextest's FAIL line).
# Optional TARGET narrows to one `property_*` test binary; default is all.
[group('test')]
prop-seed SEED TARGET="property_*":
    {{_dev}} bash -c 'AOZORA_PROPTEST_SEED={{SEED}} cargo nextest run --locked --workspace --all-features --test "{{TARGET}}" --run-ignored default'

# Run every `invariant_unit_` predicate test — narrow regression target
# that skips the full proptest sweep. Scoped to `--workspace` rather than one
# package: the predicates live in aozora-flavored-markdown-test-support today,
# and a package-pinned filter silently selected zero tests once already.
[group('test')]
invariants:
    {{_dev}} cargo nextest run --locked --workspace --lib -E 'test(invariant_unit_)'

# Both spec suites whole: CommonMark 0.31.2 (652 cases, pass = 652/652) and
# GFM 0.29 (672, of which 13 are pinned to the spec version that supersedes
# the 0.29 fixture rather than skipped — `conformance::expected` is the list).
# A `#[cfg(test)] mod` of the library, not an integration test: the spec's
# expected output needs raw-HTML passthrough, which has no public switch.
[group('gate')]
[group('test')]
spec:
    {{_dev}} cargo nextest run --locked --package aozora-flavored-markdown --lib -E 'test(conformance::)'

# The wasm exports, run on the target they ship to.
#
# This recipe is what `_COV_IGNORE` defers to. That exclusion has cited a
# `wasm-pack test` step since it was written, and until this recipe there was
# none anywhere in the repo — so ten of the crate's fourteen exports
# (`initPanicHook`, `slugsJson` and every `AozoraDocument` method) were covered
# by nothing at all. `crates/aozora-flavored-markdown-wasm/tests/wasm.rs` is
# the harness; the two native test files beside it carry the opposite `cfg`, so
# each half runs where it means something.
#
# `--node` rather than a headless browser: nothing under test touches the DOM,
# and node is already in the dev image for the playground. `web_sys::window()`
# is `None` there, which is the documented host-agnostic path through `now_ms`.
#
# `RUSTC_WRAPPER=` for the reason `wasm-build` clears it — wasm-pack shells out
# to `rustup target add`, which corrupts the sccache server.
#
# `--locked` is a trailing positional here, NOT a `-- --locked` passthrough the
# way `wasm-build` spells it: `wasm-pack test` takes cargo's own flags as
# `PATH_AND_EXTRA_OPTIONS` and forwards a `--` to the test binary instead, where
# `--locked` is not an argument at all. `lock_binding.rs` knows both shapes.
[group('gate')]
[group('test')]
test-wasm:
    {{_dev}} bash -c 'RUSTC_WRAPPER= wasm-pack test --node \
        crates/aozora-flavored-markdown-wasm --locked'

# Aozora-layer fixtures (annotation cases, golden 56656, corpus sweep)
# now live in the sibling `aozora` repo; run `just spec-aozora`
# / `just spec-golden-56656` / `just corpus-sweep` from there.

# --- fuzzing -----------------------------------------------------------------
#
# libFuzzer harnesses (`parse_render` / `render_blocks` /
# `serialize_round_trip` / `sjis_decode`) live in
# `crates/aozora-flavored-markdown/fuzz/`; they run under nightly in the dev
# container. Triaged crashes are promoted into `tests/fuzz_regressions/` so
# `just test` replays them with no nightly required.

# The triple every `fuzz*` recipe builds for. Stated rather than defaulted:
# cargo-fuzz takes `--target` from the platform ITS OWN BINARY was built for,
# and the fuzz image installs it with `cargo binstall`, which fetches the
# upstream `x86_64-unknown-linux-musl` release asset. So every recipe below
# asked for a musl build of this workspace, inside images (and on CI runners)
# that carry only `x86_64-unknown-linux-gnu` — and got two errors, neither
# about this repo's code: `sanitizer is incompatible with statically linked
# libc`, then `can't find crate for core`. That is every fuzz recipe, for every
# fuzz target, on a clean checkout: `fuzz-all-deep`'s "a clean run is the gate
# before tagging a release" was a gate that had never once run (DEV-230).
#
# Installing the musl target into the fuzz image is the other repair, and it is
# the worse one: the static libc that makes musl the attractive default is
# exactly what the sanitizer refuses, so a musl build would have to hand the
# static linking back (`-C target-feature=-crt-static`) before it could fuzz at
# all — a second toolchain target bought to arrive where the gnu one already
# is. Do not drop this flag as a redundant default. It is not one, and what
# replaces it is not a fuzz run on the host triple but no fuzz run at all.
_FUZZ_TRIPLE := "x86_64-unknown-linux-gnu"

# The one list of what the fuzz targets are. `cargo fuzz list` reads the
# `[[bin]]` tables of `fuzz/Cargo.toml`, which is the same registry
# `cargo fuzz run <name>` resolves against — so a target added there is added
# to every sweep below at the same moment, and there is no second list to
# forget. There were four of those lists, and DEV-230 was filed believing
# there were three targets when there were already four.
#
# Not `+nightly`: `list` compiles nothing, it reads a manifest, and the
# cargo-fuzz binary is toolchain-independent. Underscore-prefixed, so `just
# --list` (hence `just gates`, hence the CI matrix) does not offer a query as
# a task.
_fuzz-targets:
    @{{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && cargo fuzz list'

# Compile every fuzz target. The fuzz crate declares its own `[workspace]`
# (correctly — libfuzzer-sys is nightly-only and must not join a stable
# `--workspace` build), and the cost of that is total: `cargo check
# --workspace` has never compiled one line of it. Four harnesses that call
# this crate's public API by hand were outside every gate the repo has, so a
# rename in `src/` broke them silently and stayed broken until somebody fuzzed
# manually (DEV-270, DEV-291). This is the gate that closes it.
#
# Build rather than run: "the harnesses still match the API" is the question a
# PR asks, and it is answered by a compile in seconds. Running them is
# `fuzz.yml`'s job — `fuzz-all-quick` per PR, `fuzz-all-deep` on the nightly
# cron.
#
# It is also where this crate's `[lints.rust]` is enforced: the levels in
# `fuzz/Cargo.toml` reach rustc only when something compiles the crate, and
# before this recipe nothing did.
#
# The comparison at the end is this repo's `--locked` (see the policy header
# above): cargo-fuzz has no such flag, so a re-resolution is detected by the
# file it rewrote instead of refused before it happened.
#
# Against a copy taken a line earlier, and NOT against HEAD. `git diff --quiet`
# stood here and answers a different question — "does this file differ from the
# last commit" — which is the same answer on a clean CI checkout and the wrong
# one everywhere else. A branch carrying a lockfile this build did not touch
# (`just fuzz-lock` run, diff under review, nothing committed yet: the exact
# workflow the recipe below asks for) failed this gate with the file's checksum
# unchanged across the build and a message saying the build had rewritten it.
# A gate that is red on the correct state is a gate people learn to scroll
# past. The committed-ness of the file is not this recipe's question either:
# `lock_binding.rs` asks whether a clone would get it, and the two lockfiles
# disagreeing is what `verify-version-pins` fails on.
[group('gate')]
[group('fuzz')]
fuzz-build:
    #!/usr/bin/env bash
    set -euo pipefail
    lock="crates/aozora-flavored-markdown/fuzz/Cargo.lock"
    before=$(mktemp)
    trap 'rm -f "$before"' EXIT
    cp "$lock" "$before"
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && cargo +nightly fuzz build --target {{_FUZZ_TRIPLE}}'
    if ! cmp -s "$before" "$lock"; then
        printf 'fuzz-build: the build re-resolved %s\n' "$lock" >&2
        printf '  The fuzz workspace is pinned by that lockfile the way the workspace is by\n' >&2
        printf '  its own. Review the diff below and commit it, or restore the file.\n' >&2
        printf '  `just fuzz-lock` is the same re-resolution without the build.\n' >&2
        diff -u "$before" "$lock" >&2 || true
        exit 1
    fi

# `crates/aozora-flavored-markdown/fuzz/Cargo.lock` is a SECOND lockfile, and
# nothing moves it when the first one moves. Every cargo bump — Dependabot's
# `Cargo.lock` edit, `cargo xtask
# aozora-bump`'s `cargo update -p aozora`, a pin changed by hand — leaves
# `fuzz/Cargo.lock` on the version before, and a fuzz target built from it
# fuzzes a parser this repo does not ship. That drift is what
# `verify-version-pins` and `lock_binding.rs` fail on; this is the one command
# that clears it, and both messages name it.
#
# Dependabot cannot own this file, which is why the fix is a recipe and not a
# `directories:` entry in `.github/dependabot.yml`: the fuzz manifest declares
# `libfuzzer-sys`, `aozora` and two path dependencies, and NONE of the versions
# that drift are among them. `comrak` reaches this graph through the path
# dependency on `aozora-flavored-markdown`, whose `comrak.workspace = true`
# reads the ROOT manifest — so there is nothing in `fuzz/Cargo.toml` for
# Dependabot to raise a comrak PR against, and a bump that arrives any other
# way (aozora-bump, a hand edit) was never Dependabot's to carry either.
#
# `cargo update --workspace` rather than a re-generate: it is the sub-command
# whose whole job is rewriting a lockfile (hence its exemption from the
# `--locked` policy at the top of this file), in its minimal form. It re-locks
# the workspace member and touches a registry pin only where the recorded
# version has stopped satisfying a requirement — which is exactly the bumped
# crate, because the workspace pin moving is what put the old entry out of
# range. `libfuzzer-sys`, `arbitrary` and `jobserver`, the packages only this
# graph has, keep the versions they had.
#
# `just fuzz-build` also rewrites the file, as a side effect of compiling every
# registered target against libFuzzer under nightly, and then fails because it
# did. That is the gate; this is the fix.
#
# Re-resolve the fuzz workspace's lockfile onto the graph the workspace ships.
[group('fuzz')]
fuzz-lock:
    #!/usr/bin/env bash
    set -euo pipefail
    lock="crates/aozora-flavored-markdown/fuzz/Cargo.lock"
    # What this run changed, which is not the same question as what differs
    # from HEAD: on a branch that already carries the re-resolution, a
    # `git diff` is non-empty and this recipe did nothing.
    before=$(mktemp)
    cp "$lock" "$before"
    {{_dev}} cargo update --manifest-path crates/aozora-flavored-markdown/fuzz/Cargo.toml --workspace
    if cmp -s "$before" "$lock"; then
        printf 'fuzz-lock: %s already resolved the graph the workspace ships; unchanged\n' "$lock"
    else
        printf 'fuzz-lock: re-resolved %s — review the diff below and commit it\n' "$lock"
        git --no-pager diff -- "$lock"
    fi
    rm -f "$before"

# Run the named fuzz target with arbitrary args (escape hatch for advanced use).
[group('fuzz')]
fuzz *ARGS:
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && cargo +nightly fuzz run --target {{_FUZZ_TRIPLE}} {{ARGS}}'

# What the backstop below allows on top of libFuzzer's own `-max_total_time`,
# in seconds. It pays for a shutdown rather than for a search: libFuzzer leaves
# its loop when the budget is up, then writes back the corpus it grew and
# prints its final stats, and in front of that sits cargo satisfying itself
# that the target it is about to exec is still up to date. Thirty seconds
# covers both several times over and is still nowhere near a hang.
_FUZZ_GRACE := "30"

# Build the named target, then fuzz it for SECONDS with the `timeout` around
# the run and nothing else.
#
# Two commands rather than one, because `cargo fuzz run` is two things: it
# compiles the target, then executes it. A `timeout` wrapped around that pair
# spends the fuzzing budget on the build — and on a cold runner an
# AddressSanitizer build of this graph is minutes, not seconds, so what the
# budget bought was a SIGKILL somewhere inside the compile. fuzz.yml's
# `sweep (quick)` (90 s around a 60 s run) exited 124 on all five runs it ever
# had and libFuzzer never started once; the pull requests it was red on merged
# regardless, because a fuzz finding is a bug report and the job is advisory
# on purpose (#224).
#
# Locally it looked fine, because a warm target directory makes the build a
# no-op — which is the same reason no gate here caught it. `just ci` runs
# `fuzz-build`, and by the time anything runs a target the compile has already
# been paid for somewhere else.
#
# The backstop stays. What it bounds is real and is not the build: libFuzzer
# hanging on an input, with `-max_total_time` promising nothing about a run
# that never reaches the end of its loop. It now bounds that alone.
#
# One recipe rather than three copies of it. `fuzz-quick`, `fuzz-deep` and
# `fuzz-marathon` differ in one number, and each carried its own hand-computed
# second number beside it; the defect above was in all three and was noticed in
# the one CI happened to run. The arithmetic is here now, so a caller cannot
# get the two budgets the wrong way round and a fourth recipe cannot reopen it.
_fuzz-timed TARGET SECONDS:
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && \
        cargo +nightly fuzz build --target {{_FUZZ_TRIPLE}} {{TARGET}} && \
        timeout --kill-after=10s $(( {{SECONDS}} + {{_FUZZ_GRACE}} ))s \
            cargo +nightly fuzz run --target {{_FUZZ_TRIPLE}} {{TARGET}} -- -max_total_time={{SECONDS}}'

# 60-second smoke fuzz.
[group('fuzz')]
fuzz-quick TARGET:
    @just _fuzz-timed {{TARGET}} 60

# 5-minute deep fuzz — the gate to clear before tagging a release.
[group('fuzz')]
fuzz-deep TARGET:
    @just _fuzz-timed {{TARGET}} 300

# 15-minute marathon fuzz — strongest single-target soak; exits cleanly at 15 min.
[group('fuzz')]
fuzz-marathon TARGET:
    @just _fuzz-timed {{TARGET}} 900

# Reproduce every artifact under `fuzz/artifacts/<target>/` and print
# (bytes, panic-message) for each. Exit status is the count of artifacts
# that still crash, so this can drive a CI gate. Order is alphabetical
# by hash so output stays stable across machines.
[group('fuzz')]
fuzz-triage TARGET:
    #!/usr/bin/env bash
    set -euo pipefail
    target="{{TARGET}}"
    art_dir="crates/aozora-flavored-markdown/fuzz/artifacts/${target}"
    if [[ ! -d "$art_dir" ]]; then
        echo "fuzz-triage: no artifacts for target ${target}"
        exit 0
    fi
    failed=0
    for art in $(find "$art_dir" -type f -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' | sort); do
        # `cargo fuzz run` resolves relative paths against the crate's
        # own directory (we cd into `crates/aozora-flavored-markdown` before
        # invoking it), so strip only the `crates/aozora-flavored-markdown/`
        # prefix — `fuzz/artifacts/...` is the form cargo-fuzz wants.
        rel="${art#crates/aozora-flavored-markdown/}"
        echo "==> ${rel}"
        out=$({{_fuzz}} bash -c "cd crates/aozora-flavored-markdown && cargo +nightly fuzz run --target {{_FUZZ_TRIPLE}} ${target} ${rel} 2>&1" || true)
        # Slice out the panic block: from the `thread … panicked` line
        # through the line just before the stack trace begins. That is
        # exactly where `assert_html_invariants` prints its tier label
        # + src + html + details — the only four lines a developer
        # actually reads. If no panic block is present, fall back to
        # the tail of the output so we never go silent.
        panic_block=$(awk '
            /^thread .* panicked at/ { capturing = 1 }
            capturing {
                if (/^stack backtrace:/ || /^=================/) exit
                print
            }
        ' <<<"$out")
        if [[ -n "$panic_block" ]]; then
            printf "%s\n" "$panic_block"
        else
            tail -5 <<<"$out"
        fi
        if grep -q "exit status: 77" <<<"$out"; then
            failed=$((failed + 1))
        fi
        echo
    done
    if (( failed > 0 )); then
        echo "fuzz-triage: ${failed} artifact(s) still crash" >&2
        exit "${failed}"
    fi
    echo "fuzz-triage: every artifact replays cleanly"

# Lift a fuzz artifact into the permanent regression set so the
# `tests/fuzz_regressions.rs` integration test asserts it forever.
# Drop the matching entry from `fuzz/artifacts/` once promoted (a
# regression case lives in tests/, not in libFuzzer's working set).
[group('fuzz')]
fuzz-promote TARGET ARTIFACT:
    #!/usr/bin/env bash
    set -euo pipefail
    src="crates/aozora-flavored-markdown/fuzz/artifacts/{{TARGET}}/{{ARTIFACT}}"
    dst_dir="crates/aozora-flavored-markdown/tests/fuzz_regressions/{{TARGET}}"
    if [[ ! -f "$src" ]]; then
        echo "fuzz-promote: artifact not found: $src" >&2
        exit 1
    fi
    # The artifact was written by libFuzzer running as root inside the
    # dev container, so the move + rm must go back through the
    # container too — host-side permissions can't unlink it.
    {{_fuzz}} bash -c "mkdir -p '$dst_dir' && mv '$src' '$dst_dir/{{ARTIFACT}}'"
    echo "promoted ${src} -> ${dst_dir}/{{ARTIFACT}}"

# Run every registered fuzz target in turn for 60 s each. Smoke pass:
# typically used after touching anything in `crates/aozora-flavored-markdown/src/`
# or `crates/aozora-flavored-markdown-test-support/src/`. Also what `fuzz.yml`
# runs per pull request.
[group('fuzz')]
fuzz-all-quick:
    #!/usr/bin/env bash
    set -euo pipefail
    for target in $(just _fuzz-targets); do
        just fuzz-quick "$target"
    done

# Run every registered fuzz target in turn for 5 min each. Release
# pre-flight pass: a clean run is the gate before tagging a release, and
# `fuzz.yml`'s nightly cron is what keeps that gate warm rather than
# discovering it broken on release day.
[group('fuzz')]
fuzz-all-deep:
    #!/usr/bin/env bash
    set -euo pipefail
    for target in $(just _fuzz-targets); do
        just fuzz-deep "$target"
    done

# At-a-glance health check: how many crash artifacts are pending
# triage, how many regression cases are pinned per target. Everything it
# counts is a `find` over the working tree; the one container hop is asking
# `cargo fuzz list` what the targets are, which compiles nothing.
[group('fuzz')]
fuzz-status:
    #!/usr/bin/env bash
    set -euo pipefail
    mapfile -t targets < <(just _fuzz-targets)
    printf "%-22s  %-10s  %-12s\n" target pending_crashes pinned_regressions
    printf "%-22s  %-10s  %-12s\n" ---------------------- ---------- ------------
    for t in "${targets[@]}"; do
        crashes=0
        regressions=0
        if [[ -d "crates/aozora-flavored-markdown/fuzz/artifacts/${t}" ]]; then
            crashes=$(find "crates/aozora-flavored-markdown/fuzz/artifacts/${t}" -maxdepth 1 -type f \( -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' \) 2>/dev/null | wc -l | tr -d ' ')
        fi
        if [[ -d "crates/aozora-flavored-markdown/tests/fuzz_regressions/${t}" ]]; then
            regressions=$(find "crates/aozora-flavored-markdown/tests/fuzz_regressions/${t}" -maxdepth 1 -type f ! -name '*.txt' ! -name '*.md' 2>/dev/null | wc -l | tr -d ' ')
        fi
        printf "%-22s  %-10s  %-12s\n" "$t" "$crashes" "$regressions"
    done

# (Re)install the committed seed corpus. A fuzzer handed an empty corpus
# spends its first minutes rediscovering that markdown has headings; handed
# documents this repo already owns, it spends them where nobody has written a
# test yet. Both sources are in-tree:
#
#   * `playground/examples/*.md` — the aozora directives (ruby, bouten,
#     tate-chu-yoko, paired containers). No seed carried one, and that layer is
#     the whole reason this crate exists rather than a comrak dependency.
#   * `spec/sources/*.txt` — every CommonMark and GFM spec example, one seed
#     per example. Split rather than copied whole for a reason that is easy to
#     get backwards: given no `-max_len`, libFuzzer takes the length of the
#     LARGEST corpus file as its maximum input size, so seeding with a 205 KiB
#     spec document would make every generated input up to 205 KiB and the
#     fuzzer slower than it is with no seeds at all. The spec writes tabs as
#     `→`, so they are restored — the same substitution CommonMark's own
#     `spec_tests.py` makes.
#
# Names are content-addressed: the examples the two spec documents share
# collapse to one file, re-running is a no-op, and a source change shows up as
# the seeds it added rather than as every seed shifting by one. The `seed-`
# prefix is the half `.gitignore` tracks — everything else in a corpus
# directory is libFuzzer's own output.
#
# Not a gate. The corpus is committed, so this is the tool that rebuilds it
# when a source document changes, not something a PR has to re-run.
[group('fuzz')]
fuzz-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    corpus="crates/aozora-flavored-markdown/fuzz/corpus"
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    mkdir -p "$work/seeds"

    for doc in playground/examples/*.md; do
        cp "$doc" "$work/seeds/$(basename "$doc")"
    done

    # A spec example is: 32 backticks + ` example`, the markdown, a line
    # holding one `.`, the expected HTML, 32 backticks. Only the markdown half
    # is an input. The fence is built rather than typed so nobody has to count
    # backticks to review this.
    fence=$(head -c 32 /dev/zero | tr '\0' '`')
    awk -v out="$work/seeds" -v fence="$fence" '
        index($0, fence " example") == 1 { taking = 1; body = ""; next }
        taking && $0 == "." {
            taking = 0
            n += 1
            file = sprintf("%s/spec-%04d", out, n)
            printf "%s", body > file
            close(file)
            next
        }
        taking { gsub(/→/, "\t"); body = body $0 "\n" }
    ' spec/sources/*.txt

    for target in $(just _fuzz-targets); do
        dir="$corpus/$target"
        mkdir -p "$dir"
        rm -f "$dir"/seed-*
        for seed in "$work"/seeds/*; do
            # Two targets read something other than "the whole input is UTF-8
            # source", and each needs its seeds in the shape it reads. The
            # comment in `crates/xtask/tests/gate_wiring.rs` that excuses this
            # recipe for naming a target called the second one before it
            # existed: a `[[bin]]` with its own input format, seeded with
            # documents it cannot parse, reported by the count below as
            # cheerfully as the rest.
            #
            # `sjis_decode` hands its bytes to `decode_sjis`, which rejects a
            # UTF-8 seed at its first multi-byte character — so an unencoded
            # seed would teach it nothing. What CP932 cannot represent (an em
            # dash, say) is dropped rather than transliterated: a seed is
            # worth having only if it is what the decoder would really be
            # handed.
            #
            # `options_space` reads a two-byte option mask before its source,
            # so a seed handed over unprefixed loses its first two bytes to
            # the mask and is decoded from the third — which for a document
            # opening on a multi-byte character is not UTF-8 at all, and the
            # target rejects it. Zeroes rather than a chosen configuration:
            # the mask is the two bytes the fuzzer will mutate first, so what
            # a seed owes is the right SHAPE, and picking a value here would
            # be picking which corner of the space every seed starts from.
            if [[ "$target" == sjis_decode ]]; then
                iconv -f UTF-8 -t CP932 <"$seed" >"$work/encoded" 2>/dev/null || continue
                seed="$work/encoded"
            elif [[ "$target" == options_space ]]; then
                cat <(head -c 2 /dev/zero) "$seed" >"$work/masked"
                seed="$work/masked"
            fi
            cp "$seed" "$dir/seed-$(sha1sum <"$seed" | cut -c1-16)"
        done
        printf 'fuzz-seed: %-24s %4d seed(s)\n' "$target" \
            "$(find "$dir" -maxdepth 1 -name 'seed-*' | wc -l)"
    done

# Benchmarks (criterion)
[group('bench')]
bench *ARGS:
    {{_dev}} cargo bench --locked --workspace {{ARGS}}

# Save the current criterion numbers as a named baseline (default
# `pre-opt`). Run before a structural change; `bench-compare` diffs
# against it. criterion stores baselines under target/criterion/.
[group('bench')]
bench-baseline NAME="pre-opt":
    {{_dev}} cargo bench --locked --workspace -- --save-baseline {{NAME}}

# Re-run the benches and report the % change vs a saved baseline.
[group('bench')]
bench-compare NAME="pre-opt":
    {{_dev}} cargo bench --locked --workspace -- --baseline {{NAME}}

# Heap-allocation profile (dhat) of one large render: total allocations
# + peak resident bytes, and a dhat-heap.json for the dh_view viewer.
[group('bench')]
dhat:
    {{_dev}} cargo run --locked --release --example dhat_render -p aozora-flavored-markdown

# Small-document render latency percentiles (p50/p90/p99/max).
[group('bench')]
latency:
    {{_dev}} cargo run --locked --release --example latency_hist -p aozora-flavored-markdown

# Host-only CPU flamegraph of a render hot loop. samply needs
# perf_event_open(2), which Docker's seccomp blocks, so it records on the host
# (the ADR-0002 profiling exception). Built `--profile bench` to keep symbols.
# Needs `samply` on PATH and perf_event_paranoid <= 1; writes
# /tmp/aozora-md-render.json.gz (open at https://profiler.firefox.com).
[group('bench')]
samply-render REPEAT="200":
    cargo build --locked --profile bench --example samply_render -p aozora-flavored-markdown
    samply record --save-only --no-open -o /tmp/aozora-md-render.json.gz -r 4000 -- target/release/examples/samply_render {{REPEAT}}

# --- coverage -----------------------------------------------------------------

# Coverage gate. Fails when region coverage drops below `_COV_FLOOR`.
#
# Regions, not branches: `cargo-llvm-cov` has `--fail-under-regions` but no
# `--fail-under-branches` (branch counts need nightly); regions are finer than
# branches, so a region threshold implies the branch one on stable.
#
# The denominator is every `src/` file of every crate this workspace publishes,
# entry points included. That is a rule and not a wish: `gate_wiring.rs`'s
# `no_source_file_of_a_crate_this_repo_publishes_is_out_of_the_coverage_denominator`
# matches this regex against every member's `src/` and fails on any published
# file it reaches, whatever shape the entry that reaches it is written in.
# `_COV_IGNORE` therefore holds only what is not published source, or what a
# gate that DOES run it covers — never what a comment asserts is fine.
# Everything but the wasm crate is the first case: build output, or a member
# whose own manifest says `publish = false`. The wasm crate is the second — it
# ships to wasm32 and llvm-cov instruments the host build, so the exclusion
# defers to `just test-wasm`, which runs that crate's tests on the target it
# ships to, and
# `a_crate_excused_from_coverage_for_shipping_to_wasm_is_tested_on_wasm` goes
# red if that gate ever disappears.
#
# Test code is out of the denominator already, and by nothing written here:
# cargo-llvm-cov's own default regex drops
# `<workspace>/**/{tests,examples,benches}/` and appends ours to it — read the
# merged regex back with `cargo llvm-cov report -vv`. Naming `tests/` here
# would move no number. A `#[cfg(test)] mod tests` INSIDE a `src/` file is the
# case no filename regex can reach (same file as the code it tests), and
# `#[coverage(off)]` is nightly-only and banned by `just strict-code` — so
# those modules are counted, on both sides of the ratio.
#
# The floor is measured, not chosen, and it does not survive a change of
# denominator: 97 was set over a narrower one. Two exclusions were dropped
# here, both of which had only prose behind them (DEV-315):
#   * `/main\.rs$` — since the binaries were thinned to a shim it excused 3
#     regions per binary, and both measure 100%: the CLI integration tests
#     spawn the binary and llvm-cov collects the subprocess. It excused
#     nothing, so it can enforce something instead. What it now rests on is
#     read too:
#     `every_binary_this_repo_publishes_is_run_as_a_process_by_its_own_tests`
#     fails if a CLI's tests stop spawning it, which the floor itself would
#     never notice at 3 regions in 7317.
#   * the EPUB `compose.rs` / `package.rs` file-level exclusion — its claim
#     (`.map_err(…)` arms over an infallible `Cursor<Vec<u8>>` sink) is true of
#     ~160 regions and hid ~725 more of live OPF/NAV/ZIP logic with it, and is
#     not true at all of `package.rs`, which writes a real `fs::File` and
#     already tests one of those arms. The dead arms are a ~2.4-point tax;
#     narrowing the exemption to just them is DEV-315's job, not a reason to
#     keep two source files out of sight.
# Measured over the full published source: 95.65-95.67% (7317 regions, 317-318
# missed on consecutive runs — the proptest seeds are random, so the last
# region or two moves), where the narrower denominator read 98.04% (6226 / 122).
# Hence a whole number rather than a tight fractional floor: 95 leaves the
# ~0.6 points that variance needs and nothing like enough for untested code.
_COV_FLOOR := "95"
_COV_IGNORE := "(target/|xtask/|aozora-flavored-markdown-test-support/|aozora-flavored-markdown-wasm/)"

[group('gate')]
[group('coverage')]
coverage:
    {{_dev}} cargo llvm-cov nextest \
        --locked \
        --workspace \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

# HTML coverage report for local inspection. No threshold — intended
# for opening `coverage/html/index.html` in a browser.
[group('coverage')]
coverage-html:
    {{_dev}} cargo llvm-cov nextest \
        --locked \
        --workspace \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --html --output-dir coverage/html

# Branch-level coverage report (requires nightly for `--branch` support).
# Informational only — no threshold. Use to surface uncovered conditionals
# when working a specific file toward C1 100%.
[group('coverage')]
coverage-branch:
    {{_fuzz}} cargo +nightly llvm-cov nextest \
        --locked \
        --branch \
        --workspace \
        --ignore-filename-regex '{{_COV_IGNORE}}'

# --- lint / static analysis ---------------------------------------------------

# Run every lint. The dependency list below is the list.
[group('lint')]
lint: fmt-check clippy typos strict-code comment-discipline vale zizmor actionlint

# Retired-path gate for the half of the question no prose linter can answer,
# plus the comment volume ratchet. Both lists, and the reason each exists,
# are in `crates/xtask/src/main.rs` (RETIRED_REPO_PATHS, MAX_COMMENT_LINES).
#
# The third thing this used to do — a retired *upstream* path named in a `.rs`
# or `.toml` comment — is `just vale` now, which reads the `.md` files this
# never opened (DEV-221).
[group('gate')]
[group('lint')]
comment-discipline:
    {{_dev}} cargo run --locked --package xtask --quiet -- comment-discipline

# Forbid patterns that hide bugs or introduce unstable/unsafe surface in our
# own crates. Every check is defensive — each represents a pattern we have
# decided IS a bug-source and want rejected at the gate rather than fought
# later in code review.
[group('gate')]
[group('lint')]
strict-code:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s globstar
    files=(crates/**/*.rs)

    check() {
        local label="$1"
        local pattern="$2"
        local hits
        hits=$(grep -nE "$pattern" "${files[@]}" 2>/dev/null || true)
        if [[ -n "$hits" ]]; then
            echo "==> forbidden: $label" >&2
            echo "$hits" >&2
            return 1
        fi
    }

    failed=0

    # ---- Warning suppression -----------------------------------------------
    # `#[allow(... reason = "...")]` (Rust 1.81+ stable) is the documented
    # "I considered this lint and overrode it deliberately" idiom and is
    # allowed; a bare `#[allow(...)]` without a reason is forbidden — it
    # rots into a dead rule that hides bugs. This matches our own
    # `clippy::allow_attributes_without_reason` lint (see Cargo.toml): a
    # blanket text ban here would contradict that lint by also rejecting
    # the reasoned form it explicitly blesses. We grep with -A 5 to catch a
    # reason clause on a continuation line, then drop hits whose attribute
    # window contains `reason = "..."`.
    #
    # `build.rs` files are excluded: their string literals can embed
    # `#[allow(...)]` snippets emitted as generated code, which are not real
    # attributes under strict-code's purview. (aozora-flavored-markdown has no build.rs today;
    # the carve-out keeps parity with aozora and is future-proof.)
    src_files=()
    for f in "${files[@]}"; do
        case "$f" in
            */build.rs) ;;
            *) src_files+=("$f") ;;
        esac
    done
    bare_allow=$(grep -nE -A 5 '^\s*#!?\[allow\(' "${src_files[@]}" 2>/dev/null \
        | awk -F: '
            /#!?\[allow\(/      { capture = 1; window = ""; head = $0 }
            capture              { window = window $0 "\n" }
            capture && /\)\]/    {
                if (window !~ /reason[[:space:]]*=[[:space:]]*"/) {
                    print head
                }
                capture = 0
            }
        ' || true)
    if [[ -n "$bare_allow" ]]; then
        echo '==> forbidden: warning suppression (#[allow] without reason="...")' >&2
        echo "$bare_allow" >&2
        failed=1
    fi
    check 'cfg_attr-wrapped warning suppression' \
        '^\s*#!?\[cfg_attr\([^)]*allow\(' || failed=1

    # ---- Nightly / unstable feature gates ----------------------------------
    # We ship on Rust stable only. Feature gates silently tie us to nightly
    # and rot on toolchain bumps.
    check 'nightly feature gate (#[feature] / #![feature])' \
        '^\s*#!?\[feature\(' || failed=1

    # ---- Unsafe code -------------------------------------------------------
    # Every crate root has `#![forbid(unsafe_code)]` (checked below); this
    # text-level grep is belt-and-braces for typos that would defeat the
    # compiler gate. Excludes the legitimate `r#unsafe` raw-identifier form
    # used by comrak's `render.r#unsafe` field.
    check 'unsafe code (unsafe fn / unsafe { / unsafe impl / unsafe trait)' \
        '(^|[^a-zA-Z_#])unsafe\s+(fn|impl|trait|\{)' || failed=1

    # ---- Required deny directive -------------------------------------------
    # Each crate root must start with `#![forbid(unsafe_code)]` so accidental
    # unsafe additions are rejected at compile time.
    for root in crates/*/src/lib.rs crates/*/src/main.rs; do
        [[ -f "$root" ]] || continue
        if ! grep -q '^#!\[forbid(unsafe_code)\]' "$root"; then
            echo "==> forbidden: crate root missing '#![forbid(unsafe_code)]'" >&2
            echo "  $root" >&2
            failed=1
        fi
    done

    # ---- Toolchain pinning -------------------------------------------------
    # rust-toolchain.toml must pin a semver-numbered stable channel. Any
    # appearance of nightly/beta in the channel pin is rejected.
    if grep -qE '^\s*channel\s*=\s*"(nightly|beta)' rust-toolchain.toml; then
        echo "==> forbidden: rust-toolchain.toml pins a pre-stable channel" >&2
        grep -nE '^\s*channel' rust-toolchain.toml >&2
        failed=1
    fi

    # ---- TODO/FIXME/XXX without an issue reference -------------------------
    # Drive-by notes rot into dead reminders. Every TODO/FIXME/XXX must
    # reference either an issue (`#N`) or a milestone (`M1..M4`) so it can
    # be tracked or reclassified. Requires word-boundary match so placeholder
    # hex sequences like `U+XXXX` don't false-positive.
    todo_hits=$(grep -nE '(^|[^[:alnum:]_])(TODO|FIXME|XXX)([^[:alnum:]_]|$)' "${files[@]}" 2>/dev/null \
        | grep -vE '(#[0-9]+|M[0-9]|issue|ADR-[0-9]+)' || true)
    if [[ -n "$todo_hits" ]]; then
        echo '==> forbidden: bare TODO/FIXME/XXX without an issue or milestone reference' >&2
        echo "$todo_hits" >&2
        failed=1
    fi

    # ---- println! / eprintln! in library crates ----------------------------
    # Library crates should emit observability via `tracing`, not raw print.
    # CLI crates (aozora-flavored-markdown-cli, xtask) are expected to print, so they are scoped
    # out. Examples (`crates/*/examples/`) and fuzz targets
    # (`crates/*/fuzz/fuzz_targets/`) are also exempt — they're binary-style
    # demos, not library code. This complements clippy::print_stdout /
    # clippy::print_stderr, which cannot be selectively enabled per-crate
    # while still inheriting [workspace.lints] (rust-lang/cargo#12697).
    lib_files=(crates/aozora-flavored-markdown/**/*.rs)
    print_hits=$(grep -nE '(^|[^[:alnum:]_])e?print(ln)?!\s*\(' "${lib_files[@]}" 2>/dev/null \
        | grep -vE '/(tests|benches|examples|fuzz_targets)/' || true)
    if [[ -n "$print_hits" ]]; then
        echo '==> forbidden: println! / eprintln! in library crates (use tracing instead)' >&2
        echo "$print_hits" >&2
        failed=1
    fi

    # ---- expect() regression gate (aozora-flavored-markdown library source) ------------
    # Coarse tripwire: counts every `.expect(` under
    # `crates/aozora-flavored-markdown/src/**` (test modules included — this is a
    # no-regression ratchet, not a precise audit). The current baseline is
    # all locally-justified: `String`/`fmt::Write` sinks that cannot fail,
    # a `u32::try_from` bounded by the Phase-0 cap, and the forward-range
    # `sourcepos_to_range`. A NEW state-assertion-style `expect` in a
    # production path should be lifted into the type system or pinned by a
    # property test instead of pushed to runtime. Bump the baseline only when
    # you remove an expect.
    #
    # Raised 8 -> 12 when the spec-conformance runners moved from `tests/`
    # (which this gate excludes) into `src/conformance.rs`, and the
    # streaming-builder test followed the type it exercises into
    # `src/ir/mod.rs`. All four are `#[cfg(test)]` — one fixture decode and
    # three test-local unwraps. The production count is still 8.
    expect_files=(crates/aozora-flavored-markdown/src/**/*.rs)
    expect_count=$(grep -hcE '\.expect\(' "${expect_files[@]}" 2>/dev/null \
        | awk '{s+=$1} END {print s+0}')
    expect_baseline=12
    if [[ "$expect_count" -gt "$expect_baseline" ]]; then
        echo "==> forbidden: expect() count in aozora-flavored-markdown source grew" >&2
        echo "    baseline: $expect_baseline, found: $expect_count" >&2
        echo "    Lift the invariant into the type system or a property test" >&2
        echo "    instead of pushing it to runtime." >&2
        failed=1
    fi

    # ---- second HTML escape table ------------------------------------------
    # One escape table, in `aozora_flavored_markdown::escape_html`. A copy is
    # not a style problem: the EPUB envelope kept its own for months, the two
    # agreed character for character, and nothing in the workspace could have
    # noticed if a fix had landed on one side only — an escaper is audited for
    # what it MISSES, and a test suite that renders one of two tables green is
    # exactly the shape that hides it. This is the only gate that can state
    # "one implementation", so it is the gate; a behaviour test cannot fail on
    # duplication while both copies still agree.
    #
    # A table is a source character mapped to its entity, in either idiom:
    # `'&' => "&amp;"` and `.replace('&', "&amp;")`. Scoped to `src/`, so a
    # test may still write the mapping down as an oracle — a checker that
    # called the code under test could not fail when that code is wrong.
    escape_owner='crates/aozora-flavored-markdown/src/lib.rs'
    escape_files=(crates/*/src/**/*.rs)
    escape_from="('\\\\?[&<>\"']'|\"[&<>']\")"
    escape_to='"&(amp|lt|gt|quot|apos|#39|#x27);"'
    escape_hits=$(grep -nE "$escape_from[[:space:]]*(=>|,)[^\"]*$escape_to" "${escape_files[@]}" 2>/dev/null \
        | grep -v "^$escape_owner:" || true)
    if [[ -n "$escape_hits" ]]; then
        echo '==> forbidden: a second HTML escape table (call escape_html instead)' >&2
        echo "$escape_hits" >&2
        failed=1
    fi

    if [[ $failed -ne 0 ]]; then
        echo "" >&2
        echo "strict-code check failed. Refactor the offending sites; do not silence." >&2
        exit 1
    fi
    echo "strict-code: clean (expect-count $expect_count / baseline $expect_baseline)"

# Format check (no-write)
[group('gate')]
[group('lint')]
fmt-check:
    {{_dev}} cargo fmt --all -- --check

# Auto-format (writes)
[group('lint')]
fmt:
    {{_dev}} cargo fmt --all

# Clippy. Lint groups and carve-outs live entirely in `[workspace.lints]`;
# passing `-W clippy::<group>` here would override the per-lint allow carve-outs,
# so keep the CLI surface to `-D warnings` only.
[group('gate')]
[group('lint')]
clippy:
    {{_dev}} cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# Typo check
[group('gate')]
[group('lint')]
typos:
    {{_dev}} typos

# GitHub Actions static analysis (zizmor). The workflows are the one part of
# this repo no compiler reads, so the properties they have to hold — every
# `uses:` on an immutable commit, a checkout that doesn't leave a usable token
# behind, a job that cannot write what it has no business writing — are held
# by this instead. The policy is `zizmor.yml`; this recipe only runs it.
#
# `--offline`: the online audits want a GitHub token, and a gate has to mean
# the same thing on a fork PR, on main and on a laptop. Every audit the pinning
# policy rests on is decidable from the tree.
#
# The `command -v` line is a bridge, not the install: CI pulls the published
# `aozora-md-dev:latest`, which lags a Dockerfile tool addition by one merge,
# so the tool provisions itself into the pulled image until it republishes
# (`just shear` carries the same bridge in the form a cargo crate needs). Install
# and use share one container because `docker compose run --rm` keeps nothing.
# The version is read back out of the Dockerfile ARG that pins it, so the
# bridge cannot install a build the image would not have had.
[group('gate')]
[group('lint')]
zizmor:
    #!/usr/bin/env bash
    set -euo pipefail
    v=$(grep -oE 'ZIZMOR_VERSION=[0-9.]+' Dockerfile | head -1 | cut -d= -f2)
    url="https://github.com/zizmorcore/zizmor/releases/download/v${v}/zizmor-x86_64-unknown-linux-gnu.tar.gz"
    {{_dev}} bash -c "set -euo pipefail
        command -v zizmor >/dev/null 2>&1 || curl -fsSL '${url}' | tar -xz -C /usr/local/bin zizmor
        zizmor --offline --no-progress ."

# Workflow schema + expression lint (actionlint). Complements zizmor rather
# than overlapping it: zizmor asks whether a workflow is safe, actionlint
# whether it is a workflow at all — an unknown key, a `needs:` naming a job
# that isn't there, a `${{ }}` whose type cannot be what it is used as.
#
# shellcheck / pyflakes integration is switched OFF explicitly. actionlint
# picks both up from PATH when present, so leaving it to autodetect makes the
# gate mean one thing in the dev image (which ships neither) and another in a
# shell that happens to have them. Same Dockerfile-pinned bootstrap as
# `just zizmor` above, for the same one merge.
[group('gate')]
[group('lint')]
actionlint:
    #!/usr/bin/env bash
    set -euo pipefail
    v=$(grep -oE 'ACTIONLINT_VERSION=[0-9.]+' Dockerfile | head -1 | cut -d= -f2)
    url="https://github.com/rhysd/actionlint/releases/download/v${v}/actionlint_${v}_linux_amd64.tar.gz"
    {{_dev}} bash -c "set -euo pipefail
        command -v actionlint >/dev/null 2>&1 || curl -fsSL '${url}' | tar -xz -C /usr/local/bin actionlint
        actionlint -no-color -shellcheck= -pyflakes="

# Prose lint (Vale). The drift no compiler, clippy pass or typo checker calls
# its business: a sentence that has stopped being true. Here that is a retired
# upstream path — an internal of the sibling parser this workspace once
# reached into — named anywhere a human writes a sentence in this repo.
#
# Markdown is why this exists. The scan it replaces read `.rs` and `.toml` and
# nothing else, so a Markdown file was outside every gate in the repo:
# `UPSTREAM_DIFF.md` went stale in full — it described a vendored tree that no
# longer existed — with the whole pipeline green. Vale reads Markdown as prose
# and extracts comments from `.rs` natively, so the case that was covered and
# the case that was not are now one rule, and one list. That list is
# `styles/Aozora/RetiredPaths.yml`; what gets read is `.vale.ini`. This recipe
# only runs them.
#
# `git ls-files` with NO pathspec, which is the whole point. A list of file
# kinds is what the replaced scan got wrong, and writing the same list one
# language over would have reproduced the defect with a new tool: the first
# full-tree run found a retired crate named in this very file, in the prose
# above the expect() tripwire, where `'*.md' '*.rs' '*.toml'` could not reach
# it. What this repo authors is what git tracks; there is no second definition
# to keep in step, and nothing about the scope is written down twice.
# `every_file_this_repo_tracks_is_one_the_prose_gate_reads` holds it there.
#
# `command -v`: the one-merge bridge, described above `just zizmor`.
[group('gate')]
[group('lint')]
vale:
    #!/usr/bin/env bash
    set -euo pipefail
    v=$(grep -oE 'VALE_VERSION=[0-9.]+' Dockerfile | head -1 | cut -d= -f2)
    url="https://github.com/vale-cli/vale/releases/download/v${v}/vale_${v}_Linux_64-bit.tar.gz"
    {{_dev}} bash -c "set -euo pipefail
        command -v vale >/dev/null 2>&1 || curl -fsSL '${url}' | tar -xz -C /usr/local/bin vale
        mapfile -d '' -t files < <(git ls-files -z)
        vale \"\${files[@]}\""

# Assert tool-version pins agree across files: bun (Dockerfile /
# playground/package.json / docs.yml) and wasm-pack (Dockerfile / docs.yml).
# Fails if any pair disagrees.
[group('gate')]
[group('lint')]
verify-version-pins:
    #!/usr/bin/env bash
    set -euo pipefail
    fail=0
    extract() {
        local file="$1"
        local pattern="$2"
        grep -oE "$pattern" "$file" | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true
    }

    # bun: Dockerfile ARG / playground/package.json packageManager / docs.yml setup-bun
    bun_docker=$(extract Dockerfile 'BUN_VERSION=[0-9.]+')
    bun_pkg=$(extract playground/package.json '"bun@[0-9.]+"')
    bun_docs=$(extract .github/workflows/docs.yml "bun-version: '[0-9.]+'")
    if [[ -n "$bun_docker" && "$bun_docker" == "$bun_pkg" && "$bun_docker" == "$bun_docs" ]]; then
        printf '[OK] bun pin: %s (Dockerfile / playground/package.json / docs.yml agree)\n' "$bun_docker"
    else
        printf '[!!] bun pin drift: Dockerfile=%s playground/package.json=%s docs.yml=%s\n' \
            "$bun_docker" "$bun_pkg" "$bun_docs" >&2
        fail=1
    fi

    # wasm-pack: Dockerfile ARG / docs.yml jetli/wasm-pack-action
    wp_docker=$(extract Dockerfile 'WASM_PACK_VERSION=[0-9.]+')
    wp_docs=$(extract .github/workflows/docs.yml "version: 'v[0-9.]+'")
    if [[ -n "$wp_docker" && "$wp_docker" == "$wp_docs" ]]; then
        printf '[OK] wasm-pack pin: %s (Dockerfile / docs.yml agree)\n' "$wp_docker"
    else
        printf '[!!] wasm-pack pin drift: Dockerfile=%s docs.yml=%s\n' \
            "$wp_docker" "$wp_docs" >&2
        fail=1
    fi

    # aozora: workspace manifest / cargo-fuzz manifest. Both are rewritten in
    # one pass by `cargo xtask aozora-bump <version>`; drift here means the
    # fuzz targets are building against a different parser than the library.
    az_ws=$(extract Cargo.toml '^aozora[[:space:]]*=[[:space:]]*\{[[:space:]]*version[[:space:]]*=[[:space:]]*"=?[0-9.]+')
    az_fuzz=$(extract crates/aozora-flavored-markdown/fuzz/Cargo.toml '^aozora[[:space:]]*=[[:space:]]*\{[[:space:]]*version[[:space:]]*=[[:space:]]*"=?[0-9.]+')
    if [[ -n "$az_ws" && "$az_ws" == "$az_fuzz" ]]; then
        printf '[OK] aozora pin: %s (Cargo.toml / fuzz/Cargo.toml agree)\n' "$az_ws"
    else
        printf '[!!] aozora pin drift: Cargo.toml=%s fuzz/Cargo.toml=%s\n' \
            "$az_ws" "$az_fuzz" >&2
        fail=1
    fi

    # The same two crates, as RESOLVED rather than as declared. The block
    # above compares two manifests; this compares two lockfiles, which is the
    # half that was missing. The fuzz crate is its own workspace, `aozora` is
    # pinned there with `=` but `comrak` is not pinned there at all (it arrives
    # transitively), and cargo-fuzz has no `--locked` — so the targets could
    # have been fuzzing a different parser than the library ships with both
    # manifests still reading the same version (DEV-293). Committing
    # `fuzz/Cargo.lock` is what makes the question answerable; this is what
    # asks it.
    locked() {
        # The `version` under the first `[[package]]` whose `name` matches.
        awk -v want="$2" '
            BEGIN { quoted = "\"" want "\"" }
            $1 == "name" && $3 == quoted { seen = 1; next }
            seen && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
        ' "$1"
    }
    fuzz_lock="crates/aozora-flavored-markdown/fuzz/Cargo.lock"
    if [[ ! -f "$fuzz_lock" ]]; then
        printf '[!!] %s is missing — the fuzz workspace resolves unpinned again\n' \
            "$fuzz_lock" >&2
        fail=1
    else
        for dep in aozora comrak; do
            dep_ws=$(locked Cargo.lock "$dep")
            dep_fuzz=$(locked "$fuzz_lock" "$dep")
            if [[ -n "$dep_ws" && "$dep_ws" == "$dep_fuzz" ]]; then
                printf '[OK] %s resolved: %s (Cargo.lock / fuzz/Cargo.lock agree)\n' \
                    "$dep" "$dep_ws"
            else
                printf '[!!] %s resolution drift: Cargo.lock=%s fuzz/Cargo.lock=%s — run `just fuzz-lock`\n' \
                    "$dep" "$dep_ws" "$dep_fuzz" >&2
                fail=1
            fi
        done
    fi

    # No git source may reintroduce itself: a git dependency makes
    # `cargo publish` reject the crate (ADR-0015), and `aozora-bump` only
    # knows how to rewrite registry versions.
    if git_pins=$(grep -rn 'P4suta/aozora\.git' Cargo.toml crates/*/Cargo.toml crates/*/fuzz/Cargo.toml 2>/dev/null); then
        printf '[!!] aozora git source pin(s) found — registry versions only:\n%s\n' "$git_pins" >&2
        fail=1
    else
        echo "[OK] aozora source: registry only (no git pin)"
    fi

    if (( fail == 0 )); then
        echo "verify-version-pins: all pins agree"
        exit 0
    else
        echo "verify-version-pins: drift detected — see [!!] lines above" >&2
        exit "$fail"
    fi

# Dependency linting (licenses, advisories, bans)
[group('gate')]
[group('lint')]
deny:
    {{_dev}} cargo deny --locked check

# Unused-dependency scan (stable, syn-based). Flags deps declared in any
# Cargo.toml — including `[workspace.dependencies]` — that no crate actually
# `use`s, the blind spot rustc's `dead_code` and the nightly `udeps` compile
# graph both miss. Mirrors the sibling aozora-tools `shear` gate. On a hit:
# delete the dead dependency, or record a documented
# `[workspace.metadata.cargo-shear] ignored = [...]` for a macro/cfg-only use.
#
# `command -v`: the one-merge bridge, described above `just zizmor`, in the
# form a cargo crate needs.
[group('gate')]
[group('lint')]
shear:
    {{_dev}} bash -c 'command -v cargo-shear >/dev/null 2>&1 \
        || cargo binstall --no-confirm --locked --root /usr/local cargo-shear; \
        cargo shear --locked'

# comrak resolves from the registry like every other dependency (ADR-0024),
# so the lockfile graph is the whole graph — no shim needed to reach it.
# RustSec advisory scan over `Cargo.lock`.
#
# `--deny warnings` is what makes this a gate rather than a report. Without it
# cargo-audit exits 0 on everything RustSec files as a warning — `unmaintained`,
# `unsound`, `notice`, and a `yanked` crate — and prints it to a log nobody
# reads, so only a live vulnerability could ever fail a build. `deny.toml`
# already spells `yanked = "deny"` for cargo-deny; this is the same stance on
# the other side of the pair. The flag was not new here — the vendored-comrak
# `audit-comrak` recipe passed it, and was the only caller that did, so
# unvendoring took the strictness out with the recipe.
#
# The nightly re-run of this recipe is `.github/workflows/audit.yml`: an
# advisory is published against a lockfile that has not changed, so PR time is
# the one moment a scan cannot catch it.
[group('gate')]
[group('lint')]
audit:
    {{_dev}} cargo audit --deny warnings

# Unused dependency scan (requires nightly)
[group('gate')]
[group('lint')]
udeps:
    {{_fuzz}} cargo +nightly udeps --locked --workspace --all-targets

# Semver gate: the version this workspace declares covers the API changes in it.
#
# The recipe existed and nothing ran it. ADR-0015 deferred wiring it until "a
# baseline exists on crates.io", and that reason was wrong: `--baseline-rev`
# takes the baseline out of THIS repository's git history, so the check has
# been available since the tag it names was cut. What believing otherwise cost
# is the entire public-surface rebuild — every entry point, every IR name,
# every error type moved with the one tool that measures such a move unwired.
#
# `--baseline-rev` is scaffolding: nothing is on crates.io yet, so a tag is the
# only baseline there is. After the first publish, delete the flag — the
# registry version is cargo-semver-checks' own default, and then there is no
# literal here for anyone to keep in step. Both callers check out with
# `fetch-depth: 0`, since the flag resolves a TAG and a depth-1 clone has none.
#
# The two `--exclude`s and the baseline this recipe names are each held by a
# gate that reads them out of this line and checks them against git and against
# the packages the tag holds, so what they are and why is not written here a
# second time. Same for the vacuity of the pass while the declared version is
# a 0.y major bump ahead of the baseline: the gate that measures it says so,
# and it fails the day it stops being true.
#
# No `--locked`, per the block at the top of this file. It is the one entry
# there with nothing else covering it: the baseline build resolves its own
# graph, so which graph proved the comparison is not pinned. DEV-298 tracks
# it; there is no offline or in-repo substitute to write here instead.
#
# The private target directory is not a speed knob, it is the verdict. Both
# halves of this check are rustdoc JSON, and rustdoc names its output after the
# CRATE, not the package it came from: current and baseline are both
# `aozora-flavored-markdown`, so both land on `target/doc/<crate>.json` — the
# same output-path clash `_NO_COLLISION` above catches inside one `cargo doc`
# pass, one directory up. `cargo semver-checks` takes no `--target-dir`, so the
# only place to answer it is the environment. Sharing the directory the doc
# gates write costs a WRONG ANSWER, not a slow one: after `just doc`, cargo
# reads this crate's doc unit as fresh, skips the current-side rustdoc, and
# leaves the baseline's JSON in place — so the gate compares 0.4.1 against
# itself, reports "no change; assume minor", and fails on the `html` feature
# 0.5.0 removed. That is `just ci`'s own order (doc, doc-public, semver), so it
# was reproducible, and it reads as a real break in a workspace that has none.
[group('gate')]
[group('lint')]
semver:
    {{_dev}} bash -c 'CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}/semver-gate" \
        cargo semver-checks check-release --workspace \
        --baseline-rev v0.4.1 \
        --exclude aozora-flavored-markdown-epub \
        --exclude aozora-flavored-markdown-epub-cli'

# --- upstream sources ---------------------------------------------------------

# Move the `aozora` pin in both manifests — the workspace one and the fuzz
# crate's — to a published crates.io version in one pass, then refresh
# Cargo.lock. Idempotent. The SHA this recipe used to take stopped being an
# answer when ADR-0015 replaced the git rev with a registry version, and
# `cargo xtask aozora-bump` has rejected one ever since — which nothing
# noticed, because a recipe parameter is read by `just --list` and by nothing
# else. What the argument has to look like is the sub-command's `--help`.
[group('upstream')]
aozora-bump VERSION:
    {{_dev}} cargo run --locked --package xtask --quiet -- aozora-bump {{VERSION}}

# Regenerate `spec/*.json` from the vendored cmark-format sources under
# `spec/sources/*.txt`. Offline-pure: both the sources and the generated
# fixtures are committed to the repo. Add new `spec/sources/<name>.txt`
# files and extend the conversion block below to cover them.
[group('upstream')]
spec-refresh:
    {{_dev}} bash -c '\
        set -euo pipefail && \
        cargo run --locked --package xtask --quiet -- spec-refresh \
            --input spec/sources/commonmark-0.31.2.txt \
            --output spec/commonmark-0.31.2.json && \
        cargo run --locked --package xtask --quiet -- spec-refresh \
            --input spec/sources/gfm-0.29-gfm.txt \
            --output spec/gfm-0.29-gfm.json'

# --- docs ---------------------------------------------------------------------

# New Architecture Decision Record (MADR template)
[group('docs')]
adr TITLE:
    {{_dev}} cargo run --locked --package xtask --quiet -- new-adr {{TITLE}}

# Draft the unreleased section from Conventional-Commits history, to stdout.
#
# CHANGELOG.md is written by hand, for humans (Keep a Changelog): the entries
# that matter to a consumer explain what broke and what to do about it, which
# no commit subject carries. So this prints a draft — one line per commit, in
# cliff.toml's grouping — to check the hand-written section against before a
# release. It deliberately does NOT write the file: `-o CHANGELOG.md` would
# regenerate the whole thing and take every explanation with it.
[group('docs')]
changelog:
    {{_dev}} git-cliff --unreleased

# --- release -----------------------------------------------------------------

# Cut a version bump: every manifest, `Cargo.lock`, the `## [Unreleased]`
# section of `CHANGELOG.md` and the man page that embeds the version, in one
# command. LEVEL is `patch` / `minor` / `major`, or an explicit version.
#
# DRY RUN BY DEFAULT, and that is the confirmation step: cargo-release writes
# nothing without `--execute`, so `just release minor` prints exactly what it
# would do to which files and `just release minor --execute` does it. The
# interactive prompt is off (`--no-confirm`) rather than being a second answer
# to the same question — a prompt is also the one part of this that cannot be
# read in a review or replayed in a terminal without a TTY.
#
# THE FILE-WRITING STEPS AND NO MORE, and the split is the point. Commit, tag,
# push and publish each belong to something else: the upload is
# `publish-crates.yml`'s, behind an approval gate and an OIDC token, and the
# commit and the tag are SSH-signed with a key that is deliberately not in the
# dev image. A `git commit` from inside the container would not fail — it
# would succeed, unsigned. So this recipe stops when the files are written; the
# release commit and the annotated `v<version>` tag are made on the host
# afterwards, where the key is. `release.toml` refuses the same steps in the
# form cargo-release reads, so running the tool by hand from `just shell` lands
# in the same place.
#
# `--workspace` because the workspace has two version lines and both move, and
# `release.toml`'s `shared-version` is where they are grouped. Leaving the flag
# off selects the default members, which is a subset — and a version line
# half-bumped is the failure a shared version exists to prevent.
#
# Not a gate, and there is nothing here for one to check: this is the only
# recipe in the file that writes to the tree on purpose.
[group('release')]
release LEVEL *ARGS:
    {{_dev}} bash -c 'set -euo pipefail; \
        cargo release version {{LEVEL}} --workspace --no-confirm {{ARGS}}; \
        cargo release replace --workspace --no-confirm {{ARGS}}; \
        cargo release hook --workspace --no-confirm {{ARGS}}'

# Regenerate the shell completions + man page bundled into the release
# archives (under dist/assets/, shipped via dist-workspace.toml `include`).
# Built from the live `aozora-flavored-markdown` CLI, so re-run after changing flags/subcommands
# (and on a version bump — the man page embeds the version). Commit the diff.
[group('release')]
dist-assets:
    {{_dev}} cargo build --locked --package aozora-flavored-markdown-cli --quiet
    {{_dev}} cargo run --locked --package xtask --quiet -- gen-dist-assets

# Drift gate: fail if the committed dist assets differ from fresh generation.
# Wired into `just ci` (mirrors `types-check`); run `just dist-assets` to fix.
[group('gate')]
[group('release')]
dist-assets-check:
    {{_dev}} cargo build --locked --package aozora-flavored-markdown-cli --quiet
    {{_dev}} cargo run --locked --package xtask --quiet -- gen-dist-assets --check

# The version this workspace declares is one CHANGELOG.md has a section for.
#
# The other half of `just release`: the recipe above cuts the section, and this
# is what notices when a release was cut without it — a tag is not required to
# be a commit any pull request ever ran, and the tag is what gets published.
#
# NOT a gate, and it cannot be one: on a branch the section is called
# `## [Unreleased]` and always will be, so a pull request would fail this every
# time. It is answerable exactly once, of a release ref, which is why
# `publish-crates.yml`'s preflight is its only caller — and why the command
# lives here rather than in that file, where nothing but a dispatch would ever
# read it. Run it on a release ref and it answers; run it on `main` today and
# it says 0.5.0 has no section, which is true.
#
# `cargo pkgid` rather than a grep over `Cargo.toml`: this workspace publishes
# on two version lines, so the root manifest's version is the wrong answer for
# the EPUB pair, and reading it as text is what
# `no_workflow_reads_a_crate_version_out_of_a_manifest_by_hand` refuses. The
# package is named because the heading and the `v<version>` tag both carry the
# `workspace` line. The suffix is `#<version>`, or `#<name>@<version>` when the
# directory and the crate disagree; the substitution takes either.
[group('release')]
changelog-check:
    {{_dev}} bash -c 'set -euo pipefail; \
        id=$(cargo pkgid --locked --package aozora-flavored-markdown); \
        version=${id##*[#@]}; \
        if ! grep -qF "## [${version}]" CHANGELOG.md; then \
            printf "changelog-check: CHANGELOG.md has no \"## [%s]\" section.\n" "${version}" >&2; \
            printf "  That heading is written by the release bump, not by hand: run\n" >&2; \
            printf "  just release <level> --execute rather than tagging.\n" >&2; \
            exit 1; \
        fi; \
        printf "changelog-check: CHANGELOG.md describes %s\n" "${version}"'

# The four published crates, built the way a consumer receives them.
#
# Every other compile gate in this file builds the WORKSPACE: `--all-targets`
# over one target directory, path dependencies resolved in place, every file on
# disk reachable from every crate. A consumer gets a tarball per crate and a
# registry version per dependency, and the difference is a class of failure
# nothing above can see — a file the build reads and the package does not
# carry (an `include_str!` reaching above the crate directory is the usual
# one), a path dependency with no version to fall back on, a manifest missing
# something crates.io requires, or a rung that only builds because the rung
# under it was resolved out of the workspace instead of a registry.
#
# Nothing was asking. `publish-crates.yml` is `workflow_dispatch`-only, so the
# tarballs were verified when someone decided to publish and at no other
# moment. This recipe is that workflow's preflight, lifted to where every PR
# runs it — the workflow calls this same recipe rather than spelling the
# command out a second time.
#
# `--allow-dirty` is here because committedness is not this gate's question,
# and asking it anyway would remove the gate from the one place it earns its
# time: `just ci` is what a developer runs BEFORE the commit, so without the
# flag cargo stops on the first edited file and the packaging question never
# gets asked locally at all. On a runner the tree is a fresh checkout and the
# flag is inert. That the live upload carries no such flag is checked by
# `the_upload_still_answers_to_git_though_the_gate_does_not`, not asserted
# here.
[group('gate')]
[group('release')]
package:
    {{_dev}} cargo publish --workspace --dry-run --locked --allow-dirty

# --- playground (browser try-it-online) --------------------------------------

# Vite dev/preview server container — `--service-ports` is required so
# `docker compose run` actually publishes 5173 (it doesn't by default).
#
# Both prefixes go through the `_in` switch, like `_dev` / `_ci` / `_fuzz`.
# While they were hard-coded `docker compose run`, the recipes below were the
# only ones in this file that could not be run from inside the image — and
# `docs.yml` builds the playground on a bare runner, so it spelled the
# wasm-pack build, the bun install and the bun build out itself. Three second
# definitions of one gate's command, carried in `gate_wiring.rs`'s
# `RE_SPELLED_BUILD` as exemptions whose stated reason was exactly this. The
# switch is what retires them: the Pages job now runs `just playground-build`,
# the same recipe `just ci` runs (DEV-310).
#
# The playground service's `working_dir` is the repo root, not
# `/workspace/playground`, so both sides of the switch start where `just`
# itself starts and each recipe spells its own `cd playground` — the shape the
# `fuzz*` recipes already use for `cd crates/aozora-flavored-markdown`. A
# `cd` that only one side needed would be the drift this switch removes.
_pg := if _in == "1" { "" } else { "docker compose run --rm --service-ports playground" }

# Same container without publishing 5173. Used by `playground-install`
# and the gates below so they share the `playground-node-modules`
# named volume but don't trip "address already in use" when an existing
# Vite or dev server is bound to 5173 on the host.
_pg_install := if _in == "1" { "" } else { "docker compose run --rm playground" }

# Build the aozora-flavored-markdown-wasm package for the playground; output to `crates/aozora-flavored-markdown-wasm/pkg/`
# (referenced by `playground/package.json` as `file:../crates/aozora-flavored-markdown-wasm/pkg`).
# `RUSTC_WRAPPER=` bypasses sccache, which wasm-pack's `rustup target add`
# subprocess corrupts (SCCACHE_GHA_ENABLED); the wasm cache benefit is marginal.
# Everything after `--` goes to the `cargo build` wasm-pack shells out to, so
# that is where this build's `--locked` has to sit.
[group('playground')]
wasm-build:
    {{_dev}} bash -c 'RUSTC_WRAPPER= wasm-pack build crates/aozora-flavored-markdown-wasm \
        --target bundler --release \
        --out-dir pkg --out-name aozora_flavored_markdown_wasm -- --locked'

# Dev-profile wasm build for playground iteration. Skips wasm-opt and uses
# the `dev` cargo profile; output is 3-5× bigger and slower at runtime but
# completes in ~10-20 s vs the 60-90 s `wasm-build` release path. Do NOT
# ship the output to GitHub Pages — `just playground-build` and the docs
# workflow both use the release `wasm-build` recipe instead.
[group('playground')]
wasm-build-dev:
    {{_dev}} bash -c 'RUSTC_WRAPPER= wasm-pack build crates/aozora-flavored-markdown-wasm \
        --target bundler --dev \
        --out-dir pkg --out-name aozora_flavored_markdown_wasm -- --locked'

# Install playground deps via bun. Depends on `wasm-build` because the
# `file:` link requires the target directory to exist before `bun install`
# resolves it. Runs inside the `playground` service (no published ports)
# so `node_modules` lands in the named volume (`playground-node-modules`)
# instead of the host bind mount — important on Docker Desktop / WSL
# where cross-fs writes are slow.
#
# `--frozen-lockfile` is the JS half of the `--locked` policy above: bun.lock
# is the resolution of record and no recipe may rewrite it.
#
# A version bump was expected to break that — the `aozora-flavored-markdown-wasm`
# entry is a `file:` link into `crates/aozora-flavored-markdown-wasm/pkg`, whose
# package.json carries the workspace version — and the release bump was going
# to carry a `bun install` to repair it (DEV-295). Measured at the pinned bun
# (1.3.14, lockfile v1) it does not break: the lockfile records that dependency
# as `aozora-flavored-markdown-wasm@file:…/pkg` with no version at all, so this
# recipe passes with the two out of step. Reproduce by editing the `version`
# in `pkg/package.json` and running it. So there is no hook for this, and the
# reason it is written down is that the absence of one is now a measurement
# rather than an omission — bun recording a version here would put the drift
# back, and this comment is where the next reader looks.
[group('playground')]
playground-install: wasm-build
    {{_pg_install}} bash -c 'cd playground && bun install --frozen-lockfile'

# Vite dev server with HMR at http://localhost:5173/
[group('playground')]
playground-dev: playground-install
    {{_pg}} bash -c 'cd playground && bun run dev -- --host 0.0.0.0'

# Same as `playground-dev` but uses the fast dev-profile wasm build for
# inner-loop iteration (TS edits get HMR; wasm changes still need a
# reload after `just wasm-build-dev`).
[group('playground')]
playground-dev-fast: wasm-build-dev
    {{_pg_install}} bash -c 'cd playground && bun install --frozen-lockfile' && \
    {{_pg}} bash -c 'cd playground && bun run dev -- --host 0.0.0.0'

# Production build → playground/dist/ (consumed by .github/workflows/docs.yml)
# Also runs inside `playground` service to share the `node_modules` volume.
[group('gate')]
[group('playground')]
playground-build: playground-install
    {{_pg_install}} bash -c 'cd playground && bun run build'

# Biome over the playground: formatter, linter and import sorting in one pass
# (`biome check`), which is what replaces the eslint + prettier pair a
# TypeScript tree of this size would otherwise carry. Until it landed,
# `tsc --noEmit` inside `playground-build` was the ENTIRE static analysis over
# ~15 modules — and a type checker has no opinion about an unused import, a
# `==`, a `console.log` left in a handler, or an interactive element no
# keyboard can reach. The playground is the first consumer of the wasm
# surface, so what goes unchecked here is what nobody notices about the
# published API either.
#
# `--error-on-warnings` (in the `lint` script) is what makes this a gate
# rather than a report: several of Biome's recommended rules default to
# warn-level, and `biome check` exits 0 on those. The repo's stance on a lint
# it has decided not to take is the `#[allow(..., reason = "…")]` one — say so
# where the rule is configured — so the two declined rules say it here,
# `biome.json` being JSON and unable to hold a comment of its own:
#
#   style/noNonNullAssertion — `tsconfig.json` sets
#     `noUncheckedIndexedAccess`, so `entries[mid]` is `T | undefined` even
#     inside a binary search that just computed `mid` in range. Taking the
#     rule leaves two options at each of those sites: a `!`, or a defensive
#     branch that cannot execute. The second is worse — it is untested code
#     that looks tested.
#   suspicious/noTemplateCurlyInString — scoped off in `biome.json`'s
#     `overrides` for `editor/completion.ts` and `editor/wrapCommands.ts`
#     ONLY, where `'｜${1:base}《${2:reading}》'` is CodeMirror snippet syntax
#     in a plain string, which is exactly what that rule looks for. Everywhere
#     else it still fires.
#
# `bun run lint:fix` (or `just playground-lint-fix`) applies the safe half.
[group('gate')]
[group('playground')]
playground-lint: playground-install
    {{_pg_install}} bash -c 'cd playground && bun run lint'

# The writing half of `playground-lint`: applies Biome's safe fixes and
# rewrites formatting. Mirrors `fmt` / `fmt-check` on the Rust side.
[group('playground')]
playground-lint-fix: playground-install
    {{_pg_install}} bash -c 'cd playground && bun run lint:fix'

# Vitest over the playground's pure modules, through `vite.config.ts` — see
# the `test` block there for why the config is shared rather than split into a
# `vitest.config.ts`.
[group('gate')]
[group('playground')]
playground-test: playground-install
    {{_pg_install}} bash -c 'cd playground && bun run test'

# Preview the production build locally at http://localhost:5173/
[group('playground')]
playground-serve: playground-build
    {{_pg}} bash -c 'cd playground && bun run preview -- --host 0.0.0.0 --port 5173'

# --- native gates -------------------------------------------------------------
#
# Two gates cannot be answered from inside the dev image, and both are tagged
# `[group('native')]` on top of `[group('gate')]` so ci.yml gives them a
# hand-written job instead of a matrix leg. What they are NOT is a second
# definition: the job runs this recipe, with `AOZORA_MD_IN_CONTAINER=1` set so
# the `_in` switch at the top of this file resolves the tool directly rather
# than nesting it in `docker compose run` — which is exactly right there, since
# the point of both jobs is the tool the runner installed natively.
#
# Run from a laptop the same recipes go through the dev image, where they still
# mean something: the image pins the MSRV toolchain, and the commit range just
# defaults to the local branch instead of the PR's.

# MSRV gate: the pinned minimum still compiles the whole workspace.
#
# `rust-toolchain.toml` pins 1.96.0, so inside the dev image this is `just
# check` by another name. Its value is on the CI side, where the job installs
# a clean 1.96.0 with no dev-image layer under it and this recipe is what
# proves the declared minimum builds against it.
[group('gate')]
[group('native')]
msrv:
    {{_dev}} cargo check --locked --workspace --all-targets

# Conventional-Commits gate over a commit range (`committed`, config in
# `committed.toml`). CI passes the PR's base..head; the default covers the
# local branch, which is the same question asked before the PR exists.
#
# No self-bootstrap, unlike `just shear`: that bridge exists because CI runs
# shear inside the published dev image, which lags a Dockerfile tool addition
# by one merge. This gate's CI job installs `committed` on the runner and never
# enters the image, so the only reader of the Dockerfile copy is a laptop —
# where a non-root container cannot write /usr/local and `--rm` would discard
# the download anyway. So: say what is missing, and rebuild.
[group('gate')]
[group('native')]
commitlint RANGE="origin/main..HEAD":
    {{_dev}} bash -c 'command -v committed >/dev/null 2>&1 || { \
        echo "commitlint: no committed in the dev image — docker compose build dev" >&2; \
        exit 1; }; \
        committed --no-merge-commit "{{RANGE}}"'

# --- aggregate ----------------------------------------------------------------

# The gate manifest: every recipe tagged `[group('gate')]`, one per line,
# sorted. This is the ONE list of what a gate is. `just ci` asserts its own
# lanes against it before running anything and ci.yml generates its job matrix
# from the same command, so tagging a recipe adds it to both and there is no
# second list to keep in step. Pass a GROUP to ask a narrower question —
# `just gates native` is what ci.yml subtracts to get its matrix.
#
# `--list --group` rather than `--dump --dump-format json`: both read the same
# attribute (verified identical on the Dockerfile-pinned just and on latest),
# but the JSON form needs a JSON parser and neither the dev image nor a bare
# host is guaranteed to ship one. This form needs awk, so the manifest reads
# the same on a laptop, in the container and on a runner.
[group('meta')]
gates GROUP="gate":
    @just --list --group {{GROUP}} --list-heading '' --list-prefix '' \
        | awk '/^\[/ { next } NF { print $1 }' \
        | sort

# Local CI replica — every gate the workflow runs, slow non-compile gates overlapped to cut wall-clock.
[group('aggregate')]
ci:
    #!/usr/bin/env bash
    set -uo pipefail

    # Why this shape (no gate is weakened vs. the old sequential loop). The two
    # arrays below are the lists; this says why each lane holds what it holds.
    #   * The compile gates all share ONE cargo target dir, so they contend on
    #     its build lock and CANNOT truly run in parallel — they stay
    #     sequential, ordered cheap-to-expensive so a failure surfaces fast.
    #     `msrv` leads: inside the
    #     dev image it is a bare `cargo check`, so it is also the cheapest
    #     possible "does it still compile". `semver` follows the two doc gates
    #     because it is a third one: what it builds is rustdoc's JSON, here and
    #     again in a worktree of the baseline tag. It does NOT share their
    #     target dir — it keeps its own, and this order is why: on the shared
    #     one a preceding `cargo doc` makes it answer about the baseline twice
    #     (see the recipe).
    #   * deny / shear / audit invoke NO rustc and take no build lock (and
    #     spawn no sccache server, so no multi-server churn on the shared cache),
    #     so a BACKGROUND lane overlaps them onto the compile lane for free.
    #   * `check` is not a gate and is not run here: clippy + build both compile
    #     --all-targets, so a bare `cargo check` pass adds no coverage. ci.yml
    #     still runs it, as the fast precondition the gate matrix waits on —
    #     scheduling, not a gate. Everything `lint` bundles runs once on its
    #     own here instead of a second time inside `lint`; only `clippy` is
    #     left to run from `lint`.
    #   * fuzz-build compiles the fuzz crate, which is its own workspace with
    #     its own target dir — so it takes no lock the compile lane holds, but
    #     it does invoke rustc (and clang, for libFuzzer), so it stays in the
    #     foreground rather than joining the background lane. It runs after
    #     `udeps`, the other recipe that reaches for the nightly image.
    #   * `package` closes the host-target half of the lane. It is the one
    #     compile here that starts from the tarball instead of the working
    #     tree: each of the four published crates is unpacked under
    #     `target/package/` and built from its own manifest against the ones
    #     packaged ahead of it. Those verify builds inherit `CARGO_TARGET_DIR`,
    #     so they take the same lock and find the same warm cache as the chain
    #     above — sequential, not beside it — and it runs after that chain
    #     because everything cheaper has had its chance to fail by then.
    #   * test-wasm and the three playground gates are the wasm-pack gates and
    #     run LAST in the foreground lane, in that order: wasm-pack invokes
    #     rustc and shares the target dir, so they cannot overlap anything —
    #     but they compile the same graph for wasm32, and the ones after the
    #     first find it warm. test-wasm leads because it fails on a broken
    #     export where the playground gates fail on a broken consumer of one.
    #     Among the playground three, lint and test come before the build:
    #     each is seconds once `playground-install` has run, and the build is
    #     the minute-long one. playground-build stays last: it pulls
    #     `wasm-build` in as a dependency, so a wasm / IR / diagnostic type
    #     change can no longer pass `just ci` while silently breaking the
    #     playground's TypeScript.
    #
    # `nektos/act` — running ci.yml itself locally — was considered for this and
    # rejected. Every recipe reaches its tool through `docker compose run`, so a
    # workflow replayed inside act's own container needs a Docker daemon in
    # there: docker-in-docker, which is the arrangement ADR-0002 exists to
    # avoid. The manifest assert below buys the property act would have bought
    # (local run and CI run the same set) without the nesting.

    pipeline_start=$(date +%s)
    rc=0
    bg_dir=$(mktemp -d)

    banner() { printf '\n\033[1;36m[%s] →→→ %s\033[0m\n' "$(date +%T)" "$1"; }
    passln() { printf '\033[1;32m[%s] ✓ %s (%ds)\033[0m\n'     "$(date +%T)" "$1" "$2"; }
    failln() { printf '\n\033[1;31m[%s] ✗ %s FAILED (%ds, exit %d)\033[0m\n' \
                   "$(date +%T)" "$1" "$2" "$3"; }

    # --- background lane: slow gates that take no cargo build lock ----------
    # These overlap the compile lane. Output is buffered to a log and only
    # replayed on failure so the terminal stays readable.
    bg_steps=(deny shear audit)

    # --- foreground lane: instant text gates first (fail-fast in seconds),
    # --- then the compile pipeline (sequential — shared target dir). ---------
    fg_steps=(typos fmt-check strict-code verify-version-pins \
              zizmor actionlint vale comment-discipline commitlint \
              msrv clippy build dist-assets-check \
              test test-doc prop spec doc doc-public semver coverage udeps \
              fuzz-build package test-wasm \
              playground-lint playground-test playground-build)

    # --- manifest assert: these two lanes ARE the gate set -------------------
    # "`just ci` is a superset of CI" used to be a sentence, and a sentence is
    # a claim nothing evaluates — it was false for months (msrv and commitlint
    # ran only in CI, prop only here). This is the same claim as an assertion.
    manifest=$(just gates)
    declared=$(printf '%s\n' "${bg_steps[@]}" "${fg_steps[@]}" | sort)
    if [[ "$manifest" != "$declared" ]]; then
        printf '\033[1;31m%s\033[0m\n' "just ci: lanes disagree with [group('gate')]" >&2
        printf '  tagged gate, not run here: %s\n' \
            "$(comm -23 <(printf '%s\n' "$manifest") <(printf '%s\n' "$declared") | tr '\n' ' ')" >&2
        printf '  run here, not tagged gate: %s\n' \
            "$(comm -13 <(printf '%s\n' "$manifest") <(printf '%s\n' "$declared") | tr '\n' ' ')" >&2
        printf '  Fix the lane list above, or the attribute on the recipe.\n' >&2
        rm -rf "$bg_dir"
        exit 1
    fi

    declare -A bg_pid
    for step in "${bg_steps[@]}"; do
        # Each job records its own (exit-code, duration) so the reap below can
        # report the gate's real time, not the whole pipeline's elapsed window.
        ( s=$(date +%s)
          just "$step" >"$bg_dir/$step.log" 2>&1
          printf '%d %d' "$?" "$(( $(date +%s) - s ))" >"$bg_dir/$step.meta" ) &
        bg_pid[$step]=$!
    done
    printf '\033[1;36m[%s] ⟳ background (concurrent): %s\033[0m\n' \
        "$(date +%T)" "${bg_steps[*]}"

    halted=""
    for step in "${fg_steps[@]}"; do
        start=$(date +%s)
        banner "$step"
        if just "$step"; then
            passln "$step" $(( $(date +%s) - start ))
        else
            grc=$?
            failln "$step" $(( $(date +%s) - start )) "$grc"
            rc=$grc
            halted="$step"
            break
        fi
    done

    # --- reap background lane (wait so no container is orphaned on failure) --
    banner "background gates (${bg_steps[*]})"
    for step in "${bg_steps[@]}"; do
        wait "${bg_pid[$step]}"
        read -r brc bdur < "$bg_dir/$step.meta"
        if [[ "$brc" -eq 0 ]]; then
            passln "$step" "$bdur"
        else
            failln "$step" "$bdur" "$brc"
            echo "----- $step output -----"
            cat "$bg_dir/$step.log"
            rc="$brc"
        fi
    done
    rm -rf "$bg_dir"

    # --- summary ------------------------------------------------------------
    total=$(( ${#bg_steps[@]} + ${#fg_steps[@]} ))
    elapsed=$(( $(date +%s) - pipeline_start ))
    if [[ $rc -eq 0 ]]; then
        printf '\n\033[1;32m[%s] ✓✓✓ all %d gates passed (total %ds)\033[0m\n' \
            "$(date +%T)" "$total" "$elapsed"
    else
        [[ -n "$halted" ]] && \
            printf '\033[1;31mcompile lane halted at: %s\033[0m\n' "$halted"
        printf '\033[1;31m[%s] ✗ CI FAILED (total %ds) — see ✗ lines above\033[0m\n' \
            "$(date +%T)" "$elapsed"
        exit "$rc"
    fi

# --- developer workflow helpers ----------------------------------------------

# Builds the dev image, installs git hooks, checks the env, runs the tests.
# Idempotent, safe to re-run after a pull — the one command to run after cloning.
[group('dev')]
setup:
    docker compose build dev
    just hooks
    just doctor
    just test

# One-screen snapshot of the local environment: images, volumes and playground
# artefacts. Exit 1 = a missing prerequisite a build would trip on.
[group('dev')]
doctor:
    #!/usr/bin/env bash
    set -uo pipefail
    OK="\033[1;32m[OK]\033[0m"
    WARN="\033[1;33m[--]\033[0m"
    ERR="\033[1;31m[!!]\033[0m"

    fail=0

    # --- Docker availability ---------------------------------------------
    if command -v docker >/dev/null 2>&1; then
        printf '%b docker: %s\n' "$OK" "$(docker --version | awk '{print $3}' | tr -d ,)"
    else
        printf '%b docker: NOT INSTALLED\n' "$ERR"
        fail=1
    fi
    if docker compose version >/dev/null 2>&1; then
        printf '%b docker compose: %s\n' "$OK" "$(docker compose version --short)"
    else
        printf '%b docker compose: missing (install Compose v2)\n' "$ERR"
        fail=1
    fi

    # --- Images ----------------------------------------------------------
    # docker images Go-template strings collide with just's `}}`
    # interpolator; parse the human-readable table with awk instead.
    # Output columns: REPOSITORY TAG IMAGE-ID CREATED SIZE. NR==2 picks
    # the first data row; awk's last field is the size.
    for tag in aozora-md-dev:local aozora-md-fuzz:local aozora-md-ci:local; do
        size=$(docker images "$tag" 2>/dev/null | awk 'NR==2 {print $NF}')
        if [ -n "$size" ]; then
            printf '%b image %s (%s)\n' "$OK" "$tag" "$size"
        else
            case "$tag" in
                aozora-md-dev:local)   hint='just check        # auto-builds dev' ;;
                aozora-md-fuzz:local)  hint='docker compose build fuzz' ;;
                aozora-md-ci:local)    hint='docker compose build ci  # superset' ;;
            esac
            printf '%b image %s missing  →  %s\n' "$WARN" "$tag" "$hint"
        fi
    done

    # --- Volumes ---------------------------------------------------------
    for vol in aozora-md_cargo-registry aozora-md_cargo-git aozora-md_cargo-target aozora-md_sccache; do
        if docker volume inspect "$vol" >/dev/null 2>&1; then
            printf '%b volume %s\n' "$OK" "$vol"
        else
            printf '%b volume %s missing (created on first compose run)\n' "$WARN" "$vol"
        fi
    done

    # No aozora-pin check here. This recipe asks what a laptop is missing;
    # whether the pins agree is `just verify-version-pins`, a gate, which asks
    # it of the registry versions the manifests actually carry. The copy that
    # used to live here still grepped for a `rev = "<40 hex>"` git pin ADR-0015
    # retired, so it took the `else` branch on every tree and `just setup`
    # aborted before it reached the tests.

    # --- Playground prerequisites ----------------------------------------
    if [ -f crates/aozora-flavored-markdown-wasm/pkg/aozora_flavored_markdown_wasm_bg.wasm ]; then
        pkg_size=$(du -h crates/aozora-flavored-markdown-wasm/pkg/aozora_flavored_markdown_wasm_bg.wasm | awk '{print $1}')
        printf '%b crates/aozora-flavored-markdown-wasm/pkg (%s)\n' "$OK" "$pkg_size"
    else
        printf '%b crates/aozora-flavored-markdown-wasm/pkg missing  →  just wasm-build  (or just wasm-build-dev for fast iter)\n' "$WARN"
    fi
    if [ -d playground/node_modules ]; then
        printf '%b playground/node_modules\n' "$OK"
    else
        printf '%b playground/node_modules missing  →  just playground-install\n' "$WARN"
    fi

    # --- Summary ---------------------------------------------------------
    echo
    if [ "$fail" -eq 0 ]; then
        printf '\033[1;32mall blocking prerequisites satisfied\033[0m\n'
        exit 0
    else
        printf '\033[1;31m%d blocking issue(s) found — fix before continuing\033[0m\n' "$fail"
        exit 1
    fi

# Run after a build to verify the cache is actually warm; a first-hand
# way to notice when `RUSTC_WRAPPER` gets defeated by stray env or profile tweaks.
# Show sccache hit/miss ratio, cache size, fetch counts.
[group('dev')]
sccache-stats:
    {{_dev}} sccache --show-stats

# Useful before a measurement window:
#   just sccache-zero && just clean && just build && just sccache-stats
# Reset sccache counters to zero.
[group('dev')]
sccache-zero:
    {{_dev}} sccache --zero-stats

# Defaults to the `check` job; pass a job name to pick another, e.g.
# `just watch clippy`. Keybindings: `t` test / `c` clippy / `d` doc /
# `f` failing-only / `esc` previous job / `q` quit / Ctrl-J list jobs.
# Start the bacon file-watcher inside the dev container.
[group('dev')]
watch JOB="":
    {{_dev}} bacon {{JOB}}

# Keeps the watch loop but prints plain lines. Useful for piping output
# (`| tee`) and for sessions without a TTY.
# Headless bacon run (no TUI).
[group('dev')]
watch-headless JOB="check":
    {{_ci}} bacon --headless --job {{JOB}}

# Idempotent — re-run safely after lefthook.yml edits or to repair stubs.
# Install git hooks (pre-commit / commit-msg / pre-push).
[group('dev')]
hooks:
    {{_dev}} lefthook install

# Remove lefthook git hook stubs.
[group('dev')]
hooks-uninstall:
    {{_dev}} lefthook uninstall

# --- cleanup ------------------------------------------------------------------

# Remove build artifacts (keeps volumes; use `docker compose down -v` for volumes)
[group('dev')]
clean:
    {{_dev}} cargo clean --locked --workspace

# Tear down all compose state (destroys cached registry/target/sccache volumes)
[confirm("Destroy cached cargo registry/target/sccache volumes? Next build is cold. [y/N]")]
[group('dev')]
nuke:
    docker compose down -v --remove-orphans
