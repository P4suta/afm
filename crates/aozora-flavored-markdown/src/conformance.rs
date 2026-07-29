// CommonMark 0.31.2 and GFM 0.29 conformance, against the spec fixtures
// under `spec/` (converted from the upstream sources by `xtask spec-refresh`).
//
// comrak claims 100% CommonMark compatibility; this crate wraps it unmodified,
// so a whole-suite pass is the expectation. A drop means the wrapper — lexer
// pre-pass, option defaults, the HTML splice — perturbed upstream behaviour.
//
// These runners are the executable half of the README's compatibility claim,
// so what they render with is what that claim names and nothing adjacent:
// `Options::commonmark()` for the CommonMark suite, `Options::gfm()` for the
// GFM one. Both are public, so a reader can reach for the same constructor
// the README points at and get the behaviour measured here. `Options::new()`
// — the Aozora dialect — is deliberately NOT under test: it sets
// `hardbreaks`, which turns a soft break into a `<br>` and so cannot render
// the spec verbatim. That is a dialect decision, not a conformance gap, and
// the README now says which preset carries which promise.
//
// The one delta both runners add is raw-HTML passthrough, because the
// expected output in both fixtures contains raw HTML. `Options::with_raw_html`
// turns it on and it is `#[cfg(test)]`, which is why these runners live in
// `src/` instead of `tests/`: an integration test is a separate crate and
// could only reach that switch if it were public.
//
// Both suites run WHOLE — every example in either fixture asserted. The GFM
// runner used to take the 24 examples carrying an extension tag and skip two
// of those: 22 of 672, under a README sentence that said the suite passed.
// The 648 it left out are the GFM spec's inherited CommonMark 0.29 body, and
// they are where the interesting question lives, because `Options::gfm()`
// carries all four extensions at once — an inherited example is the one place
// an extension can contradict the text it was inherited from, and some do.
//
// The GFM examples that do not come back byte for byte are neither defects
// nor skips: `expected` below names every one of them, and names what to
// compare it against instead. Some are the same HTML written differently, so
// they are compared as XML; the rest are the 0.29 fixture being out of date,
// and each is pinned to the authority that supersedes it. A change on either
// side of any of them still fails here.
//
// One `assert_eq!` per example, rather than a divergence collector: the
// collector's own reporting is unreachable while the suite passes, and
// `pretty_assertions` already diffs the first mismatch better than a summary
// of five would.

use std::collections::{BTreeMap, BTreeSet};

use pretty_assertions::assert_eq;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Deserialize;

use crate::{Options, render};

const COMMONMARK_FIXTURE: &str = include_str!("../spec/commonmark-0.31.2.json");
const GFM_FIXTURE: &str = include_str!("../spec/gfm-0.29-gfm.json");
// The crates.io page for this crate, and the ONLY document in this repository
// that states the figures this file measures. Included rather than read at run
// time so the claim is a build dependency of the thing that proves it, and
// reached with `../` rather than a repository-root path: it is inside this
// package and is therefore available to the extracted-tarball unit test.
//
// Three other documents used to restate these figures, held together by a
// regex rule that matched the wording each sentence happened to use — so a
// copy phrased any other way was compared against nothing. The copies are
// gone instead: one document states a figure, every figure on it is formatted
// from a live run below, and drift now needs a failing test, not a pattern.
const CRATE_README: &str = include_str!("../README.md");

// Every extension tag the GFM fixture carries. `disabled` labels the
// task-list-items output (the `disabled` attribute on `<input
// type="checkbox">`), not a disabled example.
const GFM_EXTENSION_TAGS: [&str; 5] = [
    "autolink",
    "disabled",
    "strikethrough",
    "table",
    "tagfilter",
];

// What one GFM 0.29 example's expected output is worth as an oracle.
//
// Everything `expected` below does not name is `Verbatim`. The other three
// variants each carry what to check the example against instead — an
// authority, never a permission to differ — so no entry is satisfied by
// whatever this crate happens to emit today.
#[derive(Debug, Clone, Copy)]
enum Expectation {
    // The fixture's own output, byte for byte.
    Verbatim,
    // The same HTML written differently: same elements, same attributes, same
    // text, compared through `canonical_xml`.
    XmlEquivalent,
    // Superseded by CommonMark 0.31.2, whose fixture holds this same input
    // with the output this crate renders. Matched on the input rather than on
    // an example number, because the input is what makes two examples the
    // same example and the number is what a spec refresh renumbers. Every
    // input over there is distinct, so the match is unambiguous — and the
    // runner asserts that rather than assuming it.
    CommonMark0312,
    // Superseded by GFM's own "Autolinks (extension)" chapter, which the
    // preset carries and the inherited CommonMark example predates. The
    // string is what the preset renders; turning `autolinks` back off has to
    // give the fixture's own output back, which is what makes the extension
    // the whole of the difference.
    Autolinked(&'static str),
}

// The allowlist. Every entry carries the reason it is here, and no
// entry that is merely "comrak differs" — a divergence this file cannot
// attribute to a named authority is a bug in the wrapper and has to fail.
fn expected(example: u32) -> Expectation {
    match example {
        // Task list items. comrak writes `<input type="checkbox" disabled=""
        // />`, the fixture `<input disabled="" type="checkbox">`: attribute
        // order and self-closing style, nothing else. Both parse to the same
        // element and every browser renders them identically, so they are
        // compared as XML — which still fails on a changed attribute, a lost
        // one, or moved text, none of which a skip could have seen.
        279 | 280 => Expectation::XmlEquivalent,

        // Emphasis nesting. CommonMark 0.30 rewrote the rule that had
        // flattened a doubled delimiter run into a single `<strong>`, and
        // comrak implements 0.31.2. Each of these inputs is in the
        // 0.31.2 fixture verbatim, carrying the nested output this crate
        // produces — so `commonmark_0_31_2_full_pass` is already asserting
        // the same bytes from the other side, and the GFM fixture is simply
        // the older reading of the same input.
        //
        // Each with the input it is — one arm rather than that many
        // identical ones, which is also what `clippy::match_same_arms` asks
        // for:
        //   398 `__foo, __bar__, baz__`     436 `**foo **bar****`
        //   426 `foo******bar*********baz`  473 `****foo****`
        //   434 `__foo __bar__ baz__`       474 `____foo____`
        //   435 `____foo__ bar__`           475 `******foo******`
        //                                   477 `_____foo_____`
        398 | 426 | 434..=436 | 473..=475 | 477 => Expectation::CommonMark0312,

        // Bare URLs and e-mail addresses. These sit in the GFM spec's
        // inherited "Autolinks" chapter, which says a URL outside `<…>` is
        // text — and are contradicted by the GFM spec's own "Autolinks
        // (extension)" chapter, which links it. GitHub links it too. The
        // contradiction is invisible to cmark-gfm's runner, which switches on
        // one extension per example and so never has the extension on for the
        // chapter it supersedes; `Options::gfm()` is all four at once, which
        // is the configuration the README names and a caller gets.
        //
        // `<http://foo.bar/baz bim>`
        610 => Expectation::Autolinked(concat!(
            r#"<p>&lt;<a href="http://foo.bar/baz">http://foo.bar/baz</a> bim&gt;</p>"#,
            "\n"
        )),
        // `< http://foo.bar >`
        616 => Expectation::Autolinked(concat!(
            r#"<p>&lt; <a href="http://foo.bar">http://foo.bar</a> &gt;</p>"#,
            "\n"
        )),
        // `http://example.com`
        619 => Expectation::Autolinked(concat!(
            r#"<p><a href="http://example.com">http://example.com</a></p>"#,
            "\n"
        )),
        // `foo@bar.example.com`
        620 => Expectation::Autolinked(concat!(
            r#"<p><a href="mailto:foo@bar.example.com">foo@bar.example.com</a></p>"#,
            "\n"
        )),

        _ => Expectation::Verbatim,
    }
}

// The allowlist again, as (example, the input it is).
//
// Every reason in `expected` is written about an input — "`____foo____`",
// "a bare URL" — and every one of them is keyed on a number, which is the one
// thing `just spec-refresh` is free to change. Renumber the fixture and each
// reason above goes on excusing whatever lands on its number, with the suite
// green and the comment still reading like an argument. So the pairing is
// asserted rather than described.
const ALLOWLISTED: [(u32, &str); 15] = [
    (279, "- [ ] foo\n- [x] bar\n"),
    (280, "- [x] foo\n  - [ ] bar\n  - [x] baz\n- [ ] bim\n"),
    (398, "__foo, __bar__, baz__\n"),
    (426, "foo******bar*********baz\n"),
    (434, "__foo __bar__ baz__\n"),
    (435, "____foo__ bar__\n"),
    (436, "**foo **bar****\n"),
    (473, "****foo****\n"),
    (474, "____foo____\n"),
    (475, "******foo******\n"),
    (477, "_____foo_____\n"),
    (610, "<http://foo.bar/baz bim>\n"),
    (616, "< http://foo.bar >\n"),
    (619, "http://example.com\n"),
    (620, "foo@bar.example.com\n"),
];

// What the GFM suite came to, counted through `expected` rather than asserted
// from memory. One field per figure the crate page states, and one per
// `Expectation` arm, so the two stay the same list: a new arm is a new field,
// and a new field is a claim the page test has to spell or stop compiling.
struct Tally {
    total: usize,
    verbatim: usize,
    xml_equivalent: usize,
    re_specified: usize,
    autolinked: usize,
}

fn gfm_tally() -> Tally {
    let mut tally = Tally {
        total: 0,
        verbatim: 0,
        xml_equivalent: 0,
        re_specified: 0,
        autolinked: 0,
    };
    for ex in &load(GFM_FIXTURE) {
        tally.total += 1;
        match expected(ex.example) {
            Expectation::Verbatim => tally.verbatim += 1,
            Expectation::XmlEquivalent => tally.xml_equivalent += 1,
            Expectation::CommonMark0312 => tally.re_specified += 1,
            Expectation::Autolinked(_) => tally.autolinked += 1,
        }
    }
    tally
}

// One paragraph on one line, so a phrase still matches after the file it is
// in has been re-wrapped. Where a Markdown paragraph breaks is a typesetting
// decision and nothing this file has an opinion about.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// An HTML fragment with every tag's attributes in name order and `<x/>`
// written the way `<x>` is. Everything else — text, entity references,
// comments — is passed through byte for byte, deliberately: unescaping the
// text would make `&quot;` and `"` compare equal, and a suite that cannot
// see that difference is not measuring HTML any more.
fn canonical_xml(html: &str) -> String {
    let mut reader = Reader::from_str(html);
    // A fragment is not a document. `<input>` has no end tag in the fixture's
    // output, so the default end-name check would reject the very thing this
    // function exists to read.
    reader.config_mut().check_end_names = false;

    let mut out = String::with_capacity(html.len());
    loop {
        let event = reader
            .read_event()
            .unwrap_or_else(|e| panic!("comparing as XML needs XML, and this is not: {e}\n{html}"));
        match event {
            Event::Eof => break,
            // The two spellings of one element, written as one.
            Event::Start(tag) | Event::Empty(tag) => {
                let mut attributes: Vec<(String, String)> = tag
                    .attributes()
                    .map(|attribute| {
                        let attribute = attribute
                            .unwrap_or_else(|e| panic!("attribute of {html} does not parse: {e}"));
                        (
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            String::from_utf8_lossy(attribute.value.as_ref()).into_owned(),
                        )
                    })
                    .collect();
                attributes.sort();
                out.push('<');
                out.push_str(&String::from_utf8_lossy(tag.name().as_ref()));
                for (key, value) in &attributes {
                    out.push(' ');
                    out.push_str(key);
                    out.push_str("=\"");
                    out.push_str(value);
                    out.push('"');
                }
                out.push('>');
            }
            Event::End(tag) => {
                out.push_str("</");
                out.push_str(&String::from_utf8_lossy(tag.name().as_ref()));
                out.push('>');
            }
            Event::Text(text) => out.push_str(&String::from_utf8_lossy(text.as_ref())),
            // `BytesRef` is what stands between the `&` and the `;`.
            Event::GeneralRef(reference) => {
                out.push('&');
                out.push_str(&String::from_utf8_lossy(reference.as_ref()));
                out.push(';');
            }
            other => panic!("no spec example holds {other:?}, so nothing here canonicalises it"),
        }
    }
    out
}

#[derive(Debug, Deserialize)]
struct SpecExample {
    example: u32,
    section: String,
    markdown: String,
    html: String,
    // Absent throughout the CommonMark fixture, and present on the GFM
    // examples that declare an extension.
    #[serde(default)]
    extension: Option<String>,
}

fn load(fixture: &str) -> Vec<SpecExample> {
    serde_json::from_str(fixture).expect("spec fixture parses as JSON")
}

// Every example a runner reached its assertion on, against every example the
// fixture holds.
//
// This is the hole the whole file was in. A suite reports what it ran and
// never what it did not, so a runner that looped over a FILTERED list and
// `continue`d twice inside the loop — 22 of 672 — was as green as a full
// pass, and stayed that way under a README sentence claiming the suite
// passed. Nothing in the repository could see the difference: not clippy,
// not coverage (the skipped examples are data, not code), not the count
// assertions above, which measure the fixture rather than the run.
//
// Each runner records what it asserted and hands it here, so a filter or a
// `continue` reintroduced anywhere above lands on this line instead of in the
// README. The number-vs-count check catches the other way in: two examples
// under one number, where `expected` cannot name one of them and the loop
// would assert the same input twice while looking complete.
fn assert_nothing_was_skipped(asserted: &BTreeSet<u32>, all: &[SpecExample], suite: &str) {
    let held: BTreeSet<u32> = all.iter().map(|ex| ex.example).collect();
    assert_eq!(
        held.len(),
        all.len(),
        "{suite}: the fixture holds two examples under one number, so a per-example expectation \
         cannot name one of them"
    );
    assert_eq!(
        asserted, &held,
        "{suite}: these examples were loaded and never asserted. A skipped example is not a \
         passing one, and the suite cannot tell you which it was"
    );
}

// A named function rather than a `let` inside the runner, so the pin at the
// bottom of this file can read the same configuration the suite runs with
// instead of a copy of it.
fn commonmark_options() -> Options {
    Options::commonmark().with_raw_html(true)
}

#[test]
fn commonmark_0_31_2_full_pass() {
    let examples = load(COMMONMARK_FIXTURE);
    let opts = commonmark_options();
    assert_eq!(
        examples.len(),
        652,
        "fixture example count must match the spec (re-run `just spec-refresh` if this drifts)"
    );

    let mut asserted = BTreeSet::new();
    for ex in &examples {
        assert_eq!(
            render(&ex.markdown, &opts).html,
            ex.html,
            "CommonMark 0.31.2 example {} (section {:?}) markdown {:?}",
            ex.example,
            ex.section,
            ex.markdown
        );
        asserted.insert(ex.example);
    }
    assert_nothing_was_skipped(&asserted, &examples, "CommonMark 0.31.2");
}

// cmark-gfm's own runner enables only the extension the example declares.
// This does not mirror that: `Options::gfm()` carries all four at once, and a
// caller who reaches for it gets all four at once, so measuring one at a time
// would leave the interaction between them — the configuration the README
// actually names — untested. Every example therefore renders under the whole
// preset, tagged or not, which is the stronger claim of the two and the only
// way the autolink entries in `expected` are visible at all.
//
// `tagfilter` is the one tag `gfm()` does not cover: the filter only bites
// while raw HTML is passing through, which no public constructor can arrange,
// so it stays a switch the runner adds rather than surface a caller can ask
// for.
fn options_for(extension: Option<&str>) -> Options {
    let opts = Options::gfm().with_raw_html(true);
    match extension {
        // Untagged: the GFM spec's inherited CommonMark body, which the
        // preset renders exactly as it renders everything else.
        None | Some("autolink" | "strikethrough" | "table" | "disabled") => opts,
        Some("tagfilter") => opts.with_tagfilter(true),
        Some(other) => panic!("unknown GFM extension tag in fixture: {other}"),
    }
}

#[test]
fn gfm_0_29_full_pass() {
    let all = load(GFM_FIXTURE);
    let newer_spec = load(COMMONMARK_FIXTURE);
    assert_eq!(
        all.len(),
        672,
        "fixture example count must match the spec (re-run `just spec-refresh` if this drifts)"
    );
    // Measured 2026-04-23 against the GFM 0.29 spec: 24 examples carry an
    // explicit extension tag. Floor at 20 to catch a fixture that lost its
    // tags without being brittle against a minor spec refresh. The other 648
    // are asserted by the same loop, so this is no longer a statement about
    // how much of the suite runs.
    let tagged = all.iter().filter(|ex| ex.extension.is_some()).count();
    assert!(
        tagged >= 20,
        "GFM fixture should have at least 20 extension-tagged examples, got {tagged}"
    );

    let mut asserted = BTreeSet::new();
    for ex in &all {
        let opts = options_for(ex.extension.as_deref());
        let got = render(&ex.markdown, &opts).html;
        let at = format!(
            "GFM 0.29 example {} (section {:?}, extension {:?}) markdown {:?}",
            ex.example, ex.section, ex.extension, ex.markdown
        );
        match expected(ex.example) {
            Expectation::Verbatim => assert_eq!(got, ex.html, "{at}"),
            Expectation::XmlEquivalent => {
                assert_eq!(canonical_xml(&got), canonical_xml(&ex.html), "{at}");
                assert_ne!(
                    got, ex.html,
                    "{at} now matches the fixture byte for byte — compare it that way and drop \
                     its `XmlEquivalent` entry"
                );
            }
            Expectation::CommonMark0312 => {
                let supersedes: Vec<&SpecExample> = newer_spec
                    .iter()
                    .filter(|candidate| candidate.markdown == ex.markdown)
                    .collect();
                assert_eq!(
                    supersedes.len(),
                    1,
                    "{at} needs exactly one CommonMark 0.31.2 example carrying the same input for \
                     that spec version to be what supersedes it"
                );
                let supersedes = supersedes[0];
                assert_ne!(
                    supersedes.html, ex.html,
                    "{at} agrees with CommonMark 0.31.2 example {} — the two spec versions have \
                     converged, so drop its entry",
                    supersedes.example
                );
                assert_eq!(got, supersedes.html, "{at}");
            }
            Expectation::Autolinked(html) => {
                assert_ne!(
                    html, ex.html,
                    "{at} is what the fixture asks for — drop its `Autolinked` entry"
                );
                assert_eq!(got, html, "{at}");
                assert_eq!(
                    render(&ex.markdown, &opts.clone().with_autolinks(false)).html,
                    ex.html,
                    "{at} must come back verbatim once the autolink extension is off; while it \
                     does not, the extension is not what supersedes it and this is a defect"
                );
            }
        }
        asserted.insert(ex.example);
    }
    assert_nothing_was_skipped(&asserted, &all, "GFM 0.29");
}

#[test]
fn the_allowlist_is_the_inputs_it_names_and_nothing_else() {
    // `expected` is the successor of a two-element const of example numbers
    // that the runner used to `continue` past, and it inherits that const's
    // weakness: it is a `match`, so it can be widened by one line, and a
    // widened one is invisible — every arm it grows makes the suite MORE
    // green. An exception whose only reader is the person adding to it is the
    // same shape as the skip it replaced, so a widening has to show up here as
    // a table entry with a reason, and on the crate page as a changed figure.
    let all = load(GFM_FIXTURE);
    let named: BTreeMap<u32, &str> = ALLOWLISTED.into_iter().collect();
    let listed: BTreeSet<u32> = all
        .iter()
        .map(|ex| ex.example)
        .filter(|&example| !matches!(expected(example), Expectation::Verbatim))
        .collect();
    assert_eq!(
        listed,
        named.keys().copied().collect::<BTreeSet<u32>>(),
        "`expected` forgives a different set of examples than the table beside it names. Every \
         entry costs the suite one example of its authority, so growing the list is a decision \
         with a reason, and shrinking it is a divergence that has closed"
    );

    let mut matched = 0_usize;
    for ex in &all {
        if let Some(&markdown) = named.get(&ex.example) {
            matched += 1;
            assert_eq!(
                ex.markdown, markdown,
                "GFM 0.29 example {} is no longer the input its allowlist entry was written \
                 about; a refresh renumbered the fixture and the reason above it now excuses \
                 whatever landed on that number",
                ex.example
            );
        }
    }
    assert_eq!(
        matched,
        ALLOWLISTED.len(),
        "an allowlist entry names an example number the fixture does not hold, so its reason is \
         about nothing"
    );
}

#[test]
fn the_xml_comparison_forgives_attribute_order_and_self_closing_style_and_nothing_else() {
    // The two task-list examples used to be `continue`d past, and a skip is
    // the most permissive oracle there is: it accepts every possible output,
    // including none. `XmlEquivalent` is narrower — but nothing proves HOW
    // much narrower, and a normaliser that over-normalises is a skip with
    // extra steps, green for the same reason and harder to notice. So the
    // forgiveness is measured from both sides: what it must equate, and what
    // it must still refuse.
    assert_eq!(
        canonical_xml(r#"<input type="checkbox" disabled="" /> foo"#),
        canonical_xml(r#"<input disabled="" type="checkbox"> foo"#),
        "attribute order and self-closing style are the whole of what 279 and 280 are forgiven"
    );

    let all = load(GFM_FIXTURE);
    let task_list = all
        .iter()
        .find(|ex| ex.example == 279)
        .unwrap_or_else(|| panic!("GFM 0.29 example 279 is the task-list example"));
    let fixture = task_list.html.as_str();
    let mut reordered: Vec<&str> = fixture.lines().collect();
    reordered.swap(1, 2);
    let corrupted = [
        (
            "an attribute's value",
            fixture.replace(r#"checked="""#, r#"checked="1""#),
        ),
        // `kind` for `type` and nothing else: same value, and it sorts into
        // the same place among the others, so a comparison that dropped the
        // attribute NAMES would still see two identical documents here. That
        // is the mutant this case exists for, and `data-type` did not kill
        // it — it reordered the values, and so passed for the wrong reason.
        ("an attribute's name", fixture.replace("type=", "kind=")),
        (
            "a dropped attribute",
            fixture.replace(r#" disabled="""#, ""),
        ),
        (
            "an added attribute",
            fixture.replace(r#"type="checkbox""#, r#"type="checkbox" name="x""#),
        ),
        ("an element's name", fixture.replace("li>", "div>")),
        ("the text beside the element", fixture.replace("foo", "qux")),
        (
            "the space in front of that text",
            fixture.replace("> foo", ">foo"),
        ),
        ("a line break", fixture.replace("\n<li>", "<li>")),
        (
            "the order of two siblings",
            format!("{}\n", reordered.join("\n")),
        ),
    ];
    for (what, html) in corrupted {
        assert_ne!(
            html, fixture,
            "the {what} case rewrites nothing, so it asks the comparison nothing"
        );
        assert_ne!(
            canonical_xml(&html),
            canonical_xml(fixture),
            "canonical XML cannot see {what}. Comparing 279 and 280 that way is the skip it \
             replaced, spelled as a function"
        );
    }

    // The one normalisation this must NOT grow. Unescaping the text would
    // make a document that writes `&quot;` and one that writes `"` compare
    // equal, and escaping is most of what an HTML renderer is for.
    assert_ne!(
        canonical_xml("<p>&quot;</p>"),
        canonical_xml(r#"<p>"</p>"#),
        "an entity reference and the character it stands for are not the same output"
    );
}

#[test]
fn the_crate_page_states_the_figures_this_file_measures() {
    // The claim and its proof, wired together. Both were true separately for
    // months and false about each other: the page said the GFM suite passed
    // verbatim while the runner asserted 22 of 672, and every gate in the
    // repository was green — prose is not compiled, and the one thing that
    // could have compared them was the file being wrong.
    //
    // So each figure below is FORMATTED from the measurement rather than
    // written down. Rewording the page is free; changing what it claims,
    // or changing what the suite measures without saying so, is not.
    //
    // Destructured, and every field spent: an unused binding is a warning and
    // warnings are denied, so the list below cannot fall a figure behind the
    // measurement the way it did while the page spelled three as words.
    let Tally {
        total,
        verbatim,
        xml_equivalent,
        re_specified,
        autolinked,
    } = gfm_tally();
    let superseded = re_specified + autolinked;
    let commonmark = load(COMMONMARK_FIXTURE).len();
    let page = unwrapped(CRATE_README);
    let claims = [
        format!("renders all {commonmark} CommonMark 0.31.2 spec examples"),
        format!("renders all {total} GFM 0.29 spec examples"),
        format!("{verbatim} of the {total} come out verbatim"),
        format!("{xml_equivalent} more do once"),
        format!("the last {superseded} are pinned"),
        format!("{re_specified} emphasis cases"),
        format!("{autolinked} bare URLs"),
        format!("the list of {superseded}"),
        "Nothing is skipped".to_owned(),
    ];
    for claim in &claims {
        assert!(
            page.contains(claim.as_str()),
            "crates/aozora-flavored-markdown/README.md is this crate's page on crates.io and it \
             does not say {claim:?}. That is the figure this file measures; the page has to be \
             the same number, in whatever words"
        );
    }
}

#[test]
fn gfm_extension_tags_are_exhaustive() {
    // Sanity: every tag the fixture carries is one `options_for` handles.
    let tags: BTreeSet<String> = load(GFM_FIXTURE)
        .iter()
        .filter_map(|e| e.extension.clone())
        .collect();
    let known: BTreeSet<String> = GFM_EXTENSION_TAGS.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        tags, known,
        "GFM fixture contains a tag this runner does not handle; update `options_for`"
    );
}

#[test]
fn each_runner_measures_the_public_preset_the_readme_names() {
    // A green suite says the examples passed. It does not say WHICH
    // configuration passed them, and that is the whole of what the README
    // sends a reader here to check. Until this test the GFM runner built on
    // `Options::commonmark()` and switched on the single extension each
    // example declared: green, and proof of a preset no caller can name —
    // the README's sentence about `Options::gfm()` would have been untrue
    // with every gate in the repository still passing. Narrowing it back
    // that way now fails here.
    assert_eq!(
        commonmark_options().with_raw_html(false),
        Options::commonmark(),
        "the CommonMark runner must measure `Options::commonmark()` itself"
    );
    for tag in GFM_EXTENSION_TAGS.map(Some).into_iter().chain([None]) {
        let opts = options_for(tag);
        assert_eq!(
            opts.clone().with_raw_html(false).with_tagfilter(false),
            Options::gfm(),
            "the GFM runner must measure `Options::gfm()` itself, and it does not for {tag:?}"
        );
        // Both deltas are documented above `options_for`, and only one of
        // them is per-example. A tag that stopped passing raw HTML through
        // would silently stop rendering what the fixture expects; a tag that
        // started switching the filter on would be measuring cmark-gfm's
        // tagfilter configuration under the name of the whole preset.
        assert_ne!(
            opts.clone().with_raw_html(false),
            opts,
            "the fixture's expected output contains raw HTML, so {tag:?} must pass it through"
        );
        assert_eq!(
            opts.clone().with_tagfilter(false) != opts,
            tag == Some("tagfilter"),
            "only the tagfilter examples may switch the GFM tag filter on, not {tag:?}"
        );
    }
}
