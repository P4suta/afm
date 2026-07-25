# Contributing to aozora-flavored-markdown

Thanks for wanting to help. aozora-flavored-markdown is an active project with a small
surface area of rules, but those rules are strict — the guarantees
below only stay true if every contribution respects them.

## Where things live

aozora-flavored-markdown is the **Markdown ↔ Aozora composition layer**: it composes a
vendored verbatim comrak with the sibling
[`P4suta/aozora`](https://github.com/P4suta/aozora) parser /
renderer to produce HTML where CommonMark + GFM and 青空文庫記法
coexist correctly. New 青空文庫記法 / lexer / per-node render work
lands in the sibling repo, not here. aozora-md-side work falls into:

1. **Markdown ↔ Aozora glue** — `crates/aozora-flavored-markdown/`
   (`render_to_string`, `render_to_ir`, `serialize`,
   `ast_splice::splice_into_ast`, the `IrDocument` projection).
2. **CLI** — `crates/aozora-flavored-markdown-cli/` (`aozora-flavored-markdown render` / `aozora-flavored-markdown check`).
3. **WASM bridge** — `crates/aozora-flavored-markdown-wasm/` (aozora-flavored-markdown-obsidian / browser
   hosts).
4. **Documentation** — the READMEs (introduction), rustdoc (API), and
   `docs/adr/` (Architecture Decision Records).

Authoring tools (formatter / LSP / VS Code extension) live in a
third sibling repo
[`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools)
per ADR-0009.

## Ground rules

1. **Docker-only execution** (ADR-0002). Do not invoke `cargo` or
   `bun` on the host. Every automated step goes through
   `just <target>`, which shells into the dev container.
2. **Vendored comrak is hands-off** (ADR-0001). The fork sits at
   `upstream/comrak/` with a **0-line diff budget** — any change
   would be a fork divergence and needs its own ADR. Composition
   happens in `aozora-flavored-markdown::ast_splice` (AST sentinel splice),
   never inside `upstream/comrak/`.
3. **No warning suppressions.** `#[allow(...)]`, `#![allow(...)]`,
   `#[cfg_attr(..., allow(...))]`, `continue-on-error` in
   workflows, and similar escape hatches are rejected by
   `just strict-code`. Refactor the real issue instead.
4. **TDD with C1 100% branch coverage as the goal.** A failing
   test lands first, then the fix. The CI floor is currently 97%
   regions (`_COV_FLOOR` in `Justfile`), ratcheted upward as gaps
   close.
5. **No unused dependencies.** A dependency declared in any
   `Cargo.toml` — including `[workspace.dependencies]` — that no
   crate actually `use`s is rejected by `just shear` (cargo-shear).
   Delete the dead entry, or — for a macro- or `cfg`-only use
   cargo-shear's `syn` pass can't see — record a documented
   `[workspace.metadata.cargo-shear] ignored = [...]`.

## First-time setup

One command after cloning — builds the dev image, installs the git
hooks, checks the environment, and runs the tests:

```sh
just setup
```

It wraps the steps you can also run by hand:

```sh
docker compose build dev       # ~5 min first time, cached afterward
jj git init --colocate         # if jj isn't already initialised (optional)
just hooks                     # wire lefthook pre-commit / commit-msg / pre-push
just doctor                    # verify images, volumes, the aozora pin
just test                      # confirm green
```

`just setup` is idempotent, so re-run it after pulling. Prefer zero local
setup? The **Open in GitHub Codespaces** badge in the README boots a
container with the toolchain already built.

## Development loop

```sh
just watch                     # bacon watcher inside the dev container
just lint                      # fmt + clippy pedantic+nursery + typos + strict-code + comment-discipline
just test                      # full workspace nextest
just prop                      # property-based sweep (128 cases per block)
just spec-commonmark           # CommonMark 0.31.2 (652 cases)
just spec-gfm                  # GFM 0.29 spec
just coverage                  # cargo llvm-cov, fails below _COV_FLOOR
just upstream-diff             # verify upstream/comrak/ is still 0-line
just ci                        # replica of the full CI pipeline

# Before a release:
just prop-deep                 # 4096 cases per block — deeper than CI
```

`just --list` enumerates everything available.

Aozora-layer fixtures (annotation cases, golden 56656, 17 k-work
corpus sweep, fuzz) live in the sibling
[`P4suta/aozora`](https://github.com/P4suta/aozora) repo. Run them
from there.

## Your first change

A quick lap to confirm the loop works end to end:

1. `just setup` — once per clone (build image, hooks, doctor, tests).
2. `just watch` in one terminal — bacon recompiles on every save.
3. Make a small edit in `crates/aozora-flavored-markdown/` and add a test next to
   it (a `#[cfg(test)]` case, or a row in the relevant `tests/*.rs`).
   The watcher stays red until it passes.
4. `just test` for the whole suite, `just lint` for fmt + clippy.
5. `just ci` before you push — it is exactly the gate CI runs, so a
   green `just ci` means a green PR. The `pre-push` hook runs it for you.
6. Commit with a Conventional Commits subject (`feat(markdown): …`);
   the `commit-msg` hook rejects anything else.

New 青空文庫 notation does **not** start here — it lands in the sibling
[`P4suta/aozora`](https://github.com/P4suta/aozora) repo first (ADR-0010).

## Troubleshooting

Run **`just doctor`** first — it audits images, cache volumes, the
`aozora` pin, and playground prerequisites, and prints a fix hint for
anything missing.

- **First `docker compose build dev` is slow (~2–5 min).** Expected; it
  is cached afterwards. A flaky download usually fixes itself on a
  re-run (`CARGO_NET_RETRY=10` rides out transient registry blips).
- **`Blocking waiting for file lock on build directory`.** Two cargo
  commands are sharing the one `cargo-target` volume — e.g. `just watch`
  while you run `just test`. Let one finish; they serialise on the lock,
  they do not deadlock.
- **rust-analyzer shows "cannot find Cargo" / red squiggles everywhere.**
  The host has no Rust toolchain (ADR-0002) — rust-analyzer must run
  *inside* the image. Open the repo in the devcontainer / a Codespace
  (it boots `rust-analyzer` in-container), or work from `just shell`. A
  host-side rust-analyzer cannot see the toolchain and never will.
- **`just <recipe>` fails with a Docker error from inside a container.**
  Your image predates the container-aware Justfile. Rebuild it
  (`docker compose build dev`) so `AOZORA_MD_IN_CONTAINER=1` is baked in and
  recipes run their tool directly instead of nesting a container.
- **sccache looks cold (slow rebuilds).** `just sccache-stats` shows the
  hit ratio; a stray `RUSTC_WRAPPER` override or profile tweak can defeat
  it. `just sccache-zero && just clean && just build && just sccache-stats`
  gives a clean measurement window.
- **Root-owned files in the tree / permission denied.** The dev image
  runs as UID 1000 so bind-mount writes stay host-owned; if a CI run
  left root-owned artefacts behind, `just clean` (or `just nuke` to also
  drop the cache volumes) resets them.
- **Docker Desktop / WSL feels slow.** The cargo registry, target, and
  sccache caches plus the playground `node_modules` live in *named
  volumes* outside the `/workspace` bind mount on purpose — don't move
  them into the tree, that is the slow path.

## How to make a change

### aozora-flavored-markdown (the glue layer)

Most aozora-md-side changes are one of:

- **Aozora splice edge case** — landing in
  `crates/aozora-flavored-markdown/src/ast_splice.rs`. Add a unit test in the
  same module's `#[cfg(test)] mod tests` and a property-test ping
  in `tests/post_process_invariants.rs` if the change has
  cross-input semantics. Predicates that codify "must-never-be"
  HTML shapes live in the `aozora-flavored-markdown-test-support` crate —
  add new ones there with both the predicate and a unit pin
  (`invariant_unit_check_X_passes_on_clean_input`,
  `invariant_unit_check_X_fires_on_<shape>`).
- **Construct substitution** — `crates/aozora-flavored-markdown/src/constructs.rs`
  owns the table both walkers consume: one PUA sentinel per 青空文庫
  construct, substituted in source coordinates (ADR-0023). Changes there
  belong with a unit test in the same module, plus a case in
  `crates/aozora-flavored-markdown/tests/construct_spans.rs` when they touch
  what a construct's byte range covers or resolves to.
- **IR projection** — `crates/aozora-flavored-markdown/src/ir/`. Every
  notation projects to one `IrBlock::Aozora` / `IrInline::Aozora`
  (ADR-0022); paragraph dispatch flows through `IrWalker::walk_top` →
  `classify_paragraph`. Streaming-mode behaviour goes through
  `StreamingIrBuilder`. Add tests in
  `crates/aozora-flavored-markdown/tests/ir_aozora.rs`.
- **CSS class contract drift** — when the sibling renderer adds a
  new `aozora-*` class, update
  `aozora-flavored-markdown-test-support`'s `AOZORA_MD_CLASSES` (the brand
  is rewritten to `aozora-md-*` per ADR-0011) and the corresponding rule
  in `theme/aozora-md-{horizontal,vertical}.css`.
- **Public API drift** — `Options::default` defaults, new
  entry points, diagnostic shape. Lives in
  `crates/aozora-flavored-markdown/src/lib.rs`. Bumping the IR schema is a
  semver-major change because aozora-flavored-markdown-wasm and aozora-flavored-markdown-obsidian validate
  it on the JS side.
- **CLI behaviour** — `crates/aozora-flavored-markdown-cli/src/main.rs` and the binary-
  level integration tests in `crates/aozora-flavored-markdown-cli/tests/cli_integration.rs`.

### Spec / golden / corpus regression

`crates/aozora-flavored-markdown/tests/*.rs` covers the CommonMark + GFM spec
runners (`commonmark_spec.rs`, `gfm_spec.rs`) and the Aozora ×
Markdown integration surface (`aozora_parity.rs`,
`paired_container.rs`, `heading_promotion.rs`,
`block_structure_interaction.rs`,
`property_html_shape.rs`, …). Each adds a single layer of evidence;
new invariants ride on the same scaffolding.

### Adding a 青空文庫 notation

This **does not happen on the aozora-flavored-markdown side**. Lexer phases, AST
shapes, recogniser tables, and per-node renderers all live in the
sibling [`P4suta/aozora`](https://github.com/P4suta/aozora) repo
(see ADR-0010 for the rationale). Once a new construct is
classified upstream and lands in `aozora-syntax`, aozora-flavored-markdown picks it up
automatically through the workspace dep — usually with a one-line
mapping in `aozora_flavored_markdown::ir::project_inline` /
`project_block_leaf` plus a test, and a `AOZORA_MD_CLASSES` update if
the renderer adds a new CSS hook.

## Architectural changes

Any decision that shapes how a whole subsystem behaves lands first
as an **Architecture Decision Record** (MADR format) under
`docs/adr/`. Scaffold one with:

```sh
cargo xtask new-adr 'my new decision'
```

Add a row to the index ([`docs/ADR_INDEX.md`](docs/ADR_INDEX.md)) and
reference the ADR in the commit body.
`docs/adr/0021-aozora-boundary-is-the-public-surface.md` is an
example of a decision that reshapes a whole boundary;
`docs/adr/0011-brand-boundary-css-class-rewrite.md` is an example of a
small, scoped one.

## Commit style

**Conventional Commits**
([v1.0.0](https://www.conventionalcommits.org/)). The `commit-msg`
hook enforces this. Accepted types: `feat`, `fix`, `docs`,
`style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`,
`revert`. Scopes match the workspace shape — one of: `markdown`
(aozora-flavored-markdown), `cli` (aozora-flavored-markdown-cli), `wasm`
(aozora-flavored-markdown-wasm), `epub`, `xtask`, `comrak` (touches under
`upstream/comrak/`), `adr`, `release`, `dev`, `test`.

A single commit should be a single logical change. Split unrelated
edits.

## Pull requests

- PR title should be `<type>(<scope>): <summary>` matching the
  commits.
- Link any issue the PR closes (`Closes #N` in the body).
- The PR template (`.github/PULL_REQUEST_TEMPLATE.md`) walks you
  through the checklist — **keep it**. It reminds everyone
  (including the author) of the full gate: tests, coverage, ADR,
  `just ci`.
- CI runs `just ci` in the ci container image. The gate is the
  same one you ran locally; surprises mean either an environment
  mismatch or an ADR-boundary subtlety.

## Reporting bugs and asking for features

- **Bugs**: use the `bug_report` issue form. The minimal
  reproducible input (the shortest source text that triggers the
  issue) is the most valuable thing you can supply.
- **Features**: use the `feature_request` form. Concrete
  motivation (a real Aozora Bunko text that needs the notation, a
  CommonMark construction that would benefit, a corpus sweep hit)
  makes triage faster. If the feature is "support a new 青空文庫
  notation", file it on the sibling
  [`P4suta/aozora`](https://github.com/P4suta/aozora/issues) repo
  instead — that's where the parser lives.
- **Questions / discussions**: prefer GitHub Discussions over
  issues.

## Security

Security-sensitive issues (parser crashes, memory safety concerns,
sandbox escapes) should be reported privately per `SECURITY.md` —
do **not** open a public issue.

## How to release

Releases are automated by [cargo-dist](https://opensource.axo.dev/cargo-dist/)
(`dist`) and triggered by a git tag of the form `v<semver>`:

1. Update `CHANGELOG.md` — promote `[Unreleased]` to
   `[<version>] - YYYY-MM-DD` and add a fresh `[Unreleased]` stub.
2. Regenerate the bundled CLI assets: `just dist-assets`. The man page
   embeds the version, so a version bump changes
   `dist/assets/man/aozora-flavored-markdown.1` (and `just ci`'s `dist-assets-check` gate
   would otherwise fail).
3. Commit the changelog + asset bump:
   `git commit -m "chore: release v<version>"`.
4. Tag (annotated): `git tag -a v<version> -m 'v<version>'`.
5. Push: `git push origin main v<version>`.
6. `.github/workflows/release.yml` (generated by `dist`) reacts to
   the tag, builds the `aozora-flavored-markdown` binary on the five targets configured
   in `dist-workspace.toml` (aarch64/x86_64 linux-gnu, aarch64/x86_64
   macOS, x86_64 windows-msvc), packages each as an archive with the
   licences + `README.md` + bundled completions and man page, generates
   `shell`/`powershell` installer scripts and checksums, and creates the
   GitHub Release with notes derived from the changelog.
7. Sanity check: download one artefact, verify its `.sha256`, then
   `./aozora-flavored-markdown --version` to confirm the embedded version matches the tag.

The release config lives in `dist-workspace.toml`; regenerate the
workflow after editing it with `dist generate`. Every PR runs the
`plan` job (a `dist plan` dry-run) so a broken release config fails
at review time; you can also run `dist plan` locally before tagging.

**ADR-0002 scope exception**: release builds run on native GitHub
Actions runners with the matching stable rustc, not inside the dev
Docker image. The Docker-only rule applies to development and CI;
the release pipeline is deliberately host-toolchain so each binary
target matches its runner OS exactly.

## License

By contributing, you agree that your contributions are
dual-licensed under Apache-2.0 OR MIT, the same as the project.
