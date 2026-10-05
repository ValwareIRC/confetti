//! Finding every configuration file under a folder, and working out which of
//! them pulls in which.
//!
//! Point Confetti at `unrealircd/` or at `unrealircd/conf/` and it should find
//! the same set either way — so the root is normalised first, then walked.

use crate::cst::{Doc, Kind};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    /// Path relative to the root, always with forward slashes
    pub rel: String,
    /// The directory part of `rel`, empty at the top
    pub dir: String,
    pub name: String,
    pub bytes: usize,
}

/// Directories that are never worth walking into.
const SKIP: &[&str] = &[
    ".git", "node_modules", "target", "tmp", "logs", "cache", "modules", "src", "tls",
];

/// Folders that are never part of a config, wherever the config lives.
pub fn skipped(name: &str) -> bool {
    SKIP.contains(&name.to_lowercase().as_str())
}

/// Work out the sensible root for a path the user picked.
///
/// A file gives its own directory. A directory that *contains* a `conf/` is an
/// UnrealIRCd install, so the conf directory is the interesting part — but the
/// install root is kept as the base so `conf/` still shows in the tree.
pub fn normalise_root(picked: &Path) -> PathBuf {
    let dir = if picked.is_file() {
        picked.parent().unwrap_or(picked).to_path_buf()
    } else {
        picked.to_path_buf()
    };
    // Walk up at most two levels to find an install root, so that picking
    // `conf/aliases/` still shows you the whole conf tree.
    let mut candidate = dir.clone();
    for _ in 0..2 {
        if candidate.join("conf").is_dir() && candidate.join("conf").join("unrealircd.conf").is_file() {
            return candidate;
        }
        match candidate.parent() {
            Some(p) => candidate = p.to_path_buf(),
            None => break,
        }
    }
    dir
}

fn walk(dir: &Path, root: &Path, depth: usize, out: &mut Vec<Found>) {
    if depth > 4 || out.len() > 400 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if SKIP.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            walk(&p, root, depth + 1, out);
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("conf")).unwrap_or(false) {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            let (dir_part, name) = match rel.rsplit_once('/') {
                Some((d, n)) => (d.to_string(), n.to_string()),
                None => (String::new(), rel.clone()),
            };
            let bytes = std::fs::metadata(&p).map(|m| m.len() as usize).unwrap_or(0);
            out.push(Found { path: p, rel, dir: dir_part, name, bytes });
        }
    }
}

/// Every directory under the root, as rel paths. The tree groups files by
/// their directory, so a folder you have just made — with nothing in it yet —
/// would otherwise be invisible the moment you created it.
pub fn folders(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk_dirs(root, root, 0, &mut out);
    out.sort();
    out
}

fn walk_dirs(dir: &Path, root: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > 4 || out.len() > 200 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if SKIP.contains(&name.as_str()) || name.starts_with('.') {
            continue;
        }
        out.push(p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
        walk_dirs(&p, root, depth + 1, out);
    }
}

/// Every `.conf` under the root, shallowest first then alphabetical.
pub fn discover(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out.sort_by(|a, b| {
        let da = a.dir.matches('/').count() + if a.dir.is_empty() { 0 } else { 1 };
        let db = b.dir.matches('/').count() + if b.dir.is_empty() { 0 } else { 1 };
        da.cmp(&db).then(a.dir.cmp(&b.dir)).then(a.name.cmp(&b.name))
    });
    out
}

/// Which file includes which. Keys and values are `rel` paths where the target
/// could be resolved; unresolved targets (remote includes, wildcards) are
/// reported against the literal text so the tree can still show them.
pub fn include_map(files: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (rel, text) in files {
        let doc = Doc::parse(text.clone());
        let mut targets = Vec::new();
        for e in doc.root.entries() {
            let Some(name) = e.first(Kind::Name) else { continue };
            if doc.word_text(name) != "include" {
                continue;
            }
            if let Some(v) = e.first(Kind::Value) {
                targets.push(doc.word_text(v).to_string());
            }
        }
        if !targets.is_empty() {
            out.insert(rel.clone(), targets);
        }
    }
    out
}

/// Where `unrealircd.conf` lives, which is the only place include paths are
/// measured from. Empty when the folder opened *is* the config directory.
pub fn conf_dir(known: &[String]) -> String {
    // The shallowest one, in case an examples/ copy is also lying around.
    let Some(main) = known
        .iter()
        .filter(|k| k.rsplit('/').next() == Some("unrealircd.conf"))
        .min_by_key(|k| k.matches('/').count())
    else {
        return String::new();
    };
    main.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default()
}

/// Resolve an include target.
///
/// Not against the file that declared it — against the config directory. The
/// server does this in one line: `convert_to_absolute_path(&ce->value,
/// CONFDIR)`, so where the including file happens to sit makes no difference
/// at all. Measuring from the including file meant `include
/// "aliases/aliases.conf";` inside conf/aliases/anope.conf was looked for at
/// conf/aliases/aliases/aliases.conf, and reported as unfollowable.
pub fn resolve_include(conf_dir: &str, target: &str, known: &[String]) -> Option<String> {
    if target.starts_with("http://") || target.starts_with("https://") || target.contains('*') {
        return None;
    }
    let clean = target.trim_start_matches("./").replace("/./", "/");
    let mut tries = Vec::new();
    if !conf_dir.is_empty() {
        tries.push(format!("{conf_dir}/{clean}"));
    }
    // As written, from the top of the workspace. Covers the folder you opened
    // being the config directory itself, and the `include "conf/x.conf"` style
    // that some installs get away with.
    tries.push(clean.clone());
    for t in tries {
        if let Some(hit) = known.iter().find(|k| **k == t) {
            return Some(hit.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r"C:\Users\Valerie\unrealircd-pr\doc";

    /// Include paths are measured from the config directory, never from the
    /// file doing the including. `conf/aliases/anope.conf` says
    /// `include "aliases/aliases.conf";` and means `conf/aliases/aliases.conf`
    /// — not `conf/aliases/aliases/aliases.conf`, which is where Confetti used
    /// to look before reporting that it could not follow it.
    #[test]
    fn includes_are_measured_from_the_config_directory() {
        let known: Vec<String> = [
            "conf/unrealircd.conf",
            "conf/badwords.conf",
            "conf/aliases/anope.conf",
            "conf/aliases/aliases.conf",
            "conf/help/help.conf",
        ].iter().map(|s| s.to_string()).collect();
        let cdir = conf_dir(&known);
        assert_eq!(cdir, "conf");

        // The case from a real config, from a file two levels down.
        assert_eq!(resolve_include(&cdir, "aliases/aliases.conf", &known).as_deref(),
                   Some("conf/aliases/aliases.conf"));
        // And the same target named from the top. Same answer: where the
        // including file sits is not part of the question.
        assert_eq!(resolve_include(&cdir, "aliases/aliases.conf", &known).as_deref(),
                   Some("conf/aliases/aliases.conf"));
        assert_eq!(resolve_include(&cdir, "badwords.conf", &known).as_deref(),
                   Some("conf/badwords.conf"));
        assert_eq!(resolve_include(&cdir, "./help/help.conf", &known).as_deref(),
                   Some("conf/help/help.conf"));
        // A sibling named without its folder is not found, because the server
        // would not find it either.
        assert_eq!(resolve_include(&cdir, "aliases.conf", &known), None);
        // Nor is something that simply is not there.
        assert_eq!(resolve_include(&cdir, "nope.conf", &known), None);
        // URLs and wildcards are for the server to deal with.
        assert_eq!(resolve_include(&cdir, "https://example.org/x.conf", &known), None);
        assert_eq!(resolve_include(&cdir, "conf/*.conf", &known), None);
    }

    /// Opening the config directory itself is the other common way round.
    #[test]
    fn the_folder_opened_can_be_the_config_directory() {
        let known: Vec<String> = ["unrealircd.conf", "aliases/anope.conf", "aliases/aliases.conf"]
            .iter().map(|s| s.to_string()).collect();
        let cdir = conf_dir(&known);
        assert_eq!(cdir, "");
        assert_eq!(resolve_include(&cdir, "aliases/aliases.conf", &known).as_deref(),
                   Some("aliases/aliases.conf"));
    }

    /// With no unrealircd.conf to go on, everything is measured from the top.
    #[test]
    fn with_no_main_file_the_top_of_the_folder_is_used() {
        let known: Vec<String> = ["a/one.conf", "a/two.conf"].iter().map(|s| s.to_string()).collect();
        assert_eq!(conf_dir(&known), "");
        assert_eq!(resolve_include("", "a/two.conf", &known).as_deref(), Some("a/two.conf"));
        assert_eq!(resolve_include("", "two.conf", &known), None);
    }

    #[test]
    fn finds_the_shipped_conf_tree() {
        let root = Path::new(SRC);
        if !root.exists() {
            eprintln!("no doc tree, skipping");
            return;
        }
        let files = discover(root);
        println!("\n{} .conf files found:", files.len());
        for f in files.iter().take(14) {
            println!("  {:<44} {:>7} bytes", f.rel, f.bytes);
        }
        assert!(files.len() > 20, "only found {}", files.len());
        assert!(files.iter().any(|f| f.name == "unrealircd.conf" || f.name == "example.conf"));
        // Subdirectories must be included, not just the top level.
        assert!(files.iter().any(|f| f.dir.contains("aliases")), "no aliases/ files");
        assert!(files.iter().any(|f| f.dir.contains("help")), "no help/ files");
    }

    #[test]
    fn include_targets_are_read_and_resolved() {
        let text = r#"
include "modules.default.conf";
include "help/help.conf";
include "https://example.com/remote.conf";
me { sid 001; }
"#;
        let map = include_map(&[("conf/unrealircd.conf".into(), text.into())]);
        let targets = map.get("conf/unrealircd.conf").expect("targets");
        assert_eq!(targets.len(), 3);

        let known = vec![
            "conf/unrealircd.conf".to_string(),
            "conf/modules.default.conf".to_string(),
            "conf/help/help.conf".to_string(),
        ];
        // Resolution takes the config directory, not the including file. It
        // used to take the file, which is what made an include written from a
        // subfolder unfollowable.
        let cdir = conf_dir(&known);
        assert_eq!(
            resolve_include(&cdir, "modules.default.conf", &known).as_deref(),
            Some("conf/modules.default.conf")
        );
        assert_eq!(
            resolve_include(&cdir, "help/help.conf", &known).as_deref(),
            Some("conf/help/help.conf")
        );
        // A remote include has no local file to point at.
        assert_eq!(
            resolve_include(&cdir, "https://example.com/remote.conf", &known),
            None
        );
    }

    #[test]
    fn picking_a_subfolder_still_finds_the_install() {
        let install = Path::new(r"C:\Users\Valerie\unrealircd-pr\doc");
        if !install.join("conf").join("unrealircd.conf").is_file() {
            // The source tree ships examples/, not a live conf/unrealircd.conf.
            eprintln!("no live install layout here, skipping");
            return;
        }
        let from_sub = normalise_root(&install.join("conf").join("aliases"));
        assert_eq!(from_sub, install);
    }
}

/// Every file actually loaded when the server reads `start` — that file plus
/// everything it includes, transitively.
///
/// This matters more than it looks. A conf directory contains files that are
/// never loaded together: five translations of example.conf, an optional
/// modules list, spamfilter.conf that ships commented out of the include list.
/// Cross-checking those against each other invents duplicate-class and
/// port-clash errors that do not exist, and a checker that cries wolf is one
/// people turn off.
pub fn reachable(start: &str, includes: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue = vec![start.to_string()];
    while let Some(f) = queue.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        if let Some(targets) = includes.get(&f) {
            for t in targets {
                if !seen.contains(t) {
                    queue.push(t.clone());
                }
            }
        }
    }
    seen
}

/// Which file the server would be started with. Conventionally
/// `conf/unrealircd.conf`; failing that, whichever pulls in the most.
pub fn root_config(files: &[String], includes: &BTreeMap<String, Vec<String>>) -> Option<String> {
    if let Some(f) = files.iter().find(|f| f.ends_with("unrealircd.conf")) {
        return Some(f.clone());
    }
    files
        .iter()
        .max_by_key(|f| includes.get(*f).map(|v| v.len()).unwrap_or(0))
        .filter(|f| includes.get(*f).map(|v| !v.is_empty()).unwrap_or(false))
        .cloned()
}
