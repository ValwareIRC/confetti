//! Lossless concrete syntax tree for UnrealIRCd configuration files.
//!
//! Modelled on the real tokenizer in `src/conf.c`
//! (`config_parse_with_offset`), including its quirks, so that what Confetti
//! shows matches what the server will actually do with the file.
//!
//! The invariant that makes this lossless: the children of every node tile
//! that node's span exactly, in order, with no gaps and no overlaps, and the
//! document tiles the whole source. Nothing is discarded -- comments, blank
//! lines, tabs, `#` versus `//`, preprocessor directives and stray tokens are
//! all nodes. `render()` on an unedited document returns the input byte for
//! byte, and `verify_tiling()` proves the parser accounted for every byte.

use std::ops::Range;

pub type Span = Range<usize>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Document,
    /// `name [value] ;` or `name [value] { .. } [;]`
    Entry,
    /// `{ .. }` including both braces
    Block,

    // leaves
    Name,
    Value,
    /// A third or later word on one entry. conf.c calls this "Extra data
    /// detected"; we keep it so the bytes survive and flag a diagnostic.
    Extra,
    LBrace,
    RBrace,
    Semi,
    Space,
    LineComment,
    BlockComment,
    /// `@if`, `@else`, `@endif`, `@define`, `@error`, `@warning` to end of line
    Preproc,
    /// Tolerated junk: an extra `;`, a stray `*/`
    Stray,
}

impl Kind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Kind::Space | Kind::LineComment | Kind::BlockComment | Kind::Preproc | Kind::Stray
        )
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub span: Span,
    pub children: Vec<Node>,
    /// For `Name`/`Value`/`Extra`: the quote character, if the word was quoted.
    /// `'` marks conf.c's "escaped" entries, which are taken literally.
    pub quote: Option<u8>,
}

impl Node {
    fn leaf(kind: Kind, span: Span) -> Node {
        Node { kind, span, children: Vec::new(), quote: None }
    }
    fn word(kind: Kind, span: Span, quote: Option<u8>) -> Node {
        Node { kind, span, children: Vec::new(), quote }
    }

    /// Direct children of the given kind.
    pub fn childs(&self, kind: Kind) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(move |c| c.kind == kind)
    }
    pub fn first(&self, kind: Kind) -> Option<&Node> {
        self.children.iter().find(|c| c.kind == kind)
    }
    /// The `{ .. }` of an entry, if it has one.
    pub fn block(&self) -> Option<&Node> {
        self.first(Kind::Block)
    }
    /// Entries directly inside this node (document or block).
    pub fn entries(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|c| c.kind == Kind::Entry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub offset: usize,
    pub line: usize,
    pub message: String,
}

/// Two queued edits clash if they would both rewrite the same bytes.
///
/// A zero-length span is a point insert. It may sit at either end of a
/// replacement — "insert before this, and also replace it" is well defined —
/// but not inside one, and two inserts may not share a point, because then
/// their order would be arbitrary.
fn spans_conflict(a: &Span, b: &Span) -> bool {
    let a_insert = a.start == a.end;
    let b_insert = b.start == b.end;
    match (a_insert, b_insert) {
        (true, true) => a.start == b.start,
        (true, false) => a.start > b.start && a.start < b.end,
        (false, true) => b.start > a.start && b.start < a.end,
        (false, false) => a.start < b.end && b.start < a.end,
    }
}

pub struct Doc {
    text: String,
    pub root: Node,
    pub diagnostics: Vec<Diagnostic>,
    edits: Vec<(Span, String)>,
}

impl Doc {
    pub fn parse(text: impl Into<String>) -> Doc {
        let text = text.into();
        let mut p = Parser { b: text.as_bytes(), i: 0, diags: Vec::new() };
        let children = p.parse_items(None);
        let end = p.i;
        let diagnostics = p.diags;
        let root = Node { kind: Kind::Document, span: 0..end, children, quote: None };
        Doc { text, root, diagnostics, edits: Vec::new() }
    }

    pub fn source(&self) -> &str {
        &self.text
    }

    pub fn text_of(&self, span: &Span) -> &str {
        &self.text[span.clone()]
    }

    /// The text of a word node with its quotes stripped.
    pub fn word_text(&self, node: &Node) -> &str {
        let s = self.text_of(&node.span);
        match node.quote {
            Some(_) if s.len() >= 2 => &s[1..s.len() - 1],
            _ => s,
        }
    }

    /// Replace the bytes at `span`. Edits are applied at render time, so an
    /// untouched region is always emitted exactly as it was read.
    ///
    /// Refuses anything it cannot apply faithfully: a span outside the text, a
    /// span that splits a character, or one that overlaps an edit already
    /// queued. Silently dropping an overlapping edit — which is what this used
    /// to do — loses a change with no error anywhere, and a feature that
    /// queues several edits is then one refactor away from eating your file.
    pub fn edit(&mut self, span: Span, new_text: impl Into<String>) -> Result<(), String> {
        if span.start > span.end {
            return Err(format!("backwards edit {}..{}", span.start, span.end));
        }
        if span.end > self.text.len() {
            return Err(format!(
                "edit {}..{} runs past the end of the file ({} bytes)",
                span.start, span.end, self.text.len()));
        }
        if !self.text.is_char_boundary(span.start) || !self.text.is_char_boundary(span.end) {
            return Err(format!("edit {}..{} would split a character", span.start, span.end));
        }
        for (existing, _) in &self.edits {
            if spans_conflict(existing, &span) {
                return Err(format!(
                    "edit {}..{} overlaps one already queued at {}..{}",
                    span.start, span.end, existing.start, existing.end));
            }
        }
        self.edits.push((span, new_text.into()));
        Ok(())
    }

    /// How many edits are waiting. Useful for asserting a plan did what it said.
    pub fn pending(&self) -> usize {
        self.edits.len()
    }

    pub fn is_dirty(&self) -> bool {
        !self.edits.is_empty()
    }

    /// The document as it should be written back to disk.
    pub fn render(&self) -> String {
        if self.edits.is_empty() {
            return self.text.clone();
        }
        let mut edits = self.edits.clone();
        // By start, and at the same offset an insert goes before a replacement
        // so that "insert here, and also replace what follows" reads in order.
        edits.sort_by(|(a, _), (b, _)| {
            a.start.cmp(&b.start)
                .then((a.end - a.start).cmp(&(b.end - b.start)))
        });
        let mut out = String::with_capacity(self.text.len());
        let mut at = 0usize;
        for (span, new) in edits {
            // `edit()` rejects overlaps, so this cannot happen. If it ever
            // does, that is a bug worth failing loudly for rather than losing
            // somebody's change in silence.
            debug_assert!(span.start >= at, "overlapping edit slipped through validation");
            if span.start < at {
                continue;
            }
            out.push_str(&self.text[at..span.start]);
            out.push_str(&new);
            at = span.end;
        }
        out.push_str(&self.text[at..]);
        out
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.text.as_bytes()[..offset.min(self.text.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count()
            + 1
    }

    /// Proves the parser accounted for every byte: children tile their parent
    /// exactly, in order. Returns the first violation found.
    pub fn verify_tiling(&self) -> Result<(), String> {
        fn walk(n: &Node) -> Result<(), String> {
            if n.children.is_empty() {
                return Ok(());
            }
            let mut at = n.span.start;
            for c in &n.children {
                if c.span.start != at {
                    return Err(format!(
                        "{:?} child {:?} starts at {} but parent is at {}",
                        n.kind, c.kind, c.span.start, at
                    ));
                }
                if c.span.end < c.span.start {
                    return Err(format!("{:?} has reversed span {:?}", c.kind, c.span));
                }
                at = c.span.end;
                walk(c)?;
            }
            if at != n.span.end {
                return Err(format!(
                    "{:?} children end at {} but parent ends at {}",
                    n.kind, at, n.span.end
                ));
            }
            Ok(())
        }
        walk(&self.root)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    diags: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn line_at(&self, off: usize) -> usize {
        self.b[..off.min(self.b.len())].iter().filter(|&&c| c == b'\n').count() + 1
    }

    fn err(&mut self, offset: usize, msg: impl Into<String>) {
        let line = self.line_at(offset);
        self.diags.push(Diagnostic {
            severity: Severity::Error,
            offset,
            line,
            message: msg.into(),
        });
    }

    fn warn(&mut self, offset: usize, msg: impl Into<String>) {
        let line = self.line_at(offset);
        self.diags.push(Diagnostic {
            severity: Severity::Warning,
            offset,
            line,
            message: msg.into(),
        });
    }

    /// Parse entries until end of input, or until the `}` that closes the
    /// block we are inside. Leaves `self.i` on that `}`.
    fn parse_items(&mut self, inside_block: Option<usize>) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        // The entry currently being built, as a list of its parts.
        let mut cur: Option<Vec<Node>> = None;

        macro_rules! finish_entry {
            ($semi:expr) => {{
                if let Some(mut parts) = cur.take() {
                    if let Some(semi_span) = $semi {
                        parts.push(Node::leaf(Kind::Semi, semi_span));
                    }
                    let span = parts[0].span.start..parts[parts.len() - 1].span.end;
                    out.push(Node { kind: Kind::Entry, span, children: parts, quote: None });
                }
            }};
        }

        while self.i < self.b.len() {
            let c = self.b[self.i];
            let start = self.i;

            match c {
                b' ' | b'\t' | b'\n' | b'\r' | b'=' => {
                    // conf.c treats '=' as whitespace, and blanks out '\r'.
                    while self.i < self.b.len()
                        && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r' | b'=')
                    {
                        self.i += 1;
                    }
                    let node = Node::leaf(Kind::Space, start..self.i);
                    match cur.as_mut() {
                        Some(parts) => parts.push(node),
                        None => out.push(node),
                    }
                }

                b';' => {
                    self.i += 1;
                    if cur.is_some() {
                        finish_entry!(Some(start..self.i));
                    } else {
                        self.warn(start, "Ignoring extra semicolon");
                        out.push(Node::leaf(Kind::Stray, start..self.i));
                    }
                }

                b'{' => {
                    let Some(mut parts) = cur.take() else {
                        self.err(
                            start,
                            "Section start with no name. Sections need a name, like 'oper {' or 'set {'.",
                        );
                        self.i += 1;
                        out.push(Node::leaf(Kind::Stray, start..self.i));
                        continue;
                    };
                    if parts.iter().any(|p| p.kind == Kind::Block) {
                        self.err(start, "Section start but the previous section did not end properly. A '};' is probably missing.");
                    }
                    self.i += 1; // consume '{'
                    let mut block_children = vec![Node::leaf(Kind::LBrace, start..self.i)];
                    let inner_start = self.i;
                    let inner = self.parse_items(Some(start));
                    block_children.extend(inner);

                    let close_end;
                    if self.i < self.b.len() && self.b[self.i] == b'}' {
                        let rb = self.i;
                        self.i += 1;
                        block_children.push(Node::leaf(Kind::RBrace, rb..self.i));
                        close_end = self.i;
                    } else {
                        self.err(
                            start,
                            "End of file reached but this section never ended. A '};' is missing.",
                        );
                        close_end = self.i;
                        let _ = inner_start;
                    }

                    parts.push(Node {
                        kind: Kind::Block,
                        span: start..close_end,
                        children: block_children,
                        quote: None,
                    });
                    cur = Some(parts);

                    // conf.c: a '}' not followed by ';' has the ';' simulated,
                    // so both `}` and `};` close the entry. Either way the
                    // entry ends here.
                    if self.i < self.b.len() && self.b[self.i] == b';' {
                        let s = self.i;
                        self.i += 1;
                        finish_entry!(Some(s..self.i));
                    } else {
                        finish_entry!(None::<Span>);
                    }
                }

                b'}' => {
                    if inside_block.is_none() {
                        self.err(start, "One '}' too many -- there is no open section to close.");
                        self.i += 1;
                        out.push(Node::leaf(Kind::Stray, start..self.i));
                        continue;
                    }
                    if cur.is_some() {
                        self.err(start, "Missing ';' before the closing brace.");
                        finish_entry!(None::<Span>);
                    }
                    // Leave self.i on the '}' for our caller to consume.
                    return out;
                }

                b'#' => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                    let node = Node::leaf(Kind::LineComment, start..self.i);
                    match cur.as_mut() {
                        Some(parts) => parts.push(node),
                        None => out.push(node),
                    }
                }

                b'/' if self.peek(1) == Some(b'/') => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                    let node = Node::leaf(Kind::LineComment, start..self.i);
                    match cur.as_mut() {
                        Some(parts) => parts.push(node),
                        None => out.push(node),
                    }
                }

                b'/' if self.peek(1) == Some(b'*') => {
                    self.i += 2;
                    let mut closed = false;
                    while self.i < self.b.len() {
                        if self.b[self.i] == b'*' && self.peek(1) == Some(b'/') {
                            self.i += 2;
                            closed = true;
                            break;
                        }
                        if self.b[self.i] == b'/' && self.peek(1) == Some(b'*') {
                            self.warn(self.i, "Nested comments are not supported.");
                        }
                        self.i += 1;
                    }
                    if !closed {
                        self.err(start, "This comment is never closed.");
                    }
                    let node = Node::leaf(Kind::BlockComment, start..self.i);
                    match cur.as_mut() {
                        Some(parts) => parts.push(node),
                        None => out.push(node),
                    }
                }

                b'@' => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                    let node = Node::leaf(Kind::Preproc, start..self.i);
                    match cur.as_mut() {
                        Some(parts) => parts.push(node),
                        None => out.push(node),
                    }
                }

                b'"' | b'\'' => {
                    let quote = c;
                    self.i += 1;
                    let mut terminated = false;
                    while self.i < self.b.len() {
                        let d = self.b[self.i];
                        if d == b'\\' && matches!(self.peek(1), Some(b'\\') | Some(b'"') | Some(b'\'')) {
                            self.i += 2;
                            continue;
                        }
                        if d == b'\n' {
                            break;
                        }
                        if d == quote {
                            self.i += 1;
                            terminated = true;
                            break;
                        }
                        self.i += 1;
                    }
                    if !terminated {
                        self.err(start, "Unterminated quote -- this string never closes.");
                    }
                    self.push_word(&mut cur, &mut out, start..self.i, Some(quote));
                }

                b'*' if self.peek(1) == Some(b'/') => {
                    self.warn(start, "Ignoring a stray '*/' -- there is no comment open here.");
                    self.i += 2;
                    let node = Node::leaf(Kind::Stray, start..self.i);
                    match cur.as_mut() {
                        Some(parts) => parts.push(node),
                        None => out.push(node),
                    }
                }

                _ => {
                    // A bare word. conf.c breaks only on space, tab, newline
                    // and ';' -- notably not on '{' or '}', so `oper bob{`
                    // really does produce the value `bob{`.
                    while self.i < self.b.len()
                        && !matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r' | b';')
                    {
                        self.i += 1;
                    }
                    let span = start..self.i;
                    if self.b[span.clone()].contains(&b'{') || self.b[span.clone()].contains(&b'}') {
                        self.warn(
                            start,
                            "A brace here is read as part of this word. Put a space before '{'.",
                        );
                    }
                    self.push_word(&mut cur, &mut out, span, None);
                }
            }
        }

        if cur.is_some() {
            let off = self.i.saturating_sub(1);
            self.err(off, "End of file reached but this line never ended. A ';' is probably missing.");
            finish_entry!(None::<Span>);
        }
        out
    }

    fn peek(&self, n: usize) -> Option<u8> {
        self.b.get(self.i + n).copied()
    }

    fn push_word(
        &mut self,
        cur: &mut Option<Vec<Node>>,
        out: &mut Vec<Node>,
        span: Span,
        quote: Option<u8>,
    ) {
        match cur {
            None => {
                *cur = Some(vec![Node::word(Kind::Name, span, quote)]);
            }
            Some(parts) => {
                let has_name = parts.iter().any(|p| p.kind == Kind::Name);
                let has_value = parts.iter().any(|p| p.kind == Kind::Value);
                let kind = if !has_name {
                    Kind::Name
                } else if !has_value {
                    Kind::Value
                } else {
                    self.err(span.start, "Extra data on this line. Perhaps a ';' is missing, or there is one word too many.");
                    Kind::Extra
                };
                parts.push(Node::word(kind, span, quote));
            }
        }
        let _ = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> Doc {
        let d = Doc::parse(src);
        d.verify_tiling().unwrap_or_else(|e| panic!("tiling broken: {e}"));
        assert_eq!(d.render(), src, "unedited round-trip must be byte-identical");
        d
    }

    #[test]
    fn empty_and_whitespace() {
        parsed("");
        parsed("\n\n\t  \n");
    }

    #[test]
    fn simple_block() {
        let d = parsed("me {\n\tname \"irc.example.org\";\n\tsid 001;\n}\n");
        let me = d.root.entries().next().unwrap();
        assert_eq!(d.word_text(me.first(Kind::Name).unwrap()), "me");
        let block = me.block().unwrap();
        let names: Vec<_> = block
            .entries()
            .map(|e| d.word_text(e.first(Kind::Name).unwrap()).to_string())
            .collect();
        assert_eq!(names, vec!["name", "sid"]);
        let name_entry = block.entries().next().unwrap();
        assert_eq!(d.word_text(name_entry.first(Kind::Value).unwrap()), "irc.example.org");
    }

    #[test]
    fn brace_without_semicolon_closes_the_entry() {
        // conf.c simulates the ';' after '}', so both spellings parse the same.
        let a = parsed("set { x 1; }\nset { y 2; }\n");
        let b = parsed("set { x 1; };\nset { y 2; };\n");
        assert_eq!(a.root.entries().count(), 2);
        assert_eq!(b.root.entries().count(), 2);
    }

    #[test]
    fn all_three_comment_styles_survive() {
        let src = "# one\n// two\n/* three\n   still three */\nme { sid 001; }\n";
        let d = parsed(src);
        let kinds: Vec<_> = d.root.children.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&Kind::LineComment));
        assert!(kinds.contains(&Kind::BlockComment));
        assert_eq!(d.root.entries().count(), 1);
    }

    #[test]
    fn block_value_and_subtype() {
        let d = parsed("oper bobsmith {\n\tclass opers;\n}\ndeny dcc {\n\tfilename \"*.exe\";\n}\n");
        let mut it = d.root.entries();
        let oper = it.next().unwrap();
        assert_eq!(d.word_text(oper.first(Kind::Name).unwrap()), "oper");
        assert_eq!(d.word_text(oper.first(Kind::Value).unwrap()), "bobsmith");
        let deny = it.next().unwrap();
        assert_eq!(d.word_text(deny.first(Kind::Name).unwrap()), "deny");
        assert_eq!(d.word_text(deny.first(Kind::Value).unwrap()), "dcc");
    }

    #[test]
    fn single_quotes_are_literal_and_preserved() {
        let d = parsed("admin {\n\t'Bob Smith';\n\t\"widely@used.name\";\n}\n");
        let block = d.root.entries().next().unwrap().block().unwrap();
        let first = block.entries().next().unwrap();
        let w = first.first(Kind::Name).unwrap();
        assert_eq!(w.quote, Some(b'\''));
        assert_eq!(d.word_text(w), "Bob Smith");
    }

    #[test]
    fn escaped_quote_inside_string() {
        let d = parsed("me {\n\tinfo \"a \\\" quote\";\n}\n");
        let e = d.root.entries().next().unwrap().block().unwrap().entries().next().unwrap();
        assert_eq!(d.word_text(e.first(Kind::Value).unwrap()), "a \\\" quote");
    }

    #[test]
    fn equals_is_whitespace() {
        let d = parsed("set { maxchannelsperuser = 10; }\n");
        let e = d.root.entries().next().unwrap().block().unwrap().entries().next().unwrap();
        assert_eq!(d.word_text(e.first(Kind::Name).unwrap()), "maxchannelsperuser");
        assert_eq!(d.word_text(e.first(Kind::Value).unwrap()), "10");
    }

    #[test]
    fn preprocessor_lines_are_preserved() {
        let src = "@if module-loaded(\"cloak_sha256\")\nset { cloak-method sha256; }\n@endif\n";
        let d = parsed(src);
        assert_eq!(d.root.children.iter().filter(|c| c.kind == Kind::Preproc).count(), 2);
    }

    #[test]
    fn five_levels_of_nesting() {
        let src = "set {\n anti-flood {\n  everyone {\n   handshake-data-flood {\n    amount 4k;\n   }\n  }\n }\n}\n";
        let d = parsed(src);
        let l1 = d.root.entries().next().unwrap();
        let l2 = l1.block().unwrap().entries().next().unwrap();
        let l3 = l2.block().unwrap().entries().next().unwrap();
        let l4 = l3.block().unwrap().entries().next().unwrap();
        let l5 = l4.block().unwrap().entries().next().unwrap();
        assert_eq!(d.word_text(l4.first(Kind::Name).unwrap()), "handshake-data-flood");
        assert_eq!(d.word_text(l5.first(Kind::Name).unwrap()), "amount");
        assert_eq!(d.word_text(l5.first(Kind::Value).unwrap()), "4k");
    }

    #[test]
    fn editing_touches_only_the_edited_bytes() {
        let src = "class clients\n{\n\t/* keep me */\n\tsendq 200k;   # and me\n\trecvq 8000;\n}\n";
        let mut d = Doc::parse(src);
        let e = d
            .root
            .entries()
            .next()
            .unwrap()
            .block()
            .unwrap()
            .entries()
            .next()
            .unwrap();
        let span = e.first(Kind::Value).unwrap().span.clone();
        d.edit(span, "1M").unwrap();
        let out = d.render();
        assert!(out.contains("sendq 1M;   # and me"));
        assert!(out.contains("/* keep me */"));
        assert_eq!(out, src.replace("200k", "1M"));
    }

    #[test]
    fn unterminated_quote_is_reported_not_panicked() {
        let d = Doc::parse("me {\n\tinfo \"never closed;\n}\n");
        assert!(d.diagnostics.iter().any(|x| x.severity == Severity::Error));
        assert_eq!(d.render(), "me {\n\tinfo \"never closed;\n}\n");
        d.verify_tiling().unwrap();
    }

    #[test]
    fn crlf_survives_untouched() {
        let src = "me {\r\n\tsid 001;\r\n}\r\n";
        parsed(src);
    }
}

#[cfg(test)]
mod edit_safety {
    use super::*;

    #[test]
    fn an_overlapping_edit_is_refused_not_swallowed() {
        let mut d = Doc::parse("aaaa bbbb cccc".to_string());
        d.edit(0..4, "XXXX").unwrap();
        // This used to be dropped silently, losing the change with no error.
        let e = d.edit(2..2, "!").unwrap_err();
        assert!(e.contains("overlaps"), "{e}");
        assert_eq!(d.pending(), 1);
        assert_eq!(d.render(), "XXXX bbbb cccc");
    }

    #[test]
    fn two_inserts_at_the_same_point_are_refused() {
        let mut d = Doc::parse("abc".to_string());
        d.edit(1..1, "X").unwrap();
        assert!(d.edit(1..1, "Y").is_err());
    }

    #[test]
    fn an_insert_at_the_edge_of_a_replacement_is_allowed() {
        let mut d = Doc::parse("abcdef".to_string());
        d.edit(2..4, "__").unwrap();
        d.edit(2..2, ">").unwrap();   // immediately before it
        d.edit(4..4, "<").unwrap();   // immediately after it
        assert_eq!(d.render(), "ab>__<ef");
    }

    #[test]
    fn edits_outside_the_text_are_refused() {
        let mut d = Doc::parse("abc".to_string());
        assert!(d.edit(0..99, "x").is_err());
        assert!(d.edit(3..1, "x").is_err());
    }

    #[test]
    fn an_edit_that_would_split_a_character_is_refused() {
        let mut d = Doc::parse("aé b".to_string()); // é is two bytes
        assert!(d.edit(1..2, "x").is_err(), "should refuse to cut é in half");
        assert!(d.edit(0..1, "X").is_ok());
    }

    #[test]
    fn many_non_overlapping_edits_all_apply() {
        let mut d = Doc::parse("one two three four".to_string());
        d.edit(0..3, "1").unwrap();
        d.edit(4..7, "2").unwrap();
        d.edit(8..13, "3").unwrap();
        d.edit(14..18, "4").unwrap();
        assert_eq!(d.render(), "1 2 3 4");
    }
}
