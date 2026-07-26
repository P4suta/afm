# 0001. Fork comrak and vendor it in-tree

- Status: superseded by ADR-0024
- Date: 2026-04-23
- Tags: architecture, parser, fork

## Context

aozora-flavored-markdown must deliver 100% CommonMark + GFM compatibility and also parse Japanese
typography (ruby, bouten, tate-chu-yoko, `［＃...］` block annotations, gaiji)
that no upstream Markdown parser supports — including annotations that straddle
links, lists, or emphasis. Three Rust bases were evaluated: comrak
(CommonMark+GFM-compliant, full AST), pulldown-cmark (event stream), and
markdown-rs (strict extension model).

## Decision

Fork comrak at tag `v0.52.0` and vendor the tree verbatim at `/upstream/comrak/`
under a hard **0-line diff budget** enforced by `cargo xtask upstream-diff`. aozora-flavored-markdown
adds no hooks to comrak: the Aozora layer runs as a post-comrak HTML sentinel
substitution in `crates/aozora-flavored-markdown/` (see ADR-0008, the zero-parser-hook
design, now in the sibling `aozora` repo). `cargo xtask upstream-sync <tag>` is a
pure tree replace.

## Consequences

Easier:
- CommonMark + GFM tests pass for free (vendored upstream ships them).
- Rich AST available from day one; no delimiter-run parsing to re-implement.
- With no hooks, upstream syncs are a tree replace with nothing to merge.

Harder:
- Upstream drift: the vendored tree must be re-synced to follow comrak releases.
- crates.io publication requires the vendored tree under permissive licensing
  (comrak is BSD-2-Clause — compatible with our Apache-2.0 OR MIT).

## Alternatives considered

- **pulldown-cmark + event transform**: rejected — ruby parsing is intra-token
  (`｜漢字《かんじ》` inside text tokens), which the event stream exposes only
  after tokenisation; reassembling is effectively re-parsing.
- **markdown-rs fork**: rejected — its extension model accepts only standardised
  dialects (MDX, math, GFM), not project-specific annotations.
- **Fresh parser on winnow/chumsky**: rejected — the CommonMark spec is ~600
  edge cases we don't want to re-verify.

## References

- [comrak v0.52.0](https://github.com/kivikakk/comrak/tree/v0.52.0)
- [Aozora annotation spec](https://www.aozora.gr.jp/annotation/)
- [ADR-0024](0024-depend-on-crates-io-comrak.md) — retires the vendored tree
  and the `upstream/comrak/UPSTREAM_DIFF.md` policy file this ADR pointed at.
