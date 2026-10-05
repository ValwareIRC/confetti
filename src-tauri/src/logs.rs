//! Reading UnrealIRCd's JSON log, and working out where the things in it
//! belong in the configuration.
//!
//! The server writes one JSON object per line. The shape is set in
//! `src/log.c` (timestamp, level, subsystem, event_id, log_source, msg) plus
//! whatever the event attached — most usefully a `client` object built by
//! `json_expand_client` in `src/json.c`, and a `channel` object.
//!
//! What this module cares about is the handful of *facts* buried in each
//! entry: an address, a hostmask, an account name, a channel. Those are the
//! things you want to act on after reading a line of log, and they are exactly
//! the things the configuration's matchers take.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A log file the configuration says the server writes to.
#[derive(Serialize, Clone, Debug)]
pub struct LogFile {
    /// As written in the config
    pub name: String,
    /// Absolute, or remote-absolute
    pub path: String,
    pub exists: bool,
    pub bytes: u64,
    /// Which config file declared it, for the tooltip
    pub declared_in: String,
    /// The block said `type json`. A log { } can write both a human-readable
    /// file and a JSON one, and only the JSON one has anything to drag out of.
    pub json: bool,
}

/// One thing in a log line that could go into the config.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Fact {
    /// `ip`, `mask`, `host`, `nick`, `account`, `channel`, `realname`,
    /// `certfp`, `country`, `asn`, `security-group`, `operclass`, `server`
    pub kind: String,
    pub value: String,
    /// Where in the entry it came from, e.g. `client.user.account`
    pub from: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    /// 1-based line in the file
    pub line: usize,
    pub timestamp: String,
    pub level: String,
    pub subsystem: String,
    pub event_id: String,
    pub msg: String,
    pub facts: Vec<Fact>,
    /// The original line, for "copy it raw"
    pub raw: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct LogRead {
    pub path: String,
    pub entries: Vec<Entry>,
    /// Lines we could not parse as JSON — an old plain-text log, most likely
    pub unparsed: usize,
    /// True when we showed only the end of the file
    pub truncated: bool,
    pub total_lines: usize,
    /// Every distinct subsystem seen, for the filter
    pub subsystems: Vec<String>,
}

/// Pull the `file "..."` names out of every `log { destination { ... } }`.
///
/// The config knows where its own log goes, so there is no reason to make
/// anybody type the path in.
pub fn declared_files(entries: &[crate::UiEntry], declared_in: &str, out: &mut Vec<(String, String, bool)>) {
    for e in entries {
        if e.name == "log" {
            collect_destinations(&e.children, declared_in, out);
        }
        if !e.children.is_empty() {
            declared_files(&e.children, declared_in, out);
        }
    }
}

fn collect_destinations(entries: &[crate::UiEntry], declared_in: &str, out: &mut Vec<(String, String, bool)>) {
    for e in entries {
        if e.name == "destination" {
            for d in &e.children {
                if d.name == "file" {
                    // `file "ircd.log" { maxsize 100M; }` — the name is the
                    // label when there is a block, the value when there is not.
                    let name = d.label.clone().or_else(|| d.value.clone());
                    let json = d
                        .children
                        .iter()
                        .any(|t| t.name == "type" && t.value.as_deref() == Some("json"));
                    if let Some(n) = name {
                        if !n.is_empty() {
                            out.push((n, declared_in.to_string(), json));
                        }
                    }
                }
            }
        }
        if !e.children.is_empty() {
            collect_destinations(&e.children, declared_in, out);
        }
    }
}

/// Parse a whole log, newest last. `limit` caps how many lines we keep, taken
/// from the end — a busy server's log is hundreds of megabytes and nobody
/// wants to scroll from the beginning of time.
pub fn parse(path: &str, text: &str, limit: usize) -> LogRead {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let truncated = total > limit;
    let start = total.saturating_sub(limit);

    let mut entries = Vec::new();
    let mut unparsed = 0;
    let mut subsystems: Vec<String> = Vec::new();

    for (i, raw) in lines.iter().enumerate().skip(start) {
        let raw = raw.trim_end_matches('\r');
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            unparsed += 1;
            continue;
        };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let subsystem = s("subsystem");
        if !subsystem.is_empty() && !subsystems.contains(&subsystem) {
            subsystems.push(subsystem.clone());
        }
        let mut facts = Vec::new();
        harvest(&v, "", &mut facts);
        // The same thing usually turns up more than once — a channel is both
        // `channel.name` and an entry in `client.user.channels`. One chip.
        // Vec::dedup would only catch them when they happen to be adjacent.
        let mut seen: std::collections::BTreeSet<(String, String)> = Default::default();
        facts.retain(|f| seen.insert((f.kind.clone(), f.value.clone())));
        entries.push(Entry {
            line: i + 1,
            timestamp: s("timestamp"),
            level: s("level"),
            subsystem,
            event_id: s("event_id"),
            msg: s("msg"),
            facts,
            raw: raw.to_string(),
        });
    }

    subsystems.sort();
    LogRead { path: path.to_string(), entries, unparsed, truncated, total_lines: total, subsystems }
}

/// Which JSON keys hold something worth acting on. Walking the tree by key
/// name rather than by a fixed path means `target`, `rehash_client` and
/// whatever a module attaches all work without being special-cased.
fn kind_for(key: &str, path: &str) -> Option<&'static str> {
    match key {
        "ip" => Some("ip"),
        "hostname" => Some("host"),
        "details" => Some("mask"),
        "account" => Some("account"),
        "realname" => Some("realname"),
        "certfp" | "fingerprint" => Some("certfp"),
        "operclass" => Some("operclass"),
        "asn" => Some("asn"),
        "country_code" | "country" => Some("country"),
        // `name` means different things depending on where it sits.
        "name" => {
            if path.contains("channel") {
                Some("channel")
            } else if path.contains("client") || path.contains("target") {
                Some("nick")
            } else {
                None
            }
        }
        _ => None,
    }
}

fn harvest(v: &Value, path: &str, out: &mut Vec<Fact>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let child = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                if let Some(s) = val.as_str() {
                    if !s.is_empty() {
                        if let Some(kind) = kind_for(k, &child) {
                            out.push(Fact { kind: kind.into(), value: s.to_string(), from: child.clone() });
                        }
                    }
                } else if k == "security-groups" {
                    if let Some(arr) = val.as_array() {
                        for g in arr.iter().filter_map(|x| x.as_str()) {
                            out.push(Fact {
                                kind: "security-group".into(),
                                value: g.to_string(),
                                from: child.clone(),
                            });
                        }
                    }
                }
                if val.is_object() || val.is_array() {
                    harvest(val, &child, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                harvest(item, path, out);
            }
        }
        _ => {}
    }
}

/* ---------------- where a fact belongs ---------------- */

/// What a matcher calls this kind of fact. These are the criterion names from
/// `types::CRITERIA`, so a fact dropped on a `mask { }` becomes a real
/// criterion rather than a line the server will refuse to start on.
pub fn criterion_for(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "ip" => "ip",
        "mask" | "host" => "mask",
        "account" => "account",
        "channel" => "channel",
        "realname" => "realname",
        "certfp" => "certfp",
        "country" => "country",
        "asn" => "asn",
        "security-group" => "security-group",
        _ => return None,
    })
}

/// What a plain (non-matcher) block calls it. Keyed on the block's own name,
/// which is how the config is actually organised.
///
/// Deliberately short. Offering a destination that merely parses is worse than
/// offering none: it produces a config that starts and does nothing.
pub fn directive_for(block: &str, label: &str, kind: &str) -> Option<&'static str> {
    match block {
        // `allow { ip; mask; }` lets people connect. `allow dcc { filename; }`
        // and `allow channel { channel; }` are entirely different blocks that
        // happen to start with the same word, and an address means nothing in
        // either — keying on the name alone writes a line the server ignores.
        "allow" => match (label, kind) {
            ("", "ip") => Some("ip"),
            ("", "mask") | ("", "host") => Some("mask"),
            ("channel", "channel") => Some("channel"),
            _ => None,
        },
        "deny" => match (label, kind) {
            ("channel", "channel") => Some("channel"),
            ("link", "host") | ("link", "mask") | ("link", "ip") => Some("mask"),
            _ => None,
        },
        // The label says what is being banned, so it has to agree with the
        // fact: an address is not a nick.
        "ban" => match (label, kind) {
            ("ip", "ip") => Some("mask"),
            ("user", "mask") | ("user", "host") => Some("mask"),
            ("nick", "nick") => Some("mask"),
            ("realname", "realname") => Some("mask"),
            ("server", "host") => Some("mask"),
            _ => None,
        },
        "except" => match kind {
            "ip" | "mask" | "host" => Some("mask"),
            _ => None,
        },
        "oper" | "vhost" => match kind {
            "mask" | "host" | "ip" => Some("mask"),
            "account" if block == "vhost" => Some("login"),
            _ => None,
        },
        _ => None,
    }
}

/// A one-word name for a kind, as the chips show it.
pub fn label_for(kind: &str) -> &'static str {
    match kind {
        "ip" => "address",
        "mask" => "hostmask",
        "host" => "hostname",
        "nick" => "nick",
        "account" => "account",
        "channel" => "channel",
        "realname" => "real name",
        "certfp" => "certificate",
        "country" => "country",
        "asn" => "network",
        "security-group" => "group",
        "operclass" => "operclass",
        _ => "value",
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DropPlan {
    pub file: String,
    /// How the destination reads, e.g. `ban ip`
    pub block: String,
    pub directive: String,
    pub value: String,
    pub as_block: bool,
    /// Byte offset of the block the line goes inside
    pub parent_start: usize,
    /// The line exactly as it will be written, indentation included
    pub preview: String,
    /// Set when the same thing is already there
    pub already: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r##"{"timestamp":"2026-09-06T10:00:00.000Z","level":"info","subsystem":"connect","event_id":"LOCAL_CLIENT_CONNECT","log_source":"irc.example.org","msg":"Client connecting","client":{"name":"bob","id":"001AAAAAB","hostname":"host.example.net","ip":"192.0.2.7","details":"bob!bobby@host.example.net","user":{"username":"bobby","realname":"Bob","account":"bobaccount","security-groups":["known-users","tls-users"],"channels":[{"name":"#lobby"}]}},"channel":{"name":"#lobby"}}"##;

    fn facts_of(line: &str) -> Vec<Fact> {
        let r = parse("x.log", line, 100);
        r.entries[0].facts.clone()
    }

    fn has(facts: &[Fact], kind: &str, value: &str) -> bool {
        facts.iter().any(|f| f.kind == kind && f.value == value)
    }

    #[test]
    fn pulls_the_useful_things_out_of_a_connect_line() {
        let f = facts_of(LINE);
        assert!(has(&f, "ip", "192.0.2.7"));
        assert!(has(&f, "host", "host.example.net"));
        assert!(has(&f, "mask", "bob!bobby@host.example.net"));
        assert!(has(&f, "nick", "bob"));
        assert!(has(&f, "account", "bobaccount"));
        assert!(has(&f, "realname", "Bob"));
        assert!(has(&f, "channel", "#lobby"));
        assert!(has(&f, "security-group", "known-users"));
        assert!(has(&f, "security-group", "tls-users"));
    }

    #[test]
    fn a_name_means_a_channel_or_a_nick_depending_on_where_it_sits() {
        let f = facts_of(LINE);
        // client.name is a nick; channel.name is a channel. Both are "name".
        assert!(has(&f, "nick", "bob"));
        assert!(has(&f, "channel", "#lobby"));
        assert!(!has(&f, "channel", "bob"));
        assert!(!has(&f, "nick", "#lobby"));
    }

    #[test]
    fn the_header_fields_are_read() {
        let r = parse("x.log", LINE, 100);
        let e = &r.entries[0];
        assert_eq!(e.level, "info");
        assert_eq!(e.subsystem, "connect");
        assert_eq!(e.event_id, "LOCAL_CLIENT_CONNECT");
        assert_eq!(e.msg, "Client connecting");
        assert_eq!(e.line, 1);
        assert_eq!(r.subsystems, vec!["connect"]);
    }

    #[test]
    fn a_plain_text_log_is_counted_not_crashed_on() {
        let r = parse("x.log", "[2026-01-01] not json at all\nnor this\n", 100);
        assert!(r.entries.is_empty());
        assert_eq!(r.unparsed, 2);
    }

    #[test]
    fn only_the_end_of_a_long_log_is_kept_and_it_says_so() {
        let many = (0..50)
            .map(|i| format!(r##"{{"msg":"line {i}","subsystem":"connect"}}"##))
            .collect::<Vec<_>>()
            .join("\n");
        let r = parse("x.log", &many, 10);
        assert!(r.truncated);
        assert_eq!(r.total_lines, 50);
        assert_eq!(r.entries.len(), 10);
        // The last ten, and the line numbers still refer to the real file.
        assert_eq!(r.entries[0].msg, "line 40");
        assert_eq!(r.entries[0].line, 41);
        assert_eq!(r.entries[9].msg, "line 49");
    }

    #[test]
    fn the_same_fact_twice_in_one_entry_is_one_chip() {
        // client.user.channels[0].name and channel.name are both #lobby.
        let f = facts_of(LINE);
        let n = f.iter().filter(|x| x.kind == "channel" && x.value == "#lobby").count();
        assert_eq!(n, 1, "expected the duplicate channel to be folded away");
    }

    #[test]
    fn empty_values_are_not_offered() {
        let line = r##"{"msg":"x","client":{"name":"bob","ip":"","details":""}}"##;
        let f = facts_of(line);
        assert!(!f.iter().any(|x| x.value.is_empty()));
    }

    #[test]
    fn facts_map_onto_real_criteria() {
        for kind in ["ip", "mask", "host", "account", "channel", "realname", "certfp", "country", "asn"] {
            let c = criterion_for(kind).unwrap_or_else(|| panic!("no criterion for {kind}"));
            assert!(
                crate::types::criterion(c).is_some(),
                "{kind} maps to {c}, which is not in the criterion vocabulary"
            );
        }
    }

    #[test]
    fn a_block_label_decides_what_may_go_in_it() {
        // The connection allow takes an address; allow dcc is a different
        // block that merely shares a word with it.
        assert_eq!(directive_for("allow", "", "ip"), Some("ip"));
        assert_eq!(directive_for("allow", "dcc", "ip"), None);
        assert_eq!(directive_for("allow", "channel", "channel"), Some("channel"));
        // And a ban has to agree with what it says it bans.
        assert_eq!(directive_for("ban", "ip", "ip"), Some("mask"));
        assert_eq!(directive_for("ban", "nick", "ip"), None);
        assert_eq!(directive_for("deny", "dcc", "ip"), None);
        assert_eq!(directive_for("deny", "link", "host"), Some("mask"));
    }

    #[test]
    fn a_nick_is_not_droppable_onto_a_matcher() {
        // There is no "nick" criterion — matching a nick is what a ban does.
        assert!(criterion_for("nick").is_none());
        assert_eq!(directive_for("ban", "nick", "nick"), Some("mask"));
    }
}
