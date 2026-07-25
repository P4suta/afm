//! AST-level Aozora sentinel splicer.
//!
//! comrak hands us a typed AST whose paragraphs / code blocks / headings are
//! separate `NodeValue` variants. We splice the sentinels directly into that
//! AST and let `comrak::format_html` produce the final HTML in one pass, rather
//! than re-scanning a flat HTML byte stream.
//!
//! [`crate::ir`]'s `IrWalker` walks the same comrak AST to project an
//! `IrDocument`; both consume the same [`SentinelCursor`] and
//! [`paragraph_sole_block_sentinel`] / [`ParaScan`] primitives, differing only
//! in their emit target.
//!
//! ## Pipeline shape
//!
//! This module sits between `comrak::parse_document` (which leaves PUA
//! sentinels inside `Text` nodes verbatim — they are not in CommonMark's HTML
//! escape set) and `comrak::format_html`. It mutates the AST in place: each
//! sentinel character is replaced by a `NodeValue::Raw` node carrying the
//! rendered Aozora HTML.
//!
//! `Raw` is the right node kind: `comrak/src/nodes.rs` documents it
//! as "inserted verbatim into CommonMark and HTML output", and the
//! `format_html` writer emits it unconditionally — `HtmlBlock` and
//! `HtmlInline` would be filtered out by `Options::render::unsafe`.
//!
//! ## Cases
//!
//! 1. **Sole-block-sentinel paragraph** (a `<p>U+E002</p>`-shaped
//!    paragraph): [`paragraph_sole_block_sentinel`] returns
//!    `Some(kind)`. Insert a `Raw` node before the paragraph carrying
//!    the rendered output, then detach the paragraph. Paired open/close
//!    use the container stack to keep the LIFO invariant.
//! 2. **Heading-hint promotion** (`［＃「X」は大見出し］`): the first
//!    inline sentinel in the paragraph is a heading hint. Mutate the
//!    paragraph's `NodeValue` to `Heading { level, setext: false }`
//!    in place, replace its children with a single `Text(target)`,
//!    and advance the cursor past every sentinel the paragraph would
//!    otherwise have consumed.
//! 3. **Inline sentinels inside a `Text` node**: split the text
//!    around each sentinel char and weave `Raw` siblings carrying the
//!    rendered output. Block sentinels surviving into an inline
//!    context (e.g. raw text inside a fenced code block we somehow
//!    didn't mask) drop silently.
//! 4. **Orphan `［＃...］`**: a bracket run the lexer never claimed.
//!    Split the `Text` and replace the bracket span with a `Raw` node
//!    containing `<span class="aozora-md-annotation" hidden>...</span>`.
//!    No registry advance — by construction the orphan has no
//!    matching entry.

use core::fmt;
use core::mem;
use std::borrow::Cow;

use aozora::pipeline::{BorrowedLexOutput, INLINE_SENTINEL};
use aozora::render::render_node;
use aozora::syntax::Container;
use aozora::syntax::ContainerKind;
use aozora::syntax::borrowed::{AozoraNode, HeadingHint, NodeRef};
use comrak::Arena;
use comrak::nodes::{AstNode, NodeHeading, NodeValue};

use crate::sentinel_stream::{
    BlockSentinelKind, ParaScan, SentinelCursor, is_sentinel_char, paragraph_sole_block_sentinel,
};

/// Splice every Aozora sentinel embedded in `root`'s text descendants
/// into the comrak AST. After this returns, the AST contains no PUA
/// sentinel character: `comrak::format_html` will emit fully resolved
/// HTML in a single verbatim pass.
/// `sanitized` is the lexer's Phase-0 sanitized source. The splicer slices
/// it via the parallel span table to recover a sentinel's original Aozora
/// source for literal markdown contexts (inline code spans, link
/// destinations), where the notation must render verbatim rather than as
/// interpreted Aozora HTML. It must be the *sanitized* source, not the raw
/// input, because the span coordinates are in sanitized-source bytes.
pub(crate) fn splice_into_ast<'a, 'src>(
    root: &'a AstNode<'a>,
    arena: &'a Arena<'a>,
    lex_out: &BorrowedLexOutput<'src>,
    sanitized: &str,
) {
    let mut splicer = AstSplicer::<'a, 'src> {
        cursor: SentinelCursor::from_lex_out_with_source(Some(lex_out), sanitized),
        container_stack: Vec::new(),
        in_heading_depth: 0,
        arena,
    };
    splicer.walk(root);
    splicer.drain_unclosed_containers(root);
}

/// AST mutator that consumes the borrowed-AST registry in source
/// order and weaves rendered Aozora HTML into the comrak tree.
struct AstSplicer<'a, 'src> {
    cursor: SentinelCursor<'src>,
    /// `ContainerKind` of every still-open paired container, in LIFO
    /// order. Push on `BlockOpen`, pop on `BlockClose`. Tracking the
    /// kind (rather than just a depth counter) lets us synthesise a
    /// matching close node when the source ends without one.
    container_stack: Vec<ContainerKind>,
    /// Number of `Heading` ancestors the walker is currently inside.
    /// Heading bodies must satisfy Tier C (no `aozora-md-annotation`
    /// contamination) AND Tier A (no bare `［＃` leak), so when
    /// orphan brackets surface in heading text we silently drop the
    /// run rather than wrap it. The legitimate Aozora-into-heading
    /// path is the heading-hint promotion (Case 2 in
    /// [`Self::dispatch_paragraph`]).
    in_heading_depth: u32,
    arena: &'a Arena<'a>,
}

impl<'a, 'src> AstSplicer<'a, 'src> {
    /// Depth-first traversal over an explicit work stack rather than
    /// recursion.
    ///
    /// comrak can build an arbitrarily deep AST from a small input
    /// (deeply nested blockquotes `> > > …`, nested list items, or
    /// nested inline emphasis), and `handle_blockquote` carries no
    /// nesting cap. A recursive descent would exhaust the call stack —
    /// under the release profile's `panic = "abort"` that is a hard
    /// process abort, i.e. a crash on untrusted input, which both repos'
    /// `SECURITY.md` scope IN as a vulnerability. The explicit stack
    /// moves the unbounded growth to the heap, where it is bounded by
    /// the input size rather than the OS stack. comrak's own
    /// `format_html` / AST post-processing are iterative for the same
    /// reason; this brings the splice walk in line with them.
    ///
    /// Children are pushed in reverse so the `Vec`-as-stack pops them in
    /// document order, and a `Heading`'s subtree is bracketed by a
    /// [`Work::ExitHeading`] marker so `in_heading_depth` is incremented
    /// for exactly the heading's descendants — preserving, node for
    /// node, the recursive `in_heading_depth += 1; walk; -= 1` behaviour
    /// the Tier-A / Tier-C splice contract depends on. Each leaf
    /// dispatch (`split_text_node` / `handle_block_sentinel` /
    /// `handle_heading_hint`) only ever inserts fresh siblings or
    /// detaches the current node, never the already-stacked siblings, so
    /// the snapshot-on-push discipline stays sound.
    fn walk(&mut self, root: &'a AstNode<'a>) {
        let mut stack: Vec<Work<'a>> = Vec::new();
        push_children_rev(&mut stack, root);
        while let Some(work) = stack.pop() {
            let node = match work {
                Work::ExitHeading => {
                    self.in_heading_depth -= 1;
                    continue;
                }
                Work::ProcessLinkFields(node) => {
                    self.process_link_fields(node);
                    continue;
                }
                Work::Visit(node) => node,
            };
            let (action, is_heading) = {
                let data = node.data.borrow();
                (
                    classify(&data.value),
                    matches!(&data.value, NodeValue::Heading(_)),
                )
            };
            match action {
                DispatchAction::Skip => {}
                DispatchAction::TextWith(text) => self.split_text_node(node, &text),
                DispatchAction::CodeWith(literal) => self.splice_code_literal(node, &literal),
                DispatchAction::Paragraph => self.dispatch_paragraph(node, &mut stack),
                DispatchAction::RecurseLink => {
                    // Children first (link text, in source order), then the
                    // url/title fields: push the field-processing marker
                    // *before* the children so it pops *after* them.
                    stack.push(Work::ProcessLinkFields(node));
                    push_children_rev(&mut stack, node);
                }
                DispatchAction::Recurse => {
                    if is_heading {
                        self.in_heading_depth += 1;
                        stack.push(Work::ExitHeading);
                    }
                    push_children_rev(&mut stack, node);
                }
            }
        }
    }

    /// Dispatch a paragraph (Cases 1/2/3 in module-doc order). The
    /// ordinary-paragraph case descends by pushing the paragraph's
    /// children onto the shared work `stack` (a paragraph is never a
    /// `Heading`, so no depth marker is needed); the block-sentinel and
    /// heading-hint cases mutate in place and do not descend.
    fn dispatch_paragraph(&mut self, paragraph: &'a AstNode<'a>, stack: &mut Vec<Work<'a>>) {
        if let Some(kind) = paragraph_sole_block_sentinel(paragraph) {
            self.handle_block_sentinel(paragraph, kind);
            return;
        }
        let scan = ParaScan::run(paragraph, &self.cursor);
        if let Some(hint) = scan.first_heading_hint {
            self.handle_heading_hint(paragraph, hint, scan.total_sentinels);
            return;
        }
        // Case 3: ordinary paragraph — descend to children for inline
        // sentinel splitting.
        push_children_rev(stack, paragraph);
    }

    fn handle_block_sentinel(&mut self, paragraph: &'a AstNode<'a>, kind: BlockSentinelKind) {
        let Some(node_ref) = self.cursor.next() else {
            // Registry exhausted: drop the paragraph silently rather
            // than leak the PUA sentinel into the rendered HTML.
            paragraph.detach();
            return;
        };
        match (kind, node_ref) {
            (BlockSentinelKind::Leaf, NodeRef::BlockLeaf(node)) => {
                self.replace_with_block_html(paragraph, render_aozora_html(node, true));
            }
            (BlockSentinelKind::Open, NodeRef::BlockOpen(ck)) => {
                self.container_stack.push(ck);
                self.replace_with_block_html(
                    paragraph,
                    render_aozora_html(AozoraNode::Container(Container { kind: ck }), true),
                );
            }
            (BlockSentinelKind::Close, NodeRef::BlockClose(ck))
                if self.container_stack.pop().is_some() =>
            {
                self.replace_with_block_html(
                    paragraph,
                    render_aozora_html(AozoraNode::Container(Container { kind: ck }), false),
                );
            }
            _ => {
                // Mismatch (registry/AST drift) or orphan close (no
                // matching open): silently drop the paragraph rather than
                // emit an unbalanced close tag (Tier-D protection).
                paragraph.detach();
            }
        }
    }

    fn handle_heading_hint(
        &mut self,
        paragraph: &'a AstNode<'a>,
        hint: &'src HeadingHint<'src>,
        sentinels_to_consume: usize,
    ) {
        self.cursor.advance(sentinels_to_consume);
        let level = hint.level.clamp(1, 6);
        // The heading body is the hint's `target`, escaped against the
        // five-char surface (`< > & " '`). We emit a `Raw` node rather than
        // `Text` because comrak's text escape skips `'`, which we want
        // escaped. `Raw` stays inert through `format_html`, so the
        // `<h{level}>...</h{level}>` framing is generated by comrak around
        // our pre-escaped body.
        let mut escaped = String::with_capacity(hint.target.as_str().len());
        push_html_escaped(&mut escaped, hint.target.as_str());
        let children: Vec<&'a AstNode<'a>> = paragraph.children().collect();
        for child in children {
            child.detach();
        }
        paragraph.data.borrow_mut().value = NodeValue::Heading(NodeHeading {
            level,
            setext: false,
            closed: true,
        });
        paragraph.append(self.new_raw_node(escaped));
    }

    fn split_text_node(&mut self, node: &'a AstNode<'a>, text: &str) {
        let mut segments: Vec<&'a AstNode<'a>> = Vec::new();
        let mut current = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if is_sentinel_char(ch) {
                self.flush_text(&mut current, &mut segments);
                let Some(node_ref) = self.cursor.next() else {
                    continue;
                };
                if ch == INLINE_SENTINEL
                    && let NodeRef::Inline(aozora) = node_ref
                {
                    // Heading body must not carry `aozora-md-annotation`
                    // wrappers (Tier C). Annotation-shaped Aozora
                    // nodes (Unknown / AsIs / TextualNote /
                    // InvalidRubySpan / WarichuOpen / WarichuClose)
                    // all render to a `<span class="aozora-md-annotation"
                    // hidden>...</span>` wrapper, so we drop them
                    // when in_heading_depth > 0. Other inline Aozora
                    // (Ruby / Bouten / Tcy / Gaiji / Kaeriten /
                    // DoubleRuby) are explicitly allowed inside a
                    // heading per Tier C's documented contract.
                    let in_heading = self.in_heading_depth > 0;
                    let is_annotation = matches!(aozora, AozoraNode::Annotation(_));
                    if !(in_heading && is_annotation) {
                        let html = render_aozora_html(aozora, true);
                        segments.push(self.new_raw_node(html));
                    }
                }
                // Block sentinel surviving into inline context, or
                // inline-position registry mismatch: drop silently.
            } else if ch == '［' && chars.peek() == Some(&'＃') {
                // Orphan `［＃...］` run the lexer never claimed.
                chars.next(); // consume ＃
                if self.in_heading_depth > 0 {
                    // Heading bodies must satisfy both Tier A (no bare
                    // `［＃` leak) and Tier C (no `aozora-md-annotation`
                    // contamination). The wrapper would resolve Tier A
                    // but break Tier C, and emitting the literal run
                    // would break Tier A. Silently consume the orphan
                    // run instead — the canonical way to inject an
                    // Aozora annotation into a heading is the
                    // heading-hint promotion path (Case 2), not a raw
                    // bracket run that survives lexer parsing.
                    for b in chars.by_ref() {
                        if b == '］' {
                            break;
                        }
                    }
                    continue;
                }
                let mut bracket_body = String::from("［＃");
                for b in chars.by_ref() {
                    bracket_body.push(b);
                    if b == '］' {
                        break;
                    }
                }
                self.flush_text(&mut current, &mut segments);
                let mut html = String::with_capacity(bracket_body.len() + 64);
                html.push_str("<span class=\"aozora-md-annotation\" hidden>");
                push_html_escaped(&mut html, &bracket_body);
                html.push_str("</span>");
                segments.push(self.new_raw_node(html));
            } else {
                current.push(ch);
            }
        }
        self.flush_text(&mut current, &mut segments);
        if segments.is_empty() {
            // No splits happened (e.g. orphan-bracket prefix matched
            // by classify but the run actually consisted only of "［"
            // followed by non-`＃`). Leave the original node alone.
            return;
        }
        // Insert all segments after the original Text node, then
        // detach the original.
        let mut anchor: &'a AstNode<'a> = node;
        for seg in segments {
            anchor.insert_after(seg);
            anchor = seg;
        }
        node.detach();
    }

    /// Rewrite each sentinel in `s` to the original Aozora source the
    /// lexer collapsed into it, leaving non-sentinel chars untouched.
    /// Advances the cursor once per sentinel so later sentinels in
    /// ordinary text stay in lockstep. A sentinel with no registry entry
    /// (cursor exhausted) is dropped rather than leaked.
    fn rewrite_literal_context(&mut self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for ch in s.chars() {
            if is_sentinel_char(ch) {
                if let Some(literal) = self.cursor.next_literal() {
                    out.push_str(literal);
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// Inline code span (`` `…` ``): rewrite sentinels in the code literal
    /// back to their original source. Inline code is literal markdown, so
    /// `` `｜青梅《おうめ》` `` must render as the literal text, not as an
    /// interpreted ruby — and the sentinel must never leak into `<code>`.
    fn splice_code_literal(&mut self, node: &'a AstNode<'a>, literal: &str) {
        let rewritten = self.rewrite_literal_context(literal);
        let mut data = node.data.borrow_mut();
        if let NodeValue::Code(code) = &mut data.value {
            code.literal = rewritten;
        }
    }

    /// Link/image destination + title: rewrite sentinels in `url` then
    /// `title` (source order) back to their original source, so a notation
    /// written inside a URL keeps the literal URL the author typed instead
    /// of a percent-encoded sentinel. Called after the node's children, so
    /// cursor consumption follows source order (text, then url, then title).
    fn process_link_fields(&mut self, node: &'a AstNode<'a>) {
        let (url, title) = {
            let data = node.data.borrow();
            match &data.value {
                NodeValue::Link(link) | NodeValue::Image(link) => {
                    let has = link.url.chars().any(is_sentinel_char)
                        || link.title.chars().any(is_sentinel_char);
                    if !has {
                        return;
                    }
                    (link.url.clone(), link.title.clone())
                }
                _ => return,
            }
        };
        let new_url = self.rewrite_literal_context(&url);
        let new_title = self.rewrite_literal_context(&title);
        let mut data = node.data.borrow_mut();
        if let NodeValue::Link(link) | NodeValue::Image(link) = &mut data.value {
            link.url = new_url;
            link.title = new_title;
        }
    }

    fn flush_text(&self, current: &mut String, segments: &mut Vec<&'a AstNode<'a>>) {
        if !current.is_empty() {
            segments.push(self.new_text_node(mem::take(current)));
        }
    }

    fn replace_with_block_html(&self, paragraph: &'a AstNode<'a>, html: String) {
        let raw = self.new_raw_node(html);
        paragraph.insert_before(raw);
        paragraph.detach();
    }

    fn drain_unclosed_containers(&mut self, root: &'a AstNode<'a>) {
        while let Some(ck) = self.container_stack.pop() {
            let html = render_aozora_html(AozoraNode::Container(Container { kind: ck }), false);
            root.append(self.new_raw_node(html));
        }
    }

    fn new_text_node(&self, text: String) -> &'a AstNode<'a> {
        self.arena
            .alloc(AstNode::from(NodeValue::Text(Cow::Owned(text))))
    }

    fn new_raw_node(&self, html: String) -> &'a AstNode<'a> {
        self.arena.alloc(AstNode::from(NodeValue::Raw(html)))
    }
}

/// One entry on [`AstSplicer::walk`]'s explicit traversal stack.
enum Work<'a> {
    /// Classify and dispatch this node.
    Visit(&'a AstNode<'a>),
    /// Sentinel popped after a `Heading`'s entire subtree has been
    /// processed, to restore `in_heading_depth` — the iterative
    /// analogue of the recursive `in_heading_depth -= 1` on unwind.
    ExitHeading,
    /// Rewrite a link/image node's `url`/`title` fields after its children
    /// (the link text) have been processed, so sentinels in the fields
    /// consume their registry entries in source order.
    ProcessLinkFields(&'a AstNode<'a>),
}

/// Push `parent`'s children onto `stack` as [`Work::Visit`] items in
/// reverse document order, so the `Vec`-as-stack pops them
/// left-to-right. Children are snapshotted here (by being moved onto
/// the stack) before any dispatch mutates the tree, mirroring the
/// previous recursive walk's `children().collect()`.
fn push_children_rev<'a>(stack: &mut Vec<Work<'a>>, parent: &'a AstNode<'a>) {
    let start = stack.len();
    stack.extend(parent.children().map(Work::Visit));
    stack[start..].reverse();
}

/// Per-node dispatch verdict. Snapshotted from a borrowed `NodeValue`
/// so the borrow is released before the splicer mutates the tree.
#[derive(Debug)]
enum DispatchAction {
    /// Paragraph node — try Case 1 / 2 / 3 in order.
    Paragraph,
    /// Text node carrying at least one sentinel char or one orphan
    /// `［＃` prefix. The captured `String` is the text body, ready
    /// to feed into [`AstSplicer::split_text_node`] without re-borrow.
    TextWith(String),
    /// Inline code span whose literal carries at least one sentinel. The
    /// captured `String` is the code literal, fed to
    /// [`AstSplicer::splice_code_literal`] which rewrites each sentinel to
    /// its original source.
    CodeWith(String),
    /// Link or image: recurse into children, then rewrite `url`/`title`
    /// (see [`Work::ProcessLinkFields`]).
    RecurseLink,
    /// Block container that may carry interesting descendants —
    /// recurse into its children.
    Recurse,
    /// Leaf or opaque content (raw HTML, fenced code, already-rendered
    /// Raw) that must not be searched for sentinels.
    Skip,
}

fn classify(value: &NodeValue) -> DispatchAction {
    match value {
        NodeValue::Paragraph => DispatchAction::Paragraph,
        NodeValue::Text(s) => {
            if s.chars().any(is_sentinel_char) || s.contains("［＃") {
                DispatchAction::TextWith(s.clone().into_owned())
            } else {
                DispatchAction::Skip
            }
        }
        // Inline code spans are literal markdown: a sentinel here means an
        // Aozora notation that the user wrote *inside* backticks. It must
        // render as its original source, not interpreted HTML — and it
        // must still consume its registry entry so later sentinels stay in
        // lockstep. Code without a sentinel is left untouched.
        NodeValue::Code(c) => {
            if c.literal.chars().any(is_sentinel_char) {
                DispatchAction::CodeWith(c.literal.clone())
            } else {
                DispatchAction::Skip
            }
        }
        // Links/images carry sentinels in their `url`/`title` *fields*
        // (not child text). Recurse into the children first, then rewrite
        // the fields, so cursor consumption matches source order.
        NodeValue::Link(_) | NodeValue::Image(_) => DispatchAction::RecurseLink,
        NodeValue::CodeBlock(_)
        | NodeValue::HtmlBlock(_)
        | NodeValue::HtmlInline(_)
        | NodeValue::Raw(_) => DispatchAction::Skip,
        _ => DispatchAction::Recurse,
    }
}

fn render_aozora_html(node: AozoraNode<'_>, entering: bool) -> String {
    let mut out = String::new();
    render_node::render(node, entering, &mut StringSink(&mut out))
        .expect("writing AozoraNode HTML to a String cannot fail");
    // Brand boundary (ADR-0011): the sibling renderer emits classes under
    // its own `aozora-*` brand; our HTML uses `aozora-md-*`. The rewrite
    // is local to the rendered fragment we are about to wrap in a
    // `Raw` node, so a single `replace` is enough — the fragment is
    // a self-contained tag/attribute soup with no body text that
    // could legitimately contain the `aozora-` literal.
    if out.contains("aozora-") {
        out = out.replace("aozora-", "aozora-md-");
    }
    out
}

fn push_html_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

/// `fmt::Write` adapter over `&mut String` so the upstream renderer can
/// write straight into the buffer that becomes a `Raw` node payload.
struct StringSink<'s>(&'s mut String);

impl fmt::Write for StringSink<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora::pipeline::{BLOCK_LEAF_SENTINEL, lex_into_arena};
    use aozora::syntax::borrowed::Arena as AozoraArena;

    use crate::code_block_mask;

    /// Run the full aozora-flavored-markdown pipeline (mask → lex → parse → splice →
    /// `format_html` → unmask) through the AST splicer and return
    /// the produced HTML. Mirrors `crate::lib::drive_pipeline`
    /// exactly so the unit tests exercise the same code-block-mask
    /// boundary the production renderer uses.
    fn render_via_ast_splice(input: &str) -> String {
        use aozora::pipeline::lexer::sanitize;
        let (masked, originals) = code_block_mask::mask_code_block_triggers(input);
        let sanitized = sanitize(&masked);
        let aozora_arena = AozoraArena::new();
        let lex_out = lex_into_arena(&masked, &aozora_arena);
        let comrak_arena: Arena<'_> = Arena::new();
        let opts = comrak::Options::default();
        let root = comrak::parse_document(&comrak_arena, lex_out.normalized, &opts);
        splice_into_ast(root, &comrak_arena, &lex_out, &sanitized.text);
        let mut html = String::new();
        comrak::format_html(root, &opts, &mut html).expect("formatting to a String never fails");
        code_block_mask::unmask_html(&html, &originals).into_owned()
    }

    #[test]
    fn plain_text_passes_through() {
        let html = render_via_ast_splice("hello");
        assert!(html.contains("hello"), "html: {html}");
    }

    #[test]
    fn ruby_inline_sentinel_is_replaced() {
        let html = render_via_ast_splice("｜青梅《おうめ》");
        assert!(html.contains("<ruby>"), "html: {html}");
        assert!(html.contains("青梅"), "html: {html}");
        assert!(html.contains("おうめ"), "html: {html}");
        assert!(!html.contains(INLINE_SENTINEL), "sentinel leaked: {html}");
    }

    #[test]
    fn page_break_block_leaf_replaces_paragraph() {
        let html = render_via_ast_splice("前\n\n［＃改ページ］\n\n後");
        assert!(
            !html.contains(BLOCK_LEAF_SENTINEL),
            "sentinel leaked: {html}"
        );
        assert!(
            !html.contains("<p>\u{E002}</p>"),
            "block-sentinel paragraph survived: {html}"
        );
    }

    #[test]
    fn heading_hint_promotes_paragraph_to_heading() {
        let html = render_via_ast_splice("第一篇［＃「第一篇」は大見出し］");
        assert!(
            html.contains("<h1>第一篇</h1>"),
            "expected <h1>第一篇</h1>, got {html}"
        );
    }

    #[test]
    fn orphan_close_does_not_emit_div() {
        let html = render_via_ast_splice("［＃ここで字下げ終わり］");
        let opens = html.matches("<div").count();
        let closes = html.matches("</div>").count();
        assert_eq!(opens, closes, "tag-balance broken: {html}");
    }

    #[test]
    fn block_sentinel_inside_code_block_does_not_promote() {
        // Sentinels surviving into code-block context (the lexer
        // pre-stage in `code_block_mask` should normally prevent
        // this; this test pins the defensive in-AST behaviour) drop
        // silently rather than leaking PUA chars or emitting Aozora
        // HTML in the wrong place.
        let html = render_via_ast_splice("```\n［＃改ページ］\n```");
        assert!(
            !html.contains(BLOCK_LEAF_SENTINEL),
            "sentinel leaked: {html}"
        );
    }

    #[test]
    fn heading_hint_target_html_special_chars_are_escaped() {
        // `push_html_escaped` covers <, >, &, ", ' arms when a
        // HeadingHint target carries them.
        let html = render_via_ast_splice("<&\"'><&\"'>［＃「<&\"'>」は大見出し］");
        assert!(html.contains("&lt;"), "missing < escape: {html}");
        assert!(html.contains("&gt;"), "missing > escape: {html}");
        assert!(html.contains("&amp;"), "missing & escape: {html}");
        assert!(html.contains("&quot;"), "missing \" escape: {html}");
        assert!(html.contains("&#39;"), "missing ' escape: {html}");
    }

    #[test]
    fn atx_heading_with_orphan_bracket_drops_wrapper() {
        let html = render_via_ast_splice("# header［＃orphan］tail");
        assert!(
            !html.contains("aozora-md-annotation"),
            "aozora-md-annotation leaked into heading: {html}"
        );
    }

    #[test]
    fn setext_heading_with_orphan_bracket_drops_wrapper() {
        // Setext-style: paragraph followed by `===` underline becomes
        // `<h1>` whose body inherits the paragraph's inline run. The
        // orphan `［＃` must not surface as an annotation wrapper here
        // either (Tier C contamination), so the heading-depth gate has
        // to fire on setext as well as ATX headings.
        let html = render_via_ast_splice("text［＃orphan］more\n===");
        assert!(
            !html.contains("aozora-md-annotation"),
            "aozora-md-annotation leaked into setext heading: {html}"
        );
    }

    #[test]
    fn dispatch_skip_covers_inline_html_and_code() {
        // Pin the Skip arm of `classify` for HtmlInline / Code /
        // CodeBlock / HtmlBlock / Raw via inputs that surface them.
        // The `<script>alert(1)</script>` raw HTML is rejected by
        // comrak's safe-mode default (becomes `<!-- raw HTML omitted -->`),
        // but the AST traversal still has to step over the
        // `HtmlBlock` node without touching it.
        let _html = render_via_ast_splice("<div>raw</div>\n\n```\ncode\n```\n\n`x`");
    }

    #[test]
    fn orphan_bracket_wrap_respects_text_node_boundary() {
        // Pin the AST-splicer's semantics: an unclosed `［＃` only wraps
        // within its own Text node — a soft break (`\n`) inside the same
        // paragraph splits the wrap because comrak emits `Text("［＃")` +
        // `SoftBreak` + `Text("※")`, so wrap scope is structural, not
        // byte-positional. This still satisfies the Tier-A canary (no bare
        // `［＃` survives outside an `aozora-md-annotation` wrapper).
        let html = render_via_ast_splice("［＃\n※");
        assert!(
            html.contains("<span class=\"aozora-md-annotation\" hidden>［＃</span>"),
            "wrapped run did not honour Text-node boundary: {html}"
        );
        assert!(
            !html.contains("<span class=\"aozora-md-annotation\" hidden>［＃\n※"),
            "wrap leaked across SoftBreak: {html}"
        );
    }
}
