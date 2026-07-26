//! Input strategies for the property tests.
//!
//! *Stratified*: each targets one shape of input exercising one class of
//! invariants, so a shrinker homes in on the offending shape instead of
//! wandering the whole input space. Each is a flat pool of literal atoms
//! concatenated at random length — uniform selection over an explicit pool
//! keeps the atoms greppable, so covering a new notation is one row rather
//! than a new combinator.

use proptest::prelude::*;

/// Both the length and the atom choice shrink, so a failure minimises to the
/// shortest run of the simplest atoms that still reproduces it.
fn joined_atoms(pool: Vec<String>, max_atoms: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(pool), 0..=max_atoms)
        .prop_map(|pieces| pieces.concat())
}

fn owned(atoms: &[&str]) -> Vec<String> {
    atoms.iter().map(|&s| s.to_owned()).collect()
}

/// Kanji-only draws, 1 to `max_len` codepoints from U+4E00–U+9FFF.
///
/// Heading hints resolve against preceding literal text, so the properties
/// need arbitrary but well-behaved base text; staying inside one script
/// keeps the draw free of characters that are themselves triggers.
pub fn kanji_fragment(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::char::range('\u{4E00}', '\u{9FFF}'), 1..=max_len)
        .prop_map(|chars| chars.into_iter().collect())
}

/// Emitted bare and in any order, so the pool produces *unbalanced* bracket
/// shapes as readily as balanced ones; properties needing well-formed input
/// gate on the parse diagnostics instead.
///
/// The CR-bearing breaks earn their place: 青空文庫 source is historically
/// CRLF, and CRLF is one of the normalisations that decide whether the
/// renderer can address a construct by slicing the caller's own source or
/// has to recover it.
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

/// Bait for the decorative-rule invariant: a rule row under prose must stay
/// a paragraph plus `<hr>` and never promote it into a setext `<h2>`.
const RULE_CHARS: [char; 3] = ['-', '=', '_'];
const RULE_WIDTHS: [usize; 2] = [12, 35];

/// The workhorse strategy: "render is total", the Tier-A and Tier-B
/// canaries, and — via the rule rows — setext promotion.
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

/// Pass 4–8: much larger values push the shrinker deep into the malformed
/// regime, slowing iteration without surfacing new shapes.
pub fn pathological_aozora(max_depth: usize) -> impl Strategy<Value = String> {
    joined_atoms(owned(PATHOLOGICAL_ATOMS), max_depth)
}

/// The second group puts Aozora notation *inside* the Markdown constructs,
/// which is where the splice actually has to hold: a table cell, list item,
/// link label, code span and fenced block each split the AST in a way a bare
/// paragraph does not.
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
    // DEV-232: a rule row the sibling parser isolates but CommonMark did not
    // claim as a break. Each of these lost the block that owned it — the
    // list, the blockquote, the table, the paragraph — until the delegate
    // learned to lift the row rather than the node comrak made of it. The
    // pool's own rule atoms are all in a setext position, which is the one
    // place A3 already covered.
    "- item\n==========\n",
    "> quote\n==========\n",
    "| a |\n| - |\n| b |\n==========\n",
    "para\n    ----------\n",
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

/// One draw from the adversarial pool.
pub fn commonmark_adversarial() -> impl Strategy<Value = String> {
    prop::sample::select(owned(COMMONMARK_ATOMS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;

    /// The mixed-grammar atoms are why this crate owns the generator rather
    /// than borrowing a parser-side one, so a future trim must not drop
    /// them.
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

        /// A typo in the literal codepoint range would silently feed
        /// notation triggers into the heading properties.
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
