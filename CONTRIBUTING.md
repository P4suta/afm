# Contributing to aozora-flavored-markdown

This repo is the **Markdown ↔ Aozora composition layer**: it composes an
unmodified comrak with the sibling
[`P4suta/aozora`](https://github.com/P4suta/aozora) parser so CommonMark +
GFM and 青空文庫記法 coexist in one document.

**New 青空文庫記法 does not start here.** Lexer phases, AST shapes,
recogniser tables and a notation's own HTML all live in the sibling repo
(ADR-0010, ADR-0021). File it there; the next version bump brings it here
automatically.

## Ground rules

1. **Docker-only execution** (ADR-0002). Never invoke `cargo` or `bun` on
   the host — every step goes through `just <target>`, which shells into
   the dev container.
2. **comrak is a plain dependency, never patched** (ADR-0024). Composition
   happens in `ast_splice`; a change that would need comrak itself to
   behave differently is an upstream issue, not a local edit.
3. **No warning suppressions.** `#[allow(...)]`, `continue-on-error`, and
   similar escape hatches are rejected by `just strict-code`. Fix the real
   issue.
4. **TDD, with the coverage floor as a ratchet.** A failing test lands
   first. The floor is `_COV_FLOOR` in the `Justfile`; it moves up as tests
   land. It moves down only when the *denominator* widens — a PR that stops
   excusing a source file re-measures and sets the floor to what the wider
   denominator actually reads, and records both numbers.
5. **No unused dependencies.** `just shear` rejects them. For a macro- or
   `cfg`-only use its `syn` pass cannot see, record a documented
   `[workspace.metadata.cargo-shear] ignored = [...]`.
6. **Prose earns its place, and it rots like code.** `just vale` fails on a
   retired upstream path named in a document or a comment — `.vale.ini` says
   what it reads, [`styles/Aozora/RetiredPaths.yml`](styles/Aozora/RetiredPaths.yml)
   says what is banned. `just comment-discipline` fails on a line naming a
   repo path that no longer exists, and on a doc-comment count above the
   pinned ceiling. Write down *why*, not what the code already says.
7. **Workflows are linted, not reviewed by eye.** The rules they answer to —
   `uses:` on a 40-hex commit rather than a mutable tag above all — live in
   [`zizmor.yml`](zizmor.yml), and `just zizmor` / `just actionlint` are
   gates. The `# vX.Y.Z` beside a ref is a comment for humans and Dependabot.
8. **A gate is declared once.** `[group('gate')]` on a `Justfile` recipe is
   the whole declaration: `just gates` prints the set, `ci.yml` builds its job
   matrix from it, and `just ci` refuses to start when its own lanes disagree.
   Adding a check means adding the attribute and a lane — never a job in the
   workflow. The two `[group('native')]` gates (`msrv`, `commitlint`) keep a
   dedicated CI job because they need a toolchain or a commit range the dev
   image has not got, and they run the same recipe there.

## Setup and the development loop

```sh
just setup                     # build the dev image, install hooks, run tests
just watch                     # bacon watcher inside the container
just lint                      # fmt + clippy + typos + strict-code + comments + prose + workflows
just test                      # full workspace nextest
just gates                     # the gate manifest, as CI reads it
just ci                        # exactly the gate CI runs
```

The playground has gates of its own — `just playground-lint` (Biome:
formatter, linter and import order), `just playground-test` (Vitest) and
`just playground-build`. They are outside `just lint` only because each one
pulls a release wasm build in as a dependency; `just ci` runs all three, and
`just playground-lint-fix` writes the safe half of what the first reports.

`just setup` is idempotent; re-run it after pulling. `just --list`
enumerates every recipe. Before a release, `just prop-deep` runs a 4096-case
property sweep — deeper than CI.

Aozora-layer fixtures (annotation cases, the golden corpus, fuzz) live in
the sibling repo. Run them from there.

## Troubleshooting

Run **`just doctor`** first: it audits images, cache volumes, the `aozora`
pin and playground prerequisites, and prints a fix hint for anything
missing. Beyond that:

- **`Blocking waiting for file lock on build directory`** — two cargo
  commands share the one `cargo-target` volume (e.g. `just watch` during
  `just test`). They serialise; they do not deadlock.
- **rust-analyzer cannot find Cargo** — the host has no Rust toolchain
  (ADR-0002). Open the repo in the devcontainer or a Codespace, or work
  from `just shell`. A host-side rust-analyzer will never see it.
- **A `just` recipe fails with a Docker error from inside a container** —
  your image predates the container-aware `Justfile`. `docker compose build
  dev` bakes `AOZORA_MD_IN_CONTAINER=1` back in.
- **Root-owned files / permission denied** — `just clean`, or `just nuke`
  to also drop the cache volumes.
- **Docker Desktop / WSL feels slow** — the registry, target, sccache and
  `node_modules` caches live in named volumes outside the bind mount on
  purpose. Moving them into the tree is the slow path.

## Where a change lands

- **Splice edge case** — `crates/aozora-flavored-markdown/src/ast_splice.rs`,
  with a unit test in the same module. A "must never be" HTML shape belongs
  in the `aozora-flavored-markdown-test-support` crate as a predicate plus
  both unit pins (passes-on-clean, fires-on-shape).
- **Construct substitution** — `src/constructs.rs` owns the table both
  walkers consume, one PUA sentinel per construct in source coordinates
  (ADR-0023). Touching what a byte range covers needs a case in
  `tests/construct_spans.rs`.
- **IR projection** — `src/ir/`. Every notation projects to one
  `ir::Block::Aozora` / `ir::Inline::Aozora` (ADR-0022). Tests in
  `tests/ir_aozora.rs`. Bumping the IR schema is semver-major: the wasm
  bridge and the obsidian plugin validate it on the JS side.
- **CSS classes** — `classes::all()` is derived from the sibling
  renderer's list (rebranded per ADR-0011), so nothing is hand-kept. A new
  class needs a rule in both `theme/aozora-md-{horizontal,vertical}.css`;
  `tests/css_class_contract.rs` fails until they agree in both directions.
- **A notation this repo has never seen** — check `block_sentinel_of` (an
  unnamed kind falls through to the inline sentinel, which would splice
  block markup inside a `<p>`) and `inline_is_dropped`, both hand-written
  matches in `src/constructs.rs`.

## Architectural changes

Any decision that shapes a whole subsystem lands first as an ADR (MADR
format) under `docs/adr/`:

```sh
cargo xtask new-adr 'my new decision'
```

Add a row to [`docs/ADR_INDEX.md`](docs/ADR_INDEX.md) and reference the ADR
in the commit body.

## Commits and pull requests

[Conventional Commits](https://www.conventionalcommits.org/), enforced by
the `commit-msg` hook. Scopes match the workspace shape: `markdown`, `cli`,
`wasm`, `epub`, `xtask`, `comrak`, `adr`, `release`, `dev`, `test`. One
logical change per commit.

PR titles match the commits, and `Closes #N` links the issue. Keep the PR
template's checklist — it is the same gate `just ci` runs, so a green
`just ci` means a green PR.

Report bugs with the `bug_report` form; the shortest source text that
triggers the issue is the most valuable thing you can supply. Security
issues go through `SECURITY.md`, never a public issue.

## Releasing

Releases are automated by [cargo-dist](https://opensource.axo.dev/cargo-dist/)
and triggered by a `v<semver>` tag.

```sh
just release minor             # dry run: prints every file it would rewrite
just release minor --execute   # rewrites them
git commit -am 'chore(release): v0.6.0'
git tag -a v0.6.0 -m 'v0.6.0' && git push --follow-tags
```

[cargo-release](https://github.com/crate-ci/cargo-release) writes every
manifest and `Cargo.lock`, cuts the `## [Unreleased]` section of
`CHANGELOG.md` into a dated one, and regenerates the man page that embeds the
version. `release.toml` is the configuration — plus a
`[package.metadata.release]` table in three manifests, each next to the thing
it is a fact about. Writing the CHANGELOG *entries* is still yours: an entry
has to say what broke and what to do about it, which a commit subject does
not. `just changelog` prints a git-cliff draft to check the section against
before you cut it.

The two version lines move independently and neither is told what the other
is: `shared-version` groups the crates that inherit `[workspace.package]
version` as `workspace` and the two 0.1.x EPUB crates as `epub`, and a bump
applies the level within each group. A new crate that carries its own version
has to name its group, or it is bumped onto the workspace's number without a
warning.

The commit and the tag are yours by design. Both are SSH-signed and the key
is not in the dev image — a `git commit` run inside the container would not
fail, it would succeed unsigned — so `just release` stops when the files are
written. `release.toml` says the same in the form cargo-release reads
(`publish = false`, `tag = false`, `push = false`), so reaching for the tool
by hand from `just shell` lands in the same place. Publishing is
`publish-crates.yml`'s, behind a required-reviewer approval and a short-lived
OIDC token; its preflight refuses a version `CHANGELOG.md` has no section
for.

`release.yml` is **generated, never hand-edited**: `dist plan` diffs it
against the generator and fails on any drift, including the `actions/*`
refs inside it. They still have to satisfy `zizmor.yml`, and are
commit-pinned via `[dist.github-action-commits]` in `dist-workspace.toml`;
move one forward by resolving the tag, updating the entry and its comment,
running `dist generate`, and committing both files. `.github/dependabot.yml`
excludes the generated file from version updates — a *security* PR can still
rewrite it, in which case close the PR and bump the pin instead. The weekly
`release-pins` workflow fails when a pin freezes behind its upstream, and
files a rolling issue naming what froze, so the finding reaches somebody
rather than sitting in the Actions tab.

**ADR-0002 scope exception**: release builds run on native runners so each
binary target matches its runner OS. Docker-only applies to development and
CI.

## License

By contributing, you agree that your contributions are dual-licensed under
Apache-2.0 OR MIT, the same as the project.
