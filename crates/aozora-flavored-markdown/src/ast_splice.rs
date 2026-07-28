//! AST-level Aozora sentinel splicer.
//!
//! Sits between `comrak::parse_document` — which leaves PUA sentinels inside
//! `Text` nodes verbatim, being outside CommonMark's escape set — and
//! `comrak::format_html`, mutating the AST in place so the final HTML comes
//! out of one formatting pass rather than a re-scan of a flat byte stream.
//!
//! `NodeValue::Raw` is the node kind for the rendered fragments: comrak
//! documents it as inserted verbatim, and `format_html` emits it
//! unconditionally, whereas `HtmlBlock` / `HtmlInline` would be filtered out
//! by `render.unsafe`.
//!
//! [`crate::ir`]'s `IrWalker` walks the same AST off the same
//! [`ConstructCursor`] and [`ParaScan`] primitives, differing only in its
//! emit target. Four cases, referred to by number throughout:
//!
//! 1. **Sole-block-sentinel paragraph** — insert a `Raw` before the
//!    paragraph, then detach it. A close is spliced only against an open the
//!    walk has seen; an open the source never closes is closed at end of
//!    document.
//! 2. **Heading-hint promotion** (`［＃「X」は大見出し］`) — rewrite the
//!    paragraph in place to a `Heading` whose sole child is the target text,
//!    and advance the cursor past every sentinel it would have consumed.
//! 3. **Inline sentinels in a `Text` node** — split around each sentinel and
//!    weave in `Raw` siblings. A block sentinel surviving into an inline
//!    context drops silently.
//! 4. **Orphan `［＃...］`** — a bracket run no notation claimed, replaced by
//!    a hidden wrapper. No cursor advance: it has no construct.

use core::mem;
use std::borrow::Cow;

use comrak::Arena;
use comrak::nodes::{AstNode, NodeHeading, NodeValue};

/// Spelled the way the parser spells its own unclaimed bracket runs, so one
/// situation has one markup shape.
const ORPHAN_WRAPPER_OPEN: &str = r#"<span class="aozora-md-directive" hidden>"#;

use crate::constructs::{
    BlockSentinelKind, ConstructCursor, Constructs, HeadingHint, INLINE_SENTINEL, ParaScan,
    block_sentinel_of, inline_is_dropped, is_sentinel_char, paragraph_sole_block_sentinel,
};
use crate::push_html_escaped;

/// After this returns the AST carries no sentinel character, so
/// `comrak::format_html` emits fully resolved HTML in one verbatim pass.
///
/// Literal markdown contexts (code spans, link destinations) ask the table
/// for the construct's *source text* instead of its HTML, since a notation
/// must render verbatim there.
pub(crate) fn splice_into_ast<'a>(
    root: &'a AstNode<'a>,
    arena: &'a Arena<'a>,
    constructs: &Constructs,
) {
    let mut splicer = AstSplicer::<'a, '_> {
        cursor: constructs.cursor(),
        open_containers: Vec::new(),
        in_heading_depth: 0,
        arena,
    };
    splicer.walk(root);
    splicer.drain_unclosed_containers(root);
}

/// Consumes the construct table in source order, weaving rendered Aozora
/// HTML into the comrak tree.
struct AstSplicer<'a, 't> {
    cursor: ConstructCursor<'t>,
    /// Closing markup for each still-open container, innermost last. It is
    /// carried rather than looked up because a close marker renders to
    /// nothing on its own — the closing tag comes from the *opening* marker.
    open_containers: Vec<String>,
    /// A heading body must satisfy Tier C (no `aozora-md-directive`) *and*
    /// Tier A (no bare `［＃`), so an orphan bracket run surfacing there is
    /// dropped rather than wrapped. Case 2 is the legitimate way Aozora
    /// notation reaches a heading.
    in_heading_depth: u32,
    arena: &'a Arena<'a>,
}

impl<'a> AstSplicer<'a, '_> {
    /// Explicit work stack rather than recursion: comrak builds
    /// arbitrarily deep ASTs from small inputs (`> > > …`, nested list
    /// items, nested emphasis) with no nesting cap, so a recursive descent
    /// would exhaust the call stack — a hard abort under `panic = "abort"`,
    /// which `SECURITY.md` scopes IN as a crash on untrusted input. The
    /// stack bounds growth by input size instead. comrak's own `format_html`
    /// is iterative for the same reason.
    ///
    /// A `Heading`'s subtree is bracketed by [`Work::ExitHeading`] so
    /// `in_heading_depth` covers exactly its descendants, reproducing the
    /// recursive increment/decrement the Tier-A / Tier-C contract needs. The
    /// snapshot-on-push discipline stays sound because every leaf dispatch
    /// only inserts fresh siblings or detaches the current node.
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

    /// Cases 1/2/3. Only the ordinary-paragraph case descends, and it needs
    /// no depth marker because a paragraph is never a `Heading`.
    fn dispatch_paragraph(&mut self, paragraph: &'a AstNode<'a>, stack: &mut Vec<Work<'a>>) {
        if let Some(kind) = paragraph_sole_block_sentinel(paragraph) {
            self.handle_block_sentinel(paragraph, kind);
            return;
        }
        let scan = ParaScan::run(paragraph, &self.cursor);
        if let Some(hint) = scan.first_heading_hint {
            self.handle_heading_hint(paragraph, &hint, scan.total_sentinels);
            return;
        }
        // Case 3: ordinary paragraph — descend to children for inline
        // sentinel splitting.
        push_children_rev(stack, paragraph);
    }

    fn handle_block_sentinel(&mut self, paragraph: &'a AstNode<'a>, kind: BlockSentinelKind) {
        match self.block_html(kind) {
            Some(html) => self.replace_with_block_html(paragraph, html),
            // Nothing to say for this sentinel: drop the paragraph rather
            // than leak the PUA codepoint into the rendered HTML.
            None => paragraph.detach(),
        }
    }

    /// `None` when the construct contributes no block of its own. Also does
    /// the container bookkeeping, so markup and stack cannot disagree.
    fn block_html(&mut self, kind: BlockSentinelKind) -> Option<String> {
        // Table exhausted: nothing stands behind this sentinel.
        let hit = self.cursor.next()?;
        match (kind, block_sentinel_of(hit.kind)?) {
            (BlockSentinelKind::Leaf, BlockSentinelKind::Leaf) => hit.html(),
            (BlockSentinelKind::Open, BlockSentinelKind::Open) => {
                // A marker that renders to nothing opens nothing, so the
                // drain does not owe it a close.
                let (open, close) = hit.container_halves()?;
                self.open_containers.push(close);
                Some(open)
            }
            // The close the matching open carried. An orphan close (no
            // matching open) emits nothing rather than an unbalanced close
            // tag (Tier-D protection).
            (BlockSentinelKind::Close, BlockSentinelKind::Close) => self.open_containers.pop(),
            // Mismatch (table/AST drift): emit nothing.
            _ => None,
        }
    }

    fn handle_heading_hint(
        &mut self,
        paragraph: &'a AstNode<'a>,
        hint: &HeadingHint,
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
        let mut escaped = String::with_capacity(hint.target.len());
        push_html_escaped(&mut escaped, &hint.target);
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
                let Some(hit) = self.cursor.next() else {
                    continue;
                };
                if ch == INLINE_SENTINEL
                    && block_sentinel_of(hit.kind).is_none()
                    // Ruby / bouten / TCY / gaiji / kaeriten / angle quote
                    // are explicitly allowed inside a heading per Tier C's
                    // documented contract; `inline_is_dropped` names the
                    // two kinds that are not, and the IR walker asks it
                    // the same question.
                    && !inline_is_dropped(hit.kind, self.in_heading_depth > 0)
                    && let Some(html) = hit.html()
                {
                    segments.push(self.new_raw_node(html));
                }
                // Block sentinel surviving into inline context, or
                // inline-position table mismatch: drop silently.
            } else if ch == '［' && chars.peek() == Some(&'＃') {
                // Orphan `［＃...］` run no notation claimed.
                //
                // An unclaimed run may still *contain* a claimed construct —
                // `［＃改［＃「」は大見出し］` is an unclaimed prefix wrapped
                // around a heading hint — so every sentinel inside the run is
                // consumed here as well. Copying one through would publish the
                // codepoint (the run is emitted as literal text) *and* leave
                // the cursor pointing at a construct the rest of the document
                // then reads as someone else's.
                chars.next(); // consume ＃
                if self.in_heading_depth > 0 {
                    // Heading bodies must satisfy both Tier A (no bare
                    // `［＃` leak) and Tier C (no `aozora-md-directive`
                    // contamination). The wrapper would resolve Tier A
                    // but break Tier C, and emitting the literal run
                    // would break Tier A. Silently consume the orphan
                    // run instead — the canonical way to inject an
                    // Aozora annotation into a heading is the
                    // heading-hint promotion path (Case 2), not a raw
                    // bracket run that survives lexer parsing.
                    for b in chars.by_ref() {
                        if is_sentinel_char(b) {
                            self.cursor.next();
                            continue;
                        }
                        if b == '］' {
                            break;
                        }
                    }
                    continue;
                }
                let mut bracket_body = String::from("［＃");
                for b in chars.by_ref() {
                    if is_sentinel_char(b) {
                        // The run reaches the reader as the author's own
                        // bytes, so a construct inside it is restored to the
                        // source it was written as — the same answer the
                        // other literal contexts give (`rewrite_literal_context`).
                        // Its own `］` does not close the run: the run's
                        // brackets are the ones in the substituted text.
                        if let Some(literal) = self.cursor.next_literal() {
                            bracket_body.push_str(literal);
                        }
                        continue;
                    }
                    bracket_body.push(b);
                    if b == '］' {
                        break;
                    }
                }
                self.flush_text(&mut current, &mut segments);
                let mut html = String::with_capacity(bracket_body.len() + 64);
                html.push_str(ORPHAN_WRAPPER_OPEN);
                push_html_escaped(&mut html, &bracket_body);
                html.push_str("</span>");
                segments.push(self.new_raw_node(html));
            } else {
                current.push(ch);
            }
        }
        self.flush_text(&mut current, &mut segments);
        // Insert all segments after the original Text node, then detach the
        // original — including when there are none, which means every
        // character was consumed. A heading body that is nothing but an
        // orphan `［＃` run is that case: the branch above swallows the run
        // rather than break Tier A or Tier C, and an early return here left
        // the original node in place with its sentinels still in it (Tier B).
        // Reachable on `main` today through any ATX heading whose body is one
        // such run, so this is a pre-existing leak rather than one the rule
        // row uncovered; what the rule row added is the setext spellings of
        // the same shape, which is how the property suite found it.
        let mut anchor: &'a AstNode<'a> = node;
        for seg in segments {
            anchor.insert_after(seg);
            anchor = seg;
        }
        node.detach();
    }

    /// Consumes one table entry per sentinel so later ones stay in
    /// lockstep. A sentinel with no construct left is dropped, not leaked.
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

    /// Code is literal markdown: `` `｜青梅《おうめ》` `` must render as the
    /// text the author typed, not as an interpreted ruby.
    fn splice_code_literal(&mut self, node: &'a AstNode<'a>, literal: &str) {
        let rewritten = self.rewrite_literal_context(literal);
        let mut data = node.data.borrow_mut();
        match &mut data.value {
            NodeValue::Code(code) => code.literal = rewritten,
            NodeValue::CodeBlock(code) => code.literal = rewritten,
            _ => {}
        }
    }

    /// So a notation written inside a URL keeps the literal URL the author
    /// typed rather than a percent-encoded sentinel. Runs after the node's
    /// children, keeping cursor consumption in source order.
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
        // Innermost first, so the tags nest the way the source opened them.
        for close in mem::take(&mut self.open_containers).into_iter().rev() {
            root.append(self.new_raw_node(close));
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
    /// Restores `in_heading_depth` after a `Heading`'s whole subtree.
    ExitHeading,
    /// Deferred until after the link text, so the fields consume their
    /// constructs in source order.
    ProcessLinkFields(&'a AstNode<'a>),
}

/// Reverse document order, so the `Vec`-as-stack pops them left-to-right.
/// Snapshotting the children here — before any dispatch mutates the tree —
/// is what makes the iterative walk equivalent to the recursive one.
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
    /// Carries at least one sentinel char or orphan `［＃` prefix. The
    /// `String` is captured so the dispatch needs no re-borrow.
    TextWith(String),
    /// Code span or block whose literal carries at least one sentinel.
    CodeWith(String),
    /// Recurse into children, then rewrite `url`/`title`.
    RecurseLink,
    /// May carry interesting descendants — recurse.
    Recurse,
    /// Opaque content (raw HTML, fenced code, already-rendered `Raw`) that
    /// must not be searched for sentinels.
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
        // must still consume its construct so later sentinels stay in
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
        // A code block is literal markdown for the same reason a code
        // span is. A *fenced* one never carries a sentinel — the mask
        // hides the triggers inside a fence before the lexer runs
        // (ADR-0010) — but an *indented* one is context the mask
        // deliberately does not reproduce (see `crate::code_block_mask`),
        // and comrak reads one out of any four-space line. A sentinel
        // that lands there has to be written back as the source the
        // author typed, or it reaches the reader as a private-use
        // codepoint (Tier B).
        NodeValue::CodeBlock(c) => {
            if c.literal.chars().any(is_sentinel_char) {
                DispatchAction::CodeWith(c.literal.clone())
            } else {
                DispatchAction::Skip
            }
        }
        NodeValue::HtmlBlock(_) | NodeValue::HtmlInline(_) | NodeValue::Raw(_) => {
            DispatchAction::Skip
        }
        _ => DispatchAction::Recurse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::code_block_mask;
    use crate::constructs::BLOCK_LEAF_SENTINEL;

    /// Mirrors `drive_pipeline` exactly, so these unit tests exercise the
    /// same code-block-mask boundary the production renderer uses.
    fn render_via_ast_splice(input: &str) -> String {
        let (masked, originals) = code_block_mask::mask_code_block_triggers(input);
        let constructs = Constructs::build(&masked);
        let comrak_arena: Arena<'_> = Arena::new();
        let opts = comrak::Options::default();
        let root = comrak::parse_document(&comrak_arena, constructs.text(), &opts);
        splice_into_ast(root, &comrak_arena, &constructs);
        let mut html = String::new();
        comrak::format_html(root, &opts, &mut html).expect("formatting to a String never fails");
        code_block_mask::unmask(&html, &originals).into_owned()
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
            !html.contains("aozora-md-directive"),
            "aozora-md-directive leaked into heading: {html}"
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
            !html.contains("aozora-md-directive"),
            "aozora-md-directive leaked into setext heading: {html}"
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
        // `［＃` survives outside an `aozora-md-directive` wrapper).
        let html = render_via_ast_splice("［＃\n※");
        assert!(
            html.contains("<span class=\"aozora-md-directive\" hidden>［＃</span>"),
            "wrapped run did not honour Text-node boundary: {html}"
        );
        assert!(
            !html.contains("<span class=\"aozora-md-directive\" hidden>［＃\n※"),
            "wrap leaked across SoftBreak: {html}"
        );
    }
}
