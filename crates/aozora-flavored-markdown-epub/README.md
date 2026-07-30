# aozora-flavored-markdown-epub

[![crates.io](https://img.shields.io/crates/v/aozora-flavored-markdown-epub?label=crates.io)](https://crates.io/crates/aozora-flavored-markdown-epub)
[![docs.rs](https://img.shields.io/docsrs/aozora-flavored-markdown-epub?label=docs.rs)](https://docs.rs/aozora-flavored-markdown-epub)
[![license](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
[![msrv](https://img.shields.io/badge/rust-1.96%2B-orange)](https://github.com/P4suta/aozora-flavored-markdown/blob/main/rust-toolchain.toml)

Turn a directory of **Aozora Flavored Markdown** chapters into an
[EPUB 3.3](https://www.w3.org/TR/epub-33/) package: one `.epub` file, ready
for a reading system, with vertical writing (縦書き) and Aozora Bunko
(青空文庫) typography — ruby, bouten, 縦中横 — carried through.

The EPUB is assembled directly rather than by shelling out to a converter, so
there is no runtime process dependency
([ADR-0019](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0019-epub-generation-is-hand-rolled-not-via-pandoc.md)).

## Install

```sh
cargo add aozora-flavored-markdown-epub
```

For the command-line front end, install
[`aozora-flavored-markdown-epub-cli`](https://crates.io/crates/aozora-flavored-markdown-epub-cli)
instead.

## A manuscript

```text
my-book/
  book.toml
  manuscript/
    001-prologue.md
    002-chapter-1.md
```

```toml
# book.toml
title        = "吾輩は猫である"
creator      = "夏目漱石"
language     = "ja"
writing_mode = "vertical"          # or "horizontal" (the default)
# identifier = "urn:uuid:…"        # generated when omitted
# spine      = ["002-chapter-1.md", "001-prologue.md"]
```

Chapters are collected in lexicographic order unless `spine` names them, in
which case exactly those files are used, in that order. `.md` sources are
UTF-8; `.sjis`, `.shift_jis` and `.shift-jis` are decoded as Shift_JIS, which
is how Aozora Bunko itself distributes text.

## Check and build

```rust,no_run
use std::path::Path;

use aozora_flavored_markdown_epub::{BuildOptions, CheckOptions, build, check};

let check_opts = CheckOptions::new(
    Path::new("my-book/manuscript"),
    Path::new("my-book/book.toml"),
);
let checked = check(&check_opts)?;
assert!(checked.is_empty(), "{} diagnostics", checked.diagnostic_count());

let opts = BuildOptions::new(
    Path::new("my-book/manuscript"),
    Path::new("my-book/book.toml"),
    Path::new("my-book.epub"),
);
let report = build(&opts)?;

// Rendering is infallible, so a non-empty report describes an EPUB that
// *was* written. Whether that fails the run is the caller's call.
assert!(report.is_empty(), "{} diagnostics", report.diagnostic_count());
# Ok::<(), aozora_flavored_markdown_epub::Error>(())
```

`check` runs discover → validate → render → compose and never writes a package;
`build` adds the package phase. Both return a `BuildReport` naming every
chapter that raised a diagnostic, in spine order.
Those diagnostics are the renderer's own types, re-exported here rather than
copied, so a host reads one vocabulary whether it renders HTML itself or asks
for an EPUB.

An explicit `spine = []`, a rooted/parent path, a symlink that resolves outside
the manuscript root, or a character XML 1.0 cannot represent is a refusal with
a phase-specific diagnostic code. Literal TAB/LF/CR in metadata attributes are
preserved as numeric character references.

The chapter stylesheet is the canonical `aozora-md-*` theme published by the
[`aozora-flavored-markdown`](https://crates.io/crates/aozora-flavored-markdown)
crate's `theme` feature, so the classes the renderer emits and the CSS that
styles them cannot drift apart
([ADR-0020](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0020-canonicalise-aozora-md-css-at-the-next-aozora-bump.md)).

A complete manuscript is in
[`examples/sample/`](https://github.com/P4suta/aozora-flavored-markdown/tree/main/crates/aozora-flavored-markdown-epub/examples/sample).

## Related crates

| Crate | What it is |
|---|---|
| [`aozora-flavored-markdown-epub-cli`](https://crates.io/crates/aozora-flavored-markdown-epub-cli) | the `aozora-flavored-markdown-epub` command |
| [`aozora-flavored-markdown`](https://crates.io/crates/aozora-flavored-markdown) | the dialect's parser and HTML renderer |

## License

Dual-licensed under
[Apache-2.0](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
OR [MIT](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-MIT),
at your option. See `NOTICE` for the third-party attribution index.
