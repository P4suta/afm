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
#   cargo fuzz — the fuzz crate is its own workspace and its Cargo.lock is
#     git-ignored on purpose (regenerated per build), so there is nothing to
#     assert against.
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
    {{_dev}} bash -c 'RUSTDOCFLAGS="{{_DOC_DENY}}" cargo doc --locked --workspace --all-features --no-deps --document-private-items'

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
    {{_dev}} bash -c 'RUSTDOCFLAGS="{{_DOC_DENY}}" cargo doc --locked --workspace --all-features --no-deps'

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

# CommonMark 0.31.2 (652 cases, pass = 652/652) + GFM extension compliance.
# A `#[cfg(test)] mod` of the library, not an integration test: the spec's
# expected output needs raw-HTML passthrough, which has no public switch.
[group('gate')]
[group('test')]
spec:
    {{_dev}} cargo nextest run --locked --package aozora-flavored-markdown --lib -E 'test(conformance::)'

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

# Run the named fuzz target with arbitrary args (escape hatch for advanced use).
[group('fuzz')]
fuzz *ARGS:
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && cargo +nightly fuzz run {{ARGS}}'

# 60-second smoke fuzz. `timeout` is a hard backstop if libFuzzer ever hangs.
[group('fuzz')]
fuzz-quick TARGET:
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && timeout --kill-after=10s 90s cargo +nightly fuzz run {{TARGET}} -- -max_total_time=60'

# 5-minute deep fuzz — the gate to clear before tagging a release.
[group('fuzz')]
fuzz-deep TARGET:
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && timeout --kill-after=10s 360s cargo +nightly fuzz run {{TARGET}} -- -max_total_time=300'

# 15-minute marathon fuzz — strongest single-target soak; exits cleanly at 15 min.
[group('fuzz')]
fuzz-marathon TARGET:
    {{_fuzz}} bash -c 'cd crates/aozora-flavored-markdown && timeout --kill-after=10s 1000s cargo +nightly fuzz run {{TARGET}} -- -max_total_time=900'

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
        out=$({{_fuzz}} bash -c "cd crates/aozora-flavored-markdown && cargo +nightly fuzz run ${target} ${rel} 2>&1" || true)
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
# or `crates/aozora-flavored-markdown-test-support/src/`.
[group('fuzz')]
fuzz-all-quick:
    just fuzz-quick parse_render
    just fuzz-quick render_blocks
    just fuzz-quick serialize_round_trip
    just fuzz-quick sjis_decode

# Run every registered fuzz target in turn for 5 min each. Release
# pre-flight pass: a clean run is the gate before tagging a release.
[group('fuzz')]
fuzz-all-deep:
    just fuzz-deep parse_render
    just fuzz-deep render_blocks
    just fuzz-deep serialize_round_trip
    just fuzz-deep sjis_decode

# At-a-glance health check: how many crash artifacts are pending
# triage, how many regression cases are pinned per target. Nothing
# here invokes nightly, so it stays cheap and shell-friendly.
[group('fuzz')]
fuzz-status:
    #!/usr/bin/env bash
    set -euo pipefail
    targets=(parse_render render_blocks serialize_round_trip sjis_decode)
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
# Regions, not branches: `cargo-llvm-cov` 0.8.5 has `--fail-under-regions` but
# no `--fail-under-branches` (branch counts need nightly); regions are finer
# than branches, so a region threshold implies the branch one on stable.
#
# Excludes (`_COV_IGNORE`): build artefacts, CLI
# `main.rs` entrypoints, xtask tooling, test-support, and aozora-flavored-markdown-wasm (exercised
# by `wasm-pack test`, which native llvm-cov can't reach). Also the EPUB
# generator's XML/ZIP serialisation files (compose.rs / package.rs): they
# write to an in-memory `Cursor<Vec<u8>>` sink whose `io::Write` is infallible,
# so the per-call `.map_err(…)` error arms are dead defensive regions that can't
# be reached by a test — their OPF/NAV/ZIP output is covered behaviourally by
# the snapshot + build_epub integration tests instead (ADR-0018).
_COV_FLOOR := "97"
_COV_IGNORE := "(target/|/main\\.rs$|xtask/|aozora-flavored-markdown-test-support/|aozora-flavored-markdown-wasm/|aozora-flavored-markdown-epub/src/(compose|package)\\.rs)"

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

# Run all lints (fmt + clippy + typos + strict-code + comment-discipline
# + zizmor + actionlint)
[group('lint')]
lint: fmt-check clippy typos strict-code comment-discipline zizmor actionlint

# Comment drift gate. Fails when a Rust comment (`//` / `///` / `//!`) or a
# TOML manifest comment (`#`) names a path that used to exist inside the
# sibling parser but is no longer on its public API (ADR-0021). The compiler
# catches the same drift in *code*; comments rot silently, so they get their
# own gate. The banned list lives in `crates/xtask/src/main.rs`
# (RETIRED_UPSTREAM_PATHS).
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
    # property test instead of pushed to runtime. Mirrors aozora-pipeline's
    # baseline tripwire; bump the baseline only when you remove an expect.
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
# Self-bootstraps cargo-shear when absent: CI's `setup-dev-image` pulls the
# published `aozora-md-dev:latest`, which lags a Dockerfile tool addition by
# one merge, so the gate binstalls the tool into the pulled image until the
# image republishes. A local dev image already ships it, so the bootstrap is
# a no-op there.
[group('gate')]
[group('lint')]
shear:
    {{_dev}} bash -c 'command -v cargo-shear >/dev/null 2>&1 \
        || cargo binstall --no-confirm --locked --root /usr/local cargo-shear; \
        cargo shear --locked'

# comrak resolves from the registry like every other dependency (ADR-0024),
# so the lockfile graph is the whole graph — no shim needed to reach it.
# RustSec advisory scan over `Cargo.lock`.
[group('gate')]
[group('lint')]
audit:
    {{_dev}} cargo audit

# Unused dependency scan (requires nightly)
[group('gate')]
[group('lint')]
udeps:
    {{_fuzz}} cargo +nightly udeps --locked --workspace --all-targets

# Semver break detection (runs against published baseline once crates are on crates.io)
[group('lint')]
semver:
    {{_dev}} cargo semver-checks check-release --workspace

# --- upstream sources ---------------------------------------------------------

# Pin every `aozora-*` git dep in Cargo.toml to a new commit SHA in one
# pass, then refresh Cargo.lock. Idempotent (no-op when the SHA already
# matches). Use the full 40-char hex SHA from `git ls-remote
# https://github.com/P4suta/aozora.git refs/heads/main`.
[group('upstream')]
aozora-bump SHA:
    {{_dev}} cargo run --locked --package xtask --quiet -- aozora-bump {{SHA}}

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

# --- release assets ----------------------------------------------------------

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

# --- playground (browser try-it-online) --------------------------------------

# Vite dev/preview server container — `--service-ports` is required so
# `docker compose run` actually publishes 5173 (it doesn't by default).
_pg := "docker compose run --rm --service-ports playground"

# Same container without publishing 5173. Used by `playground-install`
# and `playground-build` so they share the `playground-node-modules`
# named volume but don't trip "address already in use" when an existing
# Vite or dev server is bound to 5173 on the host.
_pg_install := "docker compose run --rm playground"

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
# is the resolution of record and no recipe may rewrite it. KNOWN DRIFT
# SOURCE: the `aozora-flavored-markdown-wasm` entry is a `file:` link into
# `crates/aozora-flavored-markdown-wasm/pkg`, whose package.json carries the
# workspace version — so a version bump invalidates bun.lock and this recipe
# fails until it is regenerated. That regeneration belongs to the release
# bump, not here; until the cargo-release hook lands, run a bare
# `bun install` once by hand and commit the lockfile with the bump.
[group('playground')]
playground-install: wasm-build
    {{_pg_install}} bash -c 'bun install --frozen-lockfile'

# Vite dev server with HMR at http://localhost:5173/
[group('playground')]
playground-dev: playground-install
    {{_pg}} bash -c 'bun run dev -- --host 0.0.0.0'

# Same as `playground-dev` but uses the fast dev-profile wasm build for
# inner-loop iteration (TS edits get HMR; wasm changes still need a
# reload after `just wasm-build-dev`).
[group('playground')]
playground-dev-fast: wasm-build-dev
    {{_pg_install}} bash -c 'bun install --frozen-lockfile' && \
    {{_pg}} bash -c 'bun run dev -- --host 0.0.0.0'

# Production build → playground/dist/ (consumed by .github/workflows/docs.yml)
# Also runs inside `playground` service to share the `node_modules` volume.
[group('gate')]
[group('playground')]
playground-build: playground-install
    {{_pg_install}} bash -c 'bun run build'

# Preview the production build locally at http://localhost:5173/
[group('playground')]
playground-serve: playground-build
    {{_pg}} bash -c 'bun run preview -- --host 0.0.0.0 --port 5173'

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

    # Why this shape (no gate is weakened vs. the old sequential loop):
    #   * The compile gates (msrv/clippy/build/test/prop/spec/doc/doc-public/
    #     coverage/udeps)
    #     all share ONE cargo target dir, so they contend on its build lock and
    #     CANNOT truly run in parallel — they stay sequential, ordered
    #     cheap-to-expensive so a failure surfaces fast. `msrv` leads: inside the
    #     dev image it is a bare `cargo check`, so it is also the cheapest
    #     possible "does it still compile".
    #   * deny / shear / audit invoke NO rustc and take no build lock (and
    #     spawn no sccache server, so no multi-server churn on the shared cache),
    #     so a BACKGROUND lane overlaps them onto the compile lane for free.
    #   * `check` is not a gate and is not run here: clippy + build both compile
    #     --all-targets, so a bare `cargo check` pass adds no coverage. ci.yml
    #     still runs it, as the fast precondition the gate matrix waits on —
    #     scheduling, not a gate. The gates `lint` bundles
    #     (fmt-check/typos/strict-code/comment-discipline/zizmor/actionlint) run
    #     once on their own instead of a second time inside `lint`; only
    #     `clippy` is left to run from `lint`.
    #   * playground-build (wasm-pack + the in-repo playground's tsc/vite) runs
    #     LAST in the foreground lane: wasm-pack invokes rustc and shares the
    #     target dir, and it pulls `wasm-build` in as a dependency — so a wasm /
    #     IR / diagnostic type change can no longer pass `just ci` while
    #     silently breaking the playground's TypeScript.
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
    # deny / shear / audit overlap the compile lane. Output is buffered to a
    # log and only replayed on failure so the terminal stays readable.
    # (shear is syn-based, so it takes no cargo build lock either.)
    bg_steps=(deny shear audit)

    # --- foreground lane: instant text gates first (fail-fast in seconds),
    # --- then the compile pipeline (sequential — shared target dir). ---------
    fg_steps=(typos fmt-check strict-code verify-version-pins \
              zizmor actionlint comment-discipline commitlint \
              msrv clippy build dist-assets-check \
              test test-doc prop spec doc doc-public coverage udeps \
              playground-build)

    # --- manifest assert: these two lanes ARE the gate set -------------------
    # "`just ci` is a superset of CI" used to be a sentence, and a sentence is
    # a claim nothing evaluates — it was false for months (msrv and commitlint
    # ran only in CI, prop only here). `[group('gate')]` is now the single
    # declaration; `just gates` reads it, ci.yml builds its matrix from the same
    # command, and the lanes above have to equal it or nothing runs. A gate
    # added to the Justfile and forgotten here fails in the first second.
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
    banner "background gates (deny / shear / audit)"
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

# One-screen snapshot of the local environment: images, volumes, the aozora
# SHA pin ↔ Cargo.lock, and playground artefacts. Exit 1 = a missing
# prerequisite a build would trip on.
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

    # --- aozora SHA pin ↔ Cargo.lock --------------------------------------
    pinned=$(grep -oE 'rev = "[0-9a-f]{40}"' Cargo.toml | head -1 | grep -oE '[0-9a-f]{40}' || true)
    if [ -n "$pinned" ]; then
        if grep -q "rev = \"$pinned\"" Cargo.lock 2>/dev/null \
            || grep -q "#${pinned:0:7}" Cargo.lock 2>/dev/null; then
            printf '%b aozora rev pin: %s (Cargo.lock agrees)\n' "$OK" "${pinned:0:12}…"
        else
            printf '%b aozora rev pin %s NOT reflected in Cargo.lock  →  cargo update -p aozora\n' \
                "$ERR" "${pinned:0:12}…"
            fail=1
        fi
    else
        printf '%b aozora rev pin: not found in Cargo.toml\n' "$ERR"
        fail=1
    fi

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
