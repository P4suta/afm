//! Input strategies for the property tests.
//!
//! The strategies are *stratified*: each targets one shape of input that
//! exercises one class of invariants, so a shrinker homes in on the offending
//! shape instead of wandering the whole input space. Tests opt into the
//! strategies they need rather than drawing from one monolithic generator.
//!
//! | Strategy | Shape | Drives |
//! |---|---|---|
//! | [`kanji_fragment`] | single-script CJK runs | heading-hint targets, ruby bases |
//! | [`aozora_fragment`] | plain text + Aozora trigger glyphs + CRLF + decorative rule rows | "render is total", Tier A/B, setext promotion, construct recovery |
//! | [`pathological_aozora`] | unbalanced brackets, unpaired container opens | malformed input never panics |
//! | [`commonmark_adversarial`] | adversarial CommonMark/GFM, including blocks that carry Aozora notation | the two grammars sharing one document |
//!
//! Each fragment strategy is a flat pool of literal atoms concatenated in a
//! random order and length. Uniform selection over an explicit pool keeps the
//! atoms greppable — covering a new notation is one row, not a new combinator.

use proptest::prelude::*;

/// Concatenate `max_atoms` or fewer draws from `pool`.
///
/// The shared shape of every fragment strategy here. Both the length and the
/// atom choice shrink, so a failure minimises to the shortest run of the
/// simplest atoms that still reproduces it.
fn joined_atoms(pool: Vec<String>, max_atoms: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(pool), 0..=max_atoms)
        .prop_map(|pieces| pieces.concat())
}

fn owned(atoms: &[&str]) -> Vec<String> {
    atoms.iter().map(|&s| s.to_owned()).collect()
}

/// Generate a kanji-only string of 1 to `max_len` codepoints from the CJK
/// Unified Ideographs block (U+4E00–U+9FFF).
///
/// Heading hints (`［＃「X」は大見出し］`) resolve by matching their target
/// against preceding literal text, so the property tests need a source of
/// arbitrary but well-behaved base text. Staying inside one script keeps the
/// drawn text free of characters that are themselves notation triggers.
pub fn kanji_fragment(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::char::range('\u{4E00}', '\u{9FFF}'), 1..=max_len)
        .prop_map(|chars| chars.into_iter().collect())
}

/// Trigger glyphs, annotation bodies and filler making up a generic fragment.
///
/// Atoms are emitted bare and in any order, so the pool produces *unbalanced*
/// bracket shapes as readily as balanced ones. Properties that need
/// well-formed input gate on the parse diagnostics instead of on the
/// generator.
///
/// The CR-bearing line breaks are here for a reason beyond coverage of an
/// encoding: 青空文庫 source is historically CRLF, and CRLF is one of the
/// rewrites that decide whether the renderer can address a construct by
/// slicing the caller's own source or has to recover it. Drawn against the
/// decorative rule rows below, they reach the recovery path — where a
/// construct that cannot be placed is dropped, text and all.
const AOZORA_ATOMS: &[&str] = &[
    "｜",
    "《",
    "》",
    "［＃",
    "］",
    "※",
    "改ページ",
    "改丁",
    "漢字",
    "かんじ",
    "ABC",
    "1234",
    "\n",
    "\n\n",
    "\r\n",
    "\r\n\r\n",
    "、",
    "。",
    " ",
];

/// Characters CommonMark reads as a setext underline or a thematic break,
/// and the row widths drawn for them.
///
/// These rows are the bait for the decorative-rule invariant: a rule row
/// under prose must stay a paragraph plus `<hr>` and must never promote the
/// preceding paragraph into a setext `<h2>`. Rows are emitted bare, so the
/// surrounding atoms decide whether a newline brackets them.
const RULE_CHARS: [char; 3] = ['-', '=', '_'];
const RULE_WIDTHS: [usize; 2] = [12, 35];

/// Generate a mixed-shape Aozora fragment of 0 to `max_atoms` atoms.
///
/// The workhorse strategy: it covers "render is total", the Tier-A bracket
/// canary, the Tier-B sentinel canary and — via the decorative rule rows —
/// the setext-promotion invariant.
pub fn aozora_fragment(max_atoms: usize) -> impl Strategy<Value = String> {
    let mut pool = owned(AOZORA_ATOMS);
    pool.extend(
        RULE_CHARS
            .iter()
            .flat_map(|&c| RULE_WIDTHS.map(move |w| String::from(c).repeat(w))),
    );
    joined_atoms(pool, max_atoms)
}

/// Deliberately malformed shapes: opens stacked with no close, container
/// closes with no open, and delimiter permutations the classifier has to
/// reject without panicking.
const PATHOLOGICAL_ATOMS: &[&str] = &[
    "［＃［＃",
    "］］",
    "《《",
    "》》",
    "｜｜",
    "※［＃",
    "［＃ここから字下げ］",
    "［＃ここで字下げ終わり］",
    "［＃ここから罫囲み］",
    "［＃ここで罫囲み終わり］",
    "［＃「」は大見出し］",
    "［＃「X」に傍点］",
    "｜ABC《",
    "》DEF",
    "［＃改",
    "］",
    "\n\n",
];

/// Generate an adversarial source built from 0 to `max_depth` malformed
/// atoms.
///
/// A typical call passes 4–8. Much larger values push the shrinker deep into
/// the malformed regime, which slows iteration without surfacing new shapes.
pub fn pathological_aozora(max_depth: usize) -> impl Strategy<Value = String> {
    joined_atoms(owned(PATHOLOGICAL_ATOMS), max_depth)
}

/// Adversarial CommonMark and GFM blocks.
///
/// The first group is plain CommonMark/GFM — blockquotes nested in lists,
/// tight/loose toggles, backslash escapes, fenced code, tables, raw HTML.
/// The second group puts Aozora notation *inside* those constructs, which is
/// where this dialect's splice actually has to hold: a table cell, a list
/// item, a link label, a code span and a fenced block each split the Markdown
/// AST in a way a bare paragraph does not.
const COMMONMARK_ATOMS: &[&str] = &[
    "# heading\n\n- item\n  > quote in list\n    1. nested",
    "> outer\n> > inner\n> > > deepest\n",
    "- loose\n\n- items\n\n- here\n",
    "- tight\n- items\n- here\n",
    "\\*escaped\\* and \\[not a link\\]\n",
    "```rust\nlet x = 1;\n```\n",
    "| h1 | h2 |\n| -- | -- |\n| a  | b  |\n",
    "[link](url) and ![img](src)\n",
    "***\n\nthematic\n\n***\n",
    "  <em>inline HTML</em>\n",
    "trailing   spaces  \nhard break\n",
    // Aozora notation inside a CommonMark/GFM container.
    "| ruby | note |\n| -- | -- |\n| ｜青梅《おうめ》 | ［＃改ページ］ |\n",
    "- ｜漢字《かんじ》\n- ~~取り消し~~と［＃「X」に傍点］\n  - ｜入子《いれこ》\n",
    "> ［＃ここから字下げ］\n> ｜引用《いんよう》\n> ［＃ここで字下げ終わり］\n",
    "# ｜見出し《みだし》\n\n本文\n",
    "`｜青梅《おうめ》` in a code span, ｜青梅《おうめ》 outside\n",
    // A code span restores the literal context, so the bracket notation
    // surfaces unwrapped inside the `<code>` — the shape Tier A excepts.
    "`可哀想［＃「可哀想」に傍点］` in a code span\n",
    "```\n｜青梅《おうめ》\n［＃改ページ］\n```\n",
    "[｜青梅《おうめ》](https://example.com/#［＃)\n",
    "**強調**の中の｜漢字《かんじ》と*斜体*\n",
];

/// Generate one adversarial CommonMark block; a good share of the pool
/// carries Aozora notation inside the Markdown construct.
pub fn commonmark_adversarial() -> impl Strategy<Value = String> {
    prop::sample::select(owned(COMMONMARK_ATOMS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;

    /// The mixed-grammar atoms are the whole reason this crate owns the
    /// generator rather than borrowing a parser-side one — the parser has no
    /// Markdown to mix with. Pin them so a future trim cannot quietly drop
    /// the coverage.
    #[test]
    fn commonmark_pool_carries_both_grammars() {
        let mixed = COMMONMARK_ATOMS
            .iter()
            .filter(|atom| atom.contains('｜') || atom.contains("［＃"))
            .count();
        assert!(
            mixed > 0 && mixed < COMMONMARK_ATOMS.len(),
            "pool must hold plain *and* Aozora-bearing Markdown, got {mixed}/{}",
            COMMONMARK_ATOMS.len()
        );
    }

    proptest! {
        #![proptest_config(default_config())]

        /// The CJK range is written as a literal pair of codepoints; a typo
        /// there would silently feed notation triggers into the heading
        /// properties instead of the inert base text they expect.
        #[test]
        fn kanji_fragment_draws_only_cjk_ideographs(s in kanji_fragment(5)) {
            prop_assert!(!s.is_empty());
            prop_assert!(s.chars().count() <= 5);
            for c in s.chars() {
                prop_assert!(('\u{4E00}'..='\u{9FFF}').contains(&c), "non-CJK draw: {c:?}");
            }
        }
    }
}
