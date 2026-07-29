#![allow(
    clippy::print_stdout,
    reason = "this command-line benchmark example prints its measured latency report"
)]

//! Small-document `render` latency percentiles (p50/p90/p99).
//!
//! criterion's `render_small_doc` group reports the *mean*'s confidence
//! interval; this example reports the *latency distribution* of
//! individual renders — the p50/p99 the optimization program tracks for
//! interactive (small-document) use. Times each render with a monotonic
//! clock, sorts, and prints integer-microsecond percentiles (no float
//! casts, so the file stays warning-free).
//!
//! ```text
//! cargo run --locked --release --example latency_hist -p aozora-flavored-markdown
//! ```

use std::hint::black_box;
use std::time::Instant;

use aozora_flavored_markdown::{Options, render};

const WARMUP: usize = 200;
const ITERS: usize = 4000;

/// ~4 KiB representative small document (markdown + 青空文庫記法).
fn small_doc() -> String {
    const BLOCK: &str = "\
## 見出し\n\n\
本文に｜青空《あおぞら》のルビと［＃「強調」に傍点］を混ぜた段落。通常の \
CommonMark の **強調** や `inline code` も含む。\n\n\
- リスト項目その一\n\
- リスト項目その二\n\n\
[リンク](https://example.com) と続く本文。\n\n";
    let target = 4 * 1024;
    let mut s = String::with_capacity(target + BLOCK.len());
    while s.len() < target {
        s.push_str(BLOCK);
    }
    s
}

fn main() {
    let opts = Options::default();
    let doc = small_doc();

    for _ in 0..WARMUP {
        black_box(render(black_box(&doc), &opts));
    }

    let mut samples_ns: Vec<u128> = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = Instant::now();
        let rendered = render(black_box(&doc), &opts);
        let elapsed = start.elapsed().as_nanos();
        black_box(rendered);
        samples_ns.push(elapsed);
    }
    samples_ns.sort_unstable();

    let pct = |p: usize| -> u128 {
        let idx = (samples_ns.len() * p / 100).min(samples_ns.len().saturating_sub(1));
        samples_ns[idx]
    };

    println!(
        "render small-doc latency ({} bytes, {ITERS} runs)",
        doc.len()
    );
    println!("  p50 = {} µs", pct(50) / 1_000);
    println!("  p90 = {} µs", pct(90) / 1_000);
    println!("  p99 = {} µs", pct(99) / 1_000);
    println!("  max = {} µs", samples_ns[samples_ns.len() - 1] / 1_000);
}
