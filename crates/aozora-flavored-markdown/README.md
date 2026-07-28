# aozora-flavored-markdown

[![crates.io](https://img.shields.io/crates/v/aozora-flavored-markdown?label=crates.io)](https://crates.io/crates/aozora-flavored-markdown)
[![docs.rs](https://img.shields.io/docsrs/aozora-flavored-markdown?label=docs.rs)](https://docs.rs/aozora-flavored-markdown)
[![license](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
[![msrv](https://img.shields.io/badge/rust-1.96%2B-orange)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/rust-toolchain.toml)

The parser and HTML renderer for **Aozora Flavored Markdown**: a Markdown
dialect, modelled after [GFM](https://github.github.com/gfm/), that layers
Aozora Bunko (青空文庫) typography — ruby, bouten, 縦中横, `［＃…］`
annotations, gaiji, accent decomposition — on top of CommonMark + GFM.

It is a **strict superset** of CommonMark + GFM: pure CommonMark / GFM input
renders identically, and the Aozora extensions apply only where the input uses
them. The file extension stays `.md`.

```text
CommonMark  ──▶  GFM  ──▶  Aozora Flavored Markdown
```

## Install

```sh
cargo add aozora-flavored-markdown
```

## Render

```rust
use aozora_flavored_markdown::{Options, render};

let rendered = render("彼は｜青梅《おうめ》に行った。", &Options::default());
assert!(rendered.html.contains("<ruby>"));
assert!(rendered.diagnostics.is_empty());
```

`render` never fails — a source it cannot make sense of yields HTML plus
`Diagnostic`s, the way a compiler yields warnings. Callers with nothing to
report to can drop them:

```rust
let html = aozora_flavored_markdown::to_html("｜青梅《おうめ》");
assert!(html.contains("<ruby>"));
```

## What the dialect adds

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

Try it in the
[playground](https://p4suta.github.io/aozora-flavored-markdown/playground/).

## Entry points

| Function | Gives you |
|---|---|
| `to_html` | HTML, diagnostics dropped |
| `render` | HTML + diagnostics |
| `diagnose` | diagnostics alone, no rendering |
| `render_to_ir` | HTML + a structural IR a host can render itself |
| `render_blocks` | one IR block per top-level element, for incremental hosts |
| `canonicalize` | the source reformatted, or an `Error` |

## Features

Every feature is off by default; a consumer that renders HTML and nothing else
pays for none of them.

| Feature | What it turns on |
|---|---|
| `theme` | `theme::HORIZONTAL_CSS` / `theme::VERTICAL_CSS` — the canonical stylesheets for the emitted classes, as `pub const` data |
| `serde` | `Serialize` + `Deserialize` on the IR and the diagnostic types |
| `miette` | `impl miette::Diagnostic for Diagnostic` (trait only — the graphical renderer is a binary's choice) |
| `tsify` | TypeScript declarations for the IR, derived from the Rust types |

The rendered HTML carries stable `aozora-md-*` CSS classes, enumerated by
[`classes::all`](https://docs.rs/aozora-flavored-markdown/latest/aozora_flavored_markdown/classes/fn.all.html)
and specified by
[ADR-0011](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0011-brand-boundary-css-class-rewrite.md).

## Guarantees

- **100% CommonMark / GFM compatibility** — both spec suites pass verbatim
  (652 CommonMark 0.31.2 cases + GFM 0.29).
- **Aozora Bunko compatibility target** — every notation listed at
  <https://www.aozora.gr.jp/annotation/> parses, and no unconsumed `［＃`
  marker reaches the rendered HTML.
- **Zero parse-time hooks** in comrak — it is an unmodified crates.io
  dependency ([ADR-0024](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0024-depend-on-crates-io-comrak.md)).
  青空文庫 recognition lives in the sibling
  [`aozora`](https://crates.io/crates/aozora) crate and is spliced into the
  comrak AST here
  ([ADR-0021](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0021-aozora-boundary-is-the-public-surface.md)).

## Related crates

| Crate | What it is |
|---|---|
| [`aozora-flavored-markdown-cli`](https://crates.io/crates/aozora-flavored-markdown-cli) | the `aozora-flavored-markdown` command |
| [`aozora-flavored-markdown-epub`](https://crates.io/crates/aozora-flavored-markdown-epub) | EPUB3 packaging for a manuscript directory |
| [`aozora`](https://crates.io/crates/aozora) | the pure 青空文庫記法 parser this crate composes |

Full API documentation is on
[docs.rs](https://docs.rs/aozora-flavored-markdown); runnable snippets live
under
[`examples/`](https://github.com/P4suta/aozora-flavored-markdown/tree/main/crates/aozora-flavored-markdown/examples).

## License

Dual-licensed under
[Apache-2.0](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
OR [MIT](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-MIT),
at your option. See `NOTICE` for the third-party attribution index.
