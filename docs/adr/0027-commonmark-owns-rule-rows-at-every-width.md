# 0027. CommonMark owns rule rows at every width

- Status: accepted
- Date: 2026-07-29
- Deciders: @P4suta
- Tags: parsing, compatibility, corpus

## Context

The sibling Aozora parser treats a run of ten or more `-` characters as a
decorative rule. CommonMark assigns the same bytes according to their
Markdown context: below prose, `-` and `=` are setext underlines; elsewhere a
hyphen run may be a thematic break, list/container content, or ordinary text.
A length threshold in this crate therefore made the advertised CommonMark
superset change grammar at an unrelated width.

This is not a rare input shape. On clean sibling checkouts
`P4suta/aozora@de2ddbdf402801164da9124110099a43c4736cad` and
`aozorabunko_text@b1ec9a7fa46de8dd5acc33378428c899e86bfb32`, the audit found:

| corpus | files | long rule rows | files carrying one | old/new HTML changes |
|---|---:|---:|---:|---:|
| Aozora conformance `source.txt` fixtures | 208 | 158 | 79 | 4 |
| `aozorabunko_text` UTF-8 texts | 17,889 | 33,364 | 16,657 | 1,594 |

The comparison rendered both complete documents and corpus-derived local
witnesses with the pre-change `main` implementation and the new one. Of
16,652 Bunko rows in a possible setext position, exactly 1,594 changed under
the isolated witness and the same 1,594 complete documents changed. The
fixture corpus changed in four witnesses and four documents. This separates
the grammar decision from unrelated parser changes.

The same read-only audit exercised the new operational surfaces:

| corpus | `fmt` drift | canonicalize errors | EPUB `check` accepted | EPUB refusals |
|---|---:|---:|---:|---:|
| 208 Aozora fixtures | 124 | 0 | 208 | 0 |
| 17,889 Bunko texts | 17,244 | 0 | 17,888 | 1 |

The single EPUB refusal is
`作品/小栗虫太郎/失楽園殺人事件（667_txt）.txt`: its source contains U+000C,
which XML 1.0 cannot represent. The check reported `XmlCharacter`; it did not
write an EPUB. Accepted fixture files produced 205 renderer diagnostics, and
accepted Bunko files produced 99,668. Diagnostics remain observations rather
than EPUB refusals.

## Decision

Rule rows are hidden from Aozora preprocessing and restored before CommonMark
parsing. There is no length threshold and no option that re-enables the
Aozora decorative-rule interpretation. The caller's Markdown context is the
only authority.

The implementation tracks hidden rows out of band with source ranges. It does
not borrow control characters from author input, so U+0001–U+0003 remain the
author's bytes.

## Consequences

Long setext underlines now behave exactly like short ones, and CommonMark
conformance no longer depends on rule width. The 1,594 measured Bunko
documents are a deliberate 0.5.0 compatibility change: callers that intended
a decorative separator must write an unambiguous thematic break in a context
where CommonMark reads one.

`fmt --check` is expected to report broad drift on raw Bunko text; the audit
establishes that canonicalization is total over both corpora, not that every
upstream file is already canonical Markdown. EPUB validation rejects only
package-invalid input, independently of renderer diagnostics.

## Alternatives considered

Keeping the ten-character Aozora threshold was rejected because the same
Markdown changed from setext to decoration solely by adding hyphens.

Adding an option was rejected because it would split the documented dialect
and make CommonMark compatibility configuration-dependent.

Rewriting long rows before CommonMark was rejected because it changes block
ownership in lists, block quotes, tables and indented prose.

## References

- GitHub #246, #249 and #250
- [CommonMark 0.31.2: setext headings](https://spec.commonmark.org/0.31.2/#setext-headings)
- [verbatim rule-region implementation](../../crates/aozora-flavored-markdown/src/verbatim_regions.rs)
