//! Heap-allocation profile of one large `render`, via dhat.
//!
//! Reports total allocations + bytes and peak (`At t-gmax`) resident
//! bytes — the third metric of the optimization program (the numbers
//! that PR-A's `Vec` reduction and PR-C's heap→arena change move). Also
//! writes `dhat-heap.json` (viewable at <https://nnethercote.github.io/dh_view/dh_view.html>).
//!
//! ```text
//! cargo run --locked --release --example dhat_render -p aozora-flavored-markdown
//! ```

use std::hint::black_box;

use aozora_flavored_markdown::{Options, render};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// ~2 MiB representative aozora-flavored-markdown document (markdown + 青空文庫記法), matching
/// the bench's `representative` shape so the profile and the throughput
/// numbers describe the same workload.
fn large_doc() -> String {
    const BLOCK: &str = "\
## 見出し\n\n\
本文に｜青空《あおぞら》のルビと［＃「強調」に傍点］を混ぜた段落。通常の \
CommonMark の **強調** や `inline code` も含む。\n\n\
- リスト項目その一\n\
- リスト項目その二\n\n\
```\nfenced code block\n```\n\n\
| 列A | 列B |\n| --- | --- |\n| 1 | 2 |\n\n\
[リンク](https://example.com) と続く本文。\n\n";
    let target = 2 * 1024 * 1024;
    let mut s = String::with_capacity(target + BLOCK.len());
    while s.len() < target {
        s.push_str(BLOCK);
    }
    s
}

fn main() {
    let profiler = dhat::Profiler::new_heap();
    let opts = Options::default();
    let doc = large_doc();

    let rendered = render(black_box(&doc), black_box(&opts));
    black_box(rendered);

    // Explicit drop so the heap snapshot (printed summary + dhat-heap.json)
    // is taken here rather than at an arbitrary end-of-scope point.
    drop(profiler);
}
