# 0016. Rebrand `afm` → `aozora-flavored-markdown`

- Status: accepted
- Date: 2026-06-21
- Deciders: @P4suta
- Tags: branding, naming, release, api

## Context

Ahead of the first crates.io publish (ADR-0015) the `afm` name proved a poor
public identity: the bare `afm` crate name is already taken on crates.io (an
unrelated Adobe Font Metrics parser, `afm` v0.1.2), and the `afm` acronym is
not self-describing. The project's full name has always been *Aozora Flavored
Markdown*; only the abbreviation was `afm`.

## Decision

Rebrand the whole project from `afm` to **`aozora-flavored-markdown`**, and
align the workspace version with the sibling `aozora` crate at **0.4.1**.

**Decouple the descriptive crate identity from the high-frequency output /
infra brand** — the most idiomatic split for a greenfield design:

- **Full `aozora-flavored-markdown` / `aozora_flavored_markdown`** — crate
  names, packages, dependency keys, crate directories, import paths, and the
  CLI **binary** name. The descriptive name aids crates.io / docs.rs
  discoverability and is unambiguous next to the sibling `aozora` parser crate.
- **Short, stable `aozora-md` / `AOZORA_MD_`** — everything that appears often
  or is a wire/output contract: rendered HTML CSS classes `aozora-md-*`,
  playground UI classes `aozora-md-pg-*`, env vars `AOZORA_MD_*`, Docker image
  tags `aozora-md-{dev,ci,fuzz,book}`, and the diagnostics schema id
  `aozora-md.diagnostics.v1`. A short prefix keeps the HTML/CSS readable and is
  cheap for downstream consumers to depend on, while staying distinct from the
  raw `aozora-*` classes the parser emits (preserving the ADR-0011 boundary).

The recommended-configuration constructor `Options::afm_default()` is dropped
in favour of `impl Default for Options` (the dialect preset *is* the default);
no brand-prefixed constructor remains (`Options::default()`).

## Consequences

- `cargo install aozora-flavored-markdown-cli` / `cargo add
  aozora-flavored-markdown` become the install path; docs.rs hosts the API.
- The rendered-HTML class contract changes `afm-*` → `aozora-md-*` — a breaking
  output change, handled now because the crates are pre-publish (greenfield).
  ADR-0011 is updated for the new prefix.
- **Preserved (not renamed):** sibling repo names `aozora-flavored-markdown-obsidian` / `aozora-flavored-markdown-logseq`
  / `aozora-flavored-markdown-epub` etc. (separate repos), and the GitHub repo URLs `P4suta/aozora-flavored-markdown` /
  `p4suta.github.io/aozora-flavored-markdown` (the repo itself is not renamed here; GitHub redirects
  if it ever is).
- Downstream follow-up lives in the sibling repos: `aozora-flavored-markdown-obsidian` /
  `aozora-flavored-markdown-logseq` must move their CSS selectors `.afm-*` → `.aozora-md-*`, update
  the wasm npm package name, and the diagnostics schema id.

## Alternatives considered

**Full `aozora-flavored-markdown-*` everywhere (incl. CSS / env).** Maximally
consistent but produces 25–40-character CSS classes
(`aozora-flavored-markdown-container-indent-2`) and env vars — needless
verbosity for high-frequency, downstream-facing identifiers.

**Short `aozora-*` for output classes.** Concise, but collides with the raw
classes the parser itself emits and erases the ADR-0011 brand boundary;
`aozora-md-*` keeps them distinct.

**Keep `afm`.** Rejected: name taken on crates.io and not self-describing.

## References

- ADR-0011 (brand-boundary CSS class rewrite — now `aozora-md-*`)
- ADR-0012 (diagnostics schema — now `aozora-md.diagnostics.v1`)
- ADR-0015 (crates.io publication & semver)
- Tracking issue: P4suta/aozora-flavored-markdown#118
