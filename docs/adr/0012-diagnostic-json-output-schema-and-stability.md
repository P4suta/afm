# 0012. Diagnostic JSON output schema and stability

- Status: accepted
- Date: 2026-06-20
- Tags: cli, diagnostics, json, tooling, stability

## Context

`aozora-flavored-markdown check --format json` (and `aozora-flavored-markdown render --format json`) publishes the lexer's
diagnostics as a machine-readable stream so downstream consumers — editor / LSP
bridges, CI gates, the browser playground — can react to identifiers rather than
scraping free-form text. That makes the JSON shape a contract: once a consumer
parses it, changing field names or types breaks them silently.

The diagnostics themselves come from the sibling `aozora` crate
(`aozora_flavored_markdown::Diagnostic` re-exports `aozora::Diagnostic`). `Diagnostic` is
`#[non_exhaustive]` and exposes stable accessors: `code()` (a stable
`aozora::lex::*` string), `severity()` / `source()` (whose `as_wire_str()` give
`error|warning|note` and `source|internal`), and `span()` (byte offsets, as
`u32`). It carries no line/column — only byte spans. aozora-flavored-markdown-cli must turn this into
a contract it controls without leaking the upstream Rust enum shape, and must
not depend on `aozora`'s own `json` feature (kept out of aozora-flavored-markdown's build graph via
`default-features = false`; only aozora-flavored-markdown-wasm opts into it, for the editor-assist
envelopes).

## Decision

aozora-flavored-markdown-cli serializes diagnostics into its own envelope, versioned by a `schema`
discriminant:

```json
{
  "schema": "aozora-md.diagnostics.v1",
  "diagnostics": [
    {
      "code": "aozora::lex::unmatched_close",
      "severity": "error",
      "source": "source",
      "message": "…",
      "span": { "start": 6, "end": 9 },
      "line": 1,
      "column": 7
    }
  ]
}
```

- `code`, `severity`, `source` are the stable strings from `aozora`'s accessors.
- `span.start` / `span.end` are byte offsets into the decoded source.
- `line` / `column` are 1-based, computed CLI-side (`byte_offset_to_line_col`);
  `column` counts characters, not bytes or grapheme clusters.
- `message` is the human-readable `Display` text — **explicitly not part of the
  contract**; it may change wording at any time. Consumers key on `code`.
- Output is compact JSON followed by a trailing newline. The envelope is emitted
  even when there are no diagnostics (an empty array), so tooling always parses
  valid JSON.

**Stream routing.** `check` has no stdout payload, so its JSON goes to **stdout**
(pipe into `jq`); `render` owns stdout for HTML, so its JSON goes to **stderr**.
Human format always uses stderr. Under `--strict --format json`, the free-form
Japanese summary line is suppressed so it cannot corrupt a stdout JSON stream;
exit code 2 and the per-diagnostic `severity` carry the failure.

**Stability guarantee.** Within `aozora-md.diagnostics.v1`, changes are additive only:
new fields may appear, but existing fields are never removed or renamed. A
breaking change bumps the discriminant to `aozora-md.diagnostics.v2`. The schema is
pinned by integration tests in `crates/aozora-flavored-markdown-cli/tests/cli_integration.rs`.

## Consequences

- The set of `aozora::…` `code` strings is now part of aozora-flavored-markdown's public CLI contract;
  a future rename upstream surfaces here as a breaking change (caught by the
  code-pinning test) and must bump the schema version.
- Because `aozora::Diagnostic` is `#[non_exhaustive]`, new upstream variants add
  new `code` / `severity` values without a schema bump — consumers must tolerate
  unknown `code` strings (severities stay within the documented set).
- `message` instability is documented; a consumer that matches on message text
  rather than `code` is using the API wrong.
- aozora-flavored-markdown-cli owns the byte-offset → line/column mapping, so it is the single place
  that defines "column" semantics (1-based characters).
