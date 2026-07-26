//! `escape_html` is the workspace's only HTML escape table, so what it does
//! to a character is a repository-wide contract rather than a private detail
//! of whichever caller happened to be tested. Until the table was unified it
//! had no test of its own at all: the core reached it only through a heading
//! hint, and the EPUB envelope reached a *second copy* of it, so either side
//! could have been edited alone and every gate would still have been green.
//!
//! The two statements here characterise the function completely, which is
//! what a single owner has to be held to. The sweep fixes the image of every
//! Unicode scalar value; concatenation fixes that a character's image never
//! depends on its neighbours. Together they pin `escape_html` on all inputs
//! by induction — a sixth escaped character, a dropped fifth, or `&#x27;`
//! for `&#39;` moves one of them.
//!
//! [`ESCAPES`] is deliberately a second, independent statement of the mapping
//! rather than a call into the crate: an oracle that shares the code under
//! test cannot fail when that code is wrong.

use aozora_flavored_markdown::escape_html;
use aozora_flavored_markdown_test_support::config;
use proptest::prelude::*;

/// The five characters that carry markup significance in HTML text and in a
/// quoted attribute value alike, with the exact entity each must become.
/// `'` is numeric because HTML 4 has no `&apos;`.
const ESCAPES: &[(char, &str)] = &[
    ('&', "&amp;"),
    ('<', "&lt;"),
    ('>', "&gt;"),
    ('"', "&quot;"),
    ('\'', "&#39;"),
];

/// Rich in the characters the table owns, so a join lands one on each side of
/// the seam, and in the entity syntax itself (`&`, `amp;`, `#39;`) — the
/// shape a "don't double-encode what is already an entity" escaper needs in
/// order to give itself away.
const ATOMS: &[&str] = &[
    "&", "<", ">", "\"", "'", "amp;", "#39;", "lt", "a", "第", " ", "\n",
];

/// What `escape_html` owes for a single character, stated without reading it.
fn expected(ch: char) -> String {
    ESCAPES
        .iter()
        .find(|&&(escapable, _)| escapable == ch)
        .map_or_else(|| ch.into(), |&(_, entity)| entity.to_owned())
}

fn escapable_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(ATOMS), 0..12).prop_map(|parts| parts.concat())
}

/// Every scalar value, rather than the five a hand-written list remembers: a
/// table that *gained* a character (over-escaping, which corrupts text) fails
/// here as loudly as one that lost a character (under-escaping, which is the
/// XSS hole). Surrogates are skipped only because `char` cannot hold one.
#[test]
fn every_scalar_value_is_escaped_or_passes_through_verbatim() {
    for value in 0..=u32::from(char::MAX) {
        let Some(ch) = char::from_u32(value) else {
            continue;
        };
        let mut buf = [0u8; 4];
        assert_eq!(
            escape_html(ch.encode_utf8(&mut buf)),
            expected(ch),
            "U+{value:04X} is not what the escape table says it is"
        );
    }
}

proptest! {
    #![proptest_config(config::default())]

    /// A character's escape may not depend on what sits beside it. With the
    /// sweep above this settles every input at once: an escaper that
    /// distributes over concatenation and agrees on singletons agrees
    /// everywhere.
    ///
    /// The failure this shape is aimed at is the plausible one. An escaper
    /// that "helpfully" leaves a `&` alone when an entity name follows it
    /// passes any single-string example a reviewer would write, and is
    /// exactly the under-escape an attacker supplies `&lt;script&gt;` to.
    /// Split that input across the seam and the two halves disagree.
    #[test]
    fn escaping_a_join_is_the_join_of_the_escapes(
        head in escapable_text(),
        tail in escapable_text(),
    ) {
        prop_assert_eq!(
            escape_html(&format!("{head}{tail}")),
            format!("{}{}", escape_html(&head), escape_html(&tail))
        );
    }
}
