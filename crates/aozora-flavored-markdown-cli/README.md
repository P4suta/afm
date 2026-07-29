# aozora-flavored-markdown-cli

[![crates.io](https://img.shields.io/crates/v/aozora-flavored-markdown-cli?label=crates.io)](https://crates.io/crates/aozora-flavored-markdown-cli)
[![license](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
[![msrv](https://img.shields.io/badge/rust-1.96%2B-orange)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/rust-toolchain.toml)

The `aozora-flavored-markdown` command: render or check **Aozora Flavored
Markdown** — CommonMark + GFM with Aozora Bunko (青空文庫) typography — from a
terminal or a build script.

This crate ships the binary only. To call the same renderer from Rust, use the
[`aozora-flavored-markdown`](https://crates.io/crates/aozora-flavored-markdown)
library instead.

## Install

```sh
cargo install aozora-flavored-markdown-cli --locked
```

Pre-built binaries for Linux x86_64, macOS arm64 and Windows x86_64 are
attached to every
[release](https://github.com/P4suta/aozora-flavored-markdown/releases), with
`SHA256SUMS` alongside.

## Use

```sh
aozora-flavored-markdown render input.md > out.html
aozora-flavored-markdown render input.md -o out.html
cat input.md | aozora-flavored-markdown render -

# Parse and report, without rendering
aozora-flavored-markdown check input.md

# Check canonical form without writing, review a diff, or update in place
aozora-flavored-markdown fmt --check input.md
aozora-flavored-markdown fmt --diff input.md
aozora-flavored-markdown fmt --write input.md

# Raw Aozora Bunko files are Shift_JIS
aozora-flavored-markdown render --encoding sjis 56656.txt

# Shell completions
aozora-flavored-markdown completions zsh > _aozora-flavored-markdown
```

`--help` is the authoritative list of flags; it is rendered from the same
definitions as the man page and the completion scripts.

## Diagnostics

A source the lexer cannot make full sense of still renders — diagnostics are
observations, the way a compiler's warnings are, not refusals.

- `--strict` turns any diagnostic into exit code 2. The output is still
  written.
- `--format json` emits the stable `aozora-md.diagnostics.v1` envelope
  ([ADR-0012](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0012-diagnostic-json-output-schema-and-stability.md))
  on stderr, for tooling. The default `human` format prints graphical reports
  with a source snippet.

| Exit code | Meaning |
|---|---|
| 0 | success |
| 1 | the command failed, or `fmt --check` / `--diff` found drift |
| 2 | `--strict` and at least one diagnostic |

## Related crates

| Crate | What it is |
|---|---|
| [`aozora-flavored-markdown`](https://crates.io/crates/aozora-flavored-markdown) | the library this binary is a front end for |
| [`aozora-flavored-markdown-epub-cli`](https://crates.io/crates/aozora-flavored-markdown-epub-cli) | the same manuscript, packaged as an EPUB3 |

The dialect itself is documented in the
[repository README](https://github.com/P4suta/aozora-flavored-markdown#readme)
and can be tried in the
[playground](https://p4suta.github.io/aozora-flavored-markdown/playground/).

## License

Dual-licensed under
[Apache-2.0](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
OR [MIT](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-MIT),
at your option. See `NOTICE` for the third-party attribution index.
