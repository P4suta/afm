//! Walk the IR of a document and report how often each 青空文庫 construct
//! appears, plus the number of diagnostics the parse raised.
//!
//! Each construct is projected with the byte range its notation occupies in
//! the input, so the walk also prints how many of them can be pointed back
//! at the source (all of them, unless the parser rewrote the text before
//! lexing it — a BOM, CRLF line endings, …).
//!
//! Run:
//!
//!     cargo run --example ast-walk -p aozora-flavored-markdown -- input.md

use core::iter;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use aozora_flavored_markdown::ir::{IrBlock, IrInline};
use aozora_flavored_markdown::{Options, render_to_ir};

/// Tally of one construct kind: how many were projected, and how many of
/// those carry a source range.
#[derive(Debug, Default)]
struct Tally {
    total: usize,
    with_range: usize,
}

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: ast-walk <path/to/input.md>");
        return ExitCode::from(2);
    };

    let input = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let rendered = render_to_ir(&input, &Options::default());
    let mut counts: BTreeMap<String, Tally> = BTreeMap::new();
    count_blocks(&rendered.ir.blocks, &mut counts);

    let width = counts
        .values()
        .map(|tally| tally.total)
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(1);
    for (kind, tally) in &counts {
        let (total, with_range) = (tally.total, tally.with_range);
        println!("{total:>width$}  {kind} ({with_range} with a source range)");
    }
    let diag_count = rendered.diagnostics.len();
    println!("{diag_count:>width$}  diagnostics");
    ExitCode::SUCCESS
}

fn count_blocks(blocks: &[IrBlock], counts: &mut BTreeMap<String, Tally>) {
    for block in blocks {
        match block {
            IrBlock::Aozora { kind, span, .. } => tally(kind, span.is_some(), counts),
            IrBlock::Paragraph { children, .. } | IrBlock::Heading { children, .. } => {
                count_inlines(children, counts);
            }
            IrBlock::Blockquote { children, .. } => count_blocks(children, counts),
            IrBlock::List { items, .. } => {
                for item in items {
                    count_blocks(&item.children, counts);
                }
            }
            IrBlock::Table { header, rows, .. } => {
                for row in iter::once(header).chain(rows) {
                    for cell in &row.cells {
                        count_inlines(cell, counts);
                    }
                }
            }
            _ => {}
        }
    }
}

fn count_inlines(inlines: &[IrInline], counts: &mut BTreeMap<String, Tally>) {
    for inline in inlines {
        match inline {
            IrInline::Aozora { kind, span, .. } => tally(kind, span.is_some(), counts),
            IrInline::Strong { children, .. }
            | IrInline::Emphasis { children, .. }
            | IrInline::Link { children, .. }
            | IrInline::Image { alt: children, .. } => count_inlines(children, counts),
            _ => {}
        }
    }
}

fn tally(kind: &str, with_range: bool, counts: &mut BTreeMap<String, Tally>) {
    let entry = counts.entry(kind.to_owned()).or_default();
    entry.total += 1;
    entry.with_range += usize::from(with_range);
}
