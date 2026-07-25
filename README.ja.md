# Aozora Flavored Markdown

[English](./README.md) · [日本語](./README.ja.md)

<p align="center">
  <a href="https://github.com/P4suta/aozora-flavored-markdown/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora-flavored-markdown/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://crates.io/crates/aozora-flavored-markdown-cli"><img alt="crates.io" src="https://img.shields.io/crates/v/aozora-flavored-markdown-cli?label=aozora-flavored-markdown-cli"></a>
  <a href="https://docs.rs/aozora-flavored-markdown"><img alt="docs.rs" src="https://img.shields.io/docsrs/aozora-flavored-markdown?label=docs.rs"></a>
  <a href="https://github.com/P4suta/aozora-flavored-markdown/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/P4suta/aozora-flavored-markdown?display_name=tag&sort=semver"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="./rust-toolchain.toml"><img alt="msrv" src="https://img.shields.io/badge/rust-1.96%2B-orange"></a>
</p>

<p align="center">
  🧪 <a href="https://docs.rs/aozora-flavored-markdown"><strong>API リファレンス</strong></a>
  · 🎨 <a href="https://p4suta.github.io/aozora-flavored-markdown/playground/"><strong>Playground</strong></a>
  · 📦 <a href="https://github.com/P4suta/aozora-flavored-markdown/releases"><strong>リリース &amp; バイナリ</strong></a>
  · 📝 <a href="./CHANGELOG.md"><strong>Changelog</strong></a>
</p>

**Aozora Flavored Markdown** は、[GFM](https://github.github.com/gfm/) と
同じ系譜の Markdown 方言です。青空文庫が長年整備してきた日本語組版の記法
—— ルビ、傍点、縦中横、`［＃…］` 注記、外字、アクセント分解など —— を
CommonMark + GFM の上に重ねます。

CommonMark + GFM の **strict superset** です。純粋な CommonMark / GFM
文書はそのまま同じ HTML になり、青空文庫記法の拡張は入力が実際に使った
箇所でのみ発動します。拡張子は `.md` のままです。

```text
CommonMark  ──▶  GFM  ──▶  Aozora Flavored Markdown
```

## 書ける記法

```markdown
# 第一章                             (Markdown 見出し)
［＃「第一篇」は大見出し］            (青空文庫見出し、同じ AST へ合流)

彼は｜青梅《おうめ》に行った。        (ルビ)
それは《《強調したい》》ことだった。    (傍点)
令和［＃縦中横］2［＃縦中横終わり］年。 (縦中横)

［＃ここから字下げ］                  (ブロック字下げ)
段落……
［＃ここで字下げ終わり］
```

## クイックスタート

CLI:

```sh
cargo install aozora-flavored-markdown-cli --locked
aozora-flavored-markdown render input.md
```

Linux x86_64 / macOS arm64 / Windows x86_64 のビルド済みバイナリは各
[リリース](https://github.com/P4suta/aozora-flavored-markdown/releases)
に `SHA256SUMS` とともに添付されています。

ライブラリ:

```sh
cargo add aozora-flavored-markdown
```

```rust
use aozora_flavored_markdown::{Options, render};

let rendered = render("彼は｜青梅《おうめ》に行った。", &Options::default());
assert!(rendered.html.contains("<ruby>"));
```

出力 HTML は安定した `aozora-md-*` CSS クラスを持ちます
([`AOZORA_MD_CLASSES`](https://docs.rs/aozora-flavored-markdown/latest/aozora_flavored_markdown/static.AOZORA_MD_CLASSES.html)、
[ADR-0011](docs/adr/0011-brand-boundary-css-class-rewrite.md))。
それを style する drop-in テーマは既定 off の `theme` feature で
`theme::{HORIZONTAL_CSS, VERTICAL_CSS}` として公開しており、CSS の実体は
[`crates/aozora-flavored-markdown/theme/`](./crates/aozora-flavored-markdown/theme/)
にあります。API リファレンスは
[docs.rs](https://docs.rs/aozora-flavored-markdown)、実行できるサンプルは
[`crates/aozora-flavored-markdown/examples/`](./crates/aozora-flavored-markdown/examples/)
にあります。

## 保証

- **100% CommonMark / GFM 互換** —— 両 spec の conformance test suite を
  verbatim で全通過(CommonMark 0.31.2 の 652 ケース + GFM 0.29)。
- **青空文庫記法互換をターゲット** —— <https://www.aozora.gr.jp/annotation/>
  が列挙する記法をすべて parse し、未消費の `［＃` を HTML に漏らしません。
- **単一バイナリ**、実行時の外部プロセス依存なし。
- vendored comrak 内の **parse-time hook は 0**。青空文庫記法の認識は
  sibling repo [`P4suta/aozora`](https://github.com/P4suta/aozora) にあり、
  本リポジトリが comrak AST へ splice します。

## 開発

すべての操作は Docker 内で動作します。ホストの toolchain は直接は
呼びません(ADR-0002)。

```sh
just setup             # dev image のビルド、hooks 導入、テスト実行
just test              # Docker 経由で cargo nextest
just lint              # fmt + clippy + typos + strict-code + comment-discipline
just ci                # full gate set のローカル再現
```

引数なしの `just` が全レシピを一覧します。ワークフローは
[CONTRIBUTING.md](./CONTRIBUTING.md)、設計判断は
[docs/adr/](./docs/adr/) を参照してください。

## Sibling リポジトリ

| Repo | 内容 |
|---|---|
| [`P4suta/aozora`](https://github.com/P4suta/aozora) | 純粋な青空文庫記法パーサ —— lexer / AST / renderer / 外字テーブル。青空文庫専用のテスト面(`spec-aozora`、corpus sweep)もこちら。 |
| [`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools) | 執筆支援ツール: `aozora-fmt` / `aozora-lsp` / tree-sitter grammar / VS Code extension。 |

## セキュリティ

脆弱性は GitHub Security Advisories 経由で報告してください ——
開示フローは [`SECURITY.md`](./SECURITY.md) を参照。

## ライセンス

[Apache-2.0](./LICENSE-APACHE) OR [MIT](./LICENSE-MIT) のデュアル
ライセンスです。vendored `upstream/comrak/` は上流の BSD-2-Clause の
ままです。第三者由来素材の帰属は [NOTICE](./NOTICE) に集約しています。
