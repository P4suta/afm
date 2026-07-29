# Aozora Flavored Markdown

<p align="center">
  <a href="https://github.com/P4suta/aozora-flavored-markdown/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora-flavored-markdown/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://crates.io/crates/aozora-flavored-markdown-cli"><img alt="crates.io" src="https://img.shields.io/crates/v/aozora-flavored-markdown-cli?label=aozora-flavored-markdown-cli"></a>
  <a href="https://docs.rs/aozora-flavored-markdown"><img alt="docs.rs" src="https://img.shields.io/docsrs/aozora-flavored-markdown?label=docs.rs"></a>
  <a href="https://github.com/P4suta/aozora-flavored-markdown/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/P4suta/aozora-flavored-markdown?display_name=tag&sort=semver"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="./rust-toolchain.toml"><img alt="msrv" src="https://img.shields.io/badge/rust-1.96%2B-orange"></a>
</p>

<p align="center">
  🧪 <a href="https://docs.rs/aozora-flavored-markdown"><strong>API reference</strong></a>
  · 🎨 <a href="https://p4suta.github.io/aozora-flavored-markdown/playground/"><strong>Playground</strong></a>
  · 📦 <a href="https://github.com/P4suta/aozora-flavored-markdown/releases"><strong>Releases &amp; binaries</strong></a>
  · 📝 <a href="./CHANGELOG.md"><strong>Changelog</strong></a>
</p>

**Aozora Flavored Markdown** is a Markdown dialect, modelled after
[GFM](https://github.github.com/gfm/), that layers Aozora Bunko (青空文庫)
typography — ruby, bouten, 縦中横, `［＃…］` annotations, gaiji, accent
decomposition — on top of CommonMark + GFM.

It is a **superset** of CommonMark + GFM, and which superset is a
constructor: `Options::commonmark()` renders the CommonMark 0.31.2 spec
suite verbatim, `Options::gfm()` renders the GFM 0.29 spec suite with all
four extensions on at once, and `Options::default()` is the Aozora dialect
— GFM + 青空文庫記法 + hardbreaks. Both suites run whole rather than a
corner each: nothing is skipped, and the examples the 0.29 fixture states
differently from the spec version that supersedes it are pinned to that
later authority instead of excluded.

`default()` is the one that is not a *strict* superset, and deliberately
so: hardbreaks turns every source newline into a `<br>`, because verse and
dialogue boundaries are load-bearing in 青空文庫 source. Take it off with
`Options::default().with_hardbreaks(false)` and the Aozora extensions kick
in only where the input uses them, across both spec corpora swept whole.
One reservation is left, and it is about input no CommonMark document has:
the five private-use codepoints this crate substitutes one per 青空文庫
construct are not source text, so a source that types `U+E001`–`U+E004`
gets `U+FFFD` back from a render. The file extension stays `.md`.

CommonMark also owns rule rows at every width. In particular, a long `-` or
`=` row directly below prose is a setext heading underline, not an
Aozora-specific decorative separator. The real-corpus compatibility
measurement behind that choice is in
[ADR-0027](docs/adr/0027-commonmark-owns-rule-rows-at-every-width.md).

None of these claims is a promise, and this page deliberately states none
of the counts. They live on the
[crate page](./crates/aozora-flavored-markdown/README.md) alone, because
that is the one document inside the package whose
[`src/conformance.rs`](./crates/aozora-flavored-markdown/src/conformance.rs)
measures them: every figure there is *formatted* from a live run of both
spec corpora, so the page cannot disagree with what the suite renders.
A second copy here could, and did.
[`tests/render_commonmark_superset.rs`](./crates/aozora-flavored-markdown/tests/render_commonmark_superset.rs)
sweeps the same corpora through the dialect and holds the claim above.
The conformance suites run with the workspace tests and coverage on every PR.

```text
CommonMark  ──▶  GFM  ──▶  Aozora Flavored Markdown
commonmark()     gfm()     default()
```

## What you can write

```markdown
# 第一章                              (Markdown heading)
第一篇［＃「第一篇」は大見出し］      (Aozora heading, aliased to the same AST)

彼は｜青梅《おうめ》に行った。        (Ruby)
可哀想［＃「可哀想」に傍点］な人。    (Bouten / emphasis dots)
それは≪強調したい≫ことだった。      (Double angle quote, shown as 《…》)
昭和20［＃「20」は縦中横］年。        (Tate-chu-yoko)

［＃ここから字下げ］                  (Block indent)
段落……
［＃ここで字下げ終わり］
```

## Quickstart

CLI:

```sh
cargo install aozora-flavored-markdown-cli --locked
aozora-flavored-markdown render input.md
aozora-flavored-markdown fmt --check input.md
```

Pre-built binaries for Linux x86_64, macOS arm64 and Windows x86_64 are
attached to every [release](https://github.com/P4suta/aozora-flavored-markdown/releases),
with `SHA256SUMS` alongside.

Library:

```sh
cargo add aozora-flavored-markdown
```

```rust
use aozora_flavored_markdown::{Options, render};

let rendered = render("彼は｜青梅《おうめ》に行った。", &Options::default());
assert!(rendered.html.contains("<ruby>"));
```

The rendered HTML carries stable `aozora-md-*` CSS classes
([`classes::all`](https://docs.rs/aozora-flavored-markdown/latest/aozora_flavored_markdown/classes/fn.all.html),
[ADR-0011](docs/adr/0011-brand-boundary-css-class-rewrite.md)); the drop-in
themes that style them ship as `theme::{HORIZONTAL_CSS, VERTICAL_CSS}` under
the default-off `theme` feature, editable as plain CSS in
[`crates/aozora-flavored-markdown/theme/`](./crates/aozora-flavored-markdown/theme/).
Full API docs are on
[docs.rs](https://docs.rs/aozora-flavored-markdown); runnable snippets live under
[`crates/aozora-flavored-markdown/examples/`](./crates/aozora-flavored-markdown/examples/).

## Guarantees

- **CommonMark / GFM compatibility, measured** — both spec suites run
  whole under `Options::commonmark()` and `Options::gfm()`, nothing
  skipped; the counts are on the
  [crate page](./crates/aozora-flavored-markdown/README.md).
- **Aozora Bunko compatibility target** — every notation listed at
  <https://www.aozora.gr.jp/annotation/> parses, and no unconsumed
  `［＃` marker reaches the rendered HTML.
- **Single binary**, no runtime process dependencies.
- **Zero parse-time hooks** in comrak — it is an unmodified crates.io
  dependency. Aozora recognition lives in the sibling
  [`P4suta/aozora`](https://github.com/P4suta/aozora) crate and is spliced
  into the comrak AST here.

## Development

The supported development environment is the native, lockfile-backed mise
toolchain (ADR-0026). Install mise, trust this repository's configuration,
and install the exact resolved tools:

```sh
mise trust
mise install --locked
just test              # workspace tests through cargo-nextest
just ci                # the same five fixed suites GitHub Actions runs
```

`just` with no arguments lists every recipe. See
[CONTRIBUTING.md](./CONTRIBUTING.md) for the workflow and
[docs/adr/](./docs/adr/) for the architectural decisions.

## Sibling repositories

| Repo | What it is |
|---|---|
| [`P4suta/aozora`](https://github.com/P4suta/aozora) | Pure 青空文庫記法 parser (`aozora`), its CLI (`aozora-cli`, which carries the formatter and the language server) and the `tree-sitter-aozora` grammar. Aozora-only test surfaces — the conformance vectors and the corpus sweep — live there, as do the authoring tools absorbed from the archived `aozora-tools`. |

## Security

Vulnerabilities go through GitHub Security Advisories — see
[`SECURITY.md`](./SECURITY.md).

## License

Dual-licensed under [Apache-2.0](./LICENSE-APACHE) OR [MIT](./LICENSE-MIT).
See [NOTICE](./NOTICE) for the full third-party attribution index.
