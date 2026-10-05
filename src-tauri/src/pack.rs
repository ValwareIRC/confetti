//! A docs pack: the directive tree scraped from one UnrealIRCd release, joined
//! to the wiki snapshot that shipped with it.
//!
//! The point of pinning both together is that hints then describe the server
//! you are actually configuring. The live wiki documents the newest release,
//! which is a different thing — it will happily explain directives your 6.1.x
//! has never heard of. The ZIM in a source tree is refreshed per release (the
//! commits say so: "Update doc/unrealircd_wiki.zim for 6.2.3") and carries a
//! `Date` you can check, so a pack can prove what it is.

use crate::docs::{self, Docs};
use crate::scrape;
use crate::zim::Zim;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Info {
    /// Where in the C source this directive is read, e.g. `src/conf.c:3833`.
    /// Shown when the wiki has nothing — still better than an empty panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The wiki's prose for this path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// How the server parses the value, taken from the `config_checkval` call
    /// that reads it. Authoritative where it is set: it is what conf.c does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::types::Kind>,
    /// The source hands this directive to `conf_match_block()`, so it holds
    /// matching criteria rather than a value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub matcher: bool,
    /// Words the source compares this value against. Suggestions, not a closed
    /// set: an if-chain read from C cannot be proved complete, and offering a
    /// short list as if it were exhaustive would refuse valid configuration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// Why the server refuses this directive, where it only exists so the
    /// server can tell you to stop writing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// A wiki heading that reads like a directive but never appears as one in
    /// the documentation's own examples. Its prose is still worth showing;
    /// it is not worth offering as something to add.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prose: bool,
    /// What the server uses when you don't write this at all, read from the
    /// source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// What the documentation says the default is, where the source did not
    /// say. Second best: the wiki is occasionally a release behind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_default: Option<String>,
}

/// What the inspector shows about one directive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hint {
    /// Full prose, if we have any
    pub text: Option<String>,
    /// A sentence's worth, for the panel
    pub summary: Option<String>,
    /// `src/conf.c:3833` — where the server reads this
    pub source: Option<String>,
    /// Set when the prose describes an enclosing block rather than this exact
    /// item, so the panel can say "about the group this is in".
    pub about: Option<String>,
    /// False means this release's source has no such directive.
    pub known: bool,
}

/// Bumped whenever the scraper or extractor changes shape, so a cached pack
/// built by an older Confetti is rebuilt rather than trusted. Without this a
/// scraper fix silently does nothing until the source tree happens to change.
pub const SCHEMA_VERSION: u32 = 10;

impl Pack {
    /// How this release parses the value of a directive, if `conf.c` said.
    pub fn kind_of(&self, path: &str) -> Option<crate::types::Kind> {
        self.directives.get(path).and_then(|i| i.kind)
    }

    /// Whether this release hands the directive to the matcher parser.
    pub fn is_matcher(&self, path: &str) -> bool {
        self.directives.get(path).map(|i| i.matcher).unwrap_or(false)
    }

    /// Words this release compares the value against, if it enumerates any.
    pub fn choices_for(&self, path: &str) -> Vec<String> {
        self.directives.get(path).map(|i| i.choices.clone()).unwrap_or_default()
    }

    /// Why this release refuses the directive, if it does.
    pub fn refused(&self, path: &str) -> Option<String> {
        self.directives.get(path).and_then(|i| i.refused.clone())
    }

    /// True for a wiki heading that only looks like a directive.
    pub fn is_prose(&self, path: &str) -> bool {
        self.directives.get(path).map(|i| i.prose).unwrap_or(false)
    }

    /// What this release uses when the directive is absent.
    pub fn default_of(&self, path: &str) -> Option<String> {
        self.directives.get(path).and_then(|i| i.default.clone())
    }

    /// What the documentation says it is, where the source did not say.
    pub fn doc_default_of(&self, path: &str) -> Option<String> {
        self.directives.get(path).and_then(|i| i.doc_default.clone())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pack {
    /// Which build of Confetti produced this pack
    #[serde(default)]
    pub schema: u32,
    /// e.g. `6.2.7-git`, read from configure.ac
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The ZIM's own `Date` metadata, e.g. `2026-03-06`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zim_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zim_name: Option<String>,
    /// Top-level block names, from `_ConfigCommands[]`
    pub blocks: Vec<String>,
    /// Everything we know, keyed by `block::item` path
    pub directives: BTreeMap<String, Info>,
}

/// `AC_INIT([unrealircd], [6.2.7-git], ...)`
pub fn detect_version(root: &Path) -> Option<String> {
    let ac = std::fs::read_to_string(root.join("configure.ac")).ok()?;
    let line = ac.lines().find(|l| l.trim_start().starts_with("AC_INIT"))?;
    let mut parts = line.split('[').skip(2);
    let v = parts.next()?;
    let end = v.find(']')?;
    Some(v[..end].to_string())
}

/// Look for the wiki archive near an install: in a source tree beside it, or
/// one directory up. `make install` does not copy the ZIM, so on a built-from-
/// source box it lives next to where the build happened.
pub fn find_zim(near: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut dir = Some(near.to_path_buf());
    for _ in 0..3 {
        let Some(d) = dir else { break };
        candidates.push(d.join("doc").join("unrealircd_wiki.zim"));
        candidates.push(d.join("unrealircd_wiki.zim"));
        // sibling source trees, e.g. /home/irc/unrealircd-6.2.3/doc/...
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                candidates.push(e.path().join("doc").join("unrealircd_wiki.zim"));
            }
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Articles worth mining. Everything else on the wiki is prose about running a
/// server rather than reference for a directive, and folding it in only adds
/// keys that mean nothing.
fn is_reference_article(url: &str) -> bool {
    const TRANSLATIONS: &[&str] = &[
        "/fr", "/es", "/nl", "/pt", "/tr", "/zh-tw", "/de", "/ru", "/it", "/ja", "/pl", "/zh",
    ];
    if TRANSLATIONS.iter().any(|t| url.ends_with(t)) {
        return false;
    }
    let base = url.strip_suffix("/en").unwrap_or(url);
    const ALLOW: &[&str] = &[
        "Configuration",
        "Anti-flood_settings",
        "Mask_item",
        "Authentication_types",
        "Special_users",
        "Channel_history",
        "Reputation",
    ];
    ALLOW.contains(&base) || base.ends_with("_block") || base.ends_with("_directive")
}

pub fn build(source_root: &Path, zim_path: Option<&Path>) -> Result<Pack, String> {
    let scraped = scrape::scrape(source_root)?;

    let mut docs_map = Docs::new();
    let mut block_text: BTreeMap<String, String> = BTreeMap::new();
    let mut zim_date = None;
    let mut zim_name = None;
    // Every name the documentation shows written in a config file, so a
    // heading can be told from a directive.
    let mut written: std::collections::BTreeSet<String> = Default::default();
    // And every path its config examples actually contain, which is the
    // stronger test: `name` is written in the wiki, but never inside an oper.
    let mut examples: std::collections::BTreeSet<String> = Default::default();
    let resolved = zim_path
        .map(|p| p.to_path_buf())
        .or_else(|| find_zim(source_root));
    if let Some(zp) = resolved {
        let z = Zim::open(&zp)?;
        zim_date = z.meta("Date");
        zim_name = z.meta("Name");
        let mut urls: Vec<String> = z.article_urls().into_iter().filter(|u| is_reference_article(u)).collect();
        urls.sort();
        for u in urls {
            if let Some(html) = z.article(&u) {
                written.extend(docs::written_names(&html));
                examples.extend(docs::example_paths(&html));
                docs::extract_into(&html, &mut docs_map, &mut block_text);
            }
        }
    }

    let mut directives: BTreeMap<String, Info> = BTreeMap::new();
    for d in &scraped.directives {
        // A heading for this exact path is best. Failing that, the wiki very
        // often explains the directive in a sentence inside its block's prose —
        // and we know the directive's name from the source, so we can go and
        // find that sentence rather than hope for a heading.
        let doc = docs_map.get(&d.path).cloned().or_else(|| {
            let (parent, name) = d.path.rsplit_once("::")?;
            let name = name.to_string();
            // Try the immediate parent's prose, then the block's.
            let block = parent.split("::").next().unwrap_or(parent);
            [parent, block]
                .into_iter()
                .filter_map(|k| block_text.get(k))
                .find_map(|text| docs::prose_for(text, &name))
        });
        let doc_default = doc.as_deref().and_then(docs::stated_default);
        directives.insert(
            d.path.clone(),
            Info {
                source: Some(d.source.clone()),
                doc,
                kind: d.kind,
                matcher: scraped.matchers.contains(&d.path),
                choices: scraped.choices.get(&d.path).cloned().unwrap_or_default(),
                refused: scraped.refused.get(&d.path).cloned(),
                prose: false,
                doc_default: doc_default,
                default: scraped.defaults.get(&d.path).cloned(),
            },
        );
    }
    // Paths the wiki documents that the scrape did not reach — usually
    // directives handled by shared helpers. Keep them; a hint with no source
    // line is still a hint.
    for (path, text) in &docs_map {
        let last = path.rsplit("::").next().unwrap_or(path);
        // Inside a block, the examples decide: a heading is a directive there
        // only if some example writes it there. At the top level there is no
        // enclosing block to check, so the weaker test is all there is.
        let prose = if path.contains("::") && !examples.is_empty() {
            !examples.contains(path)
        } else {
            !written.contains(last)
        };
        directives
            .entry(path.clone())
            .or_insert_with(|| Info {
                source: None,
                doc: Some(text.clone()),
                kind: None,
                matcher: false,
                choices: Vec::new(),
                refused: None,
                prose,
                doc_default: docs::stated_default(text),
                default: None,
            })
            .doc
            .get_or_insert_with(|| text.clone());
    }

    Ok(Pack {
        schema: SCHEMA_VERSION,
        version: detect_version(source_root),
        zim_date,
        zim_name,
        blocks: scraped.blocks,
        directives,
    })
}

impl Pack {
    pub fn load(path: &Path) -> Result<Pack, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Find what we know about a directive, trying the most specific key first.
    ///
    /// `label` is a block's second word, so `ban nick { }` can find the wiki's
    /// "Ban nick block" rather than settling for "ban".
    pub fn lookup(&self, path: &str, label: Option<&str>) -> Option<&Info> {
        self.candidate_keys(path, label)
            .into_iter()
            .filter_map(|k| self.directives.get(&k))
            .find(|i| i.doc.is_some() || i.source.is_some())
    }

    fn candidate_keys(&self, path: &str, label: Option<&str>) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        let first = path.split("::").next().unwrap_or(path);
        if let Some(l) = label {
            let l = l.to_ascii_lowercase();
            keys.push(format!("{first} {l}"));
            if let Some(rest) = path.strip_prefix(first) {
                if !rest.is_empty() {
                    keys.push(format!("{first} {l}{rest}"));
                }
            }
        }
        keys.push(path.to_string());

        // The tail: `handshake-data-flood::amount`, then `amount`.
        let segs: Vec<&str> = path.split("::").collect();
        for take in (1..segs.len()).rev() {
            keys.push(segs[segs.len() - take..].join("::"));
        }
        // Last resort: whatever this thing sits inside. Explaining the group is
        // more use than an empty panel, as long as we say that's what it is.
        for drop in 1..segs.len() {
            keys.push(segs[..segs.len() - drop].join("::"));
        }
        keys
    }

    /// What the hint panel should show for a directive.
    pub fn hint(&self, path: &str, label: Option<&str>) -> Hint {
        let mut out = Hint { known: self.knows(path), ..Default::default() };
        if let Some(i) = self.directives.get(path) {
            out.source = i.source.clone();
        }
        for key in self.candidate_keys(path, label) {
            let Some(info) = self.directives.get(&key) else { continue };
            if out.source.is_none() {
                out.source = info.source.clone();
            }
            if let Some(doc) = &info.doc {
                out.text = Some(doc.clone());
                out.summary = Some(docs::summarize(doc, 240));
                // Only say "about" when the prose is genuinely about something
                // broader than what was asked for.
                if key != path && !key.contains(' ') && path.starts_with(&key) && key.len() < path.len() {
                    out.about = Some(key.clone());
                }
                break;
            }
        }
        out
    }

    pub fn knows(&self, path: &str) -> bool {
        self.directives.contains_key(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r"C:\Users\Valerie\unrealircd-pr";

    fn built() -> Option<Pack> {
        let root = Path::new(SRC);
        if !root.exists() {
            eprintln!("no source tree at {SRC}, skipping");
            return None;
        }
        Some(build(root, None).expect("build pack"))
    }

    /// The list of what you may put inside a block is the pack's directives
    /// filtered by prefix, so anything wrongly in that map is offered as a
    /// setting. Two kinds were: a wiki heading that reads like a directive
    /// ("Copy-pasting" on the operclass page) and a name the server knows only
    /// to refuse (`privileges`).
    #[test]
    fn an_operclass_offers_only_what_you_can_write_in_one() {
        let Some(p) = built() else { return };
        let inside: Vec<&str> = p
            .directives
            .keys()
            .filter_map(|k| k.strip_prefix("operclass::"))
            .filter(|k| !k.contains("::"))
            .filter(|k| !p.is_prose(&format!("operclass::{k}")))
            .filter(|k| p.refused(&format!("operclass::{k}")).is_none())
            .map(|k| k)
            .collect();
        println!("inside an operclass: {}", inside.join(" "));
        assert!(inside.contains(&"parent"), "{inside:?}");
        assert!(inside.contains(&"permissions"), "{inside:?}");
        assert!(!inside.contains(&"copy-pasting"), "{inside:?}");
        assert!(!inside.contains(&"privileges"), "{inside:?}");
        assert_eq!(inside.len(), 2, "an operclass takes exactly two things: {inside:?}");
    }

    /// The other half: prose headings must not take real directives with them.
    /// These are documented but never reached by the scrape, so they survive
    /// only if the filter is evidence-based rather than "no source, no offer".
    #[test]
    fn documented_directives_the_scrape_missed_are_still_offered() {
        let Some(p) = built() else { return };
        for path in ["allow::maxperip", "allow::global-maxperip",
                     "except::ban::mask", "extjwt::key", "extjwt::secret",
                     "link::outgoing::bind-ip"] {
            if !p.directives.contains_key(path) {
                continue; // not in this release's wiki; nothing to protect
            }
            assert!(!p.is_prose(path), "{path} was dropped as prose");
        }
        // Headings that name the block's own label, not something inside it.
        // `oper <name> { }` and `link <server-name> { }` are how you write
        // them; neither name is a directive you put inside the braces.
        for path in ["link::server-name", "oper::name"] {
            assert!(p.is_prose(path), "{path} is the block's label, not a setting inside it");
        }
        // `deny channel { }` has no exemptions directive — conf.c has no such
        // branch. The wiki heading explains how to exempt people using an
        // except block, which is a different block entirely.
        assert!(p.is_prose("deny::channel::exemptions"), "not a directive in this release");
        let prose: Vec<&String> = p.directives.iter().filter(|(_, i)| i.prose).map(|(k, _)| k).collect();
        println!("prose headings, not directives: {}", prose.len());
        for k in prose.iter().take(40) {
            println!("  {k}");
        }
    }

    /// What an oper block offers you. `name` was in this list: the wiki gives
    /// the block's label its own heading, and a word search could not tell it
    /// from the `name` inside me { }. Writing `name bob;` inside an oper block
    /// is not a thing.
    #[test]
    fn an_oper_block_offers_only_what_goes_inside_the_braces() {
        let Some(p) = built() else { return };
        let inside: Vec<&str> = p
            .directives
            .keys()
            .filter_map(|k| k.strip_prefix("oper::"))
            .filter(|k| !k.contains("::"))
            .filter(|k| !p.is_prose(&format!("oper::{k}")))
            .filter(|k| p.refused(&format!("oper::{k}")).is_none())
            .collect();
        println!("inside an oper: {}", inside.join(" "));
        assert!(!inside.contains(&"name"), "the label is not a directive: {inside:?}");
        for want in ["class", "operclass", "password", "mask", "vhost", "swhois", "auto-login"] {
            assert!(inside.contains(&want), "{want} missing from {inside:?}");
        }
        // Every one that survives is either in the source or shown inside an
        // oper block by an example. Nothing gets in on a heading alone.
        for k in &inside {
            let path = format!("oper::{k}");
            assert!(p.directives[&path].source.is_some(),
                    "{path} is offered but nothing in the source or the examples puts it there");
        }
    }

    #[test]
    fn finds_the_zim_beside_the_source() {
        let root = Path::new(SRC);
        if !root.exists() {
            return;
        }
        let z = find_zim(root).expect("should find the shipped ZIM");
        assert!(z.ends_with("unrealircd_wiki.zim"), "{}", z.display());
    }

    #[test]
    fn reads_the_version() {
        let root = Path::new(SRC);
        if !root.exists() {
            return;
        }
        let v = detect_version(root).expect("version");
        assert!(v.starts_with('6'), "{v}");
    }

    #[test]
    fn joins_source_and_docs() {
        let Some(p) = built() else { return };

        let scraped_only = p.directives.values().filter(|i| i.source.is_some() && i.doc.is_none()).count();
        let both = p.directives.values().filter(|i| i.source.is_some() && i.doc.is_some()).count();
        let docs_only = p.directives.values().filter(|i| i.source.is_none() && i.doc.is_some()).count();

        println!(
            "\nversion {:?}, wiki dated {:?}\n{} directives total\n  {} in both source and docs\n  {} in the source only (hint falls back to the C line)\n  {} in the docs only (handled by a shared helper)",
            p.version, p.zim_date, p.directives.len(), both, scraped_only, docs_only
        );

        assert!(p.zim_date.is_some(), "should have read the ZIM date");
        assert!(both > 100, "only {both} directives matched source to docs");

        // The paths a person actually edits should all resolve to prose.
        for want in [
            "class::sendq",
            "class::pingfreq",
            "me::sid",
            "oper::operclass",
            "set::cloak-keys",
            "set::anti-flood",
        ] {
            let i = p.lookup(want, None).unwrap_or_else(|| panic!("no entry for {want}"));
            assert!(i.doc.is_some(), "{want} has no prose");
        }
    }

    #[test]
    fn lookup_falls_back_to_the_tail() {
        let Some(p) = built() else { return };
        // A deep path the wiki documents under a shorter heading still resolves.
        let i = p.lookup("set::anti-flood::everyone::handshake-data-flood::ban-time", None);
        assert!(i.is_some(), "deep path found nothing at all");
    }

    #[test]
    fn round_trips_through_json() {
        let Some(p) = built() else { return };
        let dir = std::env::temp_dir().join("confetti-pack-test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("pack.json");
        p.save(&f).expect("save");
        let back = Pack::load(&f).expect("load");
        assert_eq!(back.directives.len(), p.directives.len());
        assert_eq!(back.version, p.version);
        println!("pack.json is {} bytes", std::fs::metadata(&f).unwrap().len());
    }
}
