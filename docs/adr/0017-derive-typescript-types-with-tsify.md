# 0017. Derive the TypeScript `.d.ts` with `tsify`

- Status: accepted; emitted union re-stated by ADR-0022
- Date: 2026-06-21
- Deciders: @P4suta
- Tags: ir, typescript, codegen, wasm, dx

## Context

The TypeScript surface used to come from a bespoke xtask command: ~520 lines
of hand-written `.d.ts` string assembly, kept "honest" by compile-time
variant witnesses, completeness tests, and a byte-compare drift gate over a
committed artefact. The gate checked the artefact against the *hand-written
strings*, not against the Rust shapes, so a renamed field could still slip
between the witnesses.

## Decision

Derive the `.d.ts` with [`tsify`](https://crates.io/crates/tsify).

- IR and diagnostic types derive `Tsify` behind the lib's default-off `tsify`
  feature, so host builds never pull `wasm-bindgen`.
- The wasm envelope structs derive it with `into_wasm_abi` / `from_wasm_abi`,
  typing the `#[wasm_bindgen]` entry points end to end — no
  `serde_wasm_bindgen` hand-conversion, no `as unknown as …` on the JS side.

Removed with it: `crates/xtask/src/types.rs` and its subcommand, the
committed artefact, the `just types` / `types-check` recipes and CI job, and
the `assert_*_variants` witnesses. The camelCase wire-string test stays — it
locks the serde output `tsify` now reads.

Two shape fixes fell out: recursive fields are spelled `Vec<IrBlock>` rather
than `Vec<Self>` (`tsify` would emit the invalid `Self[]`), and
`BlockResult.source_line` gained the `camelCase` rename its documented
`sourceLine` contract had always promised.

> `tsify` is the maintained crate; the `tsify-next` fork has merged back
> upstream and is deprecated (RUSTSEC-2025-0048).

## Consequences

The `.d.ts` cannot drift, and ~700 lines of generator plus its gate are gone.
Mistyped IR access downstream is a compile error. The cost is an optional,
default-off `tsify` + `wasm-bindgen` coupling in the lib, and a regeneration
in the obsidian plugin — already the expected cost of a wasm-crate bump.

**ADR-0022 (2026-07-25)** later changed the *shape* of the emitted unions by
collapsing the Aozora half of the IR. That was a breaking change for TS
consumers, but a one-line diff rather than a hand-edited union — which is the
decision here paying for itself.

## Alternatives considered

- **`ts-rs`** — derives `.ts` but does not integrate with wasm-bindgen, so
  the boundary would stay `JsValue` + casts with its own drift gate. Lighter
  coupling, but it gives up the bigger win.
- **Keep the hand-written generator** — bespoke, ~700 lines, and only
  self-consistent.
- **`typeshare`** — a multi-language CLI generator; overkill for one target
  and another tool in the image.

## References

- ADR-0013, ADR-0015, ADR-0016, ADR-0022
