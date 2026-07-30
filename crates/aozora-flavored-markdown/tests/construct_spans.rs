//! The premise the source-coordinate design rests on, pinned as a test.
//!
//! Every 青空文庫 construct is projected with the byte range its notation
//! occupies in the source. Two things have to be true of that range, or the
//! design falls over:
//!
//! 1. **It covers everything the notation resolves against.** A forward
//!    reference (`可哀想［＃「可哀想」に傍点］`) names text that *precedes*
//!    the directive; the range has to include that text, not just the
//!    bracket run.
//! 2. **The text it slices resolves the same way on its own.** Parsing the
//!    slice as a document of its own and unwrapping the paragraph it lands
//!    in has to reproduce the fragment the whole-document render produced.
//!
//! (2) is what lets the fragment for a construct be produced from its range
//! alone. It is asserted here as byte equality, so a construct whose
//! resolution actually depends on its surroundings fails loudly instead of
//! drifting.

use core::iter;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use aozora_flavored_markdown::ir::{Block, ByteSpan, Inline};
use aozora_flavored_markdown::{Options, render_to_ir};

/// One projected construct: its tag, its range, and its HTML fragment.
struct Projected {
    kind: String,
    span: Option<ByteSpan>,
    html: String,
}

/// Every construct the IR projects for `src`, in document order.
fn constructs_of(src: &str) -> Vec<Projected> {
    let mut out = Vec::new();
    collect_blocks(&render_to_ir(src, &Options::default()).ir.blocks, &mut out);
    out
}

fn collect_blocks(blocks: &[Block], out: &mut Vec<Projected>) {
    for block in blocks {
        match block {
            Block::Aozora {
                kind, span, html, ..
            } => out.push(Projected {
                kind: kind.clone(),
                span: *span,
                html: html.clone(),
            }),
            Block::Paragraph { children, .. } | Block::Heading { children, .. } => {
                collect_inlines(children, out);
            }
            Block::Blockquote { children, .. } => collect_blocks(children, out),
            Block::List { items, .. } => {
                for item in items {
                    collect_blocks(&item.children, out);
                }
            }
            Block::Table { header, rows, .. } => {
                for row in iter::once(header).chain(rows) {
                    for cell in &row.cells {
                        collect_inlines(cell, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_inlines(inlines: &[Inline], out: &mut Vec<Projected>) {
    for inline in inlines {
        match inline {
            Inline::Aozora { kind, span, html } => out.push(Projected {
                kind: kind.clone(),
                span: *span,
                html: html.clone(),
            }),
            Inline::Strong { children, .. }
            | Inline::Emphasis { children, .. }
            | Inline::Link { children, .. }
            | Inline::Image { alt: children, .. } => collect_inlines(children, out),
            _ => {}
        }
    }
}

/// The fragment a construct's own source text renders to, on its own:
/// parse the slice as a document, unwrap the paragraph the upstream
/// renderer wraps inline content in, and rebrand to this crate's classes
/// (ADR-0011).
///
/// A container marker with nothing inside it renders as one empty element,
/// and what this crate splices in its place is the half that opens it — so
/// `kind` decides which half of the fragment the projection is compared
/// against.
fn fragment_from_slice(slice: &str, kind: &str) -> String {
    let document = aozora::parse(slice.to_owned()).expect("a construct's run is small");
    let html = document.snapshot().to_html();
    let unwrapped = html
        .trim()
        .strip_prefix("<p>")
        .and_then(|rest| rest.strip_suffix("</p>"))
        .unwrap_or_else(|| html.trim());
    let rebranded = unwrapped.replace("aozora-", "aozora-md-");
    if kind == CONTAINER_OPEN {
        return rebranded
            .rfind("</")
            .map_or_else(|| rebranded.clone(), |at| rebranded[..at].to_owned());
    }
    rebranded
}

/// Tag of the block that opens a paired container.
const CONTAINER_OPEN: &str = "containerOpen";
/// Tag of the block that closes one. Its markup comes from the marker that
/// opened it — a close renders to nothing on its own — so its fragment is
/// not compared against its own run.
const CONTAINER_CLOSE: &str = "containerClose";

/// The notation zoo: one document per construct family this crate can
/// project, each with the exact source run its range must cover.
const ZOO: &[(&str, &str)] = &[
    ("彼は｜青梅《おうめ》に行った。", "｜青梅《おうめ》"),
    ("親譲《おやゆず》りの無鉄砲", "親譲《おやゆず》"),
    (
        "可哀想［＃「可哀想」に傍点］だ",
        "可哀想［＃「可哀想」に傍点］",
    ),
    ("20［＃「20」は縦中横］です", "20［＃「20」は縦中横］"),
    ("※［＃二の字点、1-2-22］の外字", "※［＃二の字点、1-2-22］"),
    ("天［＃レ］地", "［＃レ］"),
    ("≪強調≫の語", "≪強調≫"),
    ("前［＃改ページ］後", "［＃改ページ］"),
    ("前［＃改丁］後", "［＃改丁］"),
    ("［＃挿絵（fig1.png）入る］", "［＃挿絵（fig1.png）入る］"),
    ("前［＃ほげふが］後", "［＃ほげふが］"),
];

#[test]
fn every_range_covers_the_notation_the_author_wrote() {
    for (src, expected) in ZOO {
        let projected = constructs_of(src);
        let first = projected
            .first()
            .unwrap_or_else(|| panic!("{src:?} projects no construct"));
        let span = first
            .span
            .unwrap_or_else(|| panic!("{src:?} projects no range"));
        assert_eq!(
            &src[span.start as usize..span.end as usize],
            *expected,
            "the range for {src:?} must cover exactly the notation"
        );
    }
}

#[test]
fn a_ranges_own_text_resolves_to_the_same_fragment_on_its_own() {
    for (src, _) in ZOO {
        for construct in constructs_of(src) {
            let Some(span) = construct.span else {
                panic!("{src:?} projects {} without a range", construct.kind);
            };
            if construct.kind == CONTAINER_CLOSE {
                continue;
            }
            let slice = &src[span.start as usize..span.end as usize];
            assert_eq!(
                fragment_from_slice(slice, &construct.kind),
                construct.html,
                "{src:?}: the {} at {slice:?} must resolve the same on its own",
                construct.kind,
            );
        }
    }
}

/// A paired container's two markers each get their own range, so the open
/// and close halves can be produced independently.
#[test]
fn container_markers_get_a_range_each() {
    const SRC: &str = "［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］";
    let projected = constructs_of(SRC);
    let kinds: Vec<&str> = projected.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, [CONTAINER_OPEN, CONTAINER_CLOSE], "{kinds:?}");
    for construct in &projected {
        let span = construct.span.expect("both markers carry a range");
        let slice = &SRC[span.start as usize..span.end as usize];
        assert!(
            slice.starts_with("［＃") && slice.ends_with('］'),
            "the {} range must cover its marker, got {slice:?}",
            construct.kind
        );
        if construct.kind == CONTAINER_CLOSE {
            // The close half of the open marker's element — a close marker
            // renders to nothing on its own, having no open to close.
            assert_eq!(construct.html, "</div>");
            continue;
        }
        assert_eq!(
            fragment_from_slice(slice, &construct.kind),
            construct.html,
            "the {} marker must resolve the same on its own",
            construct.kind
        );
    }
}

/// What one sweep of a set of documents saw.
#[derive(Debug, Default)]
struct Swept {
    seen: usize,
    with_range: usize,
    per_kind: BTreeMap<String, usize>,
}

/// Project every construct in `documents` and check each range it carries:
/// it must slice the source (in bounds, on a codepoint boundary) and
/// resolve to the same fragment on its own.
///
/// A construct *without* a range is counted, not failed on — a document the
/// parser rewrote before lexing projects none, which is the documented
/// fallback rather than a broken premise.
fn sweep(documents: &[String]) -> Swept {
    let mut swept = Swept::default();
    for src in documents {
        for construct in constructs_of(src) {
            swept.seen += 1;
            let Some(span) = construct.span else {
                continue;
            };
            swept.with_range += 1;
            *swept.per_kind.entry(construct.kind.clone()).or_default() += 1;
            let slice = src
                .get(span.start as usize..span.end as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "a projected range must slice the source it was measured against: \
                         {span:?} of {src:?}"
                    )
                });
            if construct.kind == CONTAINER_CLOSE {
                continue;
            }
            assert_eq!(
                fragment_from_slice(slice, &construct.kind),
                construct.html,
                "the {} at {slice:?} must resolve the same on its own",
                construct.kind
            );
        }
    }
    swept
}

/// Corpus sweep. The documents this file writes are ordinary text, so every
/// construct in them must carry a range — that is the premise, and it is
/// asserted. The fuzz-regression artifacts are adversarial inputs nobody
/// wrote by hand: a CRLF or a BOM sends one down the documented fallback
/// where no range is published, so their ranges are checked one by one but
/// their *count* is reported rather than pinned.
///
/// A `----------` rule used to be on that list and no longer is: the row is
/// substituted one byte for one before the parser reads it (DEV-234), which
/// moves nothing, so a document carrying one publishes ranges like any other.
/// The two `DOCUMENTS` entries with a rule row in them are what holds that —
/// the acceptance criterion the substitution exists for, and the only reason
/// it is a substitution rather than the region lift `canonicalize` uses.
#[test]
fn corpus_ranges_slice_the_source_and_resolve_the_same_way() {
    let authored = sweep(&authored_documents());
    assert!(
        authored.seen >= 40 && authored.with_range == authored.seen,
        "every construct in a document written by hand must carry a range: {authored:?}"
    );
    let artifacts = sweep(&fuzz_artifacts());
    println!(
        "authored: {}/{} constructs carry a range, by kind: {:?}",
        authored.with_range, authored.seen, authored.per_kind
    );
    println!(
        "artifacts: {}/{} constructs carry a range, by kind: {:?}",
        artifacts.with_range, artifacts.seen, artifacts.per_kind
    );
}

/// Documents that put the notation zoo in the contexts a real one lives in:
/// nested inside markdown structure, packed several to a paragraph, inside
/// containers, and next to the literal contexts (code spans, link
/// destinations) where a notation is deliberately *not* projected.
const DOCUMENTS: &[&str] = &[
    "# ｜青梅《おうめ》の章\n\n親譲《おやゆず》りの無鉄砲で可哀想［＃「可哀想」に傍点］な\
     子供のとき、20［＃「20」は縦中横］年ほど。\n\n［＃改ページ］\n\n次の章。",
    "［＃ここから２字下げ］\n｜山椒《さんしょう》は小粒でも※［＃二の字点、1-2-22］。\n\
     ［＃ここで字下げ終わり］\n\n［＃ここから罫囲み］\n天［＃レ］地。\n\
     ［＃ここで罫囲み終わり］",
    "- ｜一《いち》\n- ｜二《に》で、《《強調》》もある\n- 天［＃レ］地\n\n\
     | 語 | 読み |\n| --- | --- |\n| ｜漢字《かんじ》 | かな |",
    "**｜太字《ふとじ》**と*｜斜体《しゃたい》*、それに `｜コード《こーど》` と\
     [リンク](http://example.com/｜url《ゆーあーるえる》)。\n\n\
     > 引用の中の｜引用《いんよう》と［＃「傍点」に傍点］。",
    "第一篇［＃「第一篇」は大見出し］\n\n本文の｜冒頭《ぼうとう》。\n\n\
     ［＃挿絵（fig1.png）入る］\n\n［＃改丁］\n\n［＃地付き］\n\n終わり。",
    "｜A《a》｜B《b》｜C《c》［＃「D」に傍点］｜E《e》｜F《f》｜G《g》",
    "```\n｜コードブロック《ぶろっく》は素通し\n```\n\n外の｜ルビ《るび》は生きる。",
    "前［＃ほげふが］後、それに［＃割り注］上｜下［＃割り注終わり］。",
    // A rule row on both sides of the width the sibling parser used to read
    // as decoration, with notation before and after it on the same document.
    // The row is hidden from that parser and revealed for the tiling, and the
    // substitution is one byte for one *so that these ranges keep addressing
    // the caller's own text* — the claim has to be made against a document
    // that has a row in it, and until DEV-234 none did.
    "凡例［＃「凡例」に傍点］です。\n----------------------------------\n\
     ｜山椒《さんしょう》は小粒でも。\n\n［＃改ページ］\n\n終わり。",
    "見出し\n===\n｜漢字《かんじ》と［＃「傍点」に傍点］、それに\n\
     ［＃ここから２字下げ］\n｜引用《いんよう》\n［＃ここで字下げ終わり］",
];

/// The documents this file writes: the notation zoo and the realistic
/// documents above.
fn authored_documents() -> Vec<String> {
    ZOO.iter()
        .map(|(src, _)| (*src).to_owned())
        .chain(DOCUMENTS.iter().map(|src| (*src).to_owned()))
        .collect()
}

/// Every permanent fuzz-regression artifact that decodes as UTF-8.
fn fuzz_artifacts() -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz_regressions");
    let mut out: Vec<String> = Vec::new();
    let Ok(targets) = fs::read_dir(&root) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for target in targets.flatten() {
        let Ok(entries) = fs::read_dir(target.path()) else {
            continue;
        };
        paths.extend(
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file() && path.extension().is_none_or(|ext| ext != "txt")),
        );
    }
    paths.sort();
    out.extend(
        paths
            .iter()
            .filter_map(|path| fs::read_to_string(path).ok()),
    );
    out
}
