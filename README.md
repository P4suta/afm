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

It is a **strict superset** of CommonMark + GFM: pure CommonMark / GFM
input renders identically, and the Aozora extensions kick in only where
the input uses them. The file extension stays `.md`.

```text
CommonMark  ──▶  GFM  ──▶  Aozora Flavored Markdown
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
([`AOZORA_MD_CLASSES`](https://docs.rs/aozora-flavored-markdown/latest/aozora_flavored_markdown/static.AOZORA_MD_CLASSES.html),
[ADR-0011](docs/adr/0011-brand-boundary-css-class-rewrite.md)); the drop-in
themes that style them ship as `theme::{HORIZONTAL_CSS, VERTICAL_CSS}` under
the default-off `theme` feature, editable as plain CSS in
[`crates/aozora-flavored-markdown/theme/`](./crates/aozora-flavored-markdown/theme/).
Full API docs are on
[docs.rs](https://docs.rs/aozora-flavored-markdown); runnable snippets live under
[`crates/aozora-flavored-markdown/examples/`](./crates/aozora-flavored-markdown/examples/).

## Guarantees

- **100% CommonMark / GFM compatibility** — both spec suites pass
  verbatim (652 CommonMark 0.31.2 cases + GFM 0.29).
- **Aozora Bunko compatibility target** — every notation listed at
  <https://www.aozora.gr.jp/annotation/> parses, and no unconsumed
  `［＃` marker reaches the rendered HTML.
- **Single binary**, no runtime process dependencies.
- **Zero parse-time hooks** in vendored comrak — Aozora recognition
  lives in the sibling [`P4suta/aozora`](https://github.com/P4suta/aozora)
  crate and is spliced into the comrak AST here.

## Development

Every operation runs inside Docker; the host toolchain is never invoked
directly (ADR-0002).

```sh
just setup             # build the dev image, install hooks, run tests
just test              # cargo nextest via Docker
just lint              # fmt + clippy + typos + strict-code + comment-discipline
just ci                # the full gate set, locally
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
The vendored `upstream/comrak/` tree stays under its upstream
BSD-2-Clause license. See [NOTICE](./NOTICE) for the full third-party
attribution index.
