//! Reading a whole configuration and saying what is wrong with it.
//!
//! UnrealIRCd's own `configtest` catches syntax and per-directive errors. What
//! it cannot tell you, and what costs people evenings, is the shape of the
//! thing across files: an `oper` pointing at a `class` that no longer exists,
//! two `listen` blocks fighting over a port, a `class` nobody uses, cloak keys
//! still set to the words the example ships with.
//!
//! One rule governs all of it: **only ever compare files the server actually
//! loads together**. A conf directory holds files that are alternatives, not
//! companions — five translations of example.conf, an optional modules list, a
//! spamfilter.conf whose include is commented out. Cross-checking those against
//! each other invents duplicate-class and port-clash errors that do not exist,
//! and a checker that cries wolf gets turned off. The caller passes only the
//! reachable set; see `workspace::reachable`.
//!
//! Everything here is read-only.

use crate::cst::{Doc, Kind, Node};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// The server will refuse to start, or behave wrongly
    Error,
    /// Works, but is very probably not what was meant
    Warning,
    /// Tidy-up: unused things. Never counted in the badge.
    Note,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub level: Level,
    /// A short machine-ish name, for grouping and for dismissing: `dangling-class`
    pub kind: String,
    /// Written for a person, saying what and why
    pub message: String,
    pub file: String,
    pub line: usize,
    /// Byte offset of the thing at fault, so the interface can jump to it
    pub start: usize,
    /// Where the other half of the problem is, for duplicates and clashes
    pub other: Option<String>,
    /// Stable across edits, so a dismissal sticks
    pub id: String,
}

/// Security groups UnrealIRCd defines itself, which never need declaring.
/// From `src/securitygroup.c`.
pub const BUILTIN_GROUPS: &[&str] = &[
    "known-users", "unknown-users", "tls-users", "webirc-users", "websocket-users",
];

/// Directives whose value names a block of another kind.
/// (owning block, directive, the kind of block its value names). An empty
/// owner means the directive means the same thing wherever it appears.
///
/// `parent` needs its owner: an operclass inherits from an operclass, and an
/// rpc-class from an rpc-class (`parent read-only;`). Matching on the
/// directive name alone reported every rpc-class parent as a reference to a
/// missing operclass.
const REFERENCES: &[(&str, &str, &str)] = &[
    ("", "class", "class"),
    ("", "operclass", "operclass"),
    ("", "security-group", "security-group"),
    ("", "exclude-security-group", "security-group"),
    ("operclass", "parent", "operclass"),
    ("rpc-class", "parent", "rpc-class"),
];

/// Blocks named by their second word, which other blocks point at.
const NAMED_BLOCKS: &[&str] =
    &["class", "operclass", "security-group", "link", "oper", "rpc-class"];

/// One file, already parsed.
pub struct Source<'a> {
    pub rel: &'a str,
    pub doc: &'a Doc,
}

fn name_of(doc: &Doc, e: &Node) -> String {
    e.first(Kind::Name).map(|w| doc.word_text(w).to_string()).unwrap_or_default()
}
fn value_of(doc: &Doc, e: &Node) -> Option<String> {
    e.first(Kind::Value).map(|w| doc.word_text(w).to_string())
}

fn walk<'a>(n: &'a Node, f: &mut impl FnMut(&'a Node)) {
    if n.kind == Kind::Entry {
        f(n);
    }
    for c in &n.children {
        walk(c, f);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Definition {
    pub kind: String,
    pub name: String,
    pub file: String,
    pub line: usize,
    pub start: usize,
    pub uses: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub kind: String,
    pub name: String,
    pub file: String,
    pub line: usize,
    pub start: usize,
    /// The block doing the referring, e.g. `oper bobsmith`
    pub from: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
}

/// Values the shipped example.conf leaves for you to change. These are not
/// heuristics — upstream marks them `CHANGE THIS` in the file itself.
const PLACEHOLDERS: &[(&str, &str)] = &[
    ("and another one",
     "a cloak key is still the example text, so anyone could work out your users' real addresses"),
    ("$argon2id..etc..",
     "the oper password is still the example placeholder, so nobody can /OPER"),
    ("set.this.to.email.address",
     "set::kline-address still literally says to set it"),
    ("changemeplease",
     "a link password is still the example placeholder"),
    ("i.hate.microsefrs.com",
     "a vhost is still the example one"),
];

fn finding(level: Level, kind: &str, message: String, file: &str, line: usize, start: usize,
           other: Option<String>) -> Finding {
    Finding {
        id: format!("{kind}:{file}:{line}"),
        level,
        kind: kind.to_string(),
        message,
        file: file.to_string(),
        line,
        start,
        other,
    }
}

pub fn analyse(sources: &[Source]) -> Report {
    let mut report = Report::default();
    let mut defs: BTreeMap<(String, String), Definition> = BTreeMap::new();
    let mut refs: Vec<Reference> = Vec::new();
    // (ip, port, file, line, start) with ranges already expanded
    let mut listens: Vec<(String, String, String, usize, usize)> = Vec::new();
    let mut me_blocks: Vec<(String, usize, usize)> = Vec::new();
    let mut me_name: Option<String> = None;

    for src in sources {
        for top in src.doc.root.entries() {
            let kind = name_of(src.doc, top);
            let label = value_of(src.doc, top);

            if NAMED_BLOCKS.contains(&kind.as_str()) && top.block().is_some() {
                if let Some(name) = &label {
                    let key = (kind.clone(), name.clone());
                    let line = src.doc.line_of(top.span.start);
                    if let Some(prev) = defs.get(&key) {
                        // What a repeated name means differs per block, and
                        // most of them are not errors. Saying "error" where the
                        // server is perfectly happy is how a checker gets muted.
                        let (level, msg) = match kind.as_str() {
                            "class" => (Level::Note, format!(
                                "Two class blocks are both called “{name}”. UnrealIRCd merges them into one class, but it resets options and connfreq each time it reads one, so the second block's settings win for those two.")),
                            "oper" => (Level::Warning, format!(
                                "Two oper blocks are both called “{name}”. That is allowed, and the later one wins — so the first block's permissions never apply.")),
                            "security-group" => (Level::Note, format!(
                                "Two security-group blocks are both called “{name}”. They merge: the criteria add together rather than replacing each other.")),
                            _ => (Level::Warning, format!(
                                "Two {kind} blocks are both called “{name}”.")),
                        };
                        report.findings.push(finding(
                            level, &format!("duplicate-{kind}"), msg,
                            src.rel, line, top.span.start,
                            Some(format!("{}:{}", prev.file, prev.line))));
                    } else {
                        defs.insert(key, Definition {
                            kind: kind.clone(), name: name.clone(),
                            file: src.rel.to_string(), line, start: top.span.start, uses: 0,
                        });
                    }
                }
            }

            if kind == "me" {
                me_blocks.push((src.rel.to_string(), src.doc.line_of(top.span.start), top.span.start));
                if let Some(b) = top.block() {
                    for c in b.entries() {
                        if name_of(src.doc, c) == "name" {
                            me_name = value_of(src.doc, c);
                        }
                    }
                }
            }

            if kind == "listen" {
                let mut ip = "*".to_string();
                let mut port = String::new();
                if let Some(b) = top.block() {
                    for c in b.entries() {
                        match name_of(src.doc, c).as_str() {
                            "ip" => ip = value_of(src.doc, c).unwrap_or_else(|| "*".into()),
                            "port" => port = value_of(src.doc, c).unwrap_or_default(),
                            "file" => port = format!("unix:{}", value_of(src.doc, c).unwrap_or_default()),
                            _ => {}
                        }
                    }
                }
                if port.is_empty() {
                    if let Some(l) = &label {
                        if let Some((a, b)) = l.rsplit_once(':') {
                            ip = a.to_string();
                            port = b.to_string();
                        }
                    }
                }
                // `port 6667-6669` is three listeners, so compare each one.
                for p in expand_ports(&port) {
                    listens.push((ip.clone(), p, src.rel.to_string(),
                                  src.doc.line_of(top.span.start), top.span.start));
                }
            }
        }

        // References and placeholders, at any depth. Each remembers the
        // top-level block it sits in, so a finding can say where it is.
        let mut found: Vec<(&Node, String)> = Vec::new();
        for top in src.doc.root.entries() {
            let owner = match value_of(src.doc, top) {
                Some(l) => format!("{} {}", name_of(src.doc, top), l),
                None => name_of(src.doc, top),
            };
            walk(top, &mut |n| found.push((n, owner.clone())));
        }

        for (e, owner) in found {
            let n = name_of(src.doc, e);

            for (owns, dir, target) in REFERENCES {
                if n != *dir {
                    continue;
                }
                // An owner-specific entry only applies inside that block.
                if !owns.is_empty() && owner.split_whitespace().next() != Some(owns) {
                    continue;
                }
                // Three shapes share these names, and only two are references:
                //   class clients { .. }      a definition — name AND block
                //   class opers;              one reference — name, no block
                //   security-group { a; b; }  a list of references — block, no name
                let has_value = e.first(Kind::Value).is_some();
                let has_block = e.block().is_some();
                if has_value && has_block {
                    continue;
                }
                let mut names: Vec<(String, usize)> = Vec::new();
                if has_block {
                    for c in e.block().unwrap().entries() {
                        names.push((name_of(src.doc, c), c.span.start));
                    }
                } else if let Some(v) = value_of(src.doc, e) {
                    names.push((v, e.span.start));
                }
                for (name, at) in names {
                    if name.is_empty() {
                        continue;
                    }
                    refs.push(Reference {
                        kind: target.to_string(),
                        name,
                        file: src.rel.to_string(),
                        line: src.doc.line_of(at),
                        start: at,
                        from: owner.clone(),
                    });
                }
            }

            // A placeholder can be the value (`kline-address 'set.this...'`)
            // or the whole line (a bare cloak key).
            let bare = e.block().is_none() && e.first(Kind::Value).is_none();
            let candidate = if bare { Some(n.clone()) } else { value_of(src.doc, e) };
            if let Some(v) = candidate {
                for (needle, why) in PLACEHOLDERS {
                    if v == *needle {
                        report.findings.push(finding(
                            Level::Warning, "placeholder",
                            format!("{why} (“{needle}”)."),
                            src.rel, src.doc.line_of(e.span.start), e.span.start, None));
                    }
                }
            }
        }
    }

    // Two listeners fighting over a port. `conf_listen_configure` finds the
    // existing listener and resets all its settings (conf.c:5549), so the last
    // block silently wins with no message at all.
    let mut byport: BTreeMap<String, Vec<&(String, String, String, usize, usize)>> = BTreeMap::new();
    for l in &listens {
        byport.entry(l.1.clone()).or_default().push(l);
    }
    for (port, group) in byport {
        let mut reported: BTreeSet<usize> = BTreeSet::new();
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                if !ips_overlap(&group[i].0, &group[j].0) || reported.contains(&j) {
                    continue;
                }
                reported.insert(j);
                let a = &group[i];
                let b = &group[j];
                let both = if a.0 == b.0 {
                    format!("both take {}:{port}", a.0)
                } else {
                    format!("take {}:{port} and {}:{port}, which overlap", a.0, b.0)
                };
                report.findings.push(finding(
                    Level::Error, "port-clash",
                    format!("Two listen blocks {both}. UnrealIRCd does not complain — it reuses the first listener and resets its settings from the second, so whichever is read last silently wins."),
                    &b.2, b.3, b.4,
                    Some(format!("{}:{}", a.2, a.3))));
            }
        }
    }

    // A link block naming this server. UnrealIRCd does not check for this.
    if let Some(my_name) = &me_name {
        for d in defs.values().filter(|d| d.kind == "link") {
            if &d.name == my_name {
                report.findings.push(finding(
                    Level::Error, "link-to-self",
                    format!("There is a link block for “{my_name}”, which is this server's own name from me::name. A server cannot link to itself."),
                    &d.file, d.line, d.start, None));
            }
        }
    }

    for m in me_blocks.iter().skip(1) {
        report.findings.push(finding(
            Level::Error, "duplicate-me",
            "There is more than one me { } block. A server has one identity.".into(),
            &m.0, m.1, m.2,
            Some(format!("{}:{}", me_blocks[0].0, me_blocks[0].1))));
    }

    // Dangling references, and use counts.
    let known_groups: BTreeSet<&str> = BUILTIN_GROUPS.iter().copied().collect();
    for r in &refs {
        if let Some(d) = defs.get_mut(&(r.kind.clone(), r.name.clone())) {
            d.uses += 1;
            continue;
        }
        if r.kind == "security-group" && known_groups.contains(r.name.as_str()) {
            continue;
        }
        // The server does not stop for these. It prints a `config_status`
        // notice that scrolls past at startup and carries on with a fallback —
        // which is worse than an error, so say what actually happens rather
        // than the word "error".
        let who = if r.from.is_empty() { "This".to_string() } else { r.from.clone() };
        let msg = match r.kind.as_str() {
            "class" => format!(
                "{who} asks for a class called “{}”, and no such class block exists. UnrealIRCd will not stop — it quietly substitutes its built-in default class, which allows 100 clients with an 8000-byte sendq. Users hit “Max SendQ exceeded” and the server caps at 100, with nothing in the log but a startup notice.",
                r.name),
            "operclass" => format!(
                "{who} asks for an operclass called “{}”, and no such operclass block exists. This is not checked when the config is tested; the oper simply ends up with the default permissions.",
                r.name),
            _ => format!(
                "{who} points at a {} called “{}”, and no {} block by that name exists in any file loaded with this one.",
                r.kind, r.name, r.kind),
        };
        report.findings.push(finding(
            Level::Warning, &format!("dangling-{}", r.kind), msg,
            &r.file, r.line, r.start, None));
    }

    // Things nothing uses. Informational only — an unused class is not a fault,
    // and calling it one makes the tool sanctimonious about a working config.
    for d in defs.values() {
        if d.uses == 0 && matches!(d.kind.as_str(), "class" | "operclass" | "security-group") {
            report.findings.push(finding(
                Level::Note, &format!("unused-{}", d.kind),
                format!("Nothing refers to the {} “{}”. Harmless, but it may be left over.", d.kind, d.name),
                &d.file, d.line, d.start, None));
        }
    }

    report.findings.sort_by(|a, b| {
        let rank = |l: &Level| match l { Level::Error => 0, Level::Warning => 1, Level::Note => 2 };
        rank(&a.level).cmp(&rank(&b.level))
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });
    report.definitions = defs.into_values().collect();
    report.references = refs;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(files: &[(&str, &str)]) -> Report {
        let docs: Vec<(String, Doc)> = files.iter()
            .map(|(n, t)| (n.to_string(), Doc::parse(t.to_string()))).collect();
        let srcs: Vec<Source> = docs.iter().map(|(n, d)| Source { rel: n, doc: d }).collect();
        analyse(&srcs)
    }

    #[test]
    fn spots_a_class_that_does_not_exist() {
        let r = run(&[("unrealircd.conf",
            "class clients { pingfreq 90; }\noper bob { class opers; operclass netadmin; }\n")]);
        let d: Vec<&Finding> = r.findings.iter().filter(|f| f.kind == "dangling-class").collect();
        assert_eq!(d.len(), 1, "{:#?}", r.findings);
        assert!(d[0].message.contains("opers"));
        assert!(d[0].message.contains("oper bob"), "should name the block: {}", d[0].message);
    }

    #[test]
    fn a_definition_is_not_a_reference_to_itself() {
        let r = run(&[("a.conf", "class clients { pingfreq 90; }\n")]);
        assert!(r.references.is_empty(), "{:#?}", r.references);
    }

    #[test]
    fn follows_references_across_files() {
        let r = run(&[("classes.conf", "class opers { pingfreq 90; }\n"),
                      ("opers.conf", "oper bob { class opers; }\n")]);
        assert!(r.findings.iter().all(|f| f.kind != "dangling-class"), "{:#?}", r.findings);
        assert_eq!(r.definitions.iter().find(|d| d.name == "opers").unwrap().uses, 1);
    }

    #[test]
    fn builtin_security_groups_are_not_dangling() {
        let r = run(&[("c.conf",
            "set { anti-flood { known-users { nick-flood 3:60; } } }\noper a { mask { security-group known-users; } }\n")]);
        assert!(r.findings.iter().all(|f| !f.kind.starts_with("dangling")), "{:#?}", r.findings);
    }

    #[test]
    fn a_security_group_list_is_a_list_of_references() {
        let r = run(&[("c.conf",
            "security-group bots { account { X; } }\noper a { mask { security-group { bots; nope; } } }\n")]);
        let d: Vec<&Finding> = r.findings.iter()
            .filter(|f| f.kind == "dangling-security-group").collect();
        assert_eq!(d.len(), 1, "{:#?}", r.findings);
        assert!(d[0].message.contains("nope"));
    }

    #[test]
    fn notices_an_unused_class() {
        let r = run(&[("c.conf",
            "class nobody { pingfreq 90; }\nclass used { pingfreq 90; }\noper a { class used; }\n")]);
        let u: Vec<&Finding> = r.findings.iter().filter(|f| f.kind == "unused-class").collect();
        assert_eq!(u.len(), 1);
        assert!(u[0].message.contains("nobody"));
        assert_eq!(u[0].level, Level::Note, "an unused class is not an error");
    }

    #[test]
    fn catches_two_blocks_on_the_same_port() {
        let r = run(&[("a.conf", "listen { ip *; port 6697; }\n"),
                      ("b.conf", "listen { ip *; port 6697; }\n")]);
        let p: Vec<&Finding> = r.findings.iter().filter(|f| f.kind == "port-clash").collect();
        assert_eq!(p.len(), 1, "{:#?}", r.findings);
        assert!(p[0].message.contains("*:6697"));
    }

    #[test]
    fn a_wildcard_listener_overlaps_a_specific_one() {
        // `ip *` binds every address the machine has, so this really does clash.
        let r = run(&[("a.conf", "listen { ip *; port 6697; }\nlisten { ip 1.2.3.4; port 6697; }\n")]);
        let p: Vec<&Finding> = r.findings.iter().filter(|f| f.kind == "port-clash").collect();
        assert_eq!(p.len(), 1, "{:#?}", r.findings);
        assert!(p[0].message.contains("overlap"), "{}", p[0].message);
    }

    #[test]
    fn a_port_range_is_expanded_before_comparing() {
        let r = run(&[("a.conf", "listen { ip *; port 6667-6669; }\nlisten { ip *; port 6668; }\n")]);
        assert!(r.findings.iter().any(|f| f.kind == "port-clash"), "{:#?}", r.findings);
    }

    #[test]
    fn a_link_block_naming_this_server_is_caught() {
        let r = run(&[("a.conf",
            "me { name \"irc.example.org\"; }\nlink irc.example.org { class servers; }\n")]);
        assert!(r.findings.iter().any(|f| f.kind == "link-to-self"), "{:#?}", r.findings);
    }

    #[test]
    fn different_ips_on_the_same_port_are_fine() {
        let r = run(&[("a.conf",
            "listen { ip 1.2.3.4; port 6697; }\nlisten { ip 5.6.7.8; port 6697; }\n")]);
        assert!(r.findings.iter().all(|f| f.kind != "port-clash"), "{:#?}", r.findings);
    }

    #[test]
    fn two_classes_with_one_name_is_an_error() {
        let r = run(&[("a.conf", "class clients { pingfreq 90; }\nclass clients { pingfreq 60; }\n")]);
        assert!(r.findings.iter().any(|f| f.kind == "duplicate-class"), "{:#?}", r.findings);
    }

    #[test]
    fn finds_the_example_placeholders() {
        let r = run(&[("a.conf", "set {\n\tcloak-keys {\n\t\t\"and another one\";\n\t}\n}\n")]);
        let p: Vec<&Finding> = r.findings.iter().filter(|f| f.kind == "placeholder").collect();
        assert_eq!(p.len(), 1, "{:#?}", r.findings);
        assert!(p[0].message.contains("cloak key"), "{}", p[0].message);
    }

    #[test]
    fn operclass_parent_counts_as_a_reference() {
        let r = run(&[("a.conf",
            "operclass base { permissions { chat; } }\noperclass derived { parent base; permissions { chat; } }\noper a { operclass derived; }\n")]);
        assert!(r.findings.iter().all(|f| !f.kind.starts_with("dangling")), "{:#?}", r.findings);
        assert_eq!(r.definitions.iter().find(|d| d.name == "base").unwrap().uses, 1);
    }

    #[test]
    fn findings_have_stable_ids_so_dismissals_stick() {
        let a = run(&[("a.conf", "oper bob { class nope; }\n")]);
        let b = run(&[("a.conf", "oper bob { class nope; }\n")]);
        assert_eq!(a.findings[0].id, b.findings[0].id);
    }
}

#[cfg(test)]
mod real {
    use super::*;
    use std::path::Path;

    /// The honest test: run it over what UnrealIRCd ships and read what it
    /// says. A false positive here is worse than a missed finding.
    #[test]
    fn against_the_shipped_configs() {
        let root = Path::new(r"C:\Users\Valerie\unrealircd-pr\doc\conf");
        if !root.exists() {
            eprintln!("no conf tree, skipping");
            return;
        }
        let files = crate::workspace::discover(root);
        let texts: Vec<(String, String)> = files.iter()
            .filter_map(|f| std::fs::read_to_string(&f.path).ok().map(|t| (f.rel.clone(), t)))
            .collect();
        let known: Vec<String> = texts.iter().map(|(r, _)| r.clone()).collect();

        let raw = crate::workspace::include_map(&texts);
        let mut resolved: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (from, targets) in &raw {
            resolved.insert(from.clone(), targets.iter()
                .filter_map(|t| crate::workspace::resolve_include(from, t, &known))
                .collect());
        }
        // This tree ships examples/ rather than a live conf/unrealircd.conf.
        let start = crate::workspace::root_config(&known, &resolved)
            .unwrap_or_else(|| "examples/example.conf".to_string());
        let active = crate::workspace::reachable(&start, &resolved);

        let loaded: Vec<(String, Doc)> = texts.iter()
            .filter(|(r, _)| active.contains(r))
            .map(|(r, t)| (r.clone(), Doc::parse(t.clone())))
            .collect();
        let srcs: Vec<Source> = loaded.iter().map(|(n, d)| Source { rel: n, doc: d }).collect();
        let r = analyse(&srcs);

        let count = |l: Level| r.findings.iter().filter(|f| f.level == l).count();
        println!("\nroot config: {start}\nloaded together: {} of {} files on disk",
                 active.len(), known.len());
        println!("{} definitions · {} references", r.definitions.len(), r.references.len());
        println!("{} errors, {} warnings, {} notes",
                 count(Level::Error), count(Level::Warning), count(Level::Note));
        for f in r.findings.iter().take(20) {
            println!("  [{:?}] {}:{}  {}", f.level, f.file, f.line, f.message);
        }

        assert!(!r.definitions.is_empty(), "found no definitions at all");
        // The point of the reachable-set rule: the five translated example
        // configs must not be compared with each other.
        assert!(active.len() < known.len(),
                "every file was treated as loaded together — the include graph did nothing");
        assert!(count(Level::Error) == 0,
                "the shipped example should analyse clean, but:\n{:#?}",
                r.findings.iter().filter(|f| f.level == Level::Error).collect::<Vec<_>>());
    }
}

/// `6667` is one port; `6667-6669` is three. Anything else yields nothing.
fn expand_ports(spec: &str) -> Vec<String> {
    if let Some(p) = spec.strip_prefix("unix:") {
        return vec![format!("unix:{p}")];
    }
    if let Some((a, b)) = spec.split_once('-') {
        if let (Ok(lo), Ok(hi)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
            if lo <= hi && hi - lo < 400 {
                return (lo..=hi).map(|p| p.to_string()).collect();
            }
        }
    }
    if spec.is_empty() { Vec::new() } else { vec![spec.to_string()] }
}

/// Do two listen addresses cover any of the same ground? `*` binds every
/// address the machine has — both IPv4 and IPv6 (conf.c:5605) — so it overlaps
/// with everything.
fn ips_overlap(a: &str, b: &str) -> bool {
    a == b || a == "*" || b == "*"
}
