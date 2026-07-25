# 0023. Substitute constructs in source coordinates

- Status: accepted
- Date: 2026-07-25
- Deciders: @P4suta
- Tags: pipeline, boundary, coordinates, ir

## Context

Rendering a document meant living in three coordinate spaces at once. The
caller's source was masked (code-block triggers hidden, ADR-0010), the parser
derived its own text from that (BOM stripped, `\r\n` folded, accent digraphs
combined inside `〔…〕`, decorative rules isolated, a source-supplied PUA
character neutralised) and then produced a third — that text with each
notation collapsed to a PUA sentinel. comrak parsed the third, this crate
walked it by looking each sentinel position up in a position-keyed table, and
the byte ranges it published came from a fourth table measured against the
second.

Everything about that shape came from the parser's internals rather than from
its contract: the sentinel text, the position-keyed lookup, the offset
newtype that distinguished the spaces. [ADR-0021](0021-aozora-boundary-is-the-public-surface.md)
says the boundary is the parser's public surface, and
[ADR-0022](0022-collapse-the-aozora-half-of-the-ir.md) left the promise that
spans would stop being conditional once the pipeline moved to source
coordinates. Neither is reachable while the substitution belongs to the
parser: the composition layer is this crate's to maintain (aozora ADR-0001
says as much), and it cannot be maintained against an internal text.

## Decision

**This crate substitutes the sentinels itself, in one coordinate space: the
source it handed the parser.**

`constructs::build` tiles that source — the bytes between constructs copied
verbatim, each construct's byte range replaced by one of this crate's four
PUA sentinels (`U+E001..=U+E004`, now declared here rather than re-exported).
comrak parses the result; both walkers (`ast_splice`, `ir`) consume the
resulting table in document order and never look a construct up by position.

Two of the parser's five pre-lex rewrites are reproduced here first: a
leading BOM is dropped and every `\r` folded to `\n`. Those are text
hygiene, not notation — every Markdown renderer folds line endings, and
comrak saw the folded text before this change too — and reproducing them is
what lets a CRLF document tile at all. 青空文庫 source is historically
Shift_JIS + CRLF, so this is the common real document, not a corner. The
other three rewrites (accent decomposition inside `〔…〕`, decorative-rule
isolation, PUA neutralisation) are the parser's to make and stay there.

The table is trusted when, and only when, it passes one exact test: **the
tiling must equal, byte for byte, the sentinel text the parser produced from
the same input.** Equality proves every range addresses the text we tiled —
every byte outside a construct matched, and every construct landed where the
parser put its sentinel. It is a whole-document proof that costs one
comparison. The ranges are *published* only when that text is the caller's
own, i.e. when no hygiene was needed: a range into a copy we made is a range
no consumer can slice.

When the test fails — the parser rewrote the text before lexing it — the
parser's own text drives comrak, so the rendering is unchanged, and no
construct carries a range: a range measured against a text nobody holds is
not a range worth publishing. The one thing still owed there is a construct's
*source text*, for the literal markdown contexts (an inline code span, a link
destination) where a notation must render as what the author typed. That is
recovered from a **source index**: the document cut into blank-line-delimited
windows, each lexed once and kept only when it passes the very same tiling
proof, since a window the parser rewrote reports shifted offsets too. A block
that fails is replaced by the lines inside it that pass — 青空文庫 source is
historically CRLF, and one `\r` should not cost a whole block. A construct is
then the candidate of its own byte length, nearest the offset the parser
reported, whose own text parses back to a notation of the same shape.

Two properties of that index are load-bearing.

**It is built once per document, on the first literal read.** A document
with no literal context never builds it; a document with thousands pays for
it once. Recovering per literal instead — re-lexing the enclosing window on
every read — is quadratic in the source, which a fuzz target reaches: 3,200
notations in literal contexts across 44 KB took 7.6 s that way and take
7.6 ms this way, and the growth is now linear (25,600 across 350 KB in
63 ms).

**Nearest, not unique.** Two notations of the same shape and the same byte
length are the norm in CJK text — `｜A《a》` and `｜B《b》` differ in no other
way — and what tells them apart is the offset the parser reported, the two
spaces differing only by what its rewrites inserted. Declining an ambiguous
window would delete the author's notation from a code span, and hand a link a
plausible-looking wrong destination rather than a visibly missing one. The
search still stops rather than reach: the candidate must be in the window
holding the reported offset, and only the nearest handful are tried.

## Consequences

- `NormalizedOffset`, the position-keyed registry lookup and the parser's
  pre-lex text-derivation pass are no longer named anywhere in this
  workspace. What the boundary carries now is: the constructs the parser
  resolved, their source ranges, its own sentinel text (as the fallback), and
  diagnostics.
- The ranges the IR publishes are ranges into the text the caller passed in,
  which is what `IrInline::Aozora`'s `span` always claimed. They are still
  withheld wherever the text had to be rewritten before the parser saw it,
  hygiene included — 0.4.1 offers no way to map its coordinates back — so
  ADR-0022's "unconditional" arrives with the version bump, where the parser
  exposes that mapping itself. The ladder is then needed only as the fragment
  side's self-check.
- **The premise the fragment side needs is now pinned by a test.**
  `tests/construct_spans.rs` asserts, over the notation zoo, realistic
  documents and the fuzz-regression corpus, that a construct's range covers
  everything it resolves against (a forward reference such as
  `可哀想［＃「可哀想」に傍点］` includes the text it points back at) and that
  parsing the range's own text reproduces the fragment the whole-document
  render produced, byte for byte. Producing a fragment from a range alone is
  therefore sound, which is what lets the per-construct renderer follow. The
  documents the sweep writes must carry a range for *every* construct; the
  fuzz-regression artifacts are adversarial inputs where the fallback is the
  expected outcome, so their ranges are each checked but their count is
  reported rather than pinned.
- A CRLF or BOM-prefixed document renders exactly as before and keeps the
  source text of every notation — including the ones in literal contexts,
  which is what an inline code span full of notation needs — but reports no
  ranges. A document carrying one of the three notation rewrites (a
  decorative rule, an accent span, a stray PUA character) additionally has to
  *recover* such a source text from the index. Recovery is best-effort by
  construction: a notation in a window the parser rewrote, line and all, is
  unplaceable, and its literal comes back empty. What it is not is expensive
  — the cost is one pass over the source, whatever the document asks of it.
- Both walkers now share one table built once per render, instead of
  materialising the stream twice. `StreamingIrBuilder::new` takes the source
  rather than the parser's text; its `walk_block` resumes at the construct
  index the previous call reached.

## Alternatives considered

**Reproduce *all* the parser's pre-lex rewrites here** so the tiling always
matches. Rejected: accent-span decomposition and decorative-rule isolation
are notation decisions, and owning them here is exactly the duplication
ADR-0021 forbids. The two that are reproduced are not — they are line-ending
and BOM handling, which this crate would owe a Markdown author anyway — and
the tiling test keeps even those honest: if our copy is not what the parser
lexed, the equality fails and the fallback takes over.

**Keep the parser's sentinel text and only re-key the table to source
ranges.** Half the change, none of the payoff: the substitution would still
be the parser's, so the composition layer would still be pinned to a text
that the next major version does not publish.

**Decide a window is rewritten by looking for the characters that trigger a
rewrite** (`\r`, a BOM, a stray PUA character). Rejected: the list is
incomplete by construction — a decorative rule triggers a rewrite and
carries none of those characters — and keeping it complete would mean owning
the parser's notation decisions here, which ADR-0021 forbids. A window runs
the same tiling proof the document runs instead: it names no rewrite, and it
cannot drift from what the parser actually does.

**Publish a recovered range, not just a recovered literal.** A recovered
range does address the caller's source, so it could be published. Rejected
for now: it would make `span` present or absent depending on whether a
best-effort search converged, which is a worse contract than "absent
whenever the text was rewritten". ADR-0022's "unconditional" still arrives
with the version bump, where the parser exposes the mapping itself.

**Recover eagerly, for every construct.** Measured before the hygiene copy
existed, when a CRLF document still landed on the fallback: a
4,000-construct document went from 27 ms to 1.77 s, because each construct
cost up to two sub-parses. The index removes that cliff — recovery is a
lookup now, not a sub-parse — but building it at all is work a document
with no literal context should not do, so it stays lazy.

## References

- [ADR-0021](0021-aozora-boundary-is-the-public-surface.md) — the boundary rule this applies to the pipeline.
- [ADR-0022](0022-collapse-the-aozora-half-of-the-ir.md) — the IR shape whose `span` promise this makes good on.
- [ADR-0010](0010-extract-aozora-core.md) — why the code-block mask (and therefore the coordinate space) is ours.
- [ADR-0011](0011-brand-boundary-css-class-rewrite.md) — the `aozora-md-*` rebrand the fragments carry.
