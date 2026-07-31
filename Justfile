# Native development and CI entry points.
#
# Install the pinned toolchain with `mise install --locked`, then run the same
# `ci-*` recipes locally that GitHub Actions invokes.

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

_DOC_FLAGS := "-D warnings"
_COV_FLOOR := "95"
_COV_IGNORE := "(target/|xtask/|aozora-flavored-markdown-test-support/|aozora-flavored-markdown-wasm/)"
_FUZZ_TOOLCHAIN := "nightly-2026-07-15"
_FUZZ_TARGET := "x86_64-unknown-linux-gnu"

[group('meta')]
default:
    @just --list

# --- Rust --------------------------------------------------------------------

[group('rust')]
check:
    cargo check --locked --workspace --all-targets --all-features

[group('rust')]
check-features:
    cargo check --locked --package aozora-flavored-markdown --lib --no-default-features
    cargo check --locked --package aozora-flavored-markdown --lib --no-default-features --features miette
    cargo check --locked --package aozora-flavored-markdown --lib --no-default-features --features serde
    cargo check --locked --package aozora-flavored-markdown --lib --no-default-features --features theme
    cargo check --locked --package aozora-flavored-markdown --lib --no-default-features --features tsify
    cargo check --locked --package aozora-flavored-markdown-wasm --no-default-features
    cargo check --locked --package aozora-flavored-markdown-wasm --all-features

[group('rust')]
build:
    cargo build --locked --workspace --all-targets --all-features

[group('rust')]
build-release:
    cargo build --locked --release --workspace

[group('rust')]
fmt-check:
    cargo fmt --all -- --check

[group('rust')]
fmt:
    cargo fmt --all

[group('rust')]
clippy:
    cargo clippy --locked --workspace --all-features \
        --lib --bins --examples -- -D warnings
    # Test and benchmark harnesses use panic/output as assertion diagnostics.
    # Production targets above keep the workspace restrictions at `-D warnings`.
    cargo clippy --locked --workspace --all-features --tests --benches -- \
        -D warnings \
        -A clippy::unwrap_used \
        -A clippy::expect_used \
        -A clippy::panic \
        -A clippy::print_stdout \
        -A clippy::print_stderr

[group('rust')]
test *ARGS:
    cargo nextest run --locked --workspace --all-targets --all-features {{ARGS}}

[group('rust')]
test-doc:
    cargo test --locked --workspace --all-features --doc

[group('rust')]
coverage:
    cargo llvm-cov nextest \
        --locked \
        --workspace \
        --all-targets \
        --all-features \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

[group('rust')]
coverage-html:
    cargo llvm-cov nextest \
        --locked \
        --workspace \
        --all-targets \
        --all-features \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --html --output-dir coverage/html

[group('rust')]
doc:
    RUSTDOCFLAGS="{{_DOC_FLAGS}}" cargo doc \
        --locked --workspace --all-features --no-deps --document-private-items

[group('rust')]
doc-public:
    RUSTDOCFLAGS="{{_DOC_FLAGS}}" cargo doc \
        --locked --workspace --all-features --no-deps

[group('rust')]
run *ARGS:
    cargo run --locked --package aozora-flavored-markdown-cli --quiet -- {{ARGS}}

# --- Web / WASM --------------------------------------------------------------

[group('web')]
test-wasm:
    RUSTC_WRAPPER= wasm-pack test --node \
        crates/aozora-flavored-markdown-wasm --locked

[group('web')]
wasm-build:
    RUSTC_WRAPPER= CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build crates/aozora-flavored-markdown-wasm \
        --target bundler --release \
        --out-dir pkg --out-name aozora_flavored_markdown_wasm -- --locked
    sh -c 'set -eu; wasm=crates/aozora-flavored-markdown-wasm/pkg/aozora_flavored_markdown_wasm_bg.wasm; \
        before=$(wc -c < "$wasm"); \
        wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int "$wasm" -o "$wasm"; \
        after=$(wc -c < "$wasm"); \
        test "$after" -lt "$before"'

[group('web')]
wasm-build-dev:
    RUSTC_WRAPPER= wasm-pack build crates/aozora-flavored-markdown-wasm \
        --target bundler --dev \
        --out-dir pkg --out-name aozora_flavored_markdown_wasm -- --locked

[group('web')]
playground-install: wasm-build
    cd playground && bun install --frozen-lockfile

[group('web')]
playground-lint: playground-install
    cd playground && bun run lint
    cd playground && bun run lint:css
    cd playground && bun run check:legacy
    cd playground && bun run vendor:verify

[group('web')]
playground-lint-fix: playground-install
    cd playground && bun run lint:fix
    cd playground && bun run lint:css:fix

[group('web')]
playground-typecheck: playground-install
    cd playground && bun run typecheck

[group('web')]
playground-test: playground-install
    cd playground && bun run test:coverage

[group('web')]
playground-build: playground-install
    cd playground && bun run build
    cd playground && bun run check:bundle

[group('web')]
playground-e2e: playground-build
    cd playground && bun x --no-install playwright install chromium firefox webkit
    cd playground && bun x --no-install playwright test

[group('web')]
playground-lighthouse: playground-build
    cd playground && bun x --no-install playwright install chromium
    cd playground && bun run lighthouse

[group('web')]
playground-dev: playground-install
    cd playground && bun run dev

[group('web')]
playground-serve: playground-build
    cd playground && bun run preview

# --- Repository checks -------------------------------------------------------

[group('repo')]
typos:
    typos

[group('repo')]
deny:
    cargo deny --locked check
    cargo deny --locked \
        --manifest-path crates/aozora-flavored-markdown/fuzz/Cargo.toml check

[group('repo')]
actionlint:
    actionlint -no-color -shellcheck=shellcheck -pyflakes=

[group('repo')]
zizmor:
    zizmor --offline --no-progress --no-ignores .

# --- Fuzzing -----------------------------------------------------------------

[group('fuzz')]
fuzz-build:
    cd crates/aozora-flavored-markdown && \
        cargo +{{_FUZZ_TOOLCHAIN}} fuzz build --target {{_FUZZ_TARGET}}

[group('fuzz')]
fuzz-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    corpus="crates/aozora-flavored-markdown/fuzz/corpus"
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    mkdir -p "$work/seeds"
    cp playground/examples/*.md "$work/seeds/"

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

    # CommonMark has LF, CRLF and lone CR line endings, but the tracked
    # source fixtures are LF-normalised. Keep one generated seed per
    # CR-sensitive structure so libFuzzer does not have to invent the byte
    # before it can exercise fence fidelity.
    printf 'a\rb\n\n```\nc\n\n\nd\n```\n' \
        >"$work/seeds/eol-cr-in-prose-shifts-a-later-fence"
    printf '```\r｜青梅《おうめ》\r［＃改ページ］\r```\r' \
        >"$work/seeds/eol-cr-is-the-only-ending"
    printf '```\r\n｜青梅《おうめ》\r｜漢字《かんじ》\r\n```\n' \
        >"$work/seeds/eol-all-three-in-one-fence"
    printf '> ```\r> ｜青梅《おうめ》\r> ```\r\n\n    ｜青梅《おうめ》\r    ｜漢字《かんじ》\n' \
        >"$work/seeds/eol-cr-behind-a-container-and-an-indent"
    cargo run --quiet -p xtask -- cp932-project \
        --input-dir "$work/seeds" --output-dir "$work/cp932"

    cd crates/aozora-flavored-markdown
    while IFS= read -r target; do
        dir="../../$corpus/$target"
        mkdir -p "$dir"
        rm -f "$dir"/seed-*
        for source in "$work"/seeds/*; do
            seed=$source
            if [[ "$target" == sjis_decode ]]; then
                seed="$work/cp932/${source##*/}"
                [[ -f "$seed" ]] || continue
            elif [[ "$target" == options_space ]]; then
                (head -c 2 /dev/zero; cat "$source") >"$work/masked"
                seed="$work/masked"
            fi
            cp "$seed" "$dir/seed-$(sha1sum <"$seed" | cut -c1-16)"
        done
        printf 'fuzz-seed: %-24s %4d seed(s)\n' "$target" \
            "$(find "$dir" -maxdepth 1 -name 'seed-*' | wc -l)"
    done < <(cargo +{{_FUZZ_TOOLCHAIN}} fuzz list)

[group('fuzz')]
fuzz-quick TARGET:
    cd crates/aozora-flavored-markdown && \
        timeout --signal=TERM --kill-after=10s 90s \
        cargo +{{_FUZZ_TOOLCHAIN}} fuzz run {{TARGET}} \
            --target {{_FUZZ_TARGET}} -- -max_total_time=60

[group('fuzz')]
fuzz-deep TARGET:
    cd crates/aozora-flavored-markdown && \
        timeout --signal=TERM --kill-after=10s 330s \
        cargo +{{_FUZZ_TOOLCHAIN}} fuzz run {{TARGET}} \
            --target {{_FUZZ_TARGET}} -- -max_total_time=300

# Replay every artifact libFuzzer left for TARGET. One line per artifact: the
# `Tier X violated` panic line when it still crashes, otherwise the tail of a
# clean run. Exit status is the number of crashing artifacts, so a CI gate can
# read it directly.
[group('fuzz')]
fuzz-triage TARGET:
    #!/usr/bin/env bash
    set -uo pipefail
    dir="crates/aozora-flavored-markdown/fuzz/artifacts/{{TARGET}}"
    bin="crates/aozora-flavored-markdown/fuzz/target/{{_FUZZ_TARGET}}/release/{{TARGET}}"
    if [[ ! -x "$bin" ]]; then
        echo "fuzz-triage: build the targets first: just fuzz-build" >&2
        exit 2
    fi
    crashing=0
    shopt -s nullglob
    for artifact in "$dir"/*; do
        log=$(mktemp)
        if ASAN_OPTIONS=detect_leaks=0 "$bin" "$artifact" -runs=1 >"$log" 2>&1; then
            printf '%s: clean\n' "${artifact##*/}"
            tail -n 5 "$log" | sed 's/^/    /'
        else
            crashing=$((crashing + 1))
            printf '%s: CRASH\n' "${artifact##*/}"
            grep -m1 -o "panicked at .*\|Tier [A-Z] ([^)]*) violated" "$log" \
                | sed 's/^/    /' || true
        fi
        rm -f "$log"
    done
    exit "$crashing"

# Move an artifact into the permanent regression set, where `just test` replays
# it without a nightly toolchain. The name is the artifact's own file name.
[group('fuzz')]
fuzz-promote TARGET ARTIFACT:
    #!/usr/bin/env bash
    set -euo pipefail
    from="crates/aozora-flavored-markdown/fuzz/artifacts/{{TARGET}}/{{ARTIFACT}}"
    into="crates/aozora-flavored-markdown/tests/fuzz_regressions/{{TARGET}}"
    [[ -f "$from" ]] || { echo "fuzz-promote: no such artifact: $from" >&2; exit 2; }
    [[ -d "$into" ]] || { echo "fuzz-promote: no such target dir: $into" >&2; exit 2; }
    mv "$from" "$into/"
    printf 'fuzz-promote: %s -> %s/\n' "{{ARTIFACT}}" "$into"

# What is waiting to be triaged, and what is already pinned.
[group('fuzz')]
fuzz-status:
    #!/usr/bin/env bash
    set -euo pipefail
    artifacts="crates/aozora-flavored-markdown/fuzz/artifacts"
    pinned="crates/aozora-flavored-markdown/tests/fuzz_regressions"
    printf '%-24s %-16s %s\n' target pending_crashes pinned_regressions
    printf '%.0s-' {1..60}; printf '\n'
    cd crates/aozora-flavored-markdown
    while IFS= read -r target; do
        cd ../..
        printf '%-24s %-16s %s\n' "$target" \
            "$(find "$artifacts/$target" -maxdepth 1 -type f 2>/dev/null | wc -l)" \
            "$(find "$pinned/$target" -maxdepth 1 -type f -not -name '.gitkeep' \
                -not -name '*.expect.txt' 2>/dev/null | wc -l)"
        cd crates/aozora-flavored-markdown
    done < <(cargo +{{_FUZZ_TOOLCHAIN}} fuzz list)

[group('fuzz')]
fuzz-all-quick:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates/aozora-flavored-markdown
    cargo +{{_FUZZ_TOOLCHAIN}} fuzz build --target {{_FUZZ_TARGET}}
    while IFS= read -r target; do
        timeout --signal=TERM --kill-after=10s 90s \
            cargo +{{_FUZZ_TOOLCHAIN}} fuzz run "$target" \
                --target {{_FUZZ_TARGET}} -- -max_total_time=60
    done < <(cargo fuzz list)

[group('fuzz')]
fuzz-all-deep:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates/aozora-flavored-markdown
    cargo +{{_FUZZ_TOOLCHAIN}} fuzz build --target {{_FUZZ_TARGET}}
    while IFS= read -r target; do
        timeout --signal=TERM --kill-after=10s 330s \
            cargo +{{_FUZZ_TOOLCHAIN}} fuzz run "$target" \
                --target {{_FUZZ_TARGET}} -- -max_total_time=300
    done < <(cargo fuzz list)

# --- Generated sources and release ------------------------------------------

[group('maintenance')]
aozora-bump VERSION:
    cargo run --locked --package xtask --quiet -- aozora-bump {{VERSION}}

[group('maintenance')]
spec-refresh:
    cargo run --locked --package xtask --quiet -- spec-refresh \
        --input spec/sources/commonmark-0.31.2.txt \
        --output crates/aozora-flavored-markdown/spec/commonmark-0.31.2.json
    cargo run --locked --package xtask --quiet -- spec-refresh \
        --input spec/sources/gfm-0.29-gfm.txt \
        --output crates/aozora-flavored-markdown/spec/gfm-0.29-gfm.json

[group('maintenance')]
adr TITLE:
    cargo run --locked --package xtask --quiet -- new-adr {{TITLE}}

[group('maintenance')]
changelog:
    git-cliff --unreleased

[group('release')]
release LEVEL *ARGS:
    cargo release version {{LEVEL}} --workspace --no-confirm {{ARGS}}
    cargo release replace --workspace --no-confirm {{ARGS}}
    cargo release hook --workspace --no-confirm {{ARGS}}

[group('release')]
dist-assets:
    cargo build --locked --package aozora-flavored-markdown-cli --quiet
    cargo run --locked --package xtask --quiet -- gen-dist-assets

[group('release')]
dist-assets-check:
    cargo build --locked --package aozora-flavored-markdown-cli --quiet
    cargo run --locked --package xtask --quiet -- gen-dist-assets --check

[group('release')]
dist-plan:
    scripts/dist-plan-check.sh

[group('release')]
changelog-check:
    #!/usr/bin/env bash
    set -euo pipefail
    id=$(cargo pkgid --locked --package aozora-flavored-markdown)
    version=${id##*[#@]}
    grep -qF "## [${version}]" CHANGELOG.md || {
        printf 'CHANGELOG.md has no "## [%s]" section\n' "$version" >&2
        exit 1
    }

[group('release')]
semver:
    cargo semver-checks check-release \
        --package aozora-flavored-markdown \
        --baseline-rev v0.4.1

[group('release')]
package-smoke SKIP_CRATES="":
    scripts/package-smoke.sh {{quote(SKIP_CRATES)}}

[group('release')]
release-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    git clone --quiet --local --no-hardlinks . "$tmp/repo"
    cd "$tmp/repo"
    git config user.name release-smoke
    git config user.email release-smoke@example.invalid
    cargo release version patch --workspace --no-confirm --execute
    cargo release replace --workspace --no-confirm --execute
    cargo release hook --workspace --no-confirm --execute
    cargo metadata --locked --no-deps --format-version 1 >/dev/null
    just changelog-check
    just dist-assets-check
    git add --all
    git commit --quiet -m "chore(release): release smoke"
    just package-smoke

# --- Documentation site ------------------------------------------------------

[group('docs')]
docs-site: doc playground-build
    #!/usr/bin/env bash
    set -euo pipefail
    site=target/site
    mkdir -p "$site"
    find "$site" -mindepth 1 -delete
    mkdir -p "$site/api" "$site/playground"
    cp -R target/doc/. "$site/api/"
    cp -R playground/dist/. "$site/playground/"
    printf '%s\n' \
        '<!doctype html><meta http-equiv="refresh" content="0; url=aozora_flavored_markdown/">' \
        > "$site/api/index.html"
    printf '%s\n' \
        '<!doctype html><meta http-equiv="refresh" content="0; url=api/">' \
        > "$site/index.html"
    touch "$site/.nojekyll"
    test -f "$site/index.html"
    test -f "$site/api/index.html"
    test -f "$site/api/aozora_flavored_markdown/index.html"
    test -f "$site/playground/index.html"

# --- Fixed CI entry points ---------------------------------------------------

[group('ci')]
ci-rust: fmt-check check-features clippy coverage test-doc doc-public

[group('ci')]
ci-web: test-wasm playground-typecheck playground-lint playground-test playground-build playground-e2e playground-lighthouse

[group('ci')]
ci-repo: deny typos actionlint zizmor

[group('ci')]
ci-release: dist-assets-check package-smoke dist-plan release-smoke

[group('ci')]
ci-fuzz: fuzz-build

[group('ci')]
ci: ci-rust ci-web ci-repo ci-release ci-fuzz
