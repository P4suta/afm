# syntax=docker/dockerfile:1.7
# aozora-flavored-markdown development / CI container. Every developer and CI job runs inside this
# image; the host toolchain is never invoked. Layered so a dependency bump
# rebuilds a minimal surface.
#
# The base image is pinned by immutable digest; Dependabot bumps tag +
# digest together weekly. Refresh by hand with
# `docker buildx imagetools inspect <tag>`. NODE_VERSION is an ARG, not a
# pinned FROM — it only parameterises an apt source URL in the node-base stage.
ARG NODE_VERSION=22

########################################################################
# Stage: toolchain — Rust stable + system deps for builds and CJK work
########################################################################
# rust:1.96.0-bookworm (digest pinned; tag kept for humans / Dependabot)
FROM rust:1.96.0-bookworm@sha256:19817ead3289c8c631c73df281e18b59b172f6a31f4f563290f69cddd06c30e9 AS toolchain

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        clang \
        mold \
        curl \
        git \
        ca-certificates \
        unzip \
        xz-utils \
        locales \
    && sed -i -e 's/# \(ja_JP.UTF-8 UTF-8\)/\1/' /etc/locale.gen \
    && sed -i -e 's/# \(en_US.UTF-8 UTF-8\)/\1/' /etc/locale.gen \
    && locale-gen

ENV LANG=en_US.UTF-8 \
    LC_ALL=en_US.UTF-8 \
    RUSTUP_PERMIT_COPY_RENAME=1

# `docker build` does NOT inherit docker-compose's x-common-env, so the cargo
# registry-resilience knobs must be set here too: the `cargo install` layers in
# cargo-tools (cargo-binstall, bacon) otherwise hit crates.io directly and flake
# on the HTTP/2 "framing layer" error. RETRY rides out transient blips;
# MULTIPLEXING=false forces HTTP/1 to sidestep the framing bug entirely.
ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_MULTIPLEXING=false

# Use mold as the default linker for faster builds
RUN mkdir -p /root/.cargo && printf '%s\n' \
    '[target.x86_64-unknown-linux-gnu]' \
    'linker = "clang"' \
    'rustflags = ["-C", "link-arg=-fuse-ld=mold"]' \
    > /root/.cargo/config.toml

########################################################################
# Stage: cargo-tools — install Rust dev utilities (cached layer)
########################################################################
FROM toolchain AS cargo-tools

# cargo-binstall fetches prebuilt release binaries for the tools below; only
# binstall itself is built from source.
ARG BINSTALL_VERSION=1.19.1
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/tmp/cargo-build \
    CARGO_TARGET_DIR=/tmp/cargo-build \
    cargo install --locked --version "${BINSTALL_VERSION}" --root /usr/local cargo-binstall

# Tier A — yearly-churn tools (longest-cached layer).
RUN cargo binstall --no-confirm --locked --root /usr/local \
        typos-cli

# Tier B — quarterly-churn test infrastructure.
RUN cargo binstall --no-confirm --locked --root /usr/local \
        cargo-nextest \
        cargo-deny \
        cargo-audit \
        cargo-insta

# Tier C — monthly-churn tools. sccache pinned to 0.10.0: 0.15+ runs a
# GHA-backend probe that errors inside cargo's rustc-wrapper even when
# SCCACHE_GHA_ENABLED is unset. Hold the pin until upstream fixes it.
RUN cargo binstall --no-confirm --locked --root /usr/local \
        cargo-llvm-cov \
        cargo-semver-checks \
        sccache@0.10.0

# Tier D — as-needed release helpers (split off the test-cycle tier).
RUN cargo binstall --no-confirm --locked --root /usr/local \
        cargo-edit \
        cargo-release

# bacon (behind `just watch`) ships no prebuilt binaries, so install it from
# source explicitly rather than relying on binstall's `compile` fallback.
RUN cargo install --locked bacon

# git-cliff for CHANGELOG generation — kept separate for the same reason.
RUN cargo binstall --no-confirm --locked --root /usr/local git-cliff

# cargo-shear — unused-dependency scanner (`just shear`, the CI audit gate,
# and the pre-push `just ci`). Kept in its own late layer so adding it never
# busts the cache for the heavier tool tiers above.
RUN cargo binstall --no-confirm --locked --root /usr/local cargo-shear

# committed — Conventional-Commits linter behind `just commitlint`. That gate
# ran only inside CI until the gate list became one manifest; running it from
# `just ci` means the tool has to exist in the image too. Same late-layer
# placement as cargo-shear above, for the same cache reason.
RUN cargo binstall --no-confirm --locked --root /usr/local committed

# zizmor — GitHub Actions static analysis (`just zizmor`). Ships a prebuilt
# release archive, so no cargo layer is spent on it. This is the tool that
# holds the repo's action-pinning policy; the policy itself is `zizmor.yml`.
ARG ZIZMOR_VERSION=1.28.0
RUN curl -fsSL \
    "https://github.com/zizmorcore/zizmor/releases/download/v${ZIZMOR_VERSION}/zizmor-x86_64-unknown-linux-gnu.tar.gz" \
    | tar -xz -C /usr/local/bin zizmor

# actionlint — workflow schema + expression lint (`just actionlint`). Declared
# in mise.toml for the host inner loop since the beginning; installed here so
# it is a gate rather than a suggestion.
ARG ACTIONLINT_VERSION=1.7.12
RUN curl -fsSL \
    "https://github.com/rhysd/actionlint/releases/download/v${ACTIONLINT_VERSION}/actionlint_${ACTIONLINT_VERSION}_linux_amd64.tar.gz" \
    | tar -xz -C /usr/local/bin actionlint

# just (task runner) installed separately; upstream provides an install script
RUN curl -fsSL https://just.systems/install.sh \
    | bash -s -- --to /usr/local/bin --tag 1.51.0

# lefthook (pre-commit manager). As of 2.x the release asset is a gzipped raw binary.
ARG LEFTHOOK_VERSION=2.1.9
RUN curl -fsSL \
    "https://github.com/evilmartians/lefthook/releases/download/v${LEFTHOOK_VERSION}/lefthook_${LEFTHOOK_VERSION}_Linux_x86_64.gz" \
    | gunzip > /usr/local/bin/lefthook \
    && chmod +x /usr/local/bin/lefthook

# wasm-pack — builds the aozora-flavored-markdown-wasm crate consumed by `playground/` and any
# browser host. Pinned alongside the workflow pin in .github/workflows/docs.yml
# so dev and CI agree on the wasm-bindgen-cli that gets auto-fetched.
ARG WASM_PACK_VERSION=0.15.0
RUN curl -fsSL \
    "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar -xz -C /usr/local/bin --strip-components=1 \
        "wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl/wasm-pack"

# bun — JavaScript runtime + package manager for the playground (TS edits,
# Vite dev server, production build). The pin
# below must agree with `playground/package.json` "packageManager" and
# `.github/workflows/docs.yml` setup-bun `bun-version:` — `just verify-
# version-pins` is the mechanical gate that catches drift.
ARG BUN_VERSION=1.3.14
RUN curl -fsSL \
    "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-linux-x64.zip" \
    -o /tmp/bun.zip \
    && unzip -d /tmp /tmp/bun.zip \
    && mv "/tmp/bun-linux-x64/bun" /usr/local/bin/bun \
    && chmod +x /usr/local/bin/bun \
    && rm -rf /tmp/bun.zip /tmp/bun-linux-x64

# vale — prose lint (`just vale`). Same shape as zizmor / actionlint above: a
# prebuilt release archive, and the policy lives beside the tool rather than in
# the recipe — `.vale.ini` says what is read, `styles/Aozora/RetiredPaths.yml`
# says what is banned. It is what replaced the retired-upstream-path half of
# `cargo xtask comment-discipline`, whose file list was `.rs` and `.toml`, so a
# Markdown document was outside every gate this repo has. Last in the stage,
# like cargo-shear and committed, so adding it busts no heavier layer's cache.
ARG VALE_VERSION=3.15.2
RUN curl -fsSL \
    "https://github.com/vale-cli/vale/releases/download/v${VALE_VERSION}/vale_${VALE_VERSION}_Linux_64-bit.tar.gz" \
    | tar -xz -C /usr/local/bin vale

########################################################################
# Stage: node — Node.js 22, the JS runtime the dev image builds on
########################################################################
FROM toolchain AS node-base

ARG NODE_VERSION
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    curl -fsSL https://deb.nodesource.com/setup_${NODE_VERSION}.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    corepack enable

########################################################################
# Stage: dev — stable-only contributor image (no nightly, no fuzz/udeps)
########################################################################
FROM node-base AS dev

COPY --from=cargo-tools /usr/local/cargo/bin/ /usr/local/cargo/bin/
COPY --from=cargo-tools /usr/local/bin/ /usr/local/bin/

# Pre-install the components `rust-toolchain.toml` requires so the first
# in-workspace `cargo` call doesn't trigger a rustup channel sync (which
# scrubs the env and trips the sccache pin above).
RUN rustup component add rustfmt clippy rust-src

# AOZORA_MD_IN_CONTAINER tells the Justfile it is already inside the dev image, so its
# recipes run tools directly instead of nesting another `docker compose run`
# (which has no daemon here). Lets `just shell`, devcontainers and Codespaces
# use the same `just` recipes as the host. Inherited by the fuzz / ci stages.
ENV CARGO_HOME=/cargo/home \
    CARGO_TARGET_DIR=/cargo/target \
    RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/cargo/sccache \
    RUST_BACKTRACE=1 \
    AOZORA_MD_IN_CONTAINER=1

# Pre-create the /cargo/* cache mount targets. They live OUTSIDE the
# /workspace bind mount on purpose: nesting them under /workspace makes the
# daemon create root-owned ./target / ./.cargo on the host.
RUN mkdir -p /cargo/target /cargo/home/registry /cargo/home/git /cargo/sccache \
    /workspace/playground/node_modules

# Run as non-root so files written into the /workspace bind mount are
# host-owned, not root. UID/GID default to 1000; override with
# `--build-arg UID=$(id -u) --build-arg GID=$(id -g)`. CI flips back to root
# via `user:` in docker-compose.yml (AOZORA_MD_UID=0) — its checkout UID is
# throwaway, so root sidesteps cross-UID write failures.
ARG UID=1000
ARG GID=1000
RUN groupadd --gid "${GID}" dev \
    && useradd --uid "${UID}" --gid "${GID}" --create-home --shell /bin/bash dev \
    && chown -R "${UID}:${GID}" /cargo /workspace
ENV HOME=/home/dev

WORKDIR /workspace
USER dev

# Default shell friendly for interactive dev sessions
CMD ["bash"]

########################################################################
# Stage: fuzz — dev superset adding nightly + cargo-fuzz + cargo-udeps.
# Only `just udeps` / `fuzz*` / `coverage-branch` need nightly, so the plain
# `dev` image skips the nightly install. nightly is opt-in `cargo +nightly`
# here, so the rust-toolchain.toml stable pin is unaffected.
########################################################################
FROM dev AS fuzz

# The dev stage ends as USER dev; the nightly toolchain + cargo-fuzz/udeps
# installs below write to /usr/local (root-owned), so switch back to root
# for the install layers and drop to dev again at the end.
USER root

# Override CARGO_HOME for this RUN only: the runtime `/cargo/home` is empty at
# build time, so rustup can't find itself there. Point it at the base image's
# `/usr/local/cargo`; the runtime setting is unaffected.
RUN CARGO_HOME=/usr/local/cargo \
    rustup toolchain install nightly --component rust-src --profile minimal

RUN cargo binstall --no-confirm --locked --root /usr/local \
        cargo-fuzz \
        cargo-udeps

USER dev

########################################################################
# Stage: ci — fuzz superset; the published GHCR image (used by CI matrix
# jobs) carries every tool every recipe might invoke.
########################################################################
FROM fuzz AS ci
