//! Differential test — this crate's HTML vs the upstream renderer's.
//!
//! Both pipelines consume the same lexer output and the same per-construct
//! renderer, so on pure-青空文庫 input the *count and presence* of every
//! `aozora-md-*` class token must match. Block structure differs (this crate
//! wraps paragraphs through comrak; upstream emits its own `<p>` tags), so
//! the differential is on histograms, not on byte equality.
//!
//! What this differential catches:
//!
//! - **Drift between the two front doors.** A regression that flips a class
//!   name upstream breaks both sides in tandem; one that only edits the
//!   aozora-md-side post-process surfaces as an asymmetry.
//! - **Class contract leakage.** Both renderers source their `aozora-md-*`
//!   classes from the same pinned list (`AOZORA_MD_CLASSES` in
//!   `aozora_flavored_markdown_test_support`). A renderer that emits an
//!   unregistered class shows up here.
//! - **Tier-A / Tier-B consistency.** Both renderers must satisfy the
//!   no-bare-bracket and no-PUA-leak contracts on lexer-clean input.
//! - **Serializer equivalence.** `aozora_flavored_markdown::serialize` is a
//!   thin delegate to the upstream serializer; on the same source they must
//!   produce identical bytes.

use std::collections::{HashMap, HashSet};

use aozora::pipeline::lex_into_arena;
use aozora::render::html as aozora_html;
use aozora::render::serialize as aozora_serialize;
use aozora::syntax::borrowed::Arena;
use aozora_flavored_markdown::html as md_html;
use aozora_flavored_markdown_test_support::AOZORA_MD_CLASSES;

/// A handful of pure-aozora source fragments — no CommonMark
/// emphasis, no headings, no lists, no code spans. Each fragment
/// exercises a distinct aozora-side recogniser so the differential
/// has signal even on a small corpus.
fn pure_aozora_fixtures() -> &'static [(&'static str, &'static str)] {
    &[
        ("plain ASCII", "Hello, world."),
        ("plain Japanese", "親譲りの無鉄砲"),
        ("explicit ruby", "｜青梅《おうめ》"),
        ("implicit ruby", "親譲《おやゆず》り"),
        ("forward bouten", "可哀想［＃「可哀想」に傍点］"),
        ("page break standalone", "［＃改ページ］"),
        ("page break mid", "前［＃改ページ］後"),
        ("section break choho", "［＃改丁］"),
        ("indent leaf", "［＃地付き］"),
        (
            "two ruby in one paragraph",
            "｜青梅《おうめ》と｜鶴見《つるみ》の間",
        ),
        (
            "indent container",
            "［＃ここから2字下げ］\n\n本文\n\n［＃ここで字下げ終わり］",
        ),
        ("multi paragraph", "first\n\nsecond"),
    ]
}

/// Render `src` through the upstream HTML renderer (pure-aozora pipeline).
fn aozora_only_render(src: &str) -> String {
    let arena = Arena::new();
    let lex_out = lex_into_arena(src, &arena);
    aozora_html::render_to_string(&lex_out)
}

/// Tally every class token starting with `prefix` in `html`. The
/// histogram key is the **stem** (the substring after `prefix`), so
/// the upstream `aozora-*` brand surface and the `aozora-md-*` brand from
/// this crate can be compared shape-for-shape despite the different
/// prefixes.
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

#[test]
fn both_renderers_agree_on_class_histogram_for_pure_aozora_input() {
    // The two surfaces emit different brands (`aozora-*` vs `aozora-md-*`)
    // by design — see the brand-boundary handling in `ast_splice`
    // (ADR-0011). The differential compares stems
    // (the part after the prefix) so divergence flags a recogniser
    // drift, not the brand difference.
    let mut diffs = Vec::new();
    for (label, src) in pure_aozora_fixtures() {
        let aozora_out = aozora_only_render(src);
        let md_out = md_html::render_to_string(src);
        let aozora_hist = class_stem_histogram(&aozora_out, "aozora-");
        let md_hist = class_stem_histogram(&md_out, "aozora-md-");
        if aozora_hist != md_hist {
            diffs.push(format!(
                "{label} ({src:?})\n  aozora-: {aozora_hist:?}\n  aozora-md-:    {md_hist:?}"
            ));
        }
    }
    assert!(
        diffs.is_empty(),
        "class stem histograms diverge between renderers:\n\n{}",
        diffs.join("\n\n"),
    );
}

#[test]
fn every_emitted_class_is_in_the_pinned_contract() {
    // The pinned list (`AOZORA_MD_CLASSES`) tracks the `aozora-md-*` stems
    // this crate emits. The `aozora-*` brand from the sibling renderer is
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
    use aozora_flavored_markdown_test_support::strip_annotation_wrappers;
    for (label, src) in pure_aozora_fixtures() {
        for (renderer, html) in [
            ("aozora", aozora_only_render(src)),
            ("aozora-md", md_html::render_to_string(src)),
        ] {
            let stripped = strip_annotation_wrappers(&html);
            assert!(
                !stripped.contains("［＃"),
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
fn aozora_flavored_markdown_serialize_matches_aozora_render_serialize() {
    // `aozora_flavored_markdown::serialize` is a thin delegate to the
    // sibling serializer, so the two must produce identical bytes for the
    // same source.
    for (label, src) in pure_aozora_fixtures() {
        let arena = Arena::new();
        let lex_out = lex_into_arena(src, &arena);
        let aozora_out = aozora_serialize::serialize(&lex_out);
        let md_out = aozora_flavored_markdown::serialize(src);
        assert_eq!(
            md_out, aozora_out,
            "serialize drift on {label} ({src:?}):\n  aozora-md: {md_out:?}\n  aozora: {aozora_out:?}"
        );
    }
}
