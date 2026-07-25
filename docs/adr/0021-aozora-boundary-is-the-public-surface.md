# 0021. The aozora boundary is aozora's public surface only

- Status: accepted
- Date: 2026-07-25
- Deciders: @P4suta
- Tags: architecture, boundary, sibling-repo, docs

## Context

Until now aozora-flavored-markdown reached into `aozora`'s internal modules —
the borrowed arena, the per-node renderer, the lexer's sanitiser, its
normalised-offset coordinate space. That worked while both repos moved
together, and it is how the current splice layer is written.

`aozora` 0.5.0 ended it. The release collapsed the published crates to three
and made the pipeline / syntax / render modules private, leaving a document
handle, a snapshot, and flat projections as the whole contract. Every internal
name aozora-flavored-markdown depended on disappeared at once, and the previous
escape hatch — depending on an internal crate directly — went with it.

So the follow-through is not a rename pass. It is a decision about where the
boundary sits, and it needs a rule that survives the next upstream curation.

Two ways to close the gap were on the table: ask upstream to re-publish a
narrow slice (e.g. "render one resolved node to an HTML fragment"), or absorb
the composition cost here.

## Decision

**aozora-flavored-markdown depends on `aozora`'s public surface and nothing
else.** Where the public surface does not offer what the splice layer needs,
this repo composes what it needs from what is published — it does not ask
upstream to widen its API.

Two consequences are load-bearing enough to state as rules:

1. **The composition layer is ours.** Sentinel substitution, the replacement
   table, HTML fragment assembly, and every coordinate translation live in
   this repo. `aozora`'s own ADR-0001 already concluded that "the
   sentinel-splice composition layer is extra machinery the integrator must
   maintain (it lives in afm, not here)"; 0.5.0 only made that visible in the
   published API.

2. **An upstream API request must be justified from upstream's side alone.**
   Not "it would be convenient here". If the request cannot be motivated by
   upstream's own consumers and measurements, it is a request to make this
   repo's job easier at the cost of upstream's contract, and the answer is to
   solve it here instead. A low-level positional accessor was demoted and then
   deleted upstream once already; re-litigating it under a new name is the same
   request.

Prose follows the same boundary: **no comment in this repo names an upstream
internal path** — `//`, `///` and `//!` alike, and in either the hyphenated or
the underscored spelling of a crate. `cargo xtask comment-discipline` (wired
into `just lint`) fails the build when one appears.

## Consequences

- Upstream can curate its API freely; this repo's build does not depend on
  internals staying published.
- Where the public surface is coarser than the internal one, this repo pays
  with re-derivation rather than with a private dependency — a real cost,
  bounded by the parity gate that would catch an infidelity.
- Comments and docs cannot silently rot into references to names that no
  longer exist; the gate turns that class of drift into a build failure.
- A genuinely upstream-motivated API addition is still possible — the rule
  raises the bar, it does not close the door.

## Alternatives considered

**Ask upstream to re-publish the internal surface.** Rejected: 0.5.0's
curation was deliberate and recent, upstream's own consumers have no measured
need for a second render entry point, and the new semantics (what a range that
splits a container means, what happens when a forward reference's target falls
outside it) would need their own gates maintained in two repos.

**Build a renderer here from the public projections.** Rejected: the published
node kinds carry family tags only, so this repo would have to re-parse the
notation to recover bouten kind, heading level, kaeriten, and the rest — the
notation would then be owned twice, against ADR-0001, and mismatches would show
up as silent drift instead of a red gate.

**Vendor `aozora`.** Rejected: it multiplies the surface that ADR-0001 exists
to keep at zero, and forfeits upstream's own test corpus.

## References

- [ADR-0001](0001-fork-comrak-vendor-in-tree.md) — the vendoring / zero-diff precedent.
- [ADR-0010](0010-extract-aozora-core.md) — why the parser lives in a sibling repo.
- [ADR-0011](0011-brand-boundary-css-class-rewrite.md) — the class-prefix half of the same boundary.
- aozora ADR-0001 — "the composition layer lives in afm, not here".
