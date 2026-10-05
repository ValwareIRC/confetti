//! Merging blocks together and splitting them apart.
//!
//! UnrealIRCd is happy with several `set { }` blocks — the shipped example has
//! three, organised by topic — and that is fine for the server but awkward for
//! a person hunting one setting. So: fold them into one, or break one apart.
//!
//! ## Why this is only offered for `set`
//!
//! Because for most other blocks, moving text changes behaviour:
//!
//! * `allow` and `deny channel` are **evaluated in reverse config order**.
//!   `_conf_allow` ends in `AddListItem` (conf.c:6065) and `add_ListItem`
//!   *prepends* (list.c:478); `AllowClient` returns on the first match. So the
//!   last `allow` in the last-included file is tried first — which is exactly
//!   why the shipped example can put `allow { mask *; }` before a specific one
//!   and have the specific one still reachable. Reordering silently rehomes
//!   clients into a different class with a different sendq.
//! * `class` blocks with one name are already merged by the server —
//!   `_conf_class` reuses the struct (conf.c:4930) — but it also resets
//!   `options` and `connfreq` each time, so merging changes the result.
//! * `security-group` merges *additively*, including onto the built-in groups.
//! * `oper` duplicates are legal and last-wins; `link` is in file order.
//!
//! ## The duplicate model
//!
//! `CheckDuplicate` (conf.c:7299) *warns* and skips validation of the second
//! copy; `_conf_set` (conf.c:7870) then applies it anyway. So for a scalar the
//! **last one wins**. But `set::anti-flood` and `set::options` are cumulative —
//! each block applies its own items and leaves the rest — and
//! `set::tls::certificate` builds a list (conf.c:3229). Commenting one of those
//! out deletes live settings, so this module refuses to auto-resolve anything
//! that is not a plain scalar.

use crate::cst::{Doc, Kind, Node};

/// Blocks it is safe to fold together. Everything else changes meaning.
pub const MERGEABLE: &[&str] = &["set"];

/// Why a given block type may not be merged, in words.
pub fn why_not_mergeable(name: &str) -> Option<&'static str> {
    match name {
        "set" => None,
        "allow" | "deny" =>
            Some("allow and deny blocks are matched in reverse order — the last one in the file is tried first — so combining them would change who lands in which class."),
        "class" =>
            Some("UnrealIRCd already merges class blocks that share a name, and it resets options and connfreq each time it sees one, so folding them by hand gives a different result."),
        "security-group" =>
            Some("security-group blocks of the same name already merge, and they add to the built-in groups rather than replacing them."),
        "oper" =>
            Some("two oper blocks with one name are legal, and the later one wins. Merging them would silently drop a set of permissions."),
        "link" =>
            Some("link blocks are used in file order and each describes one server, so there is nothing to gain by combining them."),
        "listen" =>
            Some("each listen block is one port. Combining them would leave you with one."),
        _ => Some("this kind of block is applied per-block, so folding several into one would change what the server does."),
    }
}

/// Directives that build a list rather than being overwritten, so a second
/// occurrence is not a conflict at all. `set::tls::certificate` and `key` are
/// how a dual RSA+ECDSA certificate setup is written (conf.c:3229-3247).
const ADDITIVE: &[&str] = &["certificate", "key"];

#[derive(Debug, Clone, serde::Serialize)]
pub struct Conflict {
    pub name: String,
    pub lines: Vec<usize>,
    pub values: Vec<String>,
    /// True when every copy is a plain `name value;`. Only these can be
    /// resolved automatically; a repeated sub-block is cumulative and
    /// commenting one out would delete settings that are live today.
    pub scalar: bool,
    /// What the server will do if it is left alone
    pub effect: String,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Keep {
    /// Change nothing. The server takes the last value; we say so.
    All,
    /// Comment out all but the first — note this *changes* the effective value
    First,
    /// Comment out all but the last, matching what the server already does
    Last,
}

/* ------------------------------------------------------------------ */
/* text helpers                                                        */
/* ------------------------------------------------------------------ */

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0)
}
fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map(|i| at + i + 1).unwrap_or(text.len())
}
fn indent_of(text: &str, at: usize) -> String {
    let ls = line_start(text, at);
    text[ls..at].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The line ending this file uses, so inserted text matches it. A file with
/// CRLF endings must not come back with a mixture.
fn eol_of(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count();
    if crlf > 0 && crlf * 2 >= lf { "\r\n" } else { "\n" }
}

/// Is this offset the start of its line, ignoring whitespace? Used to tell a
/// comment on its own line from one trailing another statement.
fn alone_on_line(text: &str, at: usize) -> bool {
    text[line_start(text, at)..at].chars().all(|c| c == ' ' || c == '\t')
}

fn reindent(chunk: &str, to: &str, eol: &str) -> String {
    let lines: Vec<&str> = chunk.lines().collect();
    let common = lines.iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines.iter()
        .map(|l| if l.trim().is_empty() { String::new() }
                 else { format!("{to}{}", &l[common.min(l.len())..].trim_end_matches('\r')) })
        .collect::<Vec<_>>()
        .join(eol)
}

fn entry_name(doc: &Doc, e: &Node) -> String {
    e.first(Kind::Name).map(|w| doc.word_text(w).to_string()).unwrap_or_default()
}
fn entry_label(doc: &Doc, e: &Node) -> Option<String> {
    e.first(Kind::Value).map(|w| doc.word_text(w).to_string())
}

fn find_entry(n: &Node, start: usize) -> Option<&Node> {
    if n.kind == Kind::Entry && n.span.start == start {
        return Some(n);
    }
    n.children.iter().find_map(|c| find_entry(c, start))
}

/// Where an entry's own comments begin.
///
/// Uses the parse tree, not string prefixes. A comment counts as belonging to
/// the entry only if it sits on a line of its own — otherwise
/// `class x { } /* the usual */` above a `set` block would drag the whole
/// class into it, which is exactly the bug this replaces.
fn comment_head(doc: &Doc, parent: &Node, entry_start: usize) -> usize {
    let text = doc.source();
    let idx = match parent.children.iter().position(|c| c.span.start == entry_start) {
        Some(i) => i,
        None => return line_start(text, entry_start),
    };
    let mut head = entry_start;
    let mut i = idx;
    while i > 0 {
        i -= 1;
        let prev = &parent.children[i];
        match prev.kind {
            Kind::Space => {
                // A blank line ends the association.
                let s = &text[prev.span.clone()];
                if s.matches('\n').count() > 1 {
                    break;
                }
            }
            Kind::LineComment | Kind::BlockComment => {
                if !alone_on_line(text, prev.span.start) {
                    break; // trailing another statement — not ours
                }
                head = prev.span.start;
            }
            _ => break,
        }
    }
    line_start(text, head)
}

/// How deeply nested in `@if` this offset is. Merging across a preprocessor
/// boundary would move settings into or out of a conditional, which changes
/// whether they apply at all.
fn preproc_depth_at(doc: &Doc, offset: usize) -> i32 {
    let mut depth = 0;
    fn scan(doc: &Doc, n: &Node, offset: usize, depth: &mut i32) {
        for c in &n.children {
            if c.span.start >= offset {
                return;
            }
            if c.kind == Kind::Preproc {
                let t = doc.text_of(&c.span).trim_start();
                if t.starts_with("@if") {
                    *depth += 1;
                } else if t.starts_with("@endif") {
                    *depth -= 1;
                }
            }
            scan(doc, c, offset, depth);
        }
    }
    scan(doc, &doc.root, offset, &mut depth);
    depth
}

/* ------------------------------------------------------------------ */
/* merge                                                               */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, serde::Serialize)]
pub struct MergePlan {
    pub block: String,
    pub label: Option<String>,
    pub count: usize,
    pub conflicts: Vec<Conflict>,
    /// The whole file as it would be, for a diff before anything is committed
    pub text: String,
}

pub fn merge(text: &str, starts: &[usize], keep: Keep) -> Result<MergePlan, String> {
    if starts.len() < 2 {
        return Err("Pick at least two blocks to merge.".into());
    }
    let doc = Doc::parse(text.to_string());
    let eol = eol_of(text);

    let mut picked: Vec<&Node> = Vec::new();
    for s in starts {
        picked.push(find_entry(&doc.root, *s)
            .ok_or("One of those blocks has moved — reopen the file and try again.")?);
    }
    picked.sort_by_key(|e| e.span.start);

    // All the same kind, all the same name, and a kind that may be merged.
    let name = entry_name(&doc, picked[0]);
    let label = entry_label(&doc, picked[0]);
    if let Some(why) = why_not_mergeable(&name) {
        return Err(format!("Confetti won't merge {name} blocks: {why}"));
    }
    for e in &picked {
        if entry_name(&doc, e) != name {
            return Err("Those aren't all the same kind of block.".into());
        }
        if entry_label(&doc, e) != label {
            return Err(format!(
                "Those {name} blocks have different names. `set {{ }}` and `set unknown-users {{ }}` are different things, so merging them would change what they mean."));
        }
        if e.block().is_none() {
            return Err("One of those has no { } to merge.".into());
        }
    }

    // Never merge across a preprocessor boundary.
    let base_depth = preproc_depth_at(&doc, picked[0].span.start);
    for e in picked.iter().skip(1) {
        if preproc_depth_at(&doc, e.span.start) != base_depth {
            return Err(
                "Those blocks aren't in the same @if section, so merging them would change whether their settings apply at all."
                    .into());
        }
    }

    let dest = picked[0];
    let dest_block = dest.block().unwrap();
    let dest_rb = dest_block.children.iter().rfind(|c| c.kind == Kind::RBrace)
        .ok_or("The first block isn't closed.")?;
    let inner_indent = match dest_block.entries().last() {
        Some(c) => indent_of(text, c.span.start),
        None => format!("{}\t", indent_of(text, dest.span.start)),
    };
    let closing_indent = indent_of(text, dest_rb.span.start);

    let mut moved = String::new();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for src in picked.iter().skip(1) {
        let sb = src.block().unwrap();
        let lb = sb.children.iter().find(|c| c.kind == Kind::LBrace).unwrap();
        let rb = sb.children.iter().rfind(|c| c.kind == Kind::RBrace).unwrap();
        let body = text[lb.span.end..rb.span.start].trim_matches(|c| c == '\n' || c == '\r');
        let head_from = comment_head(&doc, &doc.root, src.span.start);

        if !body.trim().is_empty() {
            let heading = text[head_from..line_start(text, src.span.start)].trim_end();
            moved.push_str(eol);
            if !heading.trim().is_empty() {
                moved.push_str(&reindent(heading, &inner_indent, eol));
                moved.push_str(eol);
            }
            moved.push_str(&reindent(body, &inner_indent, eol));
            moved.push_str(eol);
        }
        cuts.push((head_from, line_end(text, src.span.end)));
    }

    // Insert immediately before the closing brace, not at the start of its
    // line — a one-line block like `set { x; }` has its `}` in the middle of
    // the line, and inserting at line start would put the moved directives
    // *before* the block, at the top level.
    let before_rb = &text[line_start(text, dest_rb.span.start)..dest_rb.span.start];
    let mut payload = String::new();
    if !before_rb.chars().all(|c| c == ' ' || c == '\t') {
        payload.push_str(eol); // the `}` shares a line with content
    }
    payload.push_str(&moved);
    payload.push_str(&closing_indent);

    let mut out = Doc::parse(text.to_string());
    out.edit(dest_rb.span.start..dest_rb.span.start, payload)?;
    for (a, b) in &cuts {
        out.edit(*a..*b, String::new())?;
    }
    let merged_text = out.render();

    let conflicts = duplicates_in(&merged_text, &name, label.as_deref());
    let text_final = match keep {
        Keep::All => merged_text,
        k => resolve(&merged_text, &conflicts, &name, label.as_deref(), k)?,
    };

    Ok(MergePlan { block: name, label, count: picked.len(), conflicts, text: text_final })
}

/// Directives appearing more than once at the top of a block.
fn duplicates_in(text: &str, name: &str, label: Option<&str>) -> Vec<Conflict> {
    let doc = Doc::parse(text.to_string());
    let Some(block) = doc.root.entries()
        .find(|e| entry_name(&doc, e) == name && entry_label(&doc, e).as_deref() == label) else {
        return Vec::new();
    };
    let Some(b) = block.block() else { return Vec::new() };

    let mut seen: std::collections::BTreeMap<String, (Vec<usize>, Vec<String>, bool)> = Default::default();
    for c in b.entries() {
        let n = entry_name(&doc, c);
        // A bare quoted line is content, not a directive name.
        if c.first(Kind::Name).and_then(|w| w.quote).is_some() {
            continue;
        }
        if ADDITIVE.contains(&n.as_str()) {
            continue;
        }
        let is_scalar = c.block().is_none();
        let v = entry_label(&doc, c)
            .or_else(|| c.block().map(|bb| format!("{{ {} items }}", bb.entries().count())))
            .unwrap_or_else(|| "on".into());
        let e = seen.entry(n).or_insert_with(|| (Vec::new(), Vec::new(), true));
        e.0.push(doc.line_of(c.span.start));
        e.1.push(v);
        e.2 &= is_scalar;
    }
    seen.into_iter()
        .filter(|(_, (lines, _, _))| lines.len() > 1)
        .map(|(name, (lines, values, scalar))| {
            let effect = if scalar {
                format!("The server takes the last one, so “{}” wins.",
                        values.last().cloned().unwrap_or_default())
            } else {
                "These are sub-blocks, and UnrealIRCd applies all of them — each adds its own settings. Leave them alone unless you know one is redundant.".to_string()
            };
            Conflict { name, lines, values, scalar, effect }
        })
        .collect()
}

fn resolve(text: &str, conflicts: &[Conflict], name: &str, label: Option<&str>, keep: Keep)
    -> Result<String, String>
{
    let resolvable: Vec<&Conflict> = conflicts.iter().filter(|c| c.scalar).collect();
    if resolvable.is_empty() {
        return Ok(text.to_string());
    }
    let doc = Doc::parse(text.to_string());
    let Some(block) = doc.root.entries()
        .find(|e| entry_name(&doc, e) == name && entry_label(&doc, e).as_deref() == label) else {
        return Ok(text.to_string());
    };
    let Some(b) = block.block() else { return Ok(text.to_string()) };

    let names: std::collections::BTreeSet<&str> = resolvable.iter().map(|c| c.name.as_str()).collect();
    let mut by_name: std::collections::BTreeMap<String, Vec<&Node>> = Default::default();
    for c in b.entries() {
        let n = entry_name(&doc, c);
        if names.contains(n.as_str()) && c.block().is_none() {
            by_name.entry(n).or_default().push(c);
        }
    }

    let mut out = Doc::parse(text.to_string());
    for (_, group) in by_name {
        if group.len() < 2 {
            continue;
        }
        let losers: Vec<&&Node> = match keep {
            Keep::First => group.iter().skip(1).collect(),
            Keep::Last => group.iter().take(group.len() - 1).collect(),
            Keep::All => continue,
        };
        for l in losers {
            comment_span(text, &mut out, l.span.start, l.span.end)?;
        }
    }
    Ok(out.render())
}

fn comment_span(text: &str, doc: &mut Doc, from: usize, to: usize) -> Result<(), String> {
    let mut at = line_start(text, from);
    while at < to {
        let indent: usize = text[at..].chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| c.len_utf8())
            .sum();
        doc.edit(at + indent..at + indent, "//".to_string())?;
        at = match text[at..].find('\n') {
            Some(i) => at + i + 1,
            None => break,
        };
    }
    Ok(())
}

/* ------------------------------------------------------------------ */
/* split                                                               */
/* ------------------------------------------------------------------ */

pub fn split(text: &str, block_start: usize, child_starts: &[usize]) -> Result<String, String> {
    if child_starts.is_empty() {
        return Err("Pick at least one directive to split out.".into());
    }
    let doc = Doc::parse(text.to_string());
    let eol = eol_of(text);
    let block = find_entry(&doc.root, block_start)
        .ok_or("That block has moved — reopen the file and try again.")?;
    let b = block.block().ok_or("That entry has no { } to split.")?;
    let name = entry_name(&doc, block);
    let label = entry_label(&doc, block);
    if let Some(why) = why_not_mergeable(&name) {
        return Err(format!("Confetti won't split {name} blocks: {why}"));
    }
    let rb = b.children.iter().rfind(|c| c.kind == Kind::RBrace)
        .ok_or("That block isn't closed.")?;

    let mut kids: Vec<&Node> = Vec::new();
    for s in child_starts {
        kids.push(b.entries().find(|c| c.span.start == *s)
            .ok_or("One of those directives isn't in that block.")?);
    }
    if kids.len() == b.entries().count() {
        return Err("That would empty the block — split out fewer than all of them.".into());
    }
    kids.sort_by_key(|k| k.span.start);

    let outer_indent = indent_of(text, block.span.start);
    let inner_indent = format!("{outer_indent}\t");

    let mut moved = String::new();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for k in &kids {
        let from = comment_head(&doc, b, k.span.start);
        // Never run past the closing brace: a directive written on the same
        // line as `}` would otherwise take the brace with it and the new block
        // would end up nested inside the old one.
        let mut to = line_end(text, k.span.end);
        if to > rb.span.start {
            to = k.span.end;
        }
        moved.push_str(&reindent(text[from..to].trim_end_matches(['\n', '\r']), &inner_indent, eol));
        moved.push_str(eol);
        cuts.push((from, to));
    }

    let head = match &label {
        Some(l) => format!("{name} {l}"),
        None => name.clone(),
    };
    let after = line_end(text, block.span.end).min(text.len());
    let new_block = format!("{eol}{outer_indent}{head} {{{eol}{moved}{outer_indent}}}{eol}");

    let mut out = Doc::parse(text.to_string());
    out.edit(after..after, new_block)?;
    for (a, bb) in &cuts {
        out.edit(*a..*bb, String::new())?;
    }
    Ok(out.render())
}

/* ------------------------------------------------------------------ */
/* invariants — what a safe move must preserve                         */
/* ------------------------------------------------------------------ */

/// Every value in the file, with its quoting, sorted.
///
/// Deliberately excludes directive names: merging three `set { }` blocks into
/// one is *supposed* to remove two `set` tokens, and that is the operation
/// rather than a loss. Names are covered by `paths` below, which is the
/// stronger check anyway because it pins each one to its depth.
pub fn values(text: &str) -> Vec<String> {
    let doc = Doc::parse(text.to_string());
    let mut out = Vec::new();
    fn walk(doc: &Doc, n: &Node, out: &mut Vec<String>) {
        for c in &n.children {
            if matches!(c.kind, Kind::Value | Kind::Extra) {
                out.push(format!("{:?}:{}", c.quote, doc.text_of(&c.span)));
            }
            walk(doc, c, out);
        }
    }
    walk(&doc, &doc.root, &mut out);
    out.sort();
    out
}

/// Every leaf directive as `path = value`, sorted. Catches a directive that
/// ended up at the wrong depth — `set::set::maxchannelsperuser` — which token
/// conservation alone would happily allow.
pub fn paths(text: &str) -> Vec<String> {
    let doc = Doc::parse(text.to_string());
    let mut out = Vec::new();
    fn walk(doc: &Doc, n: &Node, prefix: &str, out: &mut Vec<String>) {
        for e in n.entries() {
            let name = entry_name(doc, e);
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}::{name}") };
            match e.block() {
                Some(b) => walk(doc, b, &path, out),
                None => out.push(format!("{path} = {}", entry_label(doc, e).unwrap_or_default())),
            }
        }
    }
    walk(&doc, &doc.root, "", &mut out);
    out.sort();
    out
}

/// The mix of line endings, so a CRLF file does not come back mixed.
pub fn eol_counts(text: &str) -> (usize, usize) {
    let crlf = text.matches("\r\n").count();
    (crlf, text.matches('\n').count() - crlf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts_of(text: &str, name: &str) -> Vec<usize> {
        let doc = Doc::parse(text.to_string());
        doc.root.entries()
            .filter(|e| entry_name(&doc, e) == name)
            .map(|e| e.span.start)
            .collect()
    }

    /// Every safe move keeps these. Run it on both sides of any merge/split.
    fn assert_preserved(before: &str, after: &str) {
        assert_eq!(values(before), values(after), "a value was lost, gained or changed");
        assert_eq!(paths(before), paths(after), "a directive changed name or depth");
        assert_eq!(eol_counts(before).0 > 0, eol_counts(after).0 > 0,
                   "line endings changed kind");
        let d = Doc::parse(after.to_string());
        d.verify_tiling().unwrap();
        assert!(d.diagnostics.iter().all(|x| x.severity != crate::cst::Severity::Error),
                "result does not parse: {:?}\n{after}", d.diagnostics);
    }

    const THREE_SETS: &str = "\
/* Network configuration */
set {
\tnetwork-name \"ExampleNET\";
\tdefault-server \"irc.example.org\";
}

/* Server specific configuration */
set {
\tkline-address 'set@example.org';
\tmodes-on-connect \"+ixw\";
}

set {
\tconnthrottle {
\t\tnew-users { local-throttle 20:60; }
\t}
}
";

    #[test]
    fn merges_three_set_blocks_into_one() {
        let starts = starts_of(THREE_SETS, "set");
        assert_eq!(starts.len(), 3);
        let plan = merge(THREE_SETS, &starts, Keep::All).expect("merge");
        assert_eq!(plan.count, 3);
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        assert_preserved(THREE_SETS, &plan.text);

        let doc = Doc::parse(plan.text.clone());
        assert_eq!(doc.root.entries().filter(|e| entry_name(&doc, e) == "set").count(), 1,
                   "\n{}", plan.text);
        assert!(plan.text.contains("Server specific configuration"), "lost a comment:\n{}", plan.text);
        assert!(plan.text.contains("local-throttle 20:60;"));
    }

    #[test]
    fn paths_are_preserved_so_nothing_changes_depth() {
        let starts = starts_of(THREE_SETS, "set");
        let plan = merge(THREE_SETS, &starts, Keep::All).unwrap();
        let before: Vec<String> = paths(THREE_SETS);
        let after: Vec<String> = paths(&plan.text);
        assert_eq!(before, after, "a directive changed depth\n{}", plan.text);
    }

    // ---- the five corruptions the council found ----

    #[test]
    fn a_one_line_destination_block_does_not_get_directives_put_before_it() {
        let text = "set { network-name \"X\"; }\nset { maxchannelsperuser 10; }\n";
        let starts = starts_of(text, "set");
        let plan = merge(text, &starts, Keep::All).unwrap();
        assert_preserved(text, &plan.text);
        let d = Doc::parse(plan.text.clone());
        let tops: Vec<String> = d.root.entries().map(|e| entry_name(&d, e)).collect();
        assert_eq!(tops, vec!["set"], "a directive escaped to the top level:\n{}", plan.text);
        assert_eq!(paths(text), paths(&plan.text));
    }

    #[test]
    fn a_comment_trailing_another_block_is_not_stolen() {
        let text = "class clients { sendq 200k; } /* the usual */\nset { a 1; }\nset { b 2; }\n";
        let starts = starts_of(text, "set");
        let plan = merge(text, &starts, Keep::All).unwrap();
        let d = Doc::parse(plan.text.clone());
        assert_eq!(d.root.entries().filter(|e| entry_name(&d, e) == "class").count(), 1,
                   "the class block was swallowed:\n{}", plan.text);
        assert_preserved(text, &plan.text);
    }

    #[test]
    fn a_whole_line_comment_above_a_block_does_travel_with_it() {
        let text = "set { a 1; }\n\n/* about b */\nset { b 2; }\n";
        let starts = starts_of(text, "set");
        let plan = merge(text, &starts, Keep::All).unwrap();
        assert!(plan.text.contains("about b"), "lost the comment:\n{}", plan.text);
        assert_preserved(text, &plan.text);
    }

    #[test]
    fn refuses_to_merge_across_an_at_if() {
        let text = "set {\n\ta 1;\n}\n@if module-loaded(\"x\")\nset {\n\tb 2;\n}\n@endif\n";
        let starts = starts_of(text, "set");
        let err = merge(text, &starts, Keep::All).unwrap_err();
        assert!(err.contains("@if"), "{err}");
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let text = "set {\r\n\ta 1;\r\n}\r\n\r\nset {\r\n\tb 2;\r\n}\r\n";
        let starts = starts_of(text, "set");
        let plan = merge(text, &starts, Keep::All).unwrap();
        assert!(!plan.text.contains("\n\n\r"), "mixed endings:\n{:?}", plan.text);
        let (crlf, lf) = eol_counts(&plan.text);
        assert_eq!(lf, 0, "bare LF crept into a CRLF file: {crlf} crlf, {lf} lf\n{:?}", plan.text);
        assert_preserved(text, &plan.text);
    }

    #[test]
    fn splitting_a_directive_that_shares_a_line_with_the_brace() {
        let text = "set {\n\tnetwork-name \"X\";\n\tmaxchannelsperuser 10; }\n";
        let doc = Doc::parse(text.to_string());
        let set = doc.root.entries().next().unwrap();
        let last = set.block().unwrap().entries().last().unwrap();
        let out = split(text, set.span.start, &[last.span.start]).unwrap();
        assert_preserved(text, &out);
        let d = Doc::parse(out.clone());
        let tops: Vec<String> = d.root.entries().map(|e| entry_name(&d, e)).collect();
        assert_eq!(tops, vec!["set", "set"], "nested instead of split:\n{out}");
        assert!(!paths(&out).iter().any(|p| p.contains("set::set")), "nested:\n{out}");
    }

    // ---- the duplicate model ----

    const CLASHING: &str = "set {\n\tmaxchannelsperuser 10;\n}\n\nset {\n\tmaxchannelsperuser 25;\n}\n";

    #[test]
    fn a_scalar_duplicate_says_the_last_one_wins() {
        let starts = starts_of(CLASHING, "set");
        let plan = merge(CLASHING, &starts, Keep::All).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        let c = &plan.conflicts[0];
        assert_eq!(c.name, "maxchannelsperuser");
        assert!(c.scalar);
        assert!(c.effect.contains("25"), "{}", c.effect);
    }

    #[test]
    fn repeated_sub_blocks_are_not_offered_for_resolution() {
        // Two anti-flood blocks are cumulative in the server: each applies its
        // own items. Commenting one out would delete live settings.
        let text = "set {\n\tanti-flood {\n\t\tknown-users { nick-flood 3:60; }\n\t}\n}\n\n\
set {\n\tanti-flood {\n\t\tunknown-users { nick-flood 2:60; }\n\t}\n}\n";
        let starts = starts_of(text, "set");
        let plan = merge(text, &starts, Keep::All).unwrap();
        let c = plan.conflicts.iter().find(|c| c.name == "anti-flood").expect("noticed");
        assert!(!c.scalar, "a sub-block is not a scalar conflict");
        assert!(c.effect.contains("applies all of them"), "{}", c.effect);

        // And Keep::Last must not touch it.
        let plan2 = merge(text, &starts, Keep::Last).unwrap();
        assert!(!plan2.text.contains("//"), "commented out a cumulative block:\n{}", plan2.text);
        assert_eq!(paths(text), paths(&plan2.text));
    }

    #[test]
    fn additive_directives_are_not_conflicts() {
        // A dual RSA+ECDSA setup writes certificate twice on purpose.
        let text = "set {\n\ttls {\n\t\tcertificate \"a.pem\";\n\t}\n}\n\n\
set {\n\ttls {\n\t\tcertificate \"b.pem\";\n\t}\n}\n";
        let starts = starts_of(text, "set");
        let plan = merge(text, &starts, Keep::All).unwrap();
        assert!(plan.conflicts.iter().all(|c| c.name != "certificate"),
                "certificate is additive, not a conflict: {:?}", plan.conflicts);
    }

    #[test]
    fn keeping_the_last_matches_what_the_server_already_does() {
        let starts = starts_of(CLASHING, "set");
        let plan = merge(CLASHING, &starts, Keep::Last).unwrap();
        assert!(plan.text.contains("\t//maxchannelsperuser 10;"), "\n{}", plan.text);
        let d = Doc::parse(plan.text.clone());
        assert_eq!(d.root.entries().next().unwrap().block().unwrap().entries()
                    .filter(|e| entry_name(&d, e) == "maxchannelsperuser").count(), 1);
    }

    // ---- refusals ----

    #[test]
    fn refuses_block_types_where_merging_changes_behaviour() {
        for (kind, text) in [
            ("allow", "allow { mask *; class a; }\nallow { mask b; class a; }\n"),
            ("class", "class a { pingfreq 90; }\nclass a { sendq 1M; }\n"),
            ("oper", "oper a { class x; }\noper a { class y; }\n"),
            ("link", "link a { class x; }\nlink b { class x; }\n"),
        ] {
            let starts = starts_of(text, kind);
            let err = merge(text, &starts, Keep::All).unwrap_err();
            assert!(err.contains(kind), "{kind}: {err}");
        }
    }

    #[test]
    fn refuses_to_merge_differently_named_set_blocks() {
        let text = "set {\n\tauto-join \"#a\";\n}\n\nset unknown-users {\n\tauto-join \"#b\";\n}\n";
        let starts = starts_of(text, "set");
        let err = merge(text, &starts, Keep::All).unwrap_err();
        assert!(err.contains("different names"), "{err}");
    }

    // ---- split ----

    const ONE_SET: &str = "\
set {
\tnetwork-name \"ExampleNET\";
\t/* how hard people may push */
\tanti-flood {
\t\tchannel { default-profile normal; }
\t}
\tmaxchannelsperuser 10;
}
";

    #[test]
    fn splits_directives_into_a_new_block() {
        let doc = Doc::parse(ONE_SET.to_string());
        let set = doc.root.entries().next().unwrap();
        let af = set.block().unwrap().entries()
            .find(|e| entry_name(&doc, e) == "anti-flood").unwrap();
        let out = split(ONE_SET, set.span.start, &[af.span.start]).unwrap();
        assert_preserved(ONE_SET, &out);
        assert_eq!(paths(ONE_SET), paths(&out));

        let d2 = Doc::parse(out.clone());
        assert_eq!(d2.root.entries().filter(|e| entry_name(&d2, e) == "set").count(), 2, "\n{out}");
        assert!(out.contains("how hard people may push"), "lost the comment:\n{out}");
    }

    #[test]
    fn refuses_to_split_out_everything() {
        let doc = Doc::parse(ONE_SET.to_string());
        let set = doc.root.entries().next().unwrap();
        let all: Vec<usize> = set.block().unwrap().entries().map(|e| e.span.start).collect();
        assert!(split(ONE_SET, set.span.start, &all).is_err());
    }

    #[test]
    fn merge_then_split_keeps_every_directive() {
        let starts = starts_of(THREE_SETS, "set");
        let merged = merge(THREE_SETS, &starts, Keep::All).unwrap().text;
        let d = Doc::parse(merged.clone());
        let set = d.root.entries().next().unwrap();
        let kids: Vec<usize> = set.block().unwrap().entries().map(|e| e.span.start).collect();
        let out = split(&merged, set.span.start, &kids[2..]).unwrap();
        assert_preserved(THREE_SETS, &out);
        assert_eq!(paths(THREE_SETS), paths(&out), "a directive moved depth\n{out}");
    }
}

#[cfg(test)]
mod corpus {
    use super::*;
    use std::path::Path;

    /// Merge every `set` block in every shipped config and check the
    /// invariants hold on real files, not just constructed ones.
    #[test]
    fn merging_the_real_configs_preserves_everything() {
        let root = Path::new(r"C:\Users\Valerie\unrealircd-pr\doc\conf");
        if !root.exists() {
            eprintln!("no conf tree, skipping");
            return;
        }
        let mut merged_files = 0;
        for f in crate::workspace::discover(root) {
            let Ok(text) = std::fs::read_to_string(&f.path) else { continue };
            let doc = Doc::parse(text.clone());
            let starts: Vec<usize> = doc.root.entries()
                .filter(|e| entry_name(&doc, e) == "set" && e.block().is_some())
                .map(|e| e.span.start)
                .collect();
            if starts.len() < 2 {
                continue;
            }
            match merge(&text, &starts, Keep::All) {
                Ok(plan) => {
                    merged_files += 1;
                    assert_eq!(values(&text), values(&plan.text),
                               "{}: a value changed", f.rel);
                    assert_eq!(paths(&text), paths(&plan.text),
                               "{}: a directive changed depth", f.rel);
                    let d = Doc::parse(plan.text.clone());
                    assert!(d.diagnostics.iter().all(|x| x.severity != crate::cst::Severity::Error),
                            "{}: result does not parse: {:?}", f.rel, d.diagnostics);
                    println!("  {} — {} set blocks merged, {} conflicts",
                             f.rel, starts.len(), plan.conflicts.len());
                }
                Err(e) => println!("  {} — declined: {e}", f.rel),
            }
        }
        println!("\nmerged {merged_files} real files with every invariant intact");
        assert!(merged_files > 0, "no file had two set blocks to merge");
    }
}
