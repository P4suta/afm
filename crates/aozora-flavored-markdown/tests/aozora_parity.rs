//! Fidelity gate — this crate's HTML against the parser's own.
//!
//! This crate does not know what a 青空文庫 notation renders to. It asks the
//! parser, one construct at a time, by handing back the source run the
//! construct occupies and keeping the HTML that comes back
//! (`crate::fragment`). That is the whole of its rendering, and this is the
//! test that says it worked: render a pure-青空文庫 document both ways —
//! through this crate, and through the parser's own front door — and require
//! the 青空文庫 markup in the two outputs to be the same.
//!
//! Only the public surface is used on the parser's side ([`Document`],
//! [`Document::parse`], `to_html`, `serialize`), because that is all this
//! crate is allowed to depend on. A regression that reaches for something
//! else would not be caught here; a regression in what a construct renders
//! to would be, immediately.
//!
//! Block structure legitimately differs — comrak owns it here, the parser
//! owns it there — so the comparison is on the 青空文庫 markup rather than on
//! bytes:
//!
//! * every branded class token, counted (`aozora-md-bouten-goma` here has to
//!   be `aozora-bouten-goma` there, the same number of times);
//! * every `data-*` attribute value, counted (a gaiji's codepoint, an
//!   indent's amount, an alignment's offset — the payload that does not
//!   reach a class name).
//!
//! Where the two are *meant* to differ, they differ in unbranded markup and
//! this test stays quiet by construction: a heading hint promotes a
//! paragraph to `<h1>` here and renders as a comment there; an orphan
//! container close is dropped here and emitted unbalanced there; a container
//! the source never closes is closed here. None of those carry a class or a
//! `data-` attribute.

use std::collections::{HashMap, HashSet};

use aozora::Document;
use aozora_flavored_markdown::Options;
use aozora_flavored_markdown::html as md_html;
use aozora_flavored_markdown_test_support::{AOZORA_MD_CLASSES, check_html_tag_balance};

/// Pure-青空文庫 documents: no CommonMark emphasis, headings, lists or code
/// spans, so the only markup either side emits for them is 青空文庫 markup.
///
/// The corpus is this gate's reach. It carries one document per recogniser
/// this crate can reach — the inline families, the block leaves, the paired
/// containers and the three ways a container can go wrong — plus the inputs
/// that decide *how* a construct's source run is found.
///
/// That last group is what makes this a gate rather than a smoke test. A run
/// is sliced straight out of the source only when this crate could prove the
/// parser's ranges address it; otherwise it has to be *recovered*, and a
/// recovery that comes up empty renders the construct — the author's text
/// included — as nothing at all. So the corpus carries a document per way of
/// reaching that path: one the parser rewrites (an accent digraph), one this
/// crate rewrites on its behalf (CRLF), and the combination every real
/// 青空文庫 file is — CRLF plus a decorative rule — where the two rewrites
/// cancel out into an offset that lands outside the block it names.
fn pure_aozora_fixtures() -> &'static [(&'static str, &'static str)] {
    &[
        ("plain ASCII", "Hello, world."),
        ("plain Japanese", "親譲りの無鉄砲"),
        ("explicit ruby", "｜青梅《おうめ》"),
        ("implicit ruby", "親譲《おやゆず》り"),
        (
            "two ruby in one paragraph",
            "｜青梅《おうめ》と｜鶴見《つるみ》の間",
        ),
        (
            "the same ruby twice",
            "｜青梅《おうめ》から｜青梅《おうめ》まで",
        ),
        ("forward bouten", "可哀想［＃「可哀想」に傍点］"),
        ("bouten on the left", "可哀想［＃「可哀想」の左に傍点］"),
        ("tate chu yoko", "20［＃「20」は縦中横］"),
        ("kaeriten", "天［＃レ］"),
        ("gaiji", "※［＃「木＋吶のつくり」、第3水準1-85-54］の字"),
        ("sashie", "［＃挿絵（fig1.png）入る］"),
        ("unknown annotation", "［＃本文終わり］"),
        ("warichu", "［＃割り注］うえ／＼した［＃割り注終わり］"),
        ("page break standalone", "［＃改ページ］"),
        ("page break mid", "前［＃改ページ］後"),
        ("section break choho", "［＃改丁］"),
        ("indent leaf", "［＃地付き］"),
        ("indent amount leaf", "［＃３字下げ］"),
        ("align end leaf", "［＃地から３字上げ］"),
        (
            "indent container",
            "［＃ここから2字下げ］\n\n本文\n\n［＃ここで字下げ終わり］",
        ),
        (
            "align end container",
            "［＃ここから地から２字上げ］\n\n本文\n\n［＃ここで地から２字上げ終わり］",
        ),
        (
            "container holding notation",
            "［＃ここから3字下げ］\n\n｜青梅《おうめ》\n\n［＃ここで字下げ終わり］",
        ),
        (
            "container the source never closes",
            "［＃ここから字下げ］\n\n本文",
        ),
        ("close with no open", "本文\n\n［＃ここで字下げ終わり］"),
        ("heading hint", "第一篇［＃「第一篇」は大見出し］"),
        ("multi paragraph", "first\n\nsecond"),
        ("CRLF line endings", "｜青梅《おうめ》\r\n｜鶴見《つるみ》"),
        ("accent digraph", "〔e'tude〕と｜青梅《おうめ》"),
        (
            "accent digraph above a container",
            "［＃ここから２字下げ］\n\n〔e'tude〕本文\n\n［＃ここで字下げ終わり］",
        ),
        (
            "CRLF plus a decorative rule",
            "本文\r\n----------\r\n｜青梅《おうめ》",
        ),
        (
            "CRLF plus a decorative rule, with blocks",
            "夏目漱石\r\n\r\n----------\r\n\r\n｜親譲《おやゆず》りの無鉄砲で\r\n\r\n［＃改ページ］\r\n\r\n可哀想［＃「可哀想」に傍点］だ",
        ),
        (
            "CRLF plus a decorative rule, around a container",
            "本文\r\n----------\r\n［＃ここから２字下げ］\r\n\r\n｜青梅《おうめ》\r\n\r\n［＃ここで字下げ終わり］",
        ),
    ]
}

/// Render `src` through the parser's own front door — the whole of the
/// surface this crate is allowed to reach for.
fn aozora_only_render(src: &str) -> String {
    Document::new(src).parse().to_html()
}

/// The parser's output under this crate's brand (ADR-0011).
///
/// The Tier contracts are written against `aozora-md-*` names — an
/// annotation wrapper is what makes a bracket run legal, and the checker
/// looks for it by name. Rebranding is what lets the same checker be pointed
/// at both outputs; that the two brands really are the same names is what
/// the class-histogram test above asserts.
fn rebranded(html: &str) -> String {
    html.replace("aozora-", "aozora-md-")
}

/// Tally every class token starting with `prefix` in `html`. The
/// histogram key is the **stem** (the substring after `prefix`), so
/// the parser's `aozora-*` brand and the `aozora-md-*` brand from
/// this crate can be compared shape-for-shape despite the different
/// prefixes (ADR-0011).
fn class_stem_histogram(html: &str, prefix: &str) -> HashMap<String, usize> {
    let mut hist = HashMap::new();
    for token_run in html.split("class=\"").skip(1) {
        let Some(end) = token_run.find('"') else {
            continue;
        };
        for token in token_run[..end].split_ascii_whitespace() {
            if let Some(stem) = token.strip_prefix(prefix) {
                *hist.entry(stem.to_owned()).or_insert(0) += 1;
            }
        }
    }
    hist
}

/// Tally every `data-*` attribute in `html` as a `name=value` key.
///
/// A construct's payload does not always reach a class name: a gaiji's
/// resolved codepoint, an indent's amount and an alignment's offset all ride
/// on `data-` attributes. Comparing them alongside the classes is what makes
/// this a check on the fragment rather than on its family.
fn data_attribute_histogram(html: &str) -> HashMap<String, usize> {
    let mut hist = HashMap::new();
    for attr in html.split("data-").skip(1) {
        let Some(eq) = attr.find("=\"") else {
            continue;
        };
        let Some(end) = attr[eq + 2..].find('"') else {
            continue;
        };
        let entry = format!("{}={}", &attr[..eq], &attr[eq + 2..eq + 2 + end]);
        *hist.entry(entry).or_insert(0) += 1;
    }
    hist
}

#[test]
fn both_renderers_agree_on_the_aozora_markup_of_pure_aozora_input() {
    let mut diffs = Vec::new();
    for (label, src) in pure_aozora_fixtures() {
        let aozora_out = aozora_only_render(src);
        let md_out = md_html::render_to_string(src);
        let aozora_classes = class_stem_histogram(&aozora_out, "aozora-");
        let md_classes = class_stem_histogram(&md_out, "aozora-md-");
        if aozora_classes != md_classes {
            diffs.push(format!(
                "{label} ({src:?}) — classes\n  aozora:    {aozora_classes:?}\n  aozora-md: {md_classes:?}"
            ));
        }
        let aozora_data = data_attribute_histogram(&aozora_out);
        let md_data = data_attribute_histogram(&md_out);
        if aozora_data != md_data {
            diffs.push(format!(
                "{label} ({src:?}) — data attributes\n  aozora:    {aozora_data:?}\n  aozora-md: {md_data:?}"
            ));
        }
    }
    assert!(
        diffs.is_empty(),
        "the two renderers disagree on what a notation renders to:\n\n{}",
        diffs.join("\n\n"),
    );
}

#[test]
fn every_emitted_class_is_in_the_pinned_contract() {
    // The pinned list (`AOZORA_MD_CLASSES`) tracks the `aozora-md-*` stems
    // this crate emits. The `aozora-*` brand from the parser is
    // checked against the same stems with a `aozora-` prefix strip — same
    // family of stems, different brand prefix.
    let known: HashSet<&'static str> = AOZORA_MD_CLASSES.iter().copied().collect();
    let mut violations = Vec::new();
    for (label, src) in pure_aozora_fixtures() {
        for (renderer, html, prefix) in [
            ("aozora", aozora_only_render(src), "aozora-"),
            ("aozora-md", md_html::render_to_string(src), "aozora-md-"),
        ] {
            for (stem, _count) in class_stem_histogram(&html, prefix) {
                let full = format!("aozora-md-{stem}");
                if known.contains(full.as_str()) {
                    continue;
                }
                // Family-suffix variants — `aozora-md-indent-2`,
                // `aozora-md-section-break-choho`, `aozora-md-bouten-goma`-suffixed
                // forms, etc. Accept any suffix when the family stem
                // is in the pinned list.
                if let Some(stem_end) = full.rfind('-') {
                    let family = &full[..stem_end];
                    if known.contains(family) {
                        continue;
                    }
                }
                violations.push(format!(
                    "{renderer} emitted unknown stem {stem:?} for {label} ({src:?})"
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "unknown class stems:\n  {}",
        violations.join("\n  "),
    );
}

#[test]
fn both_renderers_satisfy_tier_a_no_bare_bracket() {
    use aozora_flavored_markdown_test_support::check_no_bare_bracket;
    for (label, src) in pure_aozora_fixtures() {
        for (renderer, html) in [
            ("aozora", rebranded(&aozora_only_render(src))),
            ("aozora-md", md_html::render_to_string(src)),
        ] {
            assert!(
                check_no_bare_bracket(&html).is_ok(),
                "{renderer} Tier A leaked ［＃ on {label} ({src:?}): {html}"
            );
        }
    }
}

#[test]
fn both_renderers_satisfy_tier_b_no_pua_leak() {
    use aozora_flavored_markdown::sentinels;
    for (label, src) in pure_aozora_fixtures() {
        for (renderer, html) in [
            ("aozora", aozora_only_render(src)),
            ("aozora-md", md_html::render_to_string(src)),
        ] {
            for s in [
                sentinels::INLINE,
                sentinels::BLOCK_LEAF,
                sentinels::BLOCK_OPEN,
                sentinels::BLOCK_CLOSE,
            ] {
                assert!(
                    !html.contains(s),
                    "{renderer} Tier B leaked sentinel {s:?} on {label} ({src:?}): {html}"
                );
            }
        }
    }
}

#[test]
fn this_crate_closes_every_tag_it_opens() {
    // The parser emits a container's open tag whether or not the source
    // closes it, and emits a close whether or not one was opened. This
    // crate does neither: it closes what it opened and drops what it did
    // not, which is the one place its block structure is deliberately
    // better rather than merely different. Worth asserting here because a
    // construct whose source run went missing used to take its markup's
    // other half with it.
    for (label, src) in pure_aozora_fixtures() {
        let html = md_html::render_to_string(src);
        assert!(
            check_html_tag_balance(&html).is_ok(),
            "unbalanced markup on {label} ({src:?}): {:?}\n{html}",
            check_html_tag_balance(&html),
        );
    }
}

#[test]
fn every_construct_of_a_pure_aozora_document_is_accounted_for() {
    // The corpus reaches the recovery path on purpose (see
    // `pure_aozora_fixtures`), and a construct lost there is reported
    // rather than dropped in silence. Nothing in the corpus may be lost:
    // the histogram test above says the *markup* agrees, and this one says
    // it agrees because every construct was found, not because two
    // documents were equally empty.
    for (label, src) in pure_aozora_fixtures() {
        let rendered = aozora_flavored_markdown::render(src, &Options::default());
        let lost: Vec<_> = rendered
            .diagnostics
            .iter()
            .filter(|d| d.code == "aozora-md::constructs_unresolved")
            .collect();
        assert!(
            lost.is_empty(),
            "{label} ({src:?}) lost a construct: {lost:?}"
        );
    }
}

#[test]
fn aozora_flavored_markdown_serialize_matches_the_parsers_own() {
    // `aozora_flavored_markdown::serialize` is a thin delegate, so the two
    // must produce identical bytes for the same source.
    for (label, src) in pure_aozora_fixtures() {
        let aozora_out = Document::new(*src).parse().serialize();
        let md_out = aozora_flavored_markdown::serialize(src);
        assert_eq!(
            md_out, aozora_out,
            "serialize drift on {label} ({src:?}):\n  aozora-md: {md_out:?}\n  aozora: {aozora_out:?}"
        );
    }
}
