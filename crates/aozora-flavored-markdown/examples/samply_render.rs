//! Parser-bound `render` hot loop for `samply record`.
//!
//! Gives the sampling profiler a steady stream of render frames so a
//! flamegraph attributes time across comrak parse/format vs the aozora
//! lex + splice — the CPU-attribution metric that pairs with criterion
//! (throughput), `latency_hist` (p50/p99), and `dhat_render` (memory).
//! Driven from the host-only `just samply-render` recipe (samply needs
//! `perf_event_open`).
//!
//! Arg 1 = render repetitions (default 200); more repetitions give
//! samply more parser-bound wall time after the one-time build.
//!
//! ```text
//! just samply-render          # 200 renders of a 128 KiB representative doc
//! just samply-render 500
//! ```

use std::env;
use std::hint::black_box;

use aozora_flavored_markdown::{Options, render};

/// A representative aozora-flavored-markdown block (CommonMark + GFM + 青空文庫記法) repeated
/// to ~128 KiB, matching the bench's `representative` shape.
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

fn main() {
    let repeat: usize = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let opts = Options::default();
    let doc = representative(128 * 1024);

    for _ in 0..repeat {
        black_box(render(black_box(&doc), black_box(&opts)));
    }
}
