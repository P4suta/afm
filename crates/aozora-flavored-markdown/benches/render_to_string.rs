//! Criterion benchmark for the aozora-flavored-markdown render pipeline.
//!
//! Tracks throughput (MB/s) on representative + pathological corpora and the
//! small-document latency distribution, plus a `render_to_ir` group so the
//! IR-projection / splice cross-path is observable.
//!
//! Corpora are generated inline (deterministic, self-contained) so a
//! fresh checkout produces numbers without any external corpus. The
//! optional `crime_and_punishment` group runs the real 罪と罰 text when
//! `AOZORA_CORPUS_ROOT` points at a corpus tree (skipped with a notice
//! otherwise), mirroring `aozora-bench`.
//!
//! ```text
//! just bench                       # all inline groups
//! AOZORA_CORPUS_ROOT=… just bench  # + the 罪と罰 group
//! ```

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;

use aozora::decode_sjis;
use aozora_flavored_markdown::{Options, render, render_to_ir};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// Relative path of 罪と罰 under `AOZORA_CORPUS_ROOT` (same layout as
/// `aozora-bench/benches/crime_and_punishment.rs`).
const CRIME_RELATIVE: &str = "000363/files/56656_ruby_74439/56656_ruby_74439.txt";

/// A representative aozora-flavored-markdown block: CommonMark + GFM (heading, prose, list,
/// fenced code, table, link) woven with 青空文庫記法 (ruby + bouten),
/// repeated to a target size. This is the mixed markdown-and-aozora
/// regime where comrak overhead and the aozora splice coexist.
fn representative(target_bytes: usize) -> String {
    const BLOCK: &str = "\
## 見出し\n\n\
本文に｜青空《あおぞら》のルビと［＃「強調」に傍点］を混ぜた段落。通常の \
CommonMark の **強調** や `inline code` も含む。\n\n\
- リスト項目その一\n\
- リスト項目その二\n\n\
```\nfenced code block\n```\n\n\
| 列A | 列B |\n| --- | --- |\n| 1 | 2 |\n\n\
[リンク](https://example.com) と続く本文。\n\n";
    let mut s = String::with_capacity(target_bytes + BLOCK.len());
    while s.len() < target_bytes {
        s.push_str(BLOCK);
    }
    s
}

/// Sentinel/annotation-dense pathological input: every unit is a ruby +
/// bouten annotation, so the registry walk and the splice path are
/// maximally exercised relative to comrak's near-constant overhead.
fn pathological(target_bytes: usize) -> String {
    const UNIT: &str = "｜親文字《おやもじ》［＃「文字」に傍点］と";
    let mut s = String::with_capacity(target_bytes + UNIT.len());
    while s.len() < target_bytes {
        s.push_str(UNIT);
    }
    s
}

/// The same document as [`representative`], in the shape a 青空文庫 file
/// actually arrives in: CRLF line endings under a decorative rule.
///
/// That pair is not cosmetic. It is what decides whether a construct's
/// source run can be sliced out of the caller's own text or has to be
/// searched for, and the search is the expensive half of the render. Benched
/// separately because the LF corpus above never reaches it, and this is the
/// corpus every real file belongs to.
fn recovered(target_bytes: usize) -> String {
    let mut s = String::from("本文\r\n----------\r\n");
    s.push_str(&representative(target_bytes).replace('\n', "\r\n"));
    s
}

fn bench_render_to_string(c: &mut Criterion) {
    let opts = Options::default();
    let mut g = c.benchmark_group("render_to_string");
    for (label, doc) in [
        ("representative_128k", representative(128 * 1024)),
        ("representative_1m", representative(1024 * 1024)),
        ("pathological_128k", pathological(128 * 1024)),
        ("recovered_128k", recovered(128 * 1024)),
    ] {
        g.throughput(Throughput::Bytes(doc.len() as u64));
        g.bench_function(label, |b| {
            b.iter(|| black_box(render(black_box(&doc), black_box(&opts))));
        });
    }
    g.finish();
}

fn bench_small_doc(c: &mut Criterion) {
    // Small-document latency. criterion's median ≈ p50 and the HTML
    // report shows the tail; `examples/latency_hist.rs` prints explicit
    // p50/p90/p99 for the same shape.
    let opts = Options::default();
    let doc = representative(4 * 1024);
    let mut g = c.benchmark_group("render_small_doc");
    g.throughput(Throughput::Bytes(doc.len() as u64));
    g.bench_function("representative_4k", |b| {
        b.iter(|| black_box(render(black_box(&doc), black_box(&opts))));
    });
    g.finish();
}

fn bench_render_to_ir(c: &mut Criterion) {
    // render_to_ir projects the IR AND splices the AST — this group
    // makes that cross-path (the PR-B double-walk candidate) observable.
    let opts = Options::default();
    let doc = representative(128 * 1024);
    let mut g = c.benchmark_group("render_to_ir");
    g.throughput(Throughput::Bytes(doc.len() as u64));
    g.bench_function("representative_128k", |b| {
        b.iter(|| black_box(render_to_ir(black_box(&doc), black_box(&opts))));
    });
    g.finish();
}

fn bench_crime_and_punishment(c: &mut Criterion) {
    let Some(root) = env::var_os("AOZORA_CORPUS_ROOT") else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping crime_and_punishment group");
        return;
    };
    let path = PathBuf::from(root).join(CRIME_RELATIVE);
    if !path.is_file() {
        eprintln!("罪と罰 not present at {CRIME_RELATIVE}; skipping crime_and_punishment group");
        return;
    }
    let bytes = fs::read(&path).expect("read 罪と罰");
    let text = decode_sjis(&bytes).expect("decode SJIS");
    let opts = Options::default();
    let mut g = c.benchmark_group("crime_and_punishment");
    g.throughput(Throughput::Bytes(text.len() as u64));
    g.bench_function("render_to_string", |b| {
        b.iter(|| black_box(render(black_box(&text), black_box(&opts))));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_render_to_string,
    bench_small_doc,
    bench_render_to_ir,
    bench_crime_and_punishment
);
criterion_main!(benches);
