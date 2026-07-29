# Changelog

All notable changes to Aozora Flavored Markdown are recorded in
this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased]

This cycle follows the sibling parser to **aozora 0.5.0**, which curated
its published crates down to three and made `Document` / `Snapshot` plus a
flat projection its whole public surface. Every internal path this
workspace used to reach through — the borrowed AST, the per-node renderer,
the normalised-coordinate registry — is private upstream now, so the
boundary was **redrawn rather than renamed**: substituting the 青空文庫
constructs, rendering their HTML and projecting them into the IR are all
owned here, and speak only the parser's public API
([ADR-0021](docs/adr/0021-aozora-boundary-is-the-public-surface.md)).

Consumers of the IR, of the TypeScript types, or of the `aozora-md-*` CSS
classes have breaking changes to absorb — see **Changed (breaking)**.

### 0.4 → 0.5 migration

0.5.0 is the cleanup boundary. The old spellings are not aliases: update the
consumer and let the compiler or TypeScript catch every remaining use.

| 0.4 surface | 0.5 replacement |
|---|---|
| `Document` | `MarkdownDocument` |
| `Range` | `SourceRange` |
| `Position` | `SourcePosition` |
| `Span` | `ByteSpan` |
| `Error` | `CanonicalizeError`; the unreachable `ParseFailed` variant is removed |
| `miette::Report::new(diagnostic)` | `miette::Report::new(diagnostic.bind_source(name, source)?)` |
| JSON option `aozoraEnabled` | `aozora`; unknown and retired keys are errors |
| public/test raw-HTML option paths | none; raw HTML is never enabled by public `Options` |

The JSON keys and enum tags of the IR, and the emitted HTML contract, otherwise
remain stable. Generated TypeScript uses the new Rust names directly, avoiding
the browser globals `Document` and `Range`.

### Added

- **Non-writing format and EPUB validation commands.**
  `aozora-flavored-markdown fmt --check|--diff|--write` separates checking,
  review and mutation; `check`/`diff` accept stdin and never write, while
  `write` accepts only a UTF-8 file. `aozora-flavored-markdown-epub check`
  executes discover → validate → render → compose without creating an EPUB.
- **Source-bound miette diagnostics.** `Diagnostic::bind_source` validates the
  byte span against the exact UTF-8 source and returns
  `SourceBoundDiagnostic`; the CLI uses this route exclusively, so a report
  cannot be paired with the wrong source text.
- **EPUB3 generator consolidated into this workspace** — the
  `aozora-flavored-markdown-epub` library and its `aozora-flavored-markdown-epub`
  CLI now live in `crates/aozora-flavored-markdown-epub{,-cli}`, absorbed from
  the former `P4suta/aozora-flavored-markdown-epub` sibling repo (now archived),
  mirroring how `aozora` absorbed `aozora-tools`. The crates are independently
  versioned (0.1.x), and the pure parser/renderer crates stay free of the
  generator's `zip` / `quick-xml` / `uuid` / `chrono` I/O dependencies (asserted
  via `cargo tree`). See
  [ADR-0018](docs/adr/0018-consolidate-the-epub-generator-into-this-workspace.md),
  [ADR-0019](docs/adr/0019-epub-generation-is-hand-rolled-not-via-pandoc.md), and
  [ADR-0020](docs/adr/0020-canonicalise-aozora-md-css-at-the-next-aozora-bump.md).
- **`AOZORA_MD_CLASSES` and `is_contract_class` are public API** of
  `aozora-flavored-markdown`, *derived* from the parser's own
  `AOZORA_CLASSES` by prefix swap (ADR-0011) instead of hand-copied. They
  used to be a hand-maintained list in the dev-only test-support crate,
  which could — and did — fall behind a parser that renames classes
  between releases. (#160)
- **`theme` feature (default-off)** — `theme::HORIZONTAL_CSS` and
  `theme::VERTICAL_CSS` publish the two canonical stylesheets as `pub
  const` data (`include_str!`, no new dependencies), so a downstream
  packager embeds the constants instead of vendoring a copy that has to
  track the class contract by hand. The `.css` sources moved to
  `crates/aozora-flavored-markdown/theme/`, because `include_str!` reaching
  outside a crate's own directory is not bundled by `cargo publish`. (#160)
- **`aozora-md::constructs_unresolved` diagnostic** — a construct whose
  source run cannot be recovered (the fallback path a parser-rewritten
  document takes) now renders as nothing *and says so*, rather than
  silently emitting empty markup. (#158)
- **Proptest input strategies in
  `aozora-flavored-markdown-test-support`** — `config::default_config`
  (the `AOZORA_PROPTEST_CASES` knob) and `generators::{kanji_fragment,
  aozora_fragment, pathological_aozora, commonmark_adversarial}`. The
  CommonMark pool now carries shapes a parser-side generator has no
  Markdown to express: Aozora notation inside a table cell, a list item,
  a link label, a code span and a fenced block. (#155)
- **`diagnose(&str, &Options) -> Vec<Diagnostic>`** — what a render would
  report, without the render. Exactly `render`'s diagnostics, reached without
  the comrak parse, the splice or the HTML formatting, so a `check` command
  and the render it precedes cannot disagree about a source. The CLI's
  `check` sub-command is what needed it: it documented "without rendering"
  and rendered unconditionally. (#190)
- **`escape_html(&str) -> String`** — the workspace's one HTML escaper, over
  `&`, `<`, `>`, `"` and `'` (numeric `&#39;`, HTML 4 having no `&apos;`), so
  one call is right in text and in a quoted attribute alike. The EPUB
  envelope's private copy is gone; the two agreed character for character and
  nothing could have noticed if a fix had landed on one side only. The shared
  implementation is covered by the library's direct tests. (#182)
- **Three ADRs** — [ADR-0021](docs/adr/0021-aozora-boundary-is-the-public-surface.md)
  (the boundary is the parser's public surface only, and an upstream API
  request must be justified from upstream's side alone),
  [ADR-0022](docs/adr/0022-collapse-the-aozora-half-of-the-ir.md) (collapse
  the Aozora half of the IR) and
  [ADR-0023](docs/adr/0023-substitute-constructs-in-source-coordinates.md)
  (substitute constructs in source coordinates). (#153, #156, #157)

### Changed (breaking)

- **Core types use domain-specific names.** `MarkdownDocument`,
  `SourceRange`, `SourcePosition`, `ByteSpan` and `CanonicalizeError` replace
  their collision-prone generic names. No compatibility aliases remain.
- **`Options` is strict and contains only production behaviour.** serde rejects
  unknown fields, including retired `aozoraEnabled`; test-only raw-HTML
  configuration is no longer part of the public type.
- **The Aozora half of the IR collapses to one variant per level.** Every
  青空文庫 notation now projects to `IrInline::Aozora { kind, span, html }` /
  `IrBlock::Aozora { kind, span, html }`, carrying an opaque `kind` tag,
  the source `span` and the rendered HTML fragment. Gone: the
  `IrInline::{Ruby, DoubleRuby, Bouten, Tcy, Gaiji, Annotation}` and
  `IrBlock::{PageBreak, SectionBreak, Container}` variants, together with
  the `ContainerSubtype` / `SectionSubtype` / `BoutenStyle` /
  `BoutenPosition` / `AnnotationKind` enums that existed to restate the
  parser's vocabulary. **The Markdown half is untouched** and stays typed.
  A consumer that matched on a notation variant now reads `kind` (and can
  render straight from `html`, which is byte-identical to what the
  notation contributes to `render`).
  [ADR-0022](docs/adr/0022-collapse-the-aozora-half-of-the-ir.md). (#156)
- **The tsify-derived TypeScript `.d.ts` changes shape with it** — the
  `IrBlock` / `IrInline` unions gain the collapsed member and lose the
  per-notation ones. Regenerate against this release; do not hand-edit
  (ADR-0017). `aozora-flavored-markdown-obsidian` is an archived snapshot
  with no follow-up to make; the playground's outline reads heading text
  out of the fragment `html`. (#156)
- **CSS class renames, inherited from the parser** —
  `aozora-md-double-ruby` → **`aozora-md-angle-quote`**,
  `aozora-md-annotation` → **`aozora-md-directive`**, `aozora-md-tcy` →
  **`aozora-md-combine-upright`**. A stylesheet or a test that pins the old
  names needs updating; `AOZORA_MD_CLASSES` is the list to check against.
  The contract went from 19 hand-kept entries to the parser's 91, so most
  of the delta is *new* classes rather than renames. (#159, #160)
- **The 二重山括弧 notation's input form flips** — `≪…≫` (U+226A/U+226B) is
  the input and `《…》` (U+300A/U+300B) the display, correcting a model that
  was exactly backwards. A stray `《《…》》` in source is now two ruby openers
  (a nested-ruby diagnostic), not this construct. (#159)
- **wasm envelope: `wire` → `json`.** The wasm crate enables the parser's
  `json` feature instead of `wire`, and the editor-assist entry points
  (`nodesJson` / `pairsJson` / `diagnosticsJson` / `slugsJson` /
  `gaijiResolutionsJson`) hand back the parser's own envelope rather than
  one assembled here. The version field is spelled `schemaVersion` (was
  `schema_version`) and reads **3** (was `1`), so a TypeScript host that
  pinned either the key or the number must update. (#159)
- **`aozora` 0.4.1 → 0.5.0**, and the workspace version moves to **0.5.0**
  with it (ADR-0016's alignment habit). `default-features = false` now
  keeps `json` / `schema` / `pandoc` / `fmt` out of the build graph; an
  intentional sync is one PR per `cargo xtask aozora-bump <version>`,
  which rewrites a registry version rather than a git rev. (#159)

### Changed

- **Long `-` / `=` rule rows belong to CommonMark at every width.** A row
  immediately under prose is therefore a setext heading even when it is ten
  or more characters long; there is no Aozora-decoration option. The measured
  compatibility impact and the formatter/EPUB corpus audit are recorded in
  [ADR-0027](docs/adr/0027-commonmark-owns-rule-rows-at-every-width.md).
- **Canonicalization preserves every leading BOM and code-region byte.**
  Outside code, CRLF and lone CR line endings normalize to LF. Rule rows and
  author-supplied U+0001–U+0003 are tracked out of band instead of borrowing
  control characters as placeholders.
- **Release gates validate the artifacts, not configuration text.** Cargo
  metadata validates retry exclusions, every cargo-dist phase is bracketed by
  `cargo metadata --locked` plus a byte-for-byte `Cargo.lock` guard, release
  write permission is scoped to the host job, and ShellCheck 0.11.0 is pinned
  in mise. Dependabot cooldown is 30/7/3 days for major/minor/patch updates.
- **Native, locked mise is the supported development and CI environment.**
  The repository now has one `mise.toml` plus `mise.lock`, with Rust, Bun,
  Node and every directly invoked tool fixed. CI has five static jobs that
  execute the same `just ci-{rust,web,repo,release,fuzz}` recipes as a local
  checkout. Correctness is established by compiler lints, official tools and
  real builds/tests rather than by parsing repository configuration.
- **Sentinel substitution moved into this crate, in one coordinate
  space.** `src/sentinel_stream.rs` is replaced by `src/constructs.rs`:
  the masked source is tiled here — bytes between constructs copied
  verbatim, each construct's byte range replaced by one of four PUA
  sentinels declared here rather than re-exported — and both walkers
  cursor over the resulting table in document order. The tiling is trusted
  by an exact test: it must equal, byte for byte, the sentinel text the
  parser produces from the same input. As a consequence an
  `IrInline::Aozora` / `IrBlock::Aozora` `span` slices *the source you
  passed in*, and is withheld (rather than published in a coordinate space
  no consumer holds) only for the documents the parser rewrites before
  lexing.
  [ADR-0023](docs/adr/0023-substitute-constructs-in-source-coordinates.md). (#157)
- **A construct's HTML comes from its own source run.** `src/fragment.rs`
  hands the run back to the parser, drops the paragraph wrapper an inline
  construct arrives in, and rebrands the class tokens (ADR-0011) — instead
  of calling the per-node renderer 0.5.0 removed. Fragments are memoised
  per run, and the rebrand is scoped to `class="…"` values, so text that
  happens to say `aozora-` (this repository's name does) survives
  verbatim. (#158)
- **`aozora-flavored-markdown-epub` enables the `theme` feature** and its
  vendored `assets/aozora-md-*.css` copy is gone; `compose` embeds the
  library's constants. The EPUB crate's own theme-coverage test is
  removed rather than rewritten — it was a substring search that a longer
  sibling class satisfied on its own. Coverage is settled once, in the
  library's tokenised sweep, and the drift gate now runs **both** ways: a
  class no theme styles fails, *and* a theme rule for a class the renderer
  cannot emit fails. (#160)
- **Docs slimmed and gated.** The mdbook site is retired (see *Removed*),
  `README.md` / `README.ja.md` are compressed to Quickstart + links (both
  languages stay), and the doc comments under `crates/*/src` no longer
  name an upstream-internal path — 21% of `src` was doc comments, much of
  it teaching imports that cannot compile.
  (#153)
- **The Tier-A predicate has exactly one definition.**
  `check_no_bare_bracket` now excepts `<code>` regions — a code element's
  body is the author's bytes verbatim per CommonMark §6.1, so notation
  *must* surface unwrapped there — and the four hand-rolled copies that
  disagreed about the same HTML all route through it. (#155)
- **Tier B (no PUA sentinel in the output) is unconditional**, and runs
  inside `assert_html_invariants` with the rest. Its caveat — "a source may
  contain U+E001, so gate on a clean parse" — was never true: a PUA
  codepoint an author types is replaced with U+FFFD. Gating on it had
  excused the check from every input that raised a diagnostic, which is the
  recovery path — where an unresolved construct is likeliest to survive.
  Un-gating it found the leak above on the first run. (#161)
- **`just changelog` prints a draft instead of overwriting `CHANGELOG.md`.**
  This file is written by hand — an entry has to say what broke and what to
  do about it, which no commit subject carries — and `git-cliff -o
  CHANGELOG.md` regenerated the whole thing, taking every explanation with
  it. The recipe now prints the Conventional-Commits draft to stdout to
  check the hand-written section against. (#161)

### Removed

- **The custom repository-policy layer and container development path.**
  The configuration-parsing tests for gate wiring, Action pins and lock
  binding are gone, together with Docker/Compose/devcontainer setup, the
  dev-image workflow, the setup composite action, and process-monitoring
  gates such as commitlint, Vale and source-shape scans. Rust, CLI, WASM,
  Playground, EPUB, Pages and publication outputs remain covered by their
  real compiler, test, package and release paths.
- **`upstream/comrak/`** — the vendored comrak fork (139 files, 2.2 MB) is
  gone; `comrak = "0.52.0"` now resolves from crates.io like every other
  dependency ([ADR-0024](docs/adr/0024-depend-on-crates-io-comrak.md),
  superseding ADR-0001 and ADR-0014). `cargo publish` used to strip the
  `path`, so consumers compiled against a graph local builds and CI never
  touched; there is one graph now, and `Cargo.lock` carries the `source` +
  `checksum` cargo verifies on every build. `cargo audit` / `cargo deny` /
  `dependency-review` / Dependabot see comrak directly for the first time.
  Going with it: `just upstream-diff`, `just upstream-sync`, `just
  audit-comrak` and their xtask implementations, the `upstream-diff` CI job
  and `audit-comrak` matrix leg, the lefthook hook, `.github/codeql/`, and
  the workspace `exclude`. **Finding:** the "verbatim" tree was not
  verbatim — a variable rename in `src/parser/inlines.rs` and a comment in
  `src/tests/sourcepos.rs` had drifted from the published crate, past a
  0-line diff budget whose gate never computed a diff. Both are
  behaviour-preserving; the measurement is recorded in ADR-0024.
- **`crates/aozora-flavored-markdown-book/`** — the mdbook site (10 files,
  791 lines) restated what the README and docs.rs already say. Getting
  started lives in the README, the API on docs.rs; mdbook drops out of CI,
  `Justfile`, `Dockerfile`, `docker-compose.yml`, the devcontainer and the
  labeler, and the `book` / `browser` image stages go with it. The Pages
  site serves rustdoc plus the playground. (#153)
- **`crates/aozora-flavored-markdown/src/ir/projection.rs`** (480 lines) —
  the AST mirror the collapsed IR no longer needs. (#156)
- **`aozora`'s `proptest` feature** from the dev-dependencies: the
  upstream generator crate is `publish = false` as of 0.5.0, so it is
  unreachable from crates.io. `proptest` never enters a published crate's
  runtime graph. (#155)
- **The four `docs/adr/000{4,6,7,8}-MOVED.md` stubs** — the ADR index
  already records that those numbers moved to the sibling repo. (#153)

### Fixed

- **Heading preprocessing no longer leaks Aozora layout directives.**
  `AlignEnd`, `Center` and `LineGothic` are dropped in headings; ruby whose
  reading is itself a directive keeps only its base text.
- **Masks and streaming coordinates now cover every source context.** Fenced
  code info strings are masked, nested/orphan closing directives are consumed
  once, and block source-line coordinates stay anchored to the caller's text.
- **EPUB validation rejects packages that cannot be conforming.** Explicitly
  empty spines, rooted/parent/symlink escapes and XML 1.0-forbidden characters
  fail with phase-specific diagnostic codes. TAB/LF/CR in XML attributes are
  emitted as numeric character references.
- **An unclaimed `［＃…］` run could publish a PUA sentinel.** A bracket run
  no notation claimed is hidden behind the directive wrapper and read as the
  author's own bytes — but a run may still *contain* a notation, as
  `［＃改［＃「あ」に傍点］］` does. The nested construct's sentinel was copied
  into the wrapper verbatim (U+E001 in the reader's HTML) and its table entry
  was left unconsumed, so the next notation in the document rendered this
  one's content. Inside a heading, where the run is dropped rather than
  wrapped, only the second half happened — silently. The run now writes each
  construct it swallowed back as source and consumes it either way.
  Found by making the Tier B invariant unconditional (below); predates this
  cycle. (#161)
- **An indented code block could publish a PUA sentinel.** A four-space
  indented line carrying ruby rendered as `<pre><code>` around a
  private-use codepoint — U+E001 in the reader's HTML. A *fenced* block
  never carries one
  (`code_block_mask` hides its triggers before the lexer runs), but an
  indented block is exactly the context that mask does not reproduce, and
  comrak makes one out of any four-space line — including one a `\r`
  created. A code block is now treated the way an inline code span already
  was: literal markdown whose sentinel is written back to the author's
  source. Predates this cycle and reproduced on 0.4.1. (#158)
- **Shift_JIS + CRLF documents with a decorative rule lost text.**
  `本文\r\n----------\r\n｜青梅《おうめ》` rendered the ruby's base text away and
  opened a `<div>` it never closed: CRLF folding takes a byte off every
  line, the parser's decorative-rule isolation adds one back, and the
  cancelled offset lands one byte before the block holding the construct.
  The recovery index now keeps one sorted candidate list for the whole
  source and takes the candidate of the right byte length *nearest* the
  reported offset that also parses back to the same construct shape. (#158)
- **A heading hint contaminated a Markdown heading.** A `#` heading whose
  text ended in `［＃「…」は大見出し］` put an `aozora-md-directive` span inside
  `<h1>`. A hint that reaches an inline walk has no paragraph to promote,
  so it renders as what it is. (#158)
- **`［＃地付き］` was styled as if it had content** — the rule gave an empty
  hook span `display: block; text-align: right`, which split the
  surrounding paragraph into two anonymous blocks and right-aligned
  nothing. Both themes now use the `p:has(> …)` shape the sibling markers
  already use, with logical `text-align: end` / `padding-inline-end`, so
  the container form works in vertical mode too. (#160)
- **`StreamingIrBuilder` never drained at end of document.**
  `render_blocks_to_ir("［＃ここから２字下げ］\n\n本文")` returned a block whose
  `html` was `</div>` and whose `ir` was empty — the IR stream opened a
  container it never closed while the HTML stream of the same call
  balanced. `finish()` is the drain its whole-document sibling always had.
  (#156)

## [0.4.1] - 2026-06-21

The project was **renamed from `afm` to `aozora-flavored-markdown`** for its
first crates.io release (ADR-0015). The
descriptive crate name (`aozora-flavored-markdown`,
binary `aozora-flavored-markdown`) is decoupled from the short, stable
`aozora-md` brand used for the rendered HTML CSS classes (`aozora-md-*`), env
vars (`AOZORA_MD_*`), Docker tags, and the `aozora-md.diagnostics.v1`
diagnostics schema — see
[ADR-0016](docs/adr/0016-rebrand-to-aozora-flavored-markdown.md). The version is
aligned with the sibling `aozora` crate at 0.4.1, and `Options::afm_default()`
is replaced by `Options::default()` (the dialect preset is now the `Default`).

### Added

- **`just setup`** — one-shot first-time setup (build the dev image,
  install git hooks, run `just doctor`, run the tests); idempotent, so
  it doubles as a "get back to green after a pull" command.
- **`just snapshot-review` / `just snapshot-accept`** — drive `cargo
  insta` for the snapshot tests that `just test` (nextest) leaves
  pending but does not apply.
- **`just prop-seed SEED`** — replay a single proptest failure from the
  seed nextest prints on its FAIL line.
- **Grouped `just` menu** — every recipe carries a `[group(...)]`, so a
  bare `just` lists recipes by area (build / test / lint / docs / …);
  the destructive `nuke` is now guarded behind `[confirm]`.
- **Contributor `Troubleshooting` + `Your first change` guide** in
  `CONTRIBUTING.md` — the common Docker / cargo-lock / sccache /
  rust-analyzer-in-container / WSL snags with their fixes, and a six-step
  first lap through the inner loop.
- **PR area auto-labeler** (`actions/labeler`) — tags a PR `area: cli` /
  `markdown` / `wasm` / `book` / `dev` / `ci` / `documentation` from the
  paths it touches. Non-blocking and not a required check.
- **stdin input for `aozora-flavored-markdown render` / `aozora-flavored-markdown check`** — pass `-` as the input
  path to read the document from standard input (`cat in.md | aozora-flavored-markdown render
  -`), honouring `--encoding sjis` on the piped byte stream. The `-`
  placeholder was already documented but previously errored.
- **`aozora-flavored-markdown render -o <file>` / `--output`** — write HTML straight to a file
  instead of redirecting stdout (`-` keeps stdout); strict failures write
  nothing.
- **`--color auto|always|never`** for error reports — `auto` honours
  `NO_COLOR` and `CLICOLOR_FORCE` and otherwise follows the stderr TTY; an
  explicit `always`/`never` wins over the environment.
- **`-v`/`-q` verbosity flags** — set the default log level without
  reaching for `RUST_LOG` (which still overrides them when set).
- **`--format human|json` machine-readable diagnostics** — `json` emits a
  stable `aozora-md.diagnostics.v1` envelope (`code` / `severity` / `source` /
  `message` / `span` / `line` / `column`) for editors, CI gates, and LSP
  bridges. `check` writes it to stdout (pipe into `jq`); `render` keeps
  stdout for HTML and writes JSON to stderr. Schema and stability are
  pinned by [ADR-0012](docs/adr/0012-diagnostic-json-output-schema-and-stability.md).
- **`aozora-flavored-markdown completions <shell>`** — generate a completion script for bash,
  zsh, fish, powershell, or elvish.
- **`--help` now shows an `EXAMPLES` section** covering stdin, `-o`,
  strict JSON checks, and completion install.
- **Release archives now bundle the shell completions and the `aozora-flavored-markdown.1`
  man page** (under `completions/` and `man/`). Regenerate the committed
  assets with `just dist-assets`; `just ci` drift-checks them.
- **Runnable doctests on the `aozora-flavored-markdown` public API** — every public
  entry point (`render_to_string`, `render_to_ir`, `render_blocks_to_ir`,
  `serialize`, `Options::default`, `html::render_to_string`) now
  carries a compiled, asserted example. `just test-doc` is wired into
  `just ci` and a CI job so the examples can never silently rot.
- **crates.io publication readiness** — `aozora-flavored-markdown` and `aozora-flavored-markdown-cli` are now
  publishable to crates.io (verified via `cargo publish --dry-run`). A manual
  `publish-crates.yml` workflow publishes the two-crate ladder
  (`aozora-flavored-markdown` → `aozora-flavored-markdown-cli`). Policy is captured in
  [ADR-0014](docs/adr/0014-comrak-vendoring-upgrade-policy.md) and
  [ADR-0015](docs/adr/0015-crates-io-publication-and-semver.md).

### Changed

- **`just ci` is faster without dropping a gate.** The non-compile gates
  (`deny` / `audit` / `book-build`) run in a background lane that
  overlaps the compile lane, and the redundant `check` step plus the
  duplicate `fmt-check` / `typos` / `strict-code` runs that `lint`
  bundled are removed. Same 18 gates; warm-cache wall-clock ~35 s
  → ~23 s. The compile lane stays sequential (one shared cargo target
  lock).
- **`just` recipes are container-aware — no more docker-in-docker.** Run
  inside the dev/ci image (a `just shell`, a VS Code devcontainer, or a
  GitHub Codespace, where `AOZORA_MD_IN_CONTAINER=1`), recipes invoke their
  tool directly instead of nesting a second `docker compose run`. The
  devcontainer now targets the full-tool `ci` image, so the complete
  `just ci` runs inside Codespaces, and `postCreateCommand` installs the
  git hooks.
- **`just strict-code` now permits reasoned `#[allow(..., reason = "…")]`**
  (Rust 1.81+) and forbids only bare `#[allow]` — matching the
  `clippy::allow_attributes_without_reason` lint the workspace already
  enforces, which the previous blanket ban contradicted. It also adds an
  `.expect()` regression tripwire over `aozora-flavored-markdown` source (baseline 8)
  and the `cargo-deny` `allow-wildcard-paths` policy for path-only
  internal dev-deps.
- **CI collapses to a single `ci-success` required check.** A
  `dorny/paths-filter` `changes` job skips the Rust compile/test/lint
  matrix on docs-only PRs, and a terminal `ci-success` aggregator gates
  on every job's result, so branch protection requires just `ci-success`
  (plus CodeQL) — adding or renaming a job no longer needs a settings
  change. The `lint` job is now a parallel matrix (fmt-check / clippy /
  typos / strict-code), and the completions/man drift check
  (`dist-assets-check`) and doctests (`test-doc`) are first-class CI jobs.
- **Public IR enums `IrBlock` / `IrInline` are now `#[non_exhaustive]`**
  ([ADR-0013](docs/adr/0013-public-ir-enums-non-exhaustive.md)) so a future
  青空文庫 notation can be added as a new variant without breaking external
  Rust `match`es. The serde/JSON contract is unchanged; the variant-
  completeness witnesses moved into `aozora-flavored-markdown` (the owning crate). The
  ADR index now also lists the previously-missing ADR-0012.
- **`aozora` is now a crates.io registry dependency** (`0.4.1`) instead of a
  git-rev pin — required for aozora-flavored-markdown to be publishable. `comrak` keeps its
  vendored path locally but publishes against the identical registry `0.52.0`.
  `cargo xtask new-adr` now renders `docs/adr/0000-template.md` (full MADR
  section set) instead of a hard-coded subset.
- **MSRV raised to Rust 1.96.0**, aligning `rust-toolchain.toml`, the workspace
  `rust-version`, `clippy.toml`, the mise/CI pins, and the README badge with the
  `rust:1.96.0` dev-image base (which previously forced a redundant 1.95.0
  rustup install). Dependabot now ignores the Docker `rust` base so a bump
  can't silently advance the MSRV.

### Fixed

- **`aozora-flavored-markdown --strict` now exits with code 2**, distinct from generic
  failures (code 1), matching the documented exit-code table; its
  `--help` text now describes "any lexer diagnostic" instead of the
  stale "unknown annotation" wording.
- **Dead `CLAUDE.md` link in the README** (the file is personal and not
  committed); readers are pointed at `CONTRIBUTING.md` and `docs/adr/`.

### Internal

- **Aozora sentinel splicing moved from byte-stream post-processing to
  AST-level mutation** (`crates/aozora-flavored-markdown/src/post_process.rs`
  → `ast_splice.rs`). The splicer now mutates comrak's typed AST in place
  (replacing each sentinel with a `NodeValue::Raw` node) and lets
  `comrak::format_html` emit the final HTML in one pass, rather than
  re-scanning a flat HTML byte stream with the former multi-pass Cow
  pipeline. This supersedes the multi-pass design described under
  [0.4.0] and **withdraws** the fully-fused aho-corasick follow-up noted
  there — the separate secondary passes it would have fused no longer
  exist as distinct scans.

## [0.4.0] - 2026-06-14

### Added

- **Playground polish (round 2):** light/dark colour-scheme toggle
  (#57); unified layout skeleton (breakpoint + footer), scaled tokens,
  and a right-anchored vertical preview (#58, #59); selection-wrap
  commands that wrap the selection in aozora notation (#60); a notation
  reference modal (#61); and a source-coordinate WASM API exposing lexer
  offsets to the editor (#62).
- **Build provenance attestation** on release artefacts via
  `actions/attest-build-provenance`: every archive is verifiable with
  `gh attestation verify <archive> --repo P4suta/aozora-flavored-markdown`, no certificates.
  (#64, #66)
- **aozora pin advanced to the tagged v0.4.0 release** (`df0f64b`) — aozora-flavored-markdown
  v0.4.0 builds against the provenance-attested aozora v0.4.0.
- **Browser playground at `/aozora-flavored-markdown/playground/`** — Solid + Vite frontend
  over `crates/aozora-flavored-markdown-wasm`, deployed to
  <https://p4suta.github.io/aozora-flavored-markdown/playground/>. CodeMirror 6 editor with
  a small Aozora syntax overlay (`｜《》`, 連結ルビ, `［＃...］`,
  `※［＃...］`); 縦書き / 横書き toggle that swaps stylesheets without
  re-rendering; seven curated example snippets; URL-shareable state
  via `lz-string`; diagnostics drawer surfacing every lexer warning /
  error. CSS imported from the existing `crates/aozora-flavored-markdown-book/theme/` —
  single source of truth, no duplication. (#27)
- **`just check`** — `cargo check --workspace --all-targets` for a
  sub-second warm "does it still compile" gate. (#27)
- **`just doctor`** — one-screen environment audit (docker images,
  named volumes, aozora SHA pin agreement, playground prerequisites)
  with explicit OK / `--` / `!!` markers so the user never wonders
  whether the local env is broken. (#27)
- **`just ci` fail-fast progress markers** — every step prints
  `[HH:MM:SS] →→→ STEP n/N: name` start banner + `✓ name (took Ns)`
  or `✗ name FAILED (after Ns)` trailer; first failure halts the run
  with that step's exit code. 17 ordered steps; typos / fmt-check /
  upstream-diff / strict-code surface in <10 s if broken. (#27, #30)
- **`just wasm-build-dev`** — fast `--dev`-profile wasm build for
  inner-loop playground iteration (4-6× faster than the release
  pipeline; output is not for shipping). (#27)
- **`just doc`** — `cargo doc --workspace --no-deps
  --document-private-items`; exercises the
  `broken_intra_doc_links = "deny"` workspace lint which no other
  `cargo` pass runs. Slotted as a Phase-2 CI gate so dead doc-links
  surface on the PR rather than post-merge in `docs.yml`. (#30)
- **`just aozora-bump SHA`** — `cargo xtask aozora-bump <sha>`
  rewrites every `aozora-*` git rev pin in workspace `Cargo.toml`
  and refreshes `Cargo.lock` against the same six packages in one
  pass. Idempotent and validates the SHA shape before any FS
  mutation. (#32)
- **`fuzz` Dockerfile stage** — dev superset that adds nightly +
  `cargo-fuzz` + `cargo-udeps`. Used only by `just udeps` /
  `just fuzz*` / `just coverage-branch` via a new `_fuzz` Justfile
  helper, so the plain `dev` image stays slim. (#27)

### Changed

- **Cold dev Docker image build dropped from 30+ min to 2m 24s**
  (12× faster). The cargo-tools layer that previously compiled 14
  cargo helpers from source now uses `cargo-binstall` to fetch
  prebuilt GitHub Release binaries; the install graph is re-tiered
  by churn frequency so a single bump no longer invalidates the
  whole layer. Image disk usage falls ~1 GB once nightly +
  cargo-fuzz / cargo-udeps move into the `fuzz` stage. (#27)
- **sccache pinned to 0.10.0** — 0.15+ aborts inside cargo's rustc-
  wrapper subprocess with "SCCACHE_GHA_ENABLED must be 'true', 'on',
  '1', 'false', 'off' or '0'" even when the env is unset, blocking
  every cargo invocation in the dev image. Hold the downgrade until
  upstream fixes. (#27)
- **`aozora-*` workspace deps pinned to a commit SHA** (currently
  `40af7769b0f81802b1bf2470f2e535e78c765269`) instead of
  `branch = "main"` so `cargo update` no longer silently advances
  the borrowed-AST surface mid-PR. Bump cadence is one PR per
  intentional sync, automated by `just aozora-bump SHA`. (#27, #32)
- **GitHub Actions `ci.yml` is now fail-fast layered**: a `check`
  job (`cargo check --workspace`) is the Phase-1 gate, and
  `build-and-test` / `spec` / `coverage` / `audit` / `doc` all
  declare `needs: check`. A syntax error surfaces in 1-2 min instead
  of after a 10-min `build && test` cycle. `setup-dev-image`
  composite action wires `mozilla-actions/sccache-action@v0.0.9` and
  forwards SCCACHE_GHA_ENABLED / ACTIONS_CACHE_URL /
  ACTIONS_RUNTIME_TOKEN into the compose services so every matrix
  job shares a hot cross-run cache. (#27, #30)
- **Playground toolchain migrated from npm to bun 1.3.14**. bun 1.3
  ships a text lockfile (`bun.lock`) by default, so diff-able
  lockfile reviews are preserved; `playground-install` +
  `playground-build` together dropped ~30 % (14 s → 9.3 s warm).
  bun lives inside the dev image (`oven-sh/bun` GitHub Release
  binary, ADR-0002 preserved). Node 22 stays in the dev image for
  the `book` / `browser` services that still consume npm tooling. (#29)
- **Playground bundle split into vendor chunks**. The previous
  monolithic 803 kB / 224 kB-gzipped `index.js` is now four files:
  `index.js` (34 kB / 11 kB gzip — app only), `vendor-codemirror.js`
  (678 kB / 203 kB), `vendor-solid.js` (13 kB / 5 kB),
  `vendor-lz-string.js` (8 kB / 2 kB). Browsers fetch in parallel;
  CodeMirror chunk survives every app-code deploy via the immutable
  content-hash URL. (#31)
- **`fuzz-quick` / `fuzz-deep` / `fuzz-marathon`** wrapped with
  `timeout --kill-after=10s Ns` as a hard backstop so a libFuzzer
  hang returns control to the caller in known time. (#27)

### Fixed

- **Prevented a deep-nesting stack overflow** and hardened the public
  API surface + CI for release (#65).
- Repaired 4 broken intra-doc links in `aozora-flavored-markdown` that turned
  `cargo doc --workspace` into a hard failure under the
  `broken_intra_doc_links = "deny"` workspace lint, blocking the
  Pages deploy. (`tests::indent_of_four_spaces_disables_the_fence`
  in `code_block_mask.rs`, `crate::ir::walker` in `ir/projection.rs`,
  `AozoraNode` in `ir/mod.rs`, `crate::post_process` × 2 in
  `sentinel_stream.rs`.) (#28)

### Changed (breaking)

- **`IrInline::Range` / `IrBlock::Range`** are now
  `{ start: Position, end: Position }` carrying 1-based line / column
  coordinates straight from comrak's `Sourcepos`. The previous
  `{ from: u32, to: u32 }` was a pseudo-byte offset
  (`(line-1)*1024 + (col-1)`) that silently broke under multi-byte
  CJK content. JS-side consumers (aozora-flavored-markdown-obsidian's CodeMirror bridge)
  no longer need to redo UTF-8 byte arithmetic. TS contract on the
  consumer side must be updated to match.
- **`pub use aozora_pipeline::*_SENTINEL`** from `aozora_flavored_markdown` is
  removed in favour of the aozora-md-side wrapper module
  `aozora_flavored_markdown::sentinels` (`INLINE` / `BLOCK_LEAF` / `BLOCK_OPEN` /
  `BLOCK_CLOSE`). The aozora-flavored-markdown public API no longer names sibling-crate
  constants, so upstream renames surface in this module rather than
  breaking every consumer.
- **`Options<'c>` lifetime parameter** removed. `Options` now wraps
  `comrak::Options<'static>` and carries no caller-side lifetime,
  collapsing the 3-arg generic on every public entry point.

### Changed

- **`crates/aozora-flavored-markdown/src/post_process.rs`** redesigned around
  `Cow<'_, str>` so the three secondary passes
  (`rebrand_aozora_classes_to_afm`, `wrap_orphan_brackets_in_place`,
  `balance_inline_tags_in_paragraphs`) borrow the previous pass'
  output on the common path and only allocate when their trigger
  pattern is present. Splicer Pass 1 is now the only mandatory
  allocation; Passes 2-4 are zero-allocation no-ops on well-formed
  input. The Cow threading already removes the redundant
  *allocations* on the common path; a fully-fused 1-pass
  aho-corasick splicer was noted as a possible follow-up (later
  withdrawn when the byte-stream post-processor was replaced by the
  AST-level splicer — see [Unreleased]).
- **`splice_into`'s `<p>` matcher** now matches both `<p>` and
  `<p attr=…>` openings (taking the earliest of the two). Previously
  only `<p>` was matched, so source-line-anchor injection
  (`<p data-aozora-md-source-line="N">`) could leak through the splicer
  unspliced. Fixes a long-standing asymmetry against
  `balance_inline_tags_in_paragraphs:127` which already handled both
  forms.
- **`source_line_anchors`** rewritten as `format_root_with_anchors`
  + `inject_anchor_into_first_open_tag`: comrak's `format_html` is
  invoked per top-level block and the anchor attribute is prepended
  to the first opening tag of each block's HTML chunk. The 226-line
  attribute-aware tag walker (with depth tracking, void-tag
  detection, attribute-value `>` handling) is gone; the new
  implementation is ~155 lines and self-contained.
- **`code_block_mask`** rewritten with `Cow<'_, str>`: when the
  source contains no fence markers (or already contains the mask
  char), the masking pass returns `Cow::Borrowed(input)` and skips
  allocation entirely. CRLF line breaks are now preserved through
  the mask/unmask round trip.
- **`ir.rs` (1318 L)** split into a `crates/aozora-flavored-markdown/src/ir/`
  module: `types.rs` (public IR enum/struct definitions),
  `projection.rs` (pure conversion helpers and enum→string
  mappers), and `mod.rs` (the stateful walker + streaming builder).
- **`IrWalker` lifetime parameters** collapsed from three (`<'c, 'src,
  'a>`) to one (`<'src>`) plus per-method `<'a>` for comrak's
  invariant `Node` lifetime. The shared `SentinelCursor` now owns
  its `Vec<NodeRef>` rather than borrowing a slice, removing the
  slice-lifetime entirely from the walker's signature.
- **`crates/aozora-flavored-markdown/src/sentinel_stream.rs`** (renamed from
  `sentinels.rs`) consolidates `walk_text_only_descendants` and
  `for_each_text_descendant` into a single
  `visit_text_leaves<F>(node, mode, f)` returning
  `core::ops::ControlFlow<()>` for early-exit. The two prior
  helpers are thin convenience wrappers around it.
- **`render_to_string` / `render_to_ir`** now delegate to a shared
  `drive_pipeline<F, T>` helper that owns the lex / parse / format
  / splice sequence. Each public entry point is ~5 lines of
  projection on top.

### Internal

- **`crates/aozora-flavored-markdown-test-support/`** new sub-crate holds the
  test predicates and invariant helpers that previously lived in
  `aozora-flavored-markdown::test_support` (1426 L behind `#[doc(hidden)] pub
  mod`). The hack is removed and the helpers are no longer part of
  `aozora-flavored-markdown`'s public surface; the integration tests pull them
  in via `[dev-dependencies]` instead.
- **`saturating_u32`** centralised in `sentinel_stream` (was
  duplicated in `ir.rs` and `lib.rs`).
- **`AOZORA_MD_CLASSES`** drift detection moved into the existing
  `css_class_contract.rs` integration test; the manual mirror in
  `test_support` carries a comment cross-referencing the sibling
  `aozora-render` source. (No build.rs codegen — the test is the
  drift detector.)
- Coverage measured at 97.86% regions across 283 tests; the 96%
  floor holds.

### Added

- **Aozora-side IR projection.** `aozora_flavored_markdown::render_to_ir` and
  `render_blocks_to_ir` now emit every Aozora variant
  (`Ruby`, `DoubleRuby`, `Bouten`, `Tcy`, `Gaiji`, `Annotation`,
  `Container`, `PageBreak`, `SectionBreak`) into the typed
  `IrDocument`, replacing the v0.1 markdown-only walker. Heading
  hints (`［＃「X」は大見出し］`) promote their host paragraph to
  `IrBlock::Heading` directly. `IrInline::Image` is also added so
  CommonMark images survive the IR boundary.
- **`aozora_flavored_markdown::ir::StreamingIrBuilder`.** Public stateful
  per-block IR builder that threads the sentinel-stream cursor
  across `walk_block` calls. aozora-flavored-markdown-obsidian's chunked-cancellation
  path uses this to checkpoint between blocks without losing
  Aozora projection lockstep.
- **`crates/aozora-flavored-markdown/src/sentinels.rs`.** New shared module
  that owns `BlockSentinelKind`, `is_sentinel_char` (subtraction-
  based fast check), `sole_block_sentinel`,
  `flatten_registry_in_source_order`, and `SentinelCursor`
  (peek / next / advance / position primitive). Both the HTML
  splicer and the IR builder consume from this single source of
  truth.
- **ADR-0011 — brand boundary CSS class rewrite.** Codifies the
  decision to keep the `aozora-*` → `aozora-md-*` HTML rewrite on the
  aozora-flavored-markdown side rather than parameterising upstream `aozora-render`,
  preserving the one-way `aozora-flavored-markdown → aozora` dependency direction.
- **`cargo xtask upstream-sync <tag>`** is now implemented as a
  pure tree-replace: shallow-clones the upstream comrak tag, drops
  the old vendored tree, copies the new source over, and updates
  `COMRAK_SHA`. The `aozora-md-side` metadata (`COMRAK_SHA`,
  `UPSTREAM_DIFF.md`) is preserved across the wipe.

### Changed (breaking)

- **`IrInline::DoubleRuby`** drops the always-empty `outer` and
  `inner` string fields. The shape is now
  `{ base: Vec<Self>, range }` matching upstream's `DoubleRuby`
  payload exactly.
- **`RenderedBlock.ir`** is now `Vec<IrBlock>` rather than a
  single `IrBlock`. This removes the `ThematicBreak` placeholder
  hack for comrak constructs without a v0.2 IR projection
  (definition list, footnote ref, raw HTML) and lets paired-
  container drains carry through the streaming boundary.
- **`AnnotationKind::Unknown`** projects to
  `Some("unknown")` in `IrInline::Annotation::resolved` instead
  of `None`. Future `#[non_exhaustive]` variants of
  `AnnotationKind` upstream will surface as `None`, so consumers
  can distinguish "the parser tried and gave up" from "aozora-flavored-markdown
  doesn't know about this kind yet".
- **`pub use comrak::Options as ComrakOptions`** removed from
  the public surface. Consumers who tweak comrak's options
  directly should import comrak themselves; the aozora-flavored-markdown public API
  no longer pins comrak's version into its surface.

### Changed

- **`aozora-flavored-markdown-wasm` diagnostic projection** now uses
  `Diagnostic::severity` / `source` / `code` plus the `Display`
  impl, replacing the hardcoded `"info"` level and `"{d:?}"`
  debug-format message. Wire shape is
  `{ level, source, code, message }`.
- **`aozora_flavored_markdown::post_process`** now consumes the shared
  `SentinelCursor` instead of carrying its own cursor fields.
- **`UPSTREAM_DIFF_BUDGET_LINES`** in `xtask` lowered from 200
  to 0, matching ADR-0001 v0.2.4.

### Removed

- **`xtask` deferred sub-commands** (`corpus-refresh`, `corpus-test`,
  and the `deferred()` helper) — moved to the sibling `aozora`
  repo per ADR-0010.
- **`aozora-corpus`** dropped from `[workspace.dependencies]`
  (not used by any member crate after ADR-0010).
- **`aozora_flavored_markdown::ir::walk_block_public`** removed in favour of
  `StreamingIrBuilder` so multi-block streaming consumers can't
  accidentally restart the cursor between blocks.

### Documentation

- **aozora-flavored-markdown-book** refreshed top-to-bottom: `library.md` rewritten
  with current `aozora_flavored_markdown` API examples (3-tier:
  `render_to_string`, `render_to_ir`, `render_blocks_to_ir`,
  plus `serialize`); `arch/pipeline.md` replaced with the
  current 3-layer + shared-cursor architecture; `arch/adr.md`
  expanded to the full 0001-0011 set with current statuses;
  `ref/api.md` re-targeted at `aozora_flavored_markdown` / `aozora_flavored_markdown_wasm` and
  the sibling `aozora-*` crates.
- **CONTRIBUTING.md** rewritten around the post-v0.2.0 glue-
  layer responsibility. The 5-step "How to add an invariant"
  flow is now aozora-md-internal; new 青空文庫 notations
  redirect to the sibling repo.
- **README.md / README.ja.md / SECURITY.md / PR template** —
  stale `aozora-md-parser` / `aozora-md-lexer` / `aozora-md-syntax` / `aozora-md-encoding`
  references and the obsolete `200-line` budget removed.
- **ADR-0003** (aozora-md-parser architecture) and **ADR-0005**
  (paired-block container hook) statuses updated to
  `Superseded by ADR-0010` / `Superseded by ADR-0008` with
  v0.2.0 / v0.2.4 historical context appended.
- **Stale code comments** in `aozora_flavored_markdown::lib`,
  `aozora_flavored_markdown::examples::{render-utf8,render-sjis}`, and
  `xtask::spec_refresh` updated to match current crate names.

### Internal

- Coverage measured at 97.23% regions across 273 tests; the 96%
  floor holds. New unit tests pin every non-exhaustive enum
  match arm (`bouten_kind_str`, `section_kind_subtype`,
  `container_subtype`, `container_indent_level`,
  `annotation_kind_resolved`, `bouten_position_str`) so future
  upstream additions surface immediately.
- `IrWalker` uses move semantics for `OpenContainer` children
  (no clone at close), and `ParaScan` runs a single descent over
  each paragraph to compute `total_sentinels` / `first_heading_hint`
  in one pass.

## [0.3.0] - 2026-04-30

Major release. Tracks aozora `0.2.6` (released same day) and locks in
the **brand boundary** between `aozora-*` (pure 青空文庫記法) and
`aozora-md-*` (Aozora Flavored Markdown).

### Changed (breaking)

- **Bumped pinned `aozora-*` crates from v0.2.5 → v0.2.6.** Picks up
  upstream PR #4 (aozora-md-* → aozora-* class prefix flip + gaiji
  `data-codepoint` / `data-description` attrs + wasm-pack pipe fix),
  PR #5 (docs overhaul / driver build integration / ADR cleanup),
  PR #6 (pymodule rename for maturin).
- **Brand boundary in `post_process::splice_aozora_html`.** The
  upstream `aozora-render` crate now emits `aozora-*` CSS classes;
  aozora-flavored-markdown's HTML output continues to carry the `aozora-md-*` brand
  (Aozora Flavored Markdown). A new
  `rebrand_aozora_classes_to_afm` post-process pass rewrites every
  `aozora-*` class token in the spliced HTML to its `aozora-md-*`
  counterpart. Touches only `class="..."` attribute values; data-*
  attributes, link targets and text bodies are preserved verbatim.

### Internal

- `aozora_parity` test runner switched to a stem-based histogram
  (`class_stem_histogram(html, prefix)`) so the differential against
  `aozora-render` compares the family of recognisers fired, not the
  brand prefix.
- Coverage measured at 98.77 % regions across 179 tests, no ignored
  cases, all eleven integration tests + four examples building
  against the new public API.

## [0.2.6] - 2026-04-30

Closes every v0.2.5 follow-up by **resolving** them (no `#[ignore]`, no
floor lowering). 179/179 tests pass with zero gates; coverage is back
above the 96 % regions floor. The `block_structure_interaction::fenced
_code_block_*` test that v0.2.5 marked as a known limitation is now a
true assertion.

### Added

- **CommonMark code-block-aware lex pre-pass.** New
  `code_block_mask` module hides 青空文庫 trigger characters
  (`｜《》［］※〔〕「」`) inside fenced code blocks before
  `aozora-lex` sees the source, then unmasks them in the rendered
  HTML. Aozora markup inside ` ``` ` / `~~~` fences now flows through
  to `<pre><code>` literally — the formerly `#[ignore]`d
  `fenced_code_block_preserves_aozora_markup_as_code` is unblocked.
- **Defensive Tier-A guard** in `post_process::splice_aozora_html`:
  any bare `［＃…］` that the upstream lexer fails to claim (e.g.
  empty annotation `［＃］` nested inside a baseless ruby pair `《》`,
  which `aozora-lex` Phase 3's replay path drops) is auto-wrapped in
  an `aozora-md-annotation` hidden span. The Tier-A canary now holds for
  every input the property tests can generate, including the three
  pathological seeds (`［＃`, `］［＃`, `《［＃］》`) that v0.2.5
  could not satisfy.
- **lib + post_process unit tests** pinning every formerly-uncovered
  region: `Options::gfm_only`, the `contains_bare_bracket` helper,
  malformed `</p>` recovery, exhausted-registry block sentinel,
  block-sentinel-inside-inline drop, HeadingHint target HTML escape.

### Changed

- **Coverage gate restored to 96 %.** `_COV_FLOOR = 96` (was 93 in
  v0.2.5), with `test_support.rs` excluded from the measurement
  because it is `#[doc(hidden)] pub mod` test-helper code, not
  production. Production coverage measures **99.26 %** across
  `lib.rs` (100 %), `html.rs` (100 %), `post_process.rs` (98.6 %),
  and `code_block_mask.rs` (98.97 %).
- **CLAUDE.md** Open-follow-ups section reframed: Aozora-only
  fixtures (`spec-aozora` / `spec-golden-56656` / `corpus-sweep`)
  now correctly point to the sibling `P4suta/aozora` repo (they
  moved there at v0.2.0 — aozora-flavored-markdown only keeps the CommonMark/GFM spec
  runners).
- **ADR-0001** carries a v0.2.4 status update documenting the diff
  budget collapse (200 → 0).
- **`.claude/settings.local.json`** added to `.gitignore` per the
  per-project Claude Code convention.

### Internal

- aozora-tools (225 tests + ADRs) and aozora-flavored-markdown-epub (placeholder) verified
  unchanged after this release: the only modifications live in
  aozora-flavored-markdown's own surface plus tooling, so the sibling repos pass
  unchanged.

## [0.2.5] - 2026-04-30

Closes the v0.2.5 follow-up list from v0.2.4. Every integration test
and example is now back on the new public API; `just test` runs the
full 159-test suite.

### Added

- **Heading-hint promotion.** A paragraph carrying a `HeadingHint`
  inline sentinel (`［＃「X」は大見出し／中見出し／小見出し］`) now
  renders as `<h{level}>{target}</h{level}>`. `post_process` peeks at
  the registry from inside the paragraph, rewrites the wrapper, and
  consumes the hint's siblings so indent / annotation classes don't
  leak into the heading body.
- **Stack-balanced container splice.** `BlockOpen` paragraphs push
  onto a `Vec<ContainerKind>`; `BlockClose` paragraphs pop. Open-less
  closes are silently dropped, and any container left open at end-of-
  document is auto-closed so the Tier-D HTML tag-balance invariant
  holds for malformed inputs too.
- **Family-suffix CSS class recognition.** `is_recognised_afm_class`
  now accepts any `<base>-<suffix>` where `<base>` is in
  `AOZORA_MD_CLASSES`, covering both numeric modifiers (`aozora-md-indent-2`,
  `aozora-md-container-indent-3`) and slug modifiers (`aozora-md-section-break-
  choho`, `aozora-md-bouten-goma`-suffixed forms) without expanding the
  pinned list per variant.

### Re-enabled

- All 11 integration tests are back in CI:
  `commonmark_spec` (652 examples), `gfm_spec` (extension-tagged 0.29
  spec), `css_class_contract`, `html_well_formed`,
  `block_structure_interaction` (1 case `#[ignore]`d — fenced code
  block contents still need a CommonMark-aware lex skip),
  `paired_container`, `heading_promotion`, `property_html_shape`,
  `property_heading_integrity`, `post_process_invariants` (redrafted
  against HTML; the AST helpers it used are gone), `aozora_parity`
  (redrafted around `aozora_lex` + `aozora_render`).

### Internal

- `splice_aozora_html` is now paragraph-aware *and* still inline-aware
  outside `<p>...</p>` boundaries (so headings, list items,
  blockquotes, table cells keep getting their inline sentinels
  resolved). The two-stage loop is documented in the module header.
- `SpliceState` replaces the previous `IntoIter` plumbing so
  `process_paragraph` can `peek()` ahead before deciding between
  heading promotion and a regular inline pass.

## [0.2.4] - 2026-04-30

This release follows aozora `0.2.5` and completes the borrowed-AST
migration that began with the v0.2.0 split. aozora-flavored-markdown is now a thin
glue crate that composes a vanilla comrak with `aozora-render` /
`aozora-lex` on a string-level sentinel substitution; comrak no longer
carries any Aozora-aware patches.

### Changed

- **comrak vendored tree is now 100 % verbatim v0.52.0.** The historical
  ~22-line patch surface (`NodeValue::Aozora` variant + `render_aozora`
  `fn` pointer + arms in cm/xml/html/sourcepos) has been removed, and
  the ADR-0001 200-line diff budget is now **0 lines**. Upstream syncs
  no longer need patch reapplication.
- **aozora-flavored-markdown switched from owned-AST AST surgery to HTML
  post-processing.** The pipeline is now `aozora_lex::lex_into_arena` →
  `comrak::parse_document` (against the normalized text) →
  `comrak::format_html` → in-process sentinel substitution that calls
  `aozora_render::render_node` for every PUA-sentinel hit. See the
  module-level docs in `crates/aozora-flavored-markdown/src/post_process.rs`.
- **Public API simplification.** The arena-coupled
  `parse(arena, input, options) -> ParseResult` and
  `serialize_from_artifacts(...)` entry points are replaced by
  `render_to_string(input, options) -> Rendered { html, diagnostics }`
  and `serialize(input) -> String`, both stateless and arena-free.
  `html::render_to_string` (no-arg shim returning `String`) is kept for
  back-compat.

### Removed

- `aozora-parser` dependency (the crate was retired in aozora 0.2.0
  Phase F.1).
- `aozora-lexer` direct dependency (aozora-flavored-markdown only consumes
  `aozora-lex` now; the underlying `aozora-lexer` is pulled in
  transitively).
- `comrak::Options::extension::render_aozora` and `serialize_aozora`
  `fn` pointers.

### Internal

- 17 integration tests (`tests/*.rs`) and 4 examples were placed behind
  `#![cfg(any())]` for this release; the borrowed-AST rewrite of those
  fixtures is tracked under task #10 of the v0.2.4 release plan and
  will land in v0.2.5. Lib-internal `#[cfg(test)] mod tests` plus the
  HTML-invariant unit tests in `test_support` (76 tests total) all pass.

## [0.1.0] - TBD

Initial public preview release of Aozora Flavored Markdown.

### Added

#### Parse pipeline

- Seven-phase pure-functional lexer (`aozora-md-lexer`) — sanitize / events /
  pair / classify / normalize / registry / validate — that resolves
  Aozora notations before the CommonMark parser runs (ADR-0008).
- Post-process AST splice in `aozora-md-parser` — inline, block-leaf, and
  paired-container surgery that reinstates Aozora nodes after vanilla
  comrak parsing.
- Round-trip serializer — inverts the lexer via sentinel registry
  substitution in one O(n) byte sweep.

#### Aozora notations

- Ruby (`｜…《…》` and implicit-delimiter forms), including nested
  gaiji/annotation segments.
- Bouten (sideline emphasis), 11 variants including `《《…》》` and the
  `［＃「X」に傍点］` forward-reference form.
- Tate-chu-yoko (`［＃縦中横］`).
- Indentation — 字下げ / 地付き / 地寄せ / 複合字詰め.
- Headings — 大見出し / 中見出し / 小見出し / 窓見出し.
- Page breaks — 改丁 / 改ページ / 改見開き / 改段.
- Kunten (返り点) and 再読文字.
- Gaiji — JIS X 0213 / Unicode / 第3水準 reference styles, all
  compile-time resolved via a `phf::Map`.
- 割注 (inline split annotation) and container variants (罫囲み, etc.).
- Accent decomposition (`〔…〕`) with a 114-entry translation table.
- Illustration and section-break markers (挿絵 / 改段).

#### Encoding

- Transparent Shift_JIS decoding via `aozora-md-encoding`.
- UTF-8 BOM sniff and strip.

#### CLI

- `aozora-flavored-markdown render` / `aozora-flavored-markdown check` subcommands.
- Global `--encoding {utf8,sjis}` and `--strict` flags.

### Quality gates

- 519 tests passing — unit + integration + snapshot + proptest.
- 96 % regions coverage CI floor.
- CommonMark 0.31.2 spec: 652 / 652 cases passing verbatim.
- GFM 0.29 spec passing verbatim.
- 17 k-work Aozora Bunko corpus sweep with four CI-gated invariants:
  I1 no panic, I2 no bare `［＃` leak, I3 round-trip fixed point,
  I4 HTML tag-balanced (ADR-0007).
- 『罪と罰』 (Aozora Bunko card 56656) Tier-A acceptance canary —
  panic-free rendering with zero unconsumed `［＃` markers.
- ~22-line diff against vendored comrak 0.52.0, well inside the 200-line
  budget from ADR-0001.
- `#![forbid(unsafe_code)]` workspace-wide; `dead_code = "deny"`;
  strict-code grep gate that rejects `#[allow(...)]`, nightly feature
  gates, and raw `println!` in library crates.

<!-- next-url -->
[Unreleased]: https://github.com/P4suta/aozora-flavored-markdown/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/P4suta/aozora-flavored-markdown/compare/v0.4.0...v0.4.1
[0.1.0]: https://github.com/P4suta/aozora-flavored-markdown/releases/tag/v0.1.0
