//! Turning the wiki's HTML into hints keyed by the `block::item` path.
//!
//! The documentation already addresses every setting the way we want to —
//! `set::anti-flood::connect-flood` — and gives most of them their own
//! heading. So the extraction is: walk the headings, work out what path each
//! one names, and keep the prose underneath it.

use std::collections::BTreeMap;

/// path -> prose
pub type Docs = BTreeMap<String, String>;

/// A heading and the text under it, in document order.
struct Section {
    level: u8,
    heading: String,
    body: String,
}

fn strip_between(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find(open) {
            None => {
                out.push_str(rest);
                return out;
            }
            Some(i) => {
                out.push_str(&rest[..i]);
                let after = &rest[i..];
                match after.find(close) {
                    None => return out,
                    Some(j) => rest = &after[j + close.len()..],
                }
            }
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(semi) = s[i..].find(';').filter(|&p| p <= 12) {
                let ent = &s[i + 1..i + semi];
                let ch = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" | "#160" => Some(' '),
                    "mdash" | "#8212" => Some('\u{2014}'),
                    "ndash" | "#8211" => Some('\u{2013}'),
                    "hellip" => Some('\u{2026}'),
                    "rsquo" | "#8217" => Some('\u{2019}'),
                    "lsquo" => Some('\u{2018}'),
                    "ldquo" => Some('\u{201C}'),
                    "rdquo" => Some('\u{201D}'),
                    _ => ent
                        .strip_prefix('#')
                        .and_then(|n| n.parse::<u32>().ok())
                        .and_then(char::from_u32),
                };
                if let Some(c) = ch {
                    out.push(c);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let c = s[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// HTML to readable plain text. Deliberately small: this input is one wiki's
/// generated markup, not the open web.
pub fn to_text(html: &str) -> String {
    let mut s = html.to_string();
    for (o, c) in [("<head", "</head>"), ("<script", "</script>"), ("<style", "</style>"), ("<!--", "-->")] {
        s = strip_between(&s, o, c);
    }

    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in s.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let t = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                let closing = tag.starts_with('/');
                match t.as_str() {
                    "br" => out.push('\n'),
                    "li" if !closing => out.push_str("\n- "),
                    "p" | "div" | "tr" | "pre" | "dd" | "dt" | "ul" | "ol" | "table"
                    | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => out.push('\n'),
                    "td" | "th" => out.push(' '),
                    _ => {}
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }

    let text = unescape(&out);
    // Tidy: drop MediaWiki furniture, collapse runs of blank lines.
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim_end();
        let l = l.trim_start_matches(|c: char| c == '\u{FEFF}');
        if l.trim() == "[edit]" || l.trim() == "edit" {
            continue;
        }
        if l.trim_start().starts_with("This article is issued from") {
            break;
        }
        lines.push(l.to_string());
    }
    let mut result = String::new();
    let mut blanks = 0;
    for l in lines {
        if l.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
            result.push('\n');
        } else {
            blanks = 0;
            result.push_str(l.trim_end());
            result.push('\n');
        }
    }
    result.trim().to_string()
}

/// Split an article into its headings and the prose beneath each.
fn sections(html: &str) -> Vec<Section> {
    let mut out = Vec::new();
    let mut rest = html;
    let mut pending: Option<(u8, String, usize)> = None;
    let mut cursor = 0usize;

    // Walk <hN ...> ... </hN> occurrences in order.
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i + 3 < bytes.len() {
        if bytes[i] == b'<' && (bytes[i + 1] == b'h' || bytes[i + 1] == b'H') && bytes[i + 2].is_ascii_digit() {
            let level = bytes[i + 2] - b'0';
            if (1..=6).contains(&level) {
                if let Some(close_rel) = html[i..].find(&format!("</h{level}")) {
                    let inner = &html[i..i + close_rel];
                    let heading = to_text(inner).trim().to_string();
                    let end_rel = html[i + close_rel..].find('>').map(|p| i + close_rel + p + 1).unwrap_or(i + close_rel);
                    if let Some((lv, hd, body_start)) = pending.take() {
                        out.push(Section { level: lv, heading: hd, body: html[body_start..i].to_string() });
                    }
                    pending = Some((level, heading, end_rel));
                    i = end_rel;
                    cursor = end_rel;
                    continue;
                }
            }
        }
        i += 1;
    }
    if let Some((lv, hd, body_start)) = pending {
        out.push(Section { level: lv, heading: hd, body: html[body_start..].to_string() });
    }
    let _ = (rest, cursor);
    rest = "";
    let _ = rest;
    out
}

/// Names the article shows being *written* in a config file, taken from its
/// own examples: `maxperip 3;`, `permissions {`, `tls;`.
///
/// A wiki heading under a block becomes a directive path, which is how most
/// of the hints are found. But an ordinary prose heading looks the same: the
/// operclass page has a section called "Copy-pasting", and Confetti offered
/// `copy-pasting` as something to put inside an operclass block. A heading is
/// only a directive if the page also shows it being used as one.
pub fn written_names(html: &str) -> std::collections::BTreeSet<String> {
    let text = to_text(html);
    let mut out = std::collections::BTreeSet::new();
    for line in text.lines() {
        let t = line.trim();
        // Config punctuation, so a sentence never counts.
        if !(t.ends_with(';') || t.ends_with('{') || t.ends_with('}')) {
            continue;
        }
        let name: String = t
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_')
            .collect();
        if name.is_empty() || name.len() == t.len() {
            continue;
        }
        let rest = &t[name.len()..];
        if rest.starts_with(' ') || rest.starts_with('\t') || rest.starts_with('{') || rest.starts_with(';') {
            out.insert(name);
        }
    }
    out
}

/// The paths the article's own config examples actually contain, read with
/// the same parser Confetti uses on your files.
///
/// A wiki heading under a block becomes a directive path, which is how most
/// hints are found — but the wiki also gives the block's *label* its own
/// section. The Oper block page has a heading "name", explaining the name in
/// `oper <name> { }`; Confetti offered it as something to put inside an oper
/// block, where it means nothing. A word search over the whole wiki cannot
/// tell those apart, because `name` really is a directive inside `me { }`.
/// Reading the examples can: no example ever writes `name` inside an oper.
pub fn example_paths(html: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for block in code_blocks(html) {
        let doc = crate::cst::Doc::parse(block);
        for top in doc.root.entries() {
            let name = entry_name(&doc, top);
            if name.is_empty() {
                continue;
            }
            let label = entry_value(&doc, top);
            collect_paths(&doc, top, &name, label.as_deref(), &mut out);
        }
    }
    out
}

fn collect_paths(
    doc: &crate::cst::Doc,
    node: &crate::cst::Node,
    path: &str,
    label: Option<&str>,
    out: &mut std::collections::BTreeSet<String>,
) {
    let Some(block) = node.block() else { return };
    for child in block.entries() {
        let name = entry_name(doc, child);
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
            continue;
        }
        let here = format!("{path}::{name}");
        out.insert(here.clone());
        // The wiki nests headings under the label as well — "Except ban block"
        // gives except::ban::mask, while the scrape calls it except::mask.
        // Both spellings have to count, or a real directive reads as prose.
        if let Some(l) = label {
            out.insert(format!("{path}::{l}::{name}"));
        }
        collect_paths(doc, child, &here, None, out);
    }
}

fn entry_name(doc: &crate::cst::Doc, n: &crate::cst::Node) -> String {
    n.first(crate::cst::Kind::Name)
        .map(|w| doc.word_text(w).to_ascii_lowercase())
        .unwrap_or_default()
}

fn entry_value(doc: &crate::cst::Doc, n: &crate::cst::Node) -> Option<String> {
    let v = n.first(crate::cst::Kind::Value).map(|w| doc.word_text(w).to_ascii_lowercase())?;
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return None;
    }
    Some(v)
}

/// The contents of every `<pre>` on the page — where the wiki puts config
/// examples, and the only part of an article that is config rather than
/// English about config.
fn code_blocks(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut at = 0usize;
    while let Some(rel) = lower[at..].find("<pre") {
        let start = at + rel;
        let Some(open_end) = lower[start..].find('>').map(|i| start + i + 1) else { break };
        let Some(close) = lower[open_end..].find("</pre").map(|i| open_end + i) else { break };
        out.push(to_text(&html[open_end..close]));
        at = close + 5;
    }
    out
}

/// A default the documentation states in words: "Default: 3", "The default is
/// yes". Kept apart from the one read out of the source, because the two
/// sometimes disagree — the wiki says set::handshake-timeout defaults to 30
/// and conf.c sets 40 — and in that argument the source wins.
pub fn stated_default(text: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        let rest = if let Some(r) = lower.strip_prefix("default value:") {
            &t[t.len() - r.len()..]
        } else if let Some(r) = lower.strip_prefix("default:") {
            &t[t.len() - r.len()..]
        } else if let Some(at) = lower.find("the default is ") {
            &t[at + 15..]
        } else if let Some(at) = lower.find("defaults to ") {
            &t[at + 12..]
        } else {
            continue;
        };
        let v = tidy_default(rest);
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

/// The value itself, without the sentence that explains it.
fn tidy_default(s: &str) -> String {
    let s = s.trim();
    // Stop at the first thing that reads as commentary rather than value.
    let mut end = s.len();
    for (i, _) in s.char_indices() {
        let rest = &s[i..];
        if rest.starts_with('.') || rest.starts_with(',') || rest.starts_with(';') {
            end = i;
            break;
        }
        for word in [" which ", " and the ", " so that ", " if you ", " because ", " but ", " unless "] {
            if rest.starts_with(word) {
                end = end.min(i);
            }
        }
        if end < s.len() {
            break;
        }
    }
    let v = s[..end].trim().trim_matches('"').trim();
    // A config snippet is not a value: "defaults to set { x yes" comes from an
    // example block, and `op" (channel operator)` from a half-eaten quote.
    if v.len() > 44 || v.contains('{') || v.contains('}') || v.contains('"') {
        return String::new();
    }
    v.to_string()
}

fn is_directive_like(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().unwrap().is_ascii_lowercase()
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == ':')
        && !s.ends_with(':')
}

/// `"Allow DCC block"` -> `"allow dcc"`, `"Set block"` -> `"set"`.
fn block_key(heading: &str) -> Option<String> {
    let h = heading.trim();
    let lower = h.to_ascii_lowercase();
    let stem = lower.strip_suffix(" block")?;
    let stem = stem.trim();
    if stem.is_empty() || stem.len() > 40 {
        return None;
    }
    Some(stem.to_string())
}

/// Headings that structure the page rather than name a setting.
const FURNITURE: &[&str] = &[
    "syntax", "example", "examples", "see also", "notes", "note", "introduction",
    "required items", "optional items", "required", "optional", "how permissions work",
    "available settings", "default", "defaults", "usage", "contents", "overview",
];

/// Build the path -> prose map from one article.
pub fn extract(html: &str, docs: &mut Docs) {
    let mut sink = BTreeMap::new();
    extract_into(html, docs, &mut sink);
}

/// As `extract`, and also accumulate the full prose under each block heading.
///
/// The second map is what rescues the majority of directives: the wiki only
/// gives some of them their own heading, and explains the rest in running text
/// ("sendq specifies the amount of data which can be in the send queue"). With
/// the block's text in hand and a directive name from the source scrape, that
/// sentence can be found directly — see `prose_for`.
pub fn extract_into(html: &str, docs: &mut Docs, block_text: &mut BTreeMap<String, String>) {
    let mut current_block: Option<(u8, String)> = None;
    let mut current_sub: Option<(u8, String)> = None;

    for sec in sections(html) {
        // Accumulate everything under whichever block heading is open.
        if let Some((lv, b)) = &current_block {
            if sec.level > *lv {
                let entry = block_text.entry(b.replace(' ', "::")).or_default();
                entry.push('\n');
                entry.push_str(&to_text(&sec.body));
            }
        }
        let heading = sec.heading.trim().to_string();
        if heading.is_empty() {
            continue;
        }
        let lower = heading.to_ascii_lowercase();
        let body = to_text(&sec.body);

        // A heading that is already a full path wins outright — the docs write
        // `set::anti-flood::connect-flood` verbatim.
        if is_directive_like(&lower) && lower.contains("::") {
            insert(docs, &lower, &body);
            current_sub = Some((sec.level, lower));
            continue;
        }

        if let Some(bk) = block_key(&heading) {
            current_block = Some((sec.level, bk.clone()));
            current_sub = None;
            insert(docs, &bk, &body);
            let keyed = bk.replace(' ', "::");
            let e = block_text.entry(keyed).or_default();
            e.push('\n');
            e.push_str(&body);
            // `deny dcc` is also reachable as plain `deny`
            if let Some((first, _)) = bk.split_once(' ') {
                insert_if_absent(docs, first, &body);
                let e = block_text.entry(first.to_string()).or_default();
                e.push('\n');
                e.push_str(&body);
            }
            continue;
        }

        if FURNITURE.contains(&lower.as_str()) {
            continue;
        }

        if is_directive_like(&lower) {
            // Nest under whichever heading is currently open above us.
            let parent = match (&current_sub, &current_block) {
                (Some((lv, p)), _) if sec.level > *lv => Some(p.clone()),
                (_, Some((lv, b))) if sec.level > *lv => Some(b.replace(' ', "::")),
                _ => current_block.as_ref().map(|(_, b)| b.replace(' ', "::")),
            };
            let path = match parent {
                Some(p) if !lower.starts_with(&format!("{p}::")) => format!("{p}::{lower}"),
                _ => lower.clone(),
            };
            insert(docs, &path, &body);
            // The bare name is a useful fallback for directives that repeat at
            // several depths (`mask`, `options`, `ban-time`).
            insert_if_absent(docs, &lower, &body);
            current_sub = Some((sec.level, path));
        }
    }
}

fn insert(docs: &mut Docs, key: &str, body: &str) {
    let b = body.trim();
    if b.is_empty() {
        return;
    }
    // Prefer the longer explanation when a path is documented twice.
    match docs.get(key) {
        Some(existing) if existing.len() >= b.len() => {}
        _ => {
            docs.insert(key.to_string(), b.to_string());
        }
    }
}

fn insert_if_absent(docs: &mut Docs, key: &str, body: &str) {
    let b = body.trim();
    if b.is_empty() || docs.contains_key(key) {
        return;
    }
    docs.insert(key.to_string(), b.to_string());
}

/// Find the sentence in a block's prose that explains one directive.
///
/// The wiki writes these as "sendq specifies the amount of data…", "pingfreq is
/// the number of seconds…", "The mask defines from which host…". We only accept
/// a line that *opens* with the name, because a wrong hint is worse than none.
pub fn prose_for(section: &str, name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();

    // Every block's article opens with a Syntax listing, and those lines start
    // with the directive name too — `pingfreq <ping-frequency>;`. Taking one of
    // those as the explanation tells the reader nothing they can't already see.
    let is_syntax = |l: &str| {
        let t = l.trim_end();
        t.ends_with(';')
            || t.ends_with('{')
            || t.ends_with('}')
            || (t.contains('<') && t.contains('>'))
            || t.starts_with('/')
    };

    for raw in section.lines() {
        if is_syntax(raw) {
            continue;
        }
        let mut line = raw.trim();
        for marker in ["- ", "* ", "\u{2022} "] {
            line = line.strip_prefix(marker).unwrap_or(line);
        }
        let l = line.trim_start_matches(['`', '*', '\'', '"']);
        let lc = l.to_ascii_lowercase();

        // "sendq specifies …", "options::nofakelag allows …"
        let opens_with_name = lc.strip_prefix(&lower).is_some_and(|rest| {
            rest.starts_with(' ') || rest.starts_with("::") || rest.starts_with(" is ")
        });
        // "The mask defines …", "The item name specifies …"
        let described = ["the ", "the item ", "the directive "].iter().any(|p| {
            lc.strip_prefix(p)
                .and_then(|r| r.strip_prefix(&lower))
                .is_some_and(|r| r.starts_with(' '))
        });

        if (opens_with_name || described) && l.len() > name.len() + 15 {
            return Some(l.to_string());
        }
    }

    // Some blocks describe several directives in one breath: "The item name
    // specifies the name of the server, info specifies the server's info line,
    // sid is an unique server id…". Pull out just the clause that names this
    // one. Only the explicit verbs, so we don't grab a passing mention.
    const VERBS: &[&str] = &[
        " is ", " specifies ", " defines ", " sets ", " contains ", " allows ",
        " can be ", " should be ", " must be ", " determines ", " configures ",
    ];
    for raw in section.lines() {
        if is_syntax(raw) {
            continue;
        }
        let line = raw.trim();
        let lc = line.to_ascii_lowercase();
        let mut from = 0usize;
        while let Some(rel) = lc[from..].find(&lower) {
            let at = from + rel;
            let after = at + lower.len();
            let before_ok = at == 0
                || !lc[..at]
                    .chars()
                    .last()
                    .map(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':')
                    .unwrap_or(false);
            if before_ok {
                if let Some(verb) = VERBS.iter().find(|v| lc[after..].starts_with(**v)) {
                    let _ = verb;
                    let tail = &line[at..];
                    let clause = match tail.find(". ") {
                        Some(i) => &tail[..i + 1],
                        None => tail,
                    };
                    let clause = clause.trim_end_matches([',', ';']).trim();
                    if clause.len() > name.len() + 20 {
                        return Some(clause.to_string());
                    }
                }
            }
            from = at + lower.len().max(1);
        }
    }
    None
}

/// The first sentence or two — what a hint panel actually shows.
pub fn summarize(text: &str, max: usize) -> String {
    let first = text
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with("- ") && l.len() > 20)
        .unwrap_or_else(|| text.lines().next().unwrap_or("").trim());
    if first.chars().count() <= max {
        return first.to_string();
    }
    let cut: String = first.chars().take(max).collect();
    match cut.rfind(". ") {
        Some(i) if i > max / 3 => cut[..i + 1].to_string(),
        _ => match cut.rfind(' ') {
            Some(i) => format!("{}…", &cut[..i]),
            None => cut,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_is_only_a_directive_if_the_page_writes_it_as_one() {
        let html = "<h2>Operclass block</h2><p>Blah.</p>\
                    <h3>Copy-pasting</h3><p>Copy the relevant block to your own file.</p>\
                    <h3>parent</h3><pre>operclass netadmin {\n\tparent admin;\n\tpermissions {\n\t\tchat;\n\t}\n}</pre>";
        let w = written_names(html);
        assert!(w.contains("parent"), "{w:?}");
        assert!(w.contains("permissions"), "{w:?}");
        assert!(w.contains("chat"), "{w:?}");
        assert!(!w.contains("copy-pasting"), "{w:?}");
        // Prose about copying is not a directive called copy.
        assert!(!w.contains("copy"), "{w:?}");
    }

    #[test]
    fn a_default_stated_in_words_is_read_as_a_value() {
        assert_eq!(stated_default("Blah.\nDefault: 3\nMore."), Some("3".into()));
        assert_eq!(stated_default("The default is yes."), Some("yes".into()));
        assert_eq!(stated_default("Defaults to 307 and the maximum is 360"), Some("307".into()));
        assert_eq!(stated_default("Default value: nick-user-host"), Some("nick-user-host".into()));
        // A sentence about defaults that never gives one.
        assert_eq!(stated_default("You should change this from the default."), None);
        // And nothing that would fill the column with an essay.
        assert_eq!(stated_default("The default is a long explanation that runs on and on past any use"), None);
        // Nor a config example that happens to sit after the words.
        assert_eq!(stated_default("defaults to set { server-notice-colors yes"), None);
        assert_eq!(stated_default("The default is \"op\" (channel operator)"), None);
    }

    #[test]
    fn config_examples_are_read_with_the_real_parser() {
        let html = "<p>The name in the <code>oper name {</code> block…</p>\
                    <pre>oper bobsmith {\n\tclass clients;\n\toperclass netadmin;\n\tmask {\n\t\tip 1.2.3.4;\n\t}\n}</pre>\
                    <pre>me {\n\tname \"irc.example.org\";\n}</pre>";
        let p = example_paths(html);
        assert!(p.contains("oper::class"), "{p:?}");
        assert!(p.contains("oper::operclass"), "{p:?}");
        assert!(p.contains("oper::mask::ip"), "{p:?}");
        assert!(p.contains("me::name"), "{p:?}");
        // The thing that started this: `name` is a directive in me { }, and
        // never one inside oper { }, however often the wiki says the word.
        assert!(!p.contains("oper::name"), "{p:?}");
        // Prose outside a <pre> is not an example, however config-shaped.
        assert!(!p.contains("oper::block"), "{p:?}");
    }

    #[test]
    fn a_labelled_block_counts_both_spellings() {
        let html = "<pre>except ban {\n\tmask *@example.org;\n\ttype blacklist;\n}</pre>";
        let p = example_paths(html);
        assert!(p.contains("except::mask"), "{p:?}");
        assert!(p.contains("except::ban::mask"), "{p:?}");
        assert!(p.contains("except::ban::type"), "{p:?}");
    }

    #[test]
    fn html_to_text_basics() {
        let t = to_text("<p>Hello <b>there</b> &amp; welcome</p><ul><li>one</li><li>two</li></ul>");
        assert!(t.contains("Hello there & welcome"));
        assert!(t.contains("- one"));
    }

    #[test]
    fn sections_split_on_headings() {
        let html = "<h2>Class block</h2><p>Intro text about classes here.</p>\
                    <h3>pingfreq</h3><p>How often to ping, in seconds.</p>";
        let mut docs = Docs::new();
        extract(html, &mut docs);
        assert!(docs.contains_key("class"), "{:?}", docs.keys().collect::<Vec<_>>());
        assert_eq!(
            docs.get("class::pingfreq").map(|s| s.trim()),
            Some("How often to ping, in seconds.")
        );
    }

    #[test]
    fn full_paths_in_headings_are_used_verbatim() {
        let html = "<h3>set::anti-flood::connect-flood</h3><p>Limits new connections.</p>";
        let mut docs = Docs::new();
        extract(html, &mut docs);
        assert_eq!(
            docs.get("set::anti-flood::connect-flood").map(|s| s.trim()),
            Some("Limits new connections.")
        );
    }

    #[test]
    fn furniture_headings_are_skipped() {
        let html = "<h2>Oper block</h2><p>About opers.</p><h3>Syntax</h3><pre>oper x {}</pre>";
        let mut docs = Docs::new();
        extract(html, &mut docs);
        assert!(!docs.contains_key("oper::syntax"));
        assert!(!docs.contains_key("syntax"));
    }

    #[test]
    fn summarize_trims_to_a_sentence() {
        let s = summarize("This is the first sentence. And this is a second one that goes on.", 40);
        assert!(s.len() <= 41, "{s}");
        assert!(s.starts_with("This is the first sentence"));
    }
}
