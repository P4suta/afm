# aozora-flavored-markdown-wasm

WebAssembly bindings for
[Aozora Flavored Markdown](https://github.com/P4suta/aozora-flavored-markdown)
— CommonMark + GFM with Aozora Bunko (青空文庫) typography — for the editor
plugins and the browser playground.

This crate is **not published to crates.io**. It ships as a `wasm-pack`
artefact: `just wasm-build` writes an npm-shaped package into `pkg/`, which the
playground and the editor hosts consume as a `file:` dependency.

## Exports

| Export | Gives you |
|---|---|
| `render(source, options?)` | IR + HTML + diagnostics for a whole document |
| `renderBlocks(source, options?)` | one IR block per top-level element, for incremental re-rendering |
| `renderAozoraOnly(text)` | the 青空文庫 layer alone, no Markdown |
| `hashSource(source)` | a cheap content hash for host-side caching |
| `slugsJson()` | the completion catalogue for `［＃…］` annotations |
| `AozoraDocument` | source-coordinate queries — nodes, pairs, diagnostics, gaiji — for hover, inlay hints, outline and folding |
| `initPanicHook()` | route Rust panics to `console.error` |

The TypeScript declarations are derived from the Rust types by `tsify`, so the
`.d.ts` a host is typed against cannot drift from the shape the ABI actually
sends
([ADR-0017](https://github.com/P4suta/aozora-flavored-markdown/blob/main/docs/adr/0017-derive-typescript-types-with-tsify.md)).

Rendering never throws: a source the lexer cannot make full sense of comes back
with diagnostics attached, and one past the span budget comes back empty with a
`source_too_large` diagnostic.

## Related crates

| Crate | What it is |
|---|---|
| [`aozora-flavored-markdown`](https://crates.io/crates/aozora-flavored-markdown) | the dialect's parser and HTML renderer, which these bindings wrap |
| [`aozora-flavored-markdown-cli`](https://crates.io/crates/aozora-flavored-markdown-cli) | the same renderer as a command |

Try the bindings in the
[playground](https://p4suta.github.io/aozora-flavored-markdown/playground/).

## License

Dual-licensed under
[Apache-2.0](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-APACHE)
OR [MIT](https://github.com/P4suta/aozora-flavored-markdown/blob/main/LICENSE-MIT),
at your option.
