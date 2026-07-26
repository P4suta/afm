//! aozora-flavored-markdown glue-cost throughput sentinel.
//!
//! Measures the cost that is *aozora-flavored-markdown's own* — comrak-wrap plus the
//! AST-level Aozora sentinel splice (`ast_splice::splice_into_ast`) —
//! rather than the hot 青空文庫 parse, which lives in the sibling `aozora`
//! workspace and is benched there.
//!
//! Both arms call the public [`aozora_flavored_markdown::render`] on the
//! *same* aozora-flavored-markdown source. They differ only in `Options`:
//!
//! * `aozora-flavored-markdown` arm — [`Options::default`]: the lexer pre-pass runs, the
//!   PUA sentinels are spliced into the comrak AST, and the brand
//!   rewrite (`aozora-` → `aozora-md-`) fires. This is the full aozora-flavored-markdown path.
//! * `comrak_only` arm — [`Options::gfm`]: the notation pass is off, so the
//!   input flows straight through vanilla comrak with no splice.
//!
//! The delta between the two arms is the aozora-md-specific overhead in
//! isolation: a regression in the splice walk or the comrak-format
//! finalisation shows up as a widening gap without the aozora parse
//! cost (shared by both arms via the lexer pre-pass… which the
//! `comrak_only` arm skips entirely, so the `aozora-flavored-markdown` arm additionally
//! carries the lexer — see the module note). The `comrak_only` arm is
//! the floor; the `aozora-flavored-markdown` arm is floor + lexer + splice.
//!
//! Input bands mirror the corpus distribution: `prose` is sentinel-free
//! Markdown (the splice walk finds nothing and must stay cheap),
//! `mixed` interleaves ruby / bouten / annotation triggers at the
//! corpus-median density, and `dense` is annotation-heavy so the
//! splice walk does maximal work.

use std::hint::black_box;

use aozora_flavored_markdown::{Options, render};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// Sentinel-free GFM prose — the splice walk traverses the AST and
/// finds nothing to rewrite. Pins the "aozora-flavored-markdown overhead on plain Markdown"
/// floor.
fn build_prose(target: usize) -> String {
    let unit = "The quick brown fox **jumps** over the _lazy_ dog.\n\n";
    let cycles = target.div_ceil(unit.len());
    unit.repeat(cycles)
}

/// Corpus-median density: roughly one Aozora construct per paragraph,
/// interleaved with plain GFM prose.
fn build_mixed(target: usize) -> String {
    let unit = "｜青梅《おうめ》の段落。**強調**もあり、［＃改ページ］を挟む。\n\n";
    let cycles = target.div_ceil(unit.len());
    unit.repeat(cycles)
}

/// Annotation-heavy pathological band: every line carries a ruby span,
/// so the splice walk rewrites a sentinel on (almost) every inline.
fn build_dense(target: usize) -> String {
    let unit = "青梅《おうめ》梅田《うめだ》大阪《おおさか》\n";
    let cycles = target.div_ceil(unit.len());
    unit.repeat(cycles)
}

fn bench_splice(c: &mut Criterion) {
    // ~64 KiB per band — large enough to dominate per-call fixed costs
    // (arena setup, comrak document scaffolding) without making the
    // bench wall-clock noticeable in the `just bench` smoke run.
    const SIZE: usize = 64 * 1024;
    let prose = build_prose(SIZE);
    let mixed = build_mixed(SIZE);
    let dense = build_dense(SIZE);

    let opts = Options::default();
    let comrak_only = Options::gfm();

    for (label, sample) in [("prose", &prose), ("mixed", &mixed), ("dense", &dense)] {
        let mut g = c.benchmark_group(label);
        g.throughput(Throughput::Bytes(sample.len() as u64));

        // Full aozora-flavored-markdown path: lexer pre-pass + comrak + AST splice + brand
        // rewrite. This is the number that regresses when aozora-flavored-markdown's own
        // glue gets slower.
        g.bench_function("aozora-flavored-markdown", |b| {
            b.iter(|| {
                let out = render(black_box(sample.as_str()), &opts);
                black_box(out);
            });
        });

        // Floor: vanilla comrak, no aozora pass. Subtract this arm from
        // the `aozora-flavored-markdown` arm to read the aozora-md-specific overhead.
        g.bench_function("comrak_only", |b| {
            b.iter(|| {
                let out = render(black_box(sample.as_str()), &comrak_only);
                black_box(out);
            });
        });

        g.finish();
    }
}

criterion_group!(splice_benches, bench_splice);
criterion_main!(splice_benches);
