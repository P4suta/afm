# aozora-flavored-markdown-epub-cli

[![crates.io](https://img.shields.io/crates/v/aozora-flavored-markdown-epub-cli?label=crates.io)](https://crates.io/crates/aozora-flavored-markdown-epub-cli)
[![license](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
[![msrv](https://img.shields.io/badge/rust-1.96%2B-orange)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/rust-toolchain.toml)

The `aozora-flavored-markdown-epub` command: package a directory of **Aozora
Flavored Markdown** chapters into an
[EPUB 3.3](https://www.w3.org/TR/epub-33/) file, with vertical writing
(縦書き) and Aozora Bunko (青空文庫) typography carried through.

This crate ships the binary only. To build an EPUB from Rust, use the
[`aozora-flavored-markdown-epub`](https://crates.io/crates/aozora-flavored-markdown-epub)
library instead — its README describes the manuscript layout and `book.toml`.

## Install

```sh
cargo install aozora-flavored-markdown-epub-cli --locked
```

## Use

```sh
aozora-flavored-markdown-epub build \
    --input    my-book/manuscript \
    --metadata my-book/book.toml \
    --output   my-book.epub
```

`--input` takes a directory or a single chapter file. `--help` is the
authoritative list of flags.

## Diagnostics

A chapter the lexer cannot make full sense of is still written into the book —
diagnostics are observations, not refusals.

- `--strict` turns any chapter diagnostic into exit code 2. The `.epub` is
  still written.
- `--format json` emits the stable `aozora-md.diagnostics.v1` envelope
  ([ADR-0012](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0012-diagnostic-json-output-schema-and-stability.md))
  for tooling. The default `human` format prints graphical reports with a
  source snippet.

| Exit code | Meaning |
|---|---|
| 0 | success |
| 1 | the build failed (missing manuscript, unreadable `book.toml`, unwritable output) |
| 2 | `--strict` and at least one chapter diagnostic |

## Related crates

| Crate | What it is |
|---|---|
| [`aozora-flavored-markdown-epub`](https://crates.io/crates/aozora-flavored-markdown-epub) | the library this binary is a front end for |
| [`aozora-flavored-markdown-cli`](https://crates.io/crates/aozora-flavored-markdown-cli) | the same manuscript, rendered to HTML |

## License

Dual-licensed under
[Apache-2.0](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
OR [MIT](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-MIT),
at your option. See `NOTICE` for the third-party attribution index.
