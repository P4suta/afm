# 0010. Extract aozora parser core into sibling repository `aozora`

- Status: accepted
- Date: 2026-04-25
- Tags: architecture, repo-layout, release-strategy, ecosystem

## Context

ADR-0009 deferred extracting the parser into its own repo until trigger
conditions held. Three things made extraction worth doing now:

1. The name `aozora-flavored-markdown` only fits the Markdown dialect, not the parser beneath it.
   "Aozora Flavored Markdown" is a CommonMark+GFM+aozora integration; the parser
   core has no opinion on Markdown — it parses 青空文庫記法 directly. Naming it
   `aozora-md-*` conflates the two.
2. `aozora-tools` (fmt + LSP) already consumes the aozora layer, not the Markdown
   layer — the second-consumer trigger of ADR-0009 is effectively met.
3. Naming the new repo `aozora` is honest about what it contains.

## Decision

> The crate names and topology below are as of 2026-04-25. See the **Note** at
> the end for what `aozora` 0.5.0 curated them into — none of the build-block
> crates named here is published today.

Extract the parser into a new sibling repo `aozora`, with crates renamed
`aozora-syntax` / `-lexer` / `-parser` / `-encoding` / `-corpus` / `-test-utils`.
Rename the remaining `aozora-md-parser` crate to `aozora-flavored-markdown`. History is preserved
per-file via `git filter-repo --path-rename`.

### Three-layer topology after this change

```
P4suta/aozora-tools/   authoring environment (LSP / fmt / VS Code)
        │ git tag
        ▼
P4suta/aozora-flavored-markdown/            CommonMark+GFM+aozora Markdown dialect
                       (aozora-flavored-markdown, aozora-flavored-markdown-cli, vendored comrak)
        │ git tag
        ▼
P4suta/aozora/         pure 青空文庫記法 parser
                       (aozora-syntax, -lexer, -parser, -encoding, -corpus, …)
```

The `aozora` repo's Cargo.toml / source / docs name no comrak, commonmark, gfm,
or markdown; the comrak adapter lives in `aozora-flavored-markdown`.

## Consequences

- The namespace tells the truth: a reader of `aozora` meets no Markdown
  vocabulary; a reader of `aozora-flavored-markdown` meets Markdown immediately and sees aozora as a
  dependency.
- Release cadence decouples; the comrak diff budget (ADR-0001) and corpus sweep
  live in the repo whose work they protect.
- Three repos must stay consistent under tag bumps; the small public surface
  (`parse`, `serialize`, `Diagnostic`, `AozoraNode`, `decode_sjis`,
  `gaiji::resolve`) keeps breakage rare.
- ADR-0008 (zero parser hooks) moves to aozora as its foundation ADR; aozora-flavored-markdown keeps
  a redirect stub.

## Note

The crate decomposition named above is **historical**. `aozora` 0.5.0 curated
its published crates down to `aozora` / `aozora-cli` / `tree-sitter-aozora`;
the build-block crates this ADR lists are no longer published, and the parser's
public surface is `parse` / `Document` / `Snapshot` plus a flat projection over
them. The extraction decision — parser core in its own repo, this workspace
composing it — is unaffected; what the composition may reach for is settled by
[ADR-0021](0021-aozora-boundary-is-the-public-surface.md). The three-layer
topology is two layers now: `aozora-tools` was absorbed by `aozora` and
archived (see the Note on
[ADR-0009](0009-authoring-tools-live-in-sibling-repositories.md)).

## References

- ADR-0001 — fork/vendor comrak. Stays in aozora-flavored-markdown.
- ADR-0008 — zero-parser-hook pipeline. Moved to aozora.
- [ADR-0021](0021-aozora-boundary-is-the-public-surface.md) — what this
  workspace may depend on across that boundary.
- ADR-0009 — authoring tools in sibling repos.
