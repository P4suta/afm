//! Walk the lexer output and report how often each 青空文庫 construct
//! kind appears, plus the number of lexer diagnostics for the input.
//!
//! Run:
//!
//!     cargo run --example ast-walk -p aozora-flavored-markdown -- input.md

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use aozora::pipeline::lex_into_arena;
use aozora::syntax::borrowed::{AozoraNode, Arena, NodeRef};
use aozora_flavored_markdown::sentinels;

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

    let arena = Arena::new();
    let lex_out = lex_into_arena(&input, &arena);

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (idx, ch) in lex_out.normalized.char_indices() {
        let is_sentinel = matches!(
            ch,
            sentinels::INLINE
                | sentinels::BLOCK_LEAF
                | sentinels::BLOCK_OPEN
                | sentinels::BLOCK_CLOSE
        );
        if !is_sentinel {
            continue;
        }
        let pos = u32::try_from(idx).expect("normalized text fits u32");
        let Some(node_ref) = lex_out.registry.node_at(aozora::NormalizedOffset(pos)) else {
            continue;
        };
        let kind = match node_ref {
            NodeRef::BlockOpen(_) => "Container(open)",
            NodeRef::BlockClose(_) => "Container(close)",
            NodeRef::BlockLeaf(node) | NodeRef::Inline(node) => match node {
                AozoraNode::Ruby(_) => "Ruby",
                AozoraNode::Bouten(_) => "Bouten",
                AozoraNode::TateChuYoko(_) => "TateChuYoko",
                AozoraNode::Gaiji(_) => "Gaiji",
                AozoraNode::Annotation(_) => "Annotation",
                AozoraNode::Kaeriten(_) => "Kaeriten",
                AozoraNode::DoubleRuby(_) => "DoubleRuby",
                AozoraNode::Sashie(_) => "Sashie",
                AozoraNode::AozoraHeading(_) => "AozoraHeading",
                AozoraNode::HeadingHint(_) => "HeadingHint",
                AozoraNode::Indent(_) => "Indent",
                AozoraNode::AlignEnd(_) => "AlignEnd",
                AozoraNode::PageBreak => "PageBreak",
                AozoraNode::SectionBreak(_) => "SectionBreak",
                _ => "Other",
            },
            _ => "Other(noderef)",
        };
        *counts.entry(kind).or_insert(0) += 1;
    }

    let width = counts
        .values()
        .copied()
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(1);
    for (kind, n) in &counts {
        println!("{n:>width$}  {kind}");
    }
    let diag_count = lex_out.diagnostics.len();
    println!("{diag_count:>width$}  lexer diagnostics");
    ExitCode::SUCCESS
}
