//! Confetti — a cozy editor for unrealircd.conf
//!
//! The backend holds one open document at a time. Every edit goes through the
//! lossless CST, so bytes we did not touch are never rewritten.

pub mod analyse;
pub mod blocks;
pub mod cst;
pub mod docs;
pub mod history;
pub mod logs;
pub mod pack;
pub mod remote;
pub mod scrape;
pub mod types;
pub mod workspace;
pub mod zim;

use cst::{Doc, Kind, Node, Severity};
use pack::Pack;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Serialize, Clone)]
pub struct UiEntry {
    /// Directive name as written, e.g. `anti-flood`
    pub name: String,
    /// The optional second word, e.g. `bobsmith` in `oper bobsmith {`
    pub label: Option<String>,
    /// The value of a leaf directive, quotes stripped
    pub value: Option<String>,
    pub quoted: bool,
    /// Has neither a second word nor a block: a flag, or one line of a list.
    pub bare: bool,
    /// Byte range of the whole entry
    pub span: [usize; 2],
    /// Byte range of the value token, for editing in place
    pub value_span: Option<[usize; 2]>,
    pub line: usize,
    pub depth: usize,
    pub has_block: bool,
    /// Documentation-style address, e.g. `set::anti-flood::everyone`
    pub path: String,
    pub children: Vec<UiEntry>,
    /// A short summary shown when the tray is folded shut
    pub summary: Option<String>,
    /// A sentence from the wiki explaining this directive
    pub hint: Option<String>,
    /// `src/conf.c:3833` — where the server reads it, when there is no prose
    pub hint_source: Option<String>,
    /// Set when the prose is about an enclosing block, not this exact item
    pub hint_about: Option<String>,
    /// True when a pack is loaded and has never heard of this directive
    pub unknown: bool,
    /// What sort of value this holds
    pub kind: types::Kind,
    /// This entry is a `mask { }` or similar matcher
    pub matcher: bool,
    /// When this sits inside a matcher, which criterion it is
    pub criterion: Option<types::Criterion>,
    /// Byte range of the name token, so a criterion can be changed
    pub name_span: Option<[usize; 2]>,
    /// Words the source compares this value against. Suggestions, not a closed
    /// set — you may still type something else.
    pub choices: Vec<String>,
    /// Why the server refuses this directive, if it does. Set on something
    /// already written in the config, so it can say what to do instead.
    pub refused: Option<String>,
    /// What the server would use if this line were not here.
    pub default: Option<String>,
}

/// Which release's documentation is loaded, and where it came from.
#[derive(Serialize, Clone, Default)]
pub struct PackStatus {
    pub loaded: bool,
    pub version: Option<String>,
    pub zim_date: Option<String>,
    pub directives: usize,
    pub with_prose: usize,
    pub source_root: Option<String>,
    /// Plain-English note for the status bar
    pub note: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Settings {
    source_root: Option<String>,
    /// Per-folder preferences — the categories you defined, where you put
    /// things on the workbench. Keyed by workspace root. The shape is the
    /// interface's business; this side just keeps it.
    #[serde(default)]
    workspaces: std::collections::BTreeMap<String, serde_json::Value>,
    /// Servers you have connected to. Never holds a password: authentication
    /// is left to your ssh agent and ~/.ssh/config.
    #[serde(default)]
    hosts: Vec<remote::Host>,
    /// Folders opened before, most recent first.
    #[serde(default)]
    recents: Vec<String>,
}

fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("confetti").join("settings.json"))
}

/// Read the settings, and keep a copy of anything unreadable.
///
/// Falling back to defaults here means the next save writes those defaults
/// over the top: your categories, what you had filed where, your servers, all
/// of it gone because of one stray byte. A UTF-8 BOM alone was enough, since
/// serde will not parse past one. So strip the BOM, and if it still will not
/// parse, move the file aside rather than quietly replace it.
fn load_settings() -> Settings {
    let Some(path) = settings_path() else { return Settings::default() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Settings::default() };
    let clean = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if clean.trim().is_empty() {
        return Settings::default();
    }
    match serde_json::from_str(clean) {
        Ok(s) => s,
        Err(e) => {
            let kept = path.with_extension(format!("unreadable-{}", std::process::id()));
            let _ = std::fs::copy(&path, &kept);
            eprintln!(
                "confetti: {} did not parse ({e}); a copy is at {}",
                path.display(),
                kept.display()
            );
            Settings::default()
        }
    }
}

fn save_settings(s: &Settings) {
    let Some(p) = settings_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(p, text);
    }
}

/// Look for an UnrealIRCd source tree near an opened config: the usual layout
/// is a build directory beside the install, so walking up a few levels and
/// glancing at the siblings finds it more often than not.
fn find_source_root(from: &Path) -> Option<PathBuf> {
    let mut dir = Some(from.to_path_buf());
    for _ in 0..5 {
        let d = dir?;
        if d.join("src").join("conf.c").is_file() {
            return Some(d);
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                if e.path().join("src").join("conf.c").is_file() {
                    return Some(e.path());
                }
            }
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

#[derive(Serialize, Clone)]
pub struct UiDiag {
    pub severity: String,
    pub line: usize,
    pub message: String,
}

#[derive(Serialize, Clone)]
pub struct OpenedDoc {
    pub path: String,
    pub text: String,
    pub entries: Vec<UiEntry>,
    pub diagnostics: Vec<UiDiag>,
    pub dirty: bool,
}

/// One file we have read and may have edited.
struct OpenFile {
    text: String,
    on_disk: String,
    /// What the file looked like on disk when Confetti last read or wrote it.
    /// Compared against a fresh stat to notice somebody else editing it,
    /// without reading every file every few seconds.
    stamp: Option<Stamp>,
}

/// Cheap evidence that a file has or hasn't moved.
#[derive(Clone, PartialEq)]
struct Stamp {
    len: u64,
    mtime: Option<std::time::SystemTime>,
}

fn stamp_of(path: &str) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp { len: m.len(), mtime: m.modified().ok() })
}

/// A file as the tree and the list need it.
#[derive(Serialize, Clone)]
pub struct WsFile {
    pub path: String,
    /// Relative to the workspace root, forward slashes
    pub rel: String,
    /// Directory part of `rel`, empty at the top level
    pub dir: String,
    pub name: String,
    pub blocks: usize,
    pub bytes: usize,
    pub dirty: bool,
    /// Files this one pulls in, as rel paths we could resolve
    pub includes: Vec<String>,
    /// Include targets we could not resolve — remote URLs, wildcards
    pub includes_unresolved: Vec<String>,
    /// True when some other file includes this one
    pub included: bool,
    pub text: String,
    pub entries: Vec<UiEntry>,
    pub diagnostics: Vec<UiDiag>,
}

#[derive(Serialize, Clone)]
pub struct Workspace {
    pub root: String,
    pub files: Vec<WsFile>,
    pub pack: PackStatus,
    /// The server these files came from, if they came from one. The interface
    /// needs to know: `configtest` and `rehash` are meaningless on a local copy.
    pub remote: Option<String>,
    /// Every directory, empty ones included, so a folder you just made shows up.
    pub folders: Vec<String>,
}

/// One tab that is not the one you are looking at.
///
/// Everything that makes a tab what it is gets parked here when you switch
/// away and put back when you switch to it. Nothing is re-read from disk on
/// the way back, because the point of a tab is that the edits you had not
/// saved are still waiting for you.
#[derive(Default)]
struct Parked {
    id: u32,
    root: Option<PathBuf>,
    files: std::collections::BTreeMap<String, OpenFile>,
    pack: Option<Pack>,
    pack_root: Option<String>,
    history: history::History,
    source: Option<remote::Host>,
}

/// A tab as the bar along the top needs it.
#[derive(Serialize, Clone)]
pub struct TabInfo {
    pub id: u32,
    /// The instance's short name — the server, or the folder it lives in
    pub title: String,
    /// Where it actually is, for the tooltip
    pub subtitle: String,
    pub dirty: usize,
    pub remote: bool,
    pub active: bool,
    /// Nothing opened in it yet
    pub empty: bool,
}

#[derive(Default)]
pub struct AppState {
    root: Mutex<Option<PathBuf>>,
    /// Every config file under the root, keyed by absolute path
    files: Mutex<std::collections::BTreeMap<String, OpenFile>>,
    /// A path given on the command line, opened once when the window loads.
    pending: Mutex<Option<String>>,
    /// The documentation pack for the release being configured.
    pack: Mutex<Option<Pack>>,
    pack_root: Mutex<Option<String>>,
    /// Undo and redo, at workspace level so a multi-file change unwinds whole
    history: Mutex<history::History>,
    /// Set when the open config lives on another machine
    source: Mutex<Option<remote::Host>>,
    /// Every tab except the one in the fields above
    parked: Mutex<Vec<Parked>>,
    /// Tab ids, left to right along the bar. Always contains `active`.
    order: Mutex<Vec<u32>>,
    active: Mutex<u32>,
    next_id: Mutex<u32>,
}

impl AppState {
    fn status(&self) -> PackStatus {
        let guard = self.pack.lock().unwrap();
        let root = self.pack_root.lock().unwrap().clone();
        match guard.as_ref() {
            None => PackStatus {
                note: "No documentation loaded — point Confetti at your UnrealIRCd folder.".into(),
                ..Default::default()
            },
            Some(p) => {
                let with_prose = p.directives.values().filter(|i| i.doc.is_some()).count();
                let note = match (&p.version, &p.zim_date) {
                    (Some(v), Some(d)) => format!("Docs {v} · wiki {d}"),
                    (Some(v), None) => format!("Docs {v} · no wiki archive"),
                    _ => "Documentation loaded".to_string(),
                };
                PackStatus {
                    loaded: true,
                    version: p.version.clone(),
                    zim_date: p.zim_date.clone(),
                    directives: p.directives.len(),
                    with_prose,
                    source_root: root,
                    note,
                }
            }
        }
    }

    /// Build the pack for a source tree, or reuse a cached one. Scraping 259
    /// C files and 357 wiki articles takes a moment; doing it once per release
    /// and caching keeps startup instant afterwards.
    fn load_pack(&self, root: &Path) -> Result<PackStatus, String> {
        let version = pack::detect_version(root);
        let cache = version.as_ref().and_then(|v| {
            settings_path().map(|s| {
                s.with_file_name("packs")
                    .join(format!("{}.json", v.replace(['/', '\\', ':'], "_")))
            })
        });

        let conf_c = root.join("src").join("conf.c");
        let fresh = |c: &Path| -> bool {
            let (Ok(cm), Ok(sm)) = (std::fs::metadata(c), std::fs::metadata(&conf_c)) else {
                return false;
            };
            match (cm.modified(), sm.modified()) {
                (Ok(a), Ok(b)) => a >= b,
                _ => false,
            }
        };

        let built = match cache.as_ref().filter(|c| c.is_file() && fresh(c)).map(|c| Pack::load(c)) {
            // A cache from an older Confetti is not to be trusted: a scraper
            // fix would otherwise sit there doing nothing until the source
            // tree happened to change.
            Some(Ok(p)) if p.schema == pack::SCHEMA_VERSION => p,
            _ => {
                let p = pack::build(root, None)?;
                if let Some(c) = &cache {
                    if let Some(dir) = c.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = p.save(c);
                }
                p
            }
        };

        *self.pack.lock().unwrap() = Some(built);
        *self.pack_root.lock().unwrap() = Some(root.to_string_lossy().to_string());
        Ok(self.status())
    }

    fn with_pack<T>(&self, f: impl FnOnce(Option<&Pack>) -> T) -> T {
        let g = self.pack.lock().unwrap();
        f(g.as_ref())
    }
}

fn word(doc: &Doc, n: &Node, kind: Kind) -> Option<String> {
    n.first(kind).map(|w| doc.word_text(w).to_string())
}

fn build(doc: &Doc, n: &Node, depth: usize, parent_path: &str, pk: Option<&Pack>, in_matcher: bool) -> UiEntry {
    let name = word(doc, n, Kind::Name).unwrap_or_default();
    let label = word(doc, n, Kind::Value);
    let block = n.block();

    let path = if parent_path.is_empty() {
        name.clone()
    } else {
        format!("{parent_path}::{name}")
    };

    // A second word reads as a label when the entry opens a block
    // (`oper bobsmith {`) and as a value when it does not (`sendq 200k;`).
    // That is a difference in how it is described, not in what it is: either
    // way it is one word you may want to change, so it always gets a span.
    //
    // The log block leans on this. `file "ircd.log" { maxsize 100M; }` names a
    // file and then says things about that file — a value *and* a block on one
    // directive. Treating the second word as a label with nowhere to click
    // meant the name of the file could not be edited at all, and the middle
    // panel showed "1 setting" where the file name should have been.
    //
    // An entry with neither is "bare": either a flag (`tls;`) or a line in a
    // list (`"Bob Smith";`, `services.example.org;`). Both are edited by
    // changing the one word they have, so the name token is what we expose.
    let bare = block.is_none() && label.is_none();
    let second = n.first(Kind::Value).map(|w| [w.span.start, w.span.end]);
    let (label, value, value_span) = if block.is_some() {
        (label, None, second)
    } else if bare {
        let span = n.first(Kind::Name).map(|w| [w.span.start, w.span.end]);
        (None, None, span)
    } else {
        let span = n.first(Kind::Value).map(|w| [w.span.start, w.span.end]);
        (None, label, span)
    };

    // `mask { }` and friends hold matching criteria, not free text, so their
    // children are rendered from a known vocabulary rather than as plain rows.
    // What the source does, then the rules of thumb for anything it did not
    // reach. A hand-written list of matcher paths is exactly how `match` and
    // half the module `except` blocks went unrecognised.
    let matcher = pk.map(|p| p.is_matcher(&path)).unwrap_or(false) || types::is_matcher(&path);
    let criterion = if in_matcher { types::criterion(&name).cloned() } else { None };
    // What the server does with the value beats what the name looks like:
    // `auto-login` is a boolean because conf.c reads it with CFG_YESNO, not
    // because anybody recognised the word.
    let kind = pk
        .and_then(|p| p.kind_of(&path))
        .unwrap_or_else(|| types::kind_of(&path));
    let name_span = n.first(Kind::Name).map(|w| [w.span.start, w.span.end]);
    let choices = pk.map(|p| p.choices_for(&path)).unwrap_or_default();
    let refused = pk.and_then(|p| p.refused(&path));
    let default = pk.and_then(|p| p.default_of(&path));

    let children: Vec<UiEntry> = block
        .map(|b| b.entries().map(|c| build(doc, c, depth + 1, &path, pk, matcher)).collect())
        .unwrap_or_default();

    let summary = block.map(|_| {
        let n = children.len();
        match n {
            0 => "empty".to_string(),
            1 => "1 setting".to_string(),
            _ => format!("{n} settings"),
        }
    });

    // A quoted bare entry is a line in a list ("Bob Smith";) — its text is
    // content, not a directive name, so ask about the block it lives in.
    let name_quoted = n.first(Kind::Name).and_then(|w| w.quote).is_some();
    let hint_path = if bare && name_quoted && !parent_path.is_empty() { parent_path } else { &path };
    let h = pk.map(|p| p.hint(hint_path, label.as_deref()));
    // Only flag a top-level block as unknown. The scrape does not reach every
    // nested directive (some are handled by shared helpers), so calling those
    // "unknown" would be crying wolf — but the block list comes straight from
    // `_ConfigCommands[]` and is complete.
    let unknown = depth == 1 && pk.map(|p| !p.blocks.contains(&name)).unwrap_or(false);

    UiEntry {
        quoted: n.first(Kind::Value).and_then(|w| w.quote).is_some(),
        hint: h.as_ref().and_then(|x| x.summary.clone()),
        hint_source: h.as_ref().and_then(|x| x.source.clone()),
        hint_about: h.as_ref().and_then(|x| x.about.clone()),
        unknown,
        kind,
        matcher,
        criterion,
        name_span,
        choices,
        refused,
        default,
        bare,
        name,
        label,
        value,
        span: [n.span.start, n.span.end],
        value_span,
        line: doc.line_of(n.span.start),
        depth,
        has_block: block.is_some(),
        path,
        children,
        summary,
    }
}

fn view(path: &str, text: &str, on_disk: &str, pk: Option<&Pack>) -> OpenedDoc {
    let doc = Doc::parse(text.to_string());
    let entries = doc.root.entries().map(|e| build(&doc, e, 1, "", pk, false)).collect();
    let diagnostics = doc
        .diagnostics
        .iter()
        .map(|d| UiDiag {
            severity: match d.severity {
                Severity::Error => "error".into(),
                Severity::Warning => "warning".into(),
            },
            line: d.line,
            message: d.message.clone(),
        })
        .collect();
    OpenedDoc {
        path: path.to_string(),
        text: text.to_string(),
        entries,
        diagnostics,
        dirty: text != on_disk,
    }
}


/* ------------------------------------------------------------------ *
 * Workspace: every .conf under one root, opened together.
 * ------------------------------------------------------------------ */

fn build_file(
    found: &workspace::Found,
    text: &str,
    on_disk: &str,
    includes: Vec<String>,
    includes_unresolved: Vec<String>,
    included: bool,
    pk: Option<&Pack>,
) -> WsFile {
    let v = view(&found.path.to_string_lossy(), text, on_disk, pk);
    WsFile {
        path: found.path.to_string_lossy().to_string(),
        rel: found.rel.clone(),
        dir: found.dir.clone(),
        name: found.name.clone(),
        blocks: v.entries.len(),
        bytes: text.len(),
        dirty: text != on_disk,
        includes,
        includes_unresolved,
        included,
        text: v.text,
        entries: v.entries,
        diagnostics: v.diagnostics,
    }
}

fn read_workspace(state: &AppState, picked: &Path) -> Result<Workspace, String> {
    let root = workspace::normalise_root(picked);
    let found = workspace::discover(&root);
    if found.is_empty() {
        return Err(format!(
            "No .conf files under {} — is that the right folder?",
            root.display()
        ));
    }

    // Anything not yet saved, so re-reading the folder does not throw it away.
    //
    // This runs after undo, after a file is created or renamed, and after a
    // few other things — all of which replaced every open file with whatever
    // was on disk. Undo restored your text and then this discarded it one
    // line later, which is why Ctrl+Z appeared to mean "revert to disk".
    let unsaved: std::collections::BTreeMap<String, String> = {
        let open = state.files.lock().unwrap();
        open.iter()
            .filter(|(_, of)| of.text != of.on_disk)
            .map(|(k, of)| (k.clone(), of.text.clone()))
            .collect()
    };

    // Read everything once, then work out the include graph from the texts.
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut store = std::collections::BTreeMap::new();
    for f in &found {
        let Ok(on_disk) = std::fs::read_to_string(&f.path) else { continue };
        let key = f.path.to_string_lossy().to_string();
        let text = unsaved.get(&key).cloned().unwrap_or_else(|| on_disk.clone());
        // The include graph follows what you are working on, not what is on
        // disk — an include line you have just added should count.
        texts.push((f.rel.clone(), text.clone()));
        store.insert(key.clone(), OpenFile { stamp: stamp_of(&key), on_disk, text });
    }

    let known: Vec<String> = found.iter().map(|f| f.rel.clone()).collect();
    let cdir = workspace::conf_dir(&known);
    let raw = workspace::include_map(&texts);
    let mut included_set: std::collections::BTreeSet<String> = Default::default();
    let mut resolved: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> = Default::default();
    for (from, targets) in &raw {
        let mut ok = Vec::new();
        let mut miss = Vec::new();
        for t in targets {
            match workspace::resolve_include(&cdir, t, &known) {
                Some(r) => {
                    included_set.insert(r.clone());
                    ok.push(r);
                }
                None => miss.push(t.clone()),
            }
        }
        resolved.insert(from.clone(), (ok, miss));
    }

    // Documentation: the folder we were pointed at may itself be a source
    // tree, or one may be sitting beside it — the usual layout is a build
    // directory next to the install.
    if state.pack.lock().unwrap().is_none() {
        if let Some(src) = find_source_root(&root) {
            let _ = state.load_pack(&src);
        }
    }

    let files: Vec<WsFile> = state.with_pack(|pk| {
        found
            .iter()
            .filter_map(|f| {
                let key = f.path.to_string_lossy().to_string();
                let of = store.get(&key)?;
                let (inc, miss) = resolved.get(&f.rel).cloned().unwrap_or_default();
                Some(build_file(f, &of.text, &of.on_disk, inc, miss, included_set.contains(&f.rel), pk))
            })
            .collect()
    });

    *state.root.lock().unwrap() = Some(root.clone());
    *state.files.lock().unwrap() = store;
    // A local folder: anything previously opened over ssh is no longer in play.
    *state.source.lock().unwrap() = None;
    remember_folder(&root);

    Ok(Workspace {
        root: root.to_string_lossy().to_string(),
        files,
        pack: state.status(),
        remote: None,
        folders: workspace::folders(&root),
    })
}

/// Re-render one file after an edit, without resending the whole workspace.
fn refresh_file(state: &AppState, path: &str) -> Result<WsFile, String> {
    let root = state.root.lock().unwrap().clone().ok_or("No folder is open.")?;
    let files = state.files.lock().unwrap();
    let of = files.get(path).ok_or("That file isn't open.")?;
    let p = PathBuf::from(path);
    let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
    let (dir, name) = match rel.rsplit_once('/') {
        Some((d, n)) => (d.to_string(), n.to_string()),
        None => (String::new(), rel.clone()),
    };
    let found = workspace::Found { path: p.clone(), rel, dir, name, bytes: of.text.len() };
    let text = of.text.clone();
    let on_disk = of.on_disk.clone();
    drop(files);
    Ok(state.with_pack(|pk| build_file(&found, &text, &on_disk, vec![], vec![], false, pk)))
}

#[tauri::command]
fn open_workspace(state: State<'_, AppState>, path: String) -> Result<Workspace, String> {
    read_workspace(&state, Path::new(&path))
}

#[tauri::command]
fn pick_folder(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Option<Workspace>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Where is UnrealIRCd? Pick the install folder, or its conf/ folder")
        .blocking_pick_folder();
    let Some(fp) = picked else { return Ok(None) };
    let dir = fp.into_path().map_err(|e| e.to_string())?;
    read_workspace(&state, &dir).map(Some)
}

/// A path on the command line opens straight away.
#[tauri::command]
fn startup_workspace(state: State<'_, AppState>) -> Result<Option<Workspace>, String> {
    let pending = { state.pending.lock().unwrap().take() };
    match pending {
        Some(p) => read_workspace(&state, Path::new(&p)).map(Some),
        None => {
            // Started from the Start menu or a double-click: reopen whatever
            // you had open last. The docs folder used to be the fallback, but
            // that is an UnrealIRCd *source tree* — the place hints are read
            // from, not a configuration — so it opened the wrong thing.
            let s = load_settings();
            let last = s
                .recents
                .iter()
                .find(|p| Path::new(p).is_dir())
                .cloned()
                .or_else(|| s.source_root.filter(|r| Path::new(r).is_dir()));
            match last {
                Some(r) => read_workspace(&state, Path::new(&r)).map(Some),
                None => Ok(None),
            }
        }
    }
}

/// Remember what these files looked like, so the change can be taken back.
/// Called before every mutation, with a label that reads as a sentence in the
/// undo button: "Undo commenting out oper::password".
fn remember(state: &AppState, label: impl Into<String>, paths: &[String]) {
    let captured: Vec<(String, String)> = {
        let files = state.files.lock().unwrap();
        paths.iter()
            .filter_map(|p| files.get(p).map(|f| (p.clone(), f.text.clone())))
            .collect()
    };
    state.history.lock().unwrap().record(label, captured);
}

fn edit_file(
    state: &AppState,
    path: &str,
    label: &str,
    f: impl FnOnce(&str, &mut Doc) -> Result<(), String>,
) -> Result<WsFile, String> {
    writable(path)?;
    remember(state, label, std::slice::from_ref(&path.to_string()));
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(path).ok_or("That file isn't open.")?;
        let mut doc = Doc::parse(of.text.clone());
        f(&of.text, &mut doc)?;
        of.text = doc.render();
    }
    refresh_file(state, path)
}

/// Replace a file's whole text in one go, remembering the old one.
fn replace_text(state: &AppState, path: &str, label: &str, text: String) -> Result<WsFile, String> {
    writable(path)?;
    remember(state, label, std::slice::from_ref(&path.to_string()));
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(path).ok_or("That file isn't open.")?;
        of.text = text;
    }
    refresh_file(state, path)
}

/// Replace one value in place. Only those bytes change.
#[tauri::command]
fn set_value(
    state: State<'_, AppState>,
    file: String,
    start: usize,
    end: usize,
    new_text: String,
) -> Result<WsFile, String> {
edit_file(&state, &file, &format!("changing {}", name_hint(&state, &file, start)), |text, doc| {
        if end > text.len() || start > end {
            return Err("That edit is outside the file.".into());
        }
        // Keep the original quoting style if there was any.
        let old = &text[start..end];
        let replacement = match old.as_bytes().first() {
            Some(&q @ (b'"' | b'\'')) => {
                let q = q as char;
                format!("{q}{}{q}", new_text.replace(q, &format!("\\{q}")))
            }
            _ if new_text.is_empty()
                || new_text.contains(|c: char| c.is_whitespace() || c == ';') =>
            {
                format!("\"{}\"", new_text.replace('"', "\\\""))
            }
            _ => new_text,
        };
        doc.edit(start..end, replacement)?;
        Ok(())
    })
}

/// Where new lines go inside a block, and what they wear.
///
/// Returns the range to replace, the indent each new line takes, and the text
/// to put either side of them.
///
/// The awkward case is a block written on one line. `mask { }` — which is
/// exactly what Confetti writes when you add a matcher — has its closing brace
/// on the same line as its opening one, so "insert at the start of the closing
/// brace's line" put the new directive *above the whole block*:
///
///     oper lol {
///             account { }
///         mask { }
///     }
///
/// A one-liner is opened out into a real block instead, keeping whatever was
/// already between the braces.
fn block_insertion(
    text: &str,
    parent: &Node,
) -> Result<(std::ops::Range<usize>, String, String, String), String> {
    let block = parent.block().ok_or("That entry has no block to add to.")?;
    let lb = block
        .children
        .iter()
        .find(|c| c.kind == Kind::LBrace)
        .ok_or("That block isn't opened.")?;
    let rb = block
        .children
        .iter()
        .rfind(|c| c.kind == Kind::RBrace)
        .ok_or("That block isn't closed.")?;

    if text[lb.span.end..rb.span.start].contains('\n') {
        let indent = match block.entries().last() {
            Some(c) => indent_at(text, c.span.start),
            None => format!("{}\t", indent_at(text, rb.span.start)),
        };
        let at = line_start(text, rb.span.start);
        return Ok((at..at, indent, String::new(), String::new()));
    }

    // One line. Open it out.
    let outer = indent_at(text, parent.span.start);
    let inner_indent = format!("{outer}\t");
    let kept = text[lb.span.end..rb.span.start].trim();
    let lead = if kept.is_empty() {
        "\n".to_string()
    } else {
        format!("\n{inner_indent}{kept}\n")
    };
    Ok((lb.span.end..rb.span.start, inner_indent, lead, outer))
}

/// Give a directive a value it does not have yet: `account;` -> `account bob;`.
///
/// A criterion added from the matcher panel was written bare, with no value
/// and so no value span, and the inspector had nothing to offer you a field
/// on. There was no way to say what an `ip` or an `account` should match.
#[tauri::command]
fn set_bare_value(
    state: State<'_, AppState>,
    file: String,
    start: usize,
    value: String,
) -> Result<WsFile, String> {
    let v = value.trim().to_string();
    if v.is_empty() {
        return Err("Nothing to set it to.".into());
    }
    edit_file(&state, &file, &format!("setting {}", name_hint(&state, &file, start)), |_text, doc| {
        let at = {
            let e = find_entry(&doc.root, start).ok_or("Couldn't find that line.")?;
            e.first(Kind::Name).ok_or("That line has no name to put a value after.")?.span.end
        };
        doc.edit(at..at, format!(" {}", quote_if_needed(&v)))?;
        Ok(())
    })
}

/// Add a directive inside an existing block, matching the indentation already
/// used in there rather than imposing our own.
#[tauri::command]
fn add_child(
    state: State<'_, AppState>,
    file: String,
    parent_start: usize,
    name: String,
    value: String,
    as_block: bool,
) -> Result<WsFile, String> {
edit_file(&state, &file, &format!("adding {name}"), |text, doc| {
        let (range, indent, lead, tail) = {
            let parent = find_entry(&doc.root, parent_start).ok_or("Couldn't find that block.")?;
            block_insertion(text, parent)?
        };
        let line = if as_block {
            format!("{} {{ }}", quote_if_needed(&name))
        } else if value.trim().is_empty() {
            // A line with no value is either a bare flag (`tls;`) or an item
            // in a list (`"Bob Smith";`). Either way the name is the content,
            // so it has to survive being written out.
            format!("{};", quote_if_needed(&name))
        } else {
            format!("{} {};", quote_if_needed(&name), value)
        };
        doc.edit(range, format!("{lead}{indent}{line}\n{tail}"))?;
        Ok(())
    })
}

/// Add a whole block at the end of a file.
#[tauri::command]
fn add_block(
    state: State<'_, AppState>,
    file: String,
    name: String,
    label: String,
) -> Result<WsFile, String> {
edit_file(&state, &file, &format!("adding a {name} block"), |text, doc| {
        let at = text.len();
        let head = if label.trim().is_empty() {
            name.clone()
        } else {
            format!("{name} {}", label.trim())
        };
        let lead = if text.ends_with('\n') || text.is_empty() { "" } else { "\n" };
        doc.edit(at..at, format!("{lead}\n{head} {{\n}}\n"))?;
        Ok(())
    })
}

/// Take a directive out by commenting it, not deleting it. Reversible, and it
/// shows up plainly in the diff.
#[tauri::command]
fn comment_out(state: State<'_, AppState>, file: String, start: usize) -> Result<WsFile, String> {
edit_file(&state, &file, "commenting a directive out", |text, doc| {
        let span = {
            let e = find_entry(&doc.root, start).ok_or("Couldn't find that line.")?;
            e.span.clone()
        };
        let from = line_start(text, span.start);
        let mut at = from;
        let mut marks = Vec::new();
        while at < span.end {
            let indent_len = text[at..]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .map(|c| c.len_utf8())
                .sum::<usize>();
            marks.push(at + indent_len);
            at = match text[at..].find('\n') {
                Some(i) => at + i + 1,
                None => break,
            };
        }
        for pos in marks {
            doc.edit(pos..pos, "//".to_string())?;
        }
        Ok(())
    })
}

#[tauri::command]
fn save_file(state: State<'_, AppState>, file: String) -> Result<WsFile, String> {
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(&file).ok_or("That file isn't open.")?;
        if of.text != of.on_disk {
            let remote_host = state.source.lock().unwrap().clone();
            match remote_host {
                // On another machine: write over ssh, keeping a `~` copy and
                // swapping the new file in atomically, so a dropped connection
                // cannot leave half a config on a running server.
                Some(host) => {
                    let base = host.root.trim_end_matches('/');
                    let rel = file.strip_prefix(base).unwrap_or(&file).trim_start_matches('/');
                    remote::write_file(&host, rel, &of.text)?;
                }
                None => {
                    let backup = format!("{file}~");
                    if Path::new(&file).exists() {
                        let _ = std::fs::copy(&file, &backup);
                    }
                    std::fs::write(&file, &of.text).map_err(|e| format!("Could not save: {e}"))?;
                    of.stamp = stamp_of(&file);
                }
            }
            of.on_disk = of.text.clone();
        }
    }
    refresh_file(&state, &file)
}

#[tauri::command]
fn save_all(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let dirty: Vec<String> = {
        let files = state.files.lock().unwrap();
        files.iter().filter(|(_, of)| of.text != of.on_disk).map(|(k, _)| k.clone()).collect()
    };
    for f in &dirty {
        save_file(state.clone(), f.clone())?;
    }
    Ok(dirty)
}

/* ------------------------------------------------------------------ *
 * Files: make, copy, rename, remove. Every one of these refuses to
 * touch anything outside the folder that is open.
 * ------------------------------------------------------------------ */

fn root_of(state: &AppState) -> Result<PathBuf, String> {
    state.root.lock().unwrap().clone().ok_or_else(|| "No folder is open.".into())
}

fn check_inside(root: &Path, p: &Path) -> Result<(), String> {
    // A file being created doesn't exist yet, so check the parent in that case.
    let target = if p.exists() { p.to_path_buf() } else { p.parent().unwrap_or(p).to_path_buf() };
    let a = target.canonicalize().map_err(|e| format!("{}: {e}", target.display()))?;
    let b = root.canonicalize().map_err(|e| format!("{}: {e}", root.display()))?;
    if a.starts_with(&b) {
        Ok(())
    } else {
        Err("That path is outside the folder Confetti has open.".into())
    }
}

fn with_conf_ext(name: &str) -> String {
    let n = name.trim();
    if n.to_lowercase().ends_with(".conf") { n.to_string() } else { format!("{n}.conf") }
}

fn reload(state: &AppState) -> Result<Workspace, String> {
    let root = root_of(state)?;
    read_workspace(state, &root)
}

#[tauri::command]
fn new_file(state: State<'_, AppState>, dir: String, name: String) -> Result<Workspace, String> {
    let name = with_conf_ext(&name);
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err("Use a plain file name, without slashes.".into());
    }
    if let Some(host) = source_of(&state) {
        let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        if remote::exists(&host, &rel)? {
            return Err(format!("{name} already exists."));
        }
        remote::put_file(&host, &rel, &format!("/* {name} */\n\n"))?;
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let target = if dir.is_empty() { root.join(&name) } else { root.join(&dir).join(&name) };
    check_inside(&root, &target)?;
    if target.exists() {
        return Err(format!("{name} already exists."));
    }
    std::fs::write(&target, format!("/* {name} */\n\n"))
        .map_err(|e| format!("Could not create {name}: {e}"))?;
    reload(&state)
}

#[tauri::command]
fn duplicate_file(state: State<'_, AppState>, file: String) -> Result<Workspace, String> {
    if let Some(host) = source_of(&state) {
        let rel = rel_on(&host, &file);
        let stem = rel.strip_suffix(".conf").unwrap_or(&rel).to_string();
        let mut to = format!("{stem} copy.conf");
        let mut n = 2;
        while remote::exists(&host, &to)? {
            to = format!("{stem} copy {n}.conf");
            n += 1;
            if n > 50 {
                return Err("Too many copies of that file already.".into());
            }
        }
        // What is on screen, unsaved edits included — the same as locally.
        let text = { state.files.lock().unwrap().get(&file).map(|o| o.text.clone()) };
        match text {
            Some(t) => remote::put_file(&host, &to, &t)?,
            None => remote::copy_path(&host, &rel, &to)?,
        }
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let src = PathBuf::from(&file);
    check_inside(&root, &src)?;
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dir = src.parent().map(|p| p.to_path_buf()).unwrap_or(root.clone());

    let mut target = dir.join(format!("{stem} copy.conf"));
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} copy {n}.conf"));
        n += 1;
        if n > 50 {
            return Err("Too many copies of that file already.".into());
        }
    }
    // Copy what's on screen, unsaved edits included.
    let text = {
        let files = state.files.lock().unwrap();
        files.get(&file).map(|o| o.text.clone())
    };
    match text {
        Some(t) => std::fs::write(&target, t).map_err(|e| e.to_string())?,
        None => { std::fs::copy(&src, &target).map_err(|e| e.to_string())?; }
    }
    reload(&state)
}

#[tauri::command]
fn rename_file(state: State<'_, AppState>, file: String, name: String) -> Result<Workspace, String> {
    writable(&file)?;
    let name = with_conf_ext(&name);
    if name.contains('/') || name.contains('\\') {
        return Err("Use a plain file name, without slashes.".into());
    }
    if let Some(host) = source_of(&state) {
        let rel = rel_on(&host, &file);
        let (dir, _) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        let to = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        if to == rel {
            return remote_reload(&state, &host);
        }
        // Unsaved edits go with it, rather than being left behind under the
        // old name where nothing will ever read them again.
        let pending = {
            let files = state.files.lock().unwrap();
            files.get(&file).filter(|o| o.text != o.on_disk).map(|o| o.text.clone())
        };
        remote::move_path(&host, &rel, &to)?;
        if let Some(t) = pending {
            remote::put_file(&host, &to, &t)?;
        }
        state.files.lock().unwrap().remove(&file);
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let src = PathBuf::from(&file);
    check_inside(&root, &src)?;
    let target = src.with_file_name(&name);
    if target == src {
        return reload(&state);
    }
    if target.exists() {
        return Err(format!("{name} already exists."));
    }
    // Carry unsaved edits across rather than losing them to the rename.
    let pending = {
        let files = state.files.lock().unwrap();
        files.get(&file).filter(|o| o.text != o.on_disk).map(|o| o.text.clone())
    };
    std::fs::rename(&src, &target).map_err(|e| format!("Could not rename: {e}"))?;
    if let Some(t) = pending {
        let _ = std::fs::write(&target, t);
    }
    reload(&state)
}

/// Remove a file, leaving a `~` copy beside it so it is recoverable. The
/// confirmation happens in the interface; by the time this is called the
/// answer was yes.
#[tauri::command]
fn delete_file(state: State<'_, AppState>, file: String) -> Result<Workspace, String> {
    writable(&file)?;
    if let Some(host) = source_of(&state) {
        remote::delete_file(&host, &rel_on(&host, &file))?;
        state.files.lock().unwrap().remove(&file);
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let src = PathBuf::from(&file);
    check_inside(&root, &src)?;
    writable(&file)?;
    if !src.is_file() {
        return Err("That file is already gone.".into());
    }
    let backup = format!("{file}~");
    std::fs::copy(&src, &backup).map_err(|e| format!("Could not back it up first: {e}"))?;
    std::fs::remove_file(&src).map_err(|e| format!("Could not delete: {e}"))?;
    state.files.lock().unwrap().remove(&file);
    reload(&state)
}

/// Move a whole block from one file to another. The text is lifted out of the
/// first file — comments that sit directly above it come too, because they are
/// almost always about the block — and appended to the second.
#[tauri::command]
fn move_block(
    state: State<'_, AppState>,
    from: String,
    start: usize,
    to: String,
) -> Result<Workspace, String> {
    writable(&from)?;
    writable(&to)?;
    if from == to {
        return reload(&state);
    }
    let lifted = {
        let mut files = state.files.lock().unwrap();
        let src = files.get(&from).ok_or("That file isn't open.")?;
        let doc = Doc::parse(src.text.clone());
        let span = find_entry(&doc.root, start).ok_or("Couldn't find that block.")?.span.clone();

        // Reach back over any comment lines immediately above it.
        let text = src.text.clone();
        let mut cut_from = line_start(&text, span.start);
        loop {
            if cut_from == 0 {
                break;
            }
            let prev_start = line_start(&text, cut_from - 1);
            let line = text[prev_start..cut_from].trim();
            let is_comment = line.starts_with("//")
                || line.starts_with('#')
                || line.starts_with("/*")
                || line.starts_with('*')
                || line.ends_with("*/");
            if is_comment {
                cut_from = prev_start;
            } else {
                break;
            }
        }
        let cut_to = match text[span.end..].find('\n') {
            Some(i) => span.end + i + 1,
            None => span.end,
        };
        let block = text[cut_from..cut_to].to_string();

        let mut d2 = Doc::parse(text);
        d2.edit(cut_from..cut_to, String::new())?;
        let src_mut = files.get_mut(&from).unwrap();
        src_mut.text = d2.render();

        let dst = files.get_mut(&to).ok_or("The destination file isn't open.")?;
        let lead = if dst.text.ends_with("\n\n") || dst.text.is_empty() { "" } else { "\n" };
        dst.text = format!("{}{lead}{}", dst.text, block.trim_end_matches('\n'));
        dst.text.push('\n');
        block
    };
    let _ = lifted;
    // Built from memory, not reloaded: the move only exists as an edit to the
    // two files' text so far, and re-reading the folder off the disk would
    // throw it straight back away.
    workspace_from_memory(&state)
}

/// Preferences for the folder that's open — your categories, and where things
/// sit on the workbench.
#[tauri::command]
fn get_config(state: State<'_, AppState>) -> serde_json::Value {
    let Ok(root) = root_of(&state) else { return serde_json::Value::Null };
    let key = config_key(&root);
    let s = load_settings();
    if let Some(v) = s.workspaces.get(&key) {
        return v.clone();
    }
    // Older settings were keyed on the path exactly as it was spelled when the
    // folder was opened, so one folder could end up holding two sets of
    // categories depending on which way its slashes went. Adopt the old one.
    for (k, v) in &s.workspaces {
        if config_key(Path::new(k)) == key {
            return v.clone();
        }
    }
    serde_json::Value::Null
}

/// One settings key per folder, however the path was spelled on the way in.
///
/// `C:/x/y` and `C:\x\y` are the same folder and used to be two entries, so
/// your categories, what you had filed where, and what you had hidden all
/// depended on how the path happened to be written when it was opened.
fn config_key(root: &Path) -> String {
    let p = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let s = p.to_string_lossy().to_string();
    // canonicalize hands back a VERBATIM-prefixed path on Windows; that prefix
    // is not part of what makes it that folder.
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

#[tauri::command]
fn set_config(state: State<'_, AppState>, value: serde_json::Value) -> Result<(), String> {
    let root = root_of(&state)?;
    let key = config_key(&root);
    let mut s = load_settings();
    // Drop any older spelling of this same folder, so it stops being two.
    let stale: Vec<String> = s
        .workspaces
        .keys()
        .filter(|k| **k != key && config_key(Path::new(k)) == key)
        .cloned()
        .collect();
    for k in stale {
        s.workspaces.remove(&k);
    }
    s.workspaces.insert(key, value);
    save_settings(&s);
    Ok(())
}

/// What to call a directive in an undo label.
fn name_hint(state: &AppState, file: &str, start: usize) -> String {
    let files = state.files.lock().unwrap();
    let Some(of) = files.get(file) else { return "a value".into() };
    let doc = Doc::parse(of.text.clone());
    match find_entry(&doc.root, start) {
        Some(e) => e.first(Kind::Name).map(|w| doc.word_text(w).to_string())
            .unwrap_or_else(|| "a value".into()),
        None => "a value".into(),
    }
}

/* ------------------------------------------------------------------ *
 * Undo and redo, across every file an action touched.
 * ------------------------------------------------------------------ */

#[derive(Serialize, Clone, Default)]
pub struct HistoryStatus {
    pub undo: Option<String>,
    pub redo: Option<String>,
}

#[tauri::command]
fn history_status(state: State<'_, AppState>) -> HistoryStatus {
    let h = state.history.lock().unwrap();
    HistoryStatus {
        undo: h.next_undo().map(|s| s.to_string()),
        redo: h.next_redo().map(|s| s.to_string()),
    }
}

fn swap_in(state: &AppState, snap: history::Snapshot) -> history::Snapshot {
    let mut files = state.files.lock().unwrap();
    let mut current = Vec::new();
    for (path, text) in &snap.files {
        if let Some(of) = files.get_mut(path) {
            current.push((path.clone(), of.text.clone()));
            of.text = text.clone();
        }
    }
    history::Snapshot { label: snap.label, files: current }
}

#[tauri::command]
fn undo(state: State<'_, AppState>) -> Result<Option<Workspace>, String> {
    let snap = { state.history.lock().unwrap().pop_undo() };
    let Some(snap) = snap else { return Ok(None) };
    let now = swap_in(&state, snap);
    state.history.lock().unwrap().push_redo(now);
    reload(&state).map(Some)
}

#[tauri::command]
fn redo(state: State<'_, AppState>) -> Result<Option<Workspace>, String> {
    let snap = { state.history.lock().unwrap().pop_redo() };
    let Some(snap) = snap else { return Ok(None) };
    let now = swap_in(&state, snap);
    state.history.lock().unwrap().push_undo(now);
    reload(&state).map(Some)
}

/* ------------------------------------------------------------------ *
 * Merge and split, always previewed before they are applied.
 * ------------------------------------------------------------------ */

#[derive(Serialize)]
pub struct Preview {
    pub plan: Option<blocks::MergePlan>,
    pub diff: history::Diff,
    /// The text that would result, held so Apply doesn't recompute it
    pub text: String,
}

fn text_of(state: &AppState, file: &str) -> Result<String, String> {
    let files = state.files.lock().unwrap();
    files.get(file).map(|o| o.text.clone()).ok_or_else(|| "That file isn't open.".into())
}

#[tauri::command]
fn merge_preview(
    state: State<'_, AppState>,
    file: String,
    starts: Vec<usize>,
    keep: blocks::Keep,
) -> Result<Preview, String> {
    let before = text_of(&state, &file)?;
    let plan = blocks::merge(&before, &starts, keep)?;
    let diff = history::diff(&before, &plan.text, 3);
    Ok(Preview { text: plan.text.clone(), plan: Some(plan), diff })
}

#[tauri::command]
fn split_preview(
    state: State<'_, AppState>,
    file: String,
    block_start: usize,
    child_starts: Vec<usize>,
) -> Result<Preview, String> {
    let before = text_of(&state, &file)?;
    let after = blocks::split(&before, block_start, &child_starts)?;
    let diff = history::diff(&before, &after, 3);
    Ok(Preview { plan: None, diff, text: after })
}

/// Commit a previewed change. The text was computed by the preview, so what
/// gets written is exactly what was shown.
#[tauri::command]
fn apply_preview(
    state: State<'_, AppState>,
    file: String,
    text: String,
    label: String,
) -> Result<WsFile, String> {
    replace_text(&state, &file, &label, text)
}

/// Which block types may be folded together, and why not for the rest.
#[tauri::command]
fn mergeable(name: String) -> Option<String> {
    blocks::why_not_mergeable(&name).map(|s| s.to_string())
}

/* ------------------------------------------------------------------ *
 * The problems report.
 * ------------------------------------------------------------------ */

#[tauri::command]
fn problems(state: State<'_, AppState>) -> Result<analyse::Report, String> {
    let root = root_of(&state)?;
    let files = state.files.lock().unwrap();
    let texts: Vec<(String, String)> = files.iter()
        .map(|(p, o)| {
            let rel = PathBuf::from(p).strip_prefix(&root).unwrap_or(Path::new(p))
                .to_string_lossy().replace('\\', "/");
            (rel, o.text.clone())
        })
        .collect();
    drop(files);

    // Only files the server loads together — see analyse.rs.
    let known: Vec<String> = texts.iter().map(|(r, _)| r.clone()).collect();
    let cdir = workspace::conf_dir(&known);
    let raw = workspace::include_map(&texts);
    let mut resolved: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (from, targets) in &raw {
        resolved.insert(from.clone(), targets.iter()
            .filter_map(|t| workspace::resolve_include(&cdir, t, &known))
            .collect());
    }
    let active: std::collections::BTreeSet<String> = match workspace::root_config(&known, &resolved) {
        Some(start) => workspace::reachable(&start, &resolved),
        // No file includes another: they all stand alone, so check each on its
        // own rather than inventing clashes between unrelated files.
        None => known.iter().cloned().collect(),
    };

    let docs: Vec<(String, Doc)> = texts.into_iter()
        .filter(|(r, _)| active.contains(r))
        .map(|(r, t)| (r, Doc::parse(t)))
        .collect();
    let srcs: Vec<analyse::Source> = docs.iter()
        .map(|(r, d)| analyse::Source { rel: r, doc: d })
        .collect();
    Ok(analyse::analyse(&srcs))
}

/* ------------------------------------------------------------------ *
 * Servers reached over ssh.
 * ------------------------------------------------------------------ */

#[tauri::command]
fn hosts() -> Vec<remote::Host> {
    load_settings().hosts
}

#[tauri::command]
fn save_host(host: remote::Host) -> Result<Vec<remote::Host>, String> {
    if host.name.trim().is_empty() || host.target.trim().is_empty() {
        return Err("A server needs a name and something to connect to.".into());
    }
    let mut s = load_settings();
    s.hosts.retain(|h| h.name != host.name);
    s.hosts.push(host);
    s.hosts.sort_by(|a, b| a.name.cmp(&b.name));
    save_settings(&s);
    Ok(s.hosts)
}

#[tauri::command]
fn forget_host(name: String) -> Vec<remote::Host> {
    let mut s = load_settings();
    s.hosts.retain(|h| h.name != name);
    save_settings(&s);
    s.hosts
}

/* ------------------------------------------------------------------ *
 * Talking to another machine.
 *
 * A synchronous Tauri command runs on the thread that draws the window, and
 * ssh takes as long as it takes. Reading a config over ssh used to open one
 * connection per file — thirty-seven handshakes for an ordinary config — with
 * the window frozen throughout, and Windows closes a program that stops
 * answering for a few seconds. It looked like a crash; the event log called it
 * AppHangB1.
 *
 * So every ssh call goes through here: off the drawing thread, and awaited.
 * ------------------------------------------------------------------ */

async fn off_thread<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("That did not finish: {e}"))?
}

/// The host, if the open config is on another machine.
fn source_of(state: &AppState) -> Option<remote::Host> {
    state.source.lock().unwrap().clone()
}

/// A path the interface handed us, as a path relative to the config folder.
///
/// Files come back as `{root}/{rel}` and folders as plain `rel`, so both are
/// accepted; what comes out is always measured from the config directory,
/// which is the only thing the other machine will accept.
fn rel_on(host: &remote::Host, path: &str) -> String {
    let base = host.root.trim_end_matches('/');
    let p = path.replace('\\', "/");
    p.strip_prefix(base).map(|r| r.trim_start_matches('/').to_string())
        .unwrap_or_else(|| p.trim_start_matches('/').to_string())
}

/// Re-read a config on another machine and rebuild the workspace, after
/// something over there has changed.
fn remote_reload(state: &AppState, host: &remote::Host) -> Result<Workspace, String> {
    let got = remote::fetch_all(host)?;
    assemble_remote(state, host, got)
}

/// Try the connection and say what answered. Never hangs: BatchMode is on, so
/// a missing key fails with a reason rather than a silent prompt.
#[tauri::command]
async fn probe_host(host: remote::Host) -> Result<String, String> {
    off_thread(move || probe_host_blocking(host)).await
}

fn probe_host_blocking(host: remote::Host) -> Result<String, String> {
    remote::probe(&host)
}

/// Open a config that lives on another machine. Everything downstream — the
/// tree, the blocks, the checker, undo — works the same; only reading and
/// writing go over ssh.
#[tauri::command]
async fn open_remote(
    state: State<'_, AppState>,
    host: remote::Host,
) -> Result<Workspace, String> {
    // One connection, off the drawing thread. Both halves of that matter: a
    // file at a time was slow enough to look like a crash, and doing it here
    // would freeze the window however fast it were.
    let h = host.clone();
    let got = off_thread(move || remote::fetch_all(&h)).await?;

    assemble_remote(&state, &host, got)
}

/// Build the workspace from a config just read off another machine.
fn assemble_remote(
    state: &AppState,
    host: &remote::Host,
    got: remote::Fetched,
) -> Result<Workspace, String> {
    let texts = got.files;
    // Anything not yet saved survives a re-read, the same as it does locally.
    let unsaved: std::collections::BTreeMap<String, String> = {
        let open = state.files.lock().unwrap();
        open.iter()
            .filter(|(_, of)| of.text != of.on_disk)
            .map(|(k, of)| (k.clone(), of.text.clone()))
            .collect()
    };
    let base = host.root.trim_end_matches('/').to_string();
    let rels: Vec<String> = texts.iter().map(|(r, _)| r.clone()).collect();
    let mut store = std::collections::BTreeMap::new();
    for (rel, text) in &texts {
        let key = format!("{base}/{rel}");
        let mine = unsaved.get(&key).cloned().unwrap_or_else(|| text.clone());
        // Over ssh there is nothing local to stat, so no stamp.
        store.insert(key, OpenFile { stamp: None, on_disk: text.clone(), text: mine });
    }

    let known: Vec<String> = rels.clone();
    let cdir = workspace::conf_dir(&known);
    let raw = workspace::include_map(&texts);
    let mut included: std::collections::BTreeSet<String> = Default::default();
    let mut resolved: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> = Default::default();
    for (from, targets) in &raw {
        let mut ok = Vec::new();
        let mut miss = Vec::new();
        for t in targets {
            match workspace::resolve_include(&cdir, t, &known) {
                Some(r) => { included.insert(r.clone()); ok.push(r); }
                None => miss.push(t.clone()),
            }
        }
        resolved.insert(from.clone(), (ok, miss));
    }

    let files: Vec<WsFile> = state.with_pack(|pk| {
        rels.iter().filter_map(|rel| {
            let key = format!("{base}/{rel}");
            let of = store.get(&key)?;
            let (dir, name) = match rel.rsplit_once('/') {
                Some((d, n)) => (d.to_string(), n.to_string()),
                None => (String::new(), rel.clone()),
            };
            let found = workspace::Found {
                path: PathBuf::from(&key), rel: rel.clone(), dir, name, bytes: of.text.len(),
            };
            let (inc, miss) = resolved.get(rel).cloned().unwrap_or_default();
            Some(build_file(&found, &of.text, &of.on_disk, inc, miss, included.contains(rel), pk))
        }).collect()
    });

    *state.root.lock().unwrap() = Some(PathBuf::from(&base));
    *state.files.lock().unwrap() = store;
    *state.source.lock().unwrap() = Some(host.clone());
    *state.history.lock().unwrap() = history::History::default();

    // The folders the other machine reported, not the ones the file paths
    // imply — otherwise a folder with no config in it yet could never appear,
    // and asking for it a second time would say it already existed.
    let mut folders: Vec<String> = got.dirs;
    for f in &files {
        if !f.dir.is_empty() {
            folders.push(f.dir.clone());
        }
    }
    folders.sort();
    folders.dedup();

    Ok(Workspace {
        root: format!("{} : {}", host.name, base),
        files,
        pack: state.status(),
        remote: Some(format!("{} \u{2014} {}", host.name, host.target)),
        folders,
    })
}

/// `configtest` on the server is the only real answer. Everything Confetti
/// computes is a model of conf.c; this is conf.c.
#[tauri::command]
async fn server_command(
    state: State<'_, AppState>,
    verb: String,
) -> Result<remote::Output, String> {
    let host = state.source.lock().unwrap().clone()
        .ok_or("That's a local folder — there's no server to ask.")?;
    off_thread(move || remote::unrealircd(&host, &verb)).await
}

/// Really remove an entry, rather than commenting it out. Undo brings it back,
/// so this is safe to offer — but the interface asks first, because the block
/// being deleted may be twenty lines the person can't see.
#[tauri::command]
fn delete_entry(state: State<'_, AppState>, file: String, start: usize) -> Result<WsFile, String> {
    edit_file(&state, &file, "deleting a block", |text, doc| {
        let span = {
            let e = find_entry(&doc.root, start).ok_or("Couldn't find that.")?;
            e.span.clone()
        };
        // Take the whole lines it occupies, and any comment sitting directly
        // above it, since that comment describes what is going.
        let mut from = line_start(text, span.start);
        loop {
            if from == 0 {
                break;
            }
            let prev = line_start(text, from - 1);
            let line = text[prev..from].trim();
            let comment = line.starts_with("//") || line.starts_with('#')
                || line.starts_with("/*") || line.starts_with('*');
            // Only a comment on a line of its own belongs to this entry.
            if comment && text[prev..].starts_with(|c: char| c == ' ' || c == '\t' || c == '/' || c == '#' || c == '*') {
                from = prev;
            } else {
                break;
            }
        }
        let to = match text[span.end..].find('\n') {
            Some(i) => span.end + i + 1,
            None => text.len(),
        };
        doc.edit(from..to, String::new())?;
        Ok(())
    })
}

/// Delete several blocks from one file in a single edit.
///
/// One at a time would work only for the first: every later start offset
/// refers to the text as it was *before* the earlier deletions, and the CST
/// refuses an edit it cannot place. One parse, all the cuts, one render.
#[tauri::command]
fn delete_entries(
    state: State<'_, AppState>,
    file: String,
    starts: Vec<usize>,
) -> Result<WsFile, String> {
    if starts.is_empty() {
        return Err("Nothing selected.".into());
    }
    let n = starts.len();
    let label = if n == 1 { "deleting a block".to_string() } else { format!("deleting {n} blocks") };
    edit_file(&state, &file, &label, |text, doc| {
        let mut cuts: Vec<std::ops::Range<usize>> = Vec::new();
        for start in &starts {
            let span = {
                let e = find_entry(&doc.root, *start).ok_or("Couldn't find one of those.")?;
                e.span.clone()
            };
            let (from, to) = with_comments_above(text, &span);
            cuts.push(from..to);
        }
        for c in merge_ranges(cuts) {
            doc.edit(c, String::new())?;
        }
        Ok(())
    })
}

/// Comment several blocks out in one edit, for the same reason.
#[tauri::command]
fn comment_out_many(
    state: State<'_, AppState>,
    file: String,
    starts: Vec<usize>,
) -> Result<WsFile, String> {
    if starts.is_empty() {
        return Err("Nothing selected.".into());
    }
    let n = starts.len();
    let label = if n == 1 {
        "commenting a block out".to_string()
    } else {
        format!("commenting {n} blocks out")
    };
    edit_file(&state, &file, &label, |text, doc| {
        let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
        for start in &starts {
            let e = find_entry(&doc.root, *start).ok_or("Couldn't find one of those.")?;
            let from = line_start(text, e.span.start);
            let to = end_of_line(text, e.span.end);
            spans.push(from..to);
        }
        for span in merge_ranges(spans) {
            let body = &text[span.clone()];
            let mut out = body
                .lines()
                .map(|l| format!("//{l}"))
                .collect::<Vec<_>>()
                .join("\n");
            if body.ends_with('\n') {
                out.push('\n');
            }
            doc.edit(span, out)?;
        }
        Ok(())
    })
}

/// Overlapping cuts happen when the comment above one block sits inside the
/// run of the block before it. Merge them rather than have the CST refuse the
/// whole edit.
fn merge_ranges(mut rs: Vec<std::ops::Range<usize>>) -> Vec<std::ops::Range<usize>> {
    rs.sort_by_key(|r| r.start);
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for c in rs {
        match out.last_mut() {
            Some(last) if c.start <= last.end => last.end = last.end.max(c.end),
            _ => out.push(c),
        }
    }
    out
}

fn end_of_line(text: &str, at: usize) -> usize {
    match text[at..].find('\n') {
        Some(i) => at + i + 1,
        None => text.len(),
    }
}

/// The byte range of an entry plus any comment lines directly above it.
fn with_comments_above(text: &str, span: &std::ops::Range<usize>) -> (usize, usize) {
    let mut from = line_start(text, span.start);
    loop {
        if from == 0 {
            break;
        }
        let prev = line_start(text, from - 1);
        let line = text[prev..from].trim();
        let comment = line.starts_with("//")
            || line.starts_with('#')
            || line.starts_with("/*")
            || line.starts_with('*');
        if comment {
            from = prev;
        } else {
            break;
        }
    }
    (from, end_of_line(text, span.end))
}

/// Move several blocks from one file to another.
#[tauri::command]
fn move_blocks(
    state: State<'_, AppState>,
    from: String,
    starts: Vec<usize>,
    to: String,
) -> Result<Workspace, String> {
    writable(&from)?;
    writable(&to)?;
    if from == to || starts.is_empty() {
        return reload(&state);
    }
    {
        let mut files = state.files.lock().unwrap();
        let text = files.get(&from).ok_or("That file isn't open.")?.text.clone();
        let doc = Doc::parse(text.clone());

        let mut cuts: Vec<std::ops::Range<usize>> = Vec::new();
        for start in &starts {
            let span = find_entry(&doc.root, *start)
                .ok_or("Couldn't find one of those.")?
                .span
                .clone();
            let (a, b) = with_comments_above(&text, &span);
            cuts.push(a..b);
        }
        let merged = merge_ranges(cuts);
        let blocks: Vec<String> = merged.iter().map(|c| text[c.clone()].to_string()).collect();

        let mut d2 = Doc::parse(text);
        for c in merged {
            d2.edit(c, String::new())?;
        }
        files.get_mut(&from).unwrap().text = d2.render();

        let dst = files.get_mut(&to).ok_or("The destination file isn't open.")?;
        for b in &blocks {
            let lead = if dst.text.ends_with("\n\n") || dst.text.is_empty() { "" } else { "\n" };
            dst.text = format!("{}{lead}{}\n", dst.text, b.trim_end_matches('\n'));
        }
    }
    // Built from memory, not reloaded: the move only exists as an edit to the
    // two files' text so far, and re-reading the folder off the disk would
    // throw it straight back away.
    workspace_from_memory(&state)
}

/// One kind of block you can add, as the pack knows it.
#[derive(Serialize, Clone)]
pub struct BlockType {
    pub name: String,
    /// A sentence from the wiki, where there is one
    pub hint: Option<String>,
    /// What the second word means, when the block takes one
    pub label_hint: Option<String>,
    /// Fixed choices for that second word, for blocks with sub-types
    pub subtypes: Vec<String>,
    /// True for blocks UnrealIRCd itself lists, rather than ones we inferred
    pub core: bool,
}

/// What the second word of a block means. The sub-typed ones are fixed
/// vocabularies, not free text, so offering a list beats asking someone to
/// remember whether it's `ban realname` or `ban gecos`.
fn label_shape(name: &str) -> (Option<&'static str>, Vec<&'static str>) {
    match name {
        "ban" => (Some("what kind of ban"), vec!["nick", "ip", "server", "user", "realname", "version"]),
        "deny" => (Some("what to deny"), vec!["dcc", "channel", "link", "version"]),
        "except" => (Some("what to make an exception for"), vec!["ban", "throttle", "tkl", "blacklist"]),
        "require" => (Some("what to require"), vec!["authentication", "sasl", "module"]),
        "oper" => (Some("the name they type in /OPER"), vec![]),
        "class" => (Some("a name for this group, like clients or opers"), vec![]),
        "link" => (Some("the other server's name"), vec![]),
        "operclass" => (Some("a name for this set of powers"), vec![]),
        "security-group" => (Some("a name for this group of people"), vec![]),
        "blacklist" => (Some("a name for this blocklist"), vec![]),
        "alias" => (Some("the command word, without the slash"), vec![]),
        "log" => (Some("a file to write to (optional)"), vec![]),
        "set" => (Some("a security-group name, to override settings for them (optional)"), vec![]),
        _ => (None, vec![]),
    }
}

#[tauri::command]
fn block_types(state: State<'_, AppState>) -> Vec<BlockType> {
    state.with_pack(|pk| {
        let names: Vec<String> = match pk {
            Some(p) => p.blocks.clone(),
            // Without a docs pack we still know the common ones.
            None => ["me", "admin", "class", "allow", "listen", "oper", "operclass", "link",
                     "ulines", "drpass", "log", "alias", "help", "set", "ban", "deny", "except",
                     "require", "vhost", "badword", "blacklist", "security-group", "tld",
                     "official-channels", "webirc", "sni", "secret", "proxy", "include",
                     "loadmodule", "blacklist-module", "files", "spamfilter"]
                .iter().map(|s| s.to_string()).collect(),
        };
        names.into_iter().map(|name| {
            let (lh, subs) = label_shape(&name);
            let hint = pk.and_then(|p| p.hint(&name, None).summary);
            BlockType {
                label_hint: lh.map(|s| s.to_string()),
                subtypes: subs.into_iter().map(|s| s.to_string()).collect(),
                core: true,
                hint,
                name,
            }
        }).collect()
    })
}

#[derive(Serialize, Clone)]
pub struct DirectiveChoice {
    pub name: String,
    pub hint: Option<String>,
    pub kind: types::Kind,
    /// True when this one holds a block rather than a value
    pub is_block: bool,
    /// True for `mask { }` and its relatives, which UnrealIRCd accepts written
    /// either way: one hostmask on a line, or a block of criteria. Nothing is
    /// documented *below* oper::mask, so `is_block` alone reports it as a
    /// plain value and the block form becomes unreachable.
    pub matcher: bool,
    /// Words the source compares this value against.
    pub choices: Vec<String>,
    /// Why the server refuses it. Never offered as something to add.
    pub refused: Option<String>,
    /// What the server uses when you leave it out.
    pub default: Option<String>,
}

/// Which directives may go inside a given path, from the scrape of conf.c.
#[tauri::command]
fn child_directives(state: State<'_, AppState>, path: String) -> Vec<DirectiveChoice> {
    state.with_pack(|pk| {
        let Some(p) = pk else { return Vec::new() };
        let prefix = format!("{path}::");
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let mut out = Vec::new();
        for key in p.directives.keys() {
            let Some(rest) = key.strip_prefix(&prefix) else { continue };
            let head = rest.split("::").next().unwrap_or(rest);
            if head.is_empty() || !seen.insert(head.to_string()) {
                continue;
            }
            let full = format!("{prefix}{head}");
            // A name the server only knows so it can refuse it. Writing
            // operclass::privileges stops UnrealIRCd from starting, so it is
            // not something to offer; the reason still reaches anyone whose
            // config already has one.
            if p.refused(&full).is_some() || p.is_prose(&full) {
                continue;
            }
            // If anything is documented below it, it holds a block.
            let is_block = p.directives.keys().any(|k| k.starts_with(&format!("{full}::")));
            out.push(DirectiveChoice {
                hint: p.hint(&full, None).summary,
                kind: p.kind_of(&full).unwrap_or_else(|| types::kind_of(&full)),
                is_block,
                matcher: p.is_matcher(&full) || types::is_matcher(&full),
                choices: p.choices_for(&full),
                refused: p.refused(&full),
                default: p.default_of(&full),
                name: head.to_string(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    })
}

/// Every `set` directive this release knows, with its default and its
/// documentation. The "effective settings" viewer needs the ones you have
/// *not* written as much as the ones you have.
#[tauri::command]
fn known_settings(state: State<'_, AppState>) -> Vec<KnownSetting> {
    state.with_pack(|pk| {
        let Some(p) = pk else { return Vec::new() };
        let mut out = Vec::new();
        for (path, info) in &p.directives {
            if !path.starts_with("set::") || info.prose || info.refused.is_some() {
                continue;
            }
            // Only the leaves: `set::anti-flood` is a place, not a setting.
            if p.directives.keys().any(|k| k.starts_with(&format!("{path}::"))) {
                continue;
            }
            out.push(KnownSetting {
                path: path.clone(),
                default: info.default.clone(),
                doc_default: info.doc_default.clone(),
                kind: info.kind.unwrap_or(types::Kind::Text),
                hint: p.hint(path, None).summary,
            });
        }
        out
    })
}

#[derive(Serialize)]
pub struct KnownSetting {
    pub path: String,
    /// From the server's own source — authoritative.
    pub default: Option<String>,
    /// From the wiki, where the source did not say. The two sometimes differ.
    pub doc_default: Option<String>,
    pub kind: types::Kind,
    pub hint: Option<String>,
}

/* ------------------------------------------------------------------ *
 * Somebody else editing the same files.
 *
 * A config folder is not private: you rehash from a shell, an editor is
 * open in another window, a deploy script rewrites a file. Confetti holds
 * every file in memory, so without this it would quietly save its own
 * stale copy over whatever you did elsewhere.
 * ------------------------------------------------------------------ */

/// One file that something else has changed since Confetti read it.
#[derive(Serialize, Clone)]
pub struct DiskChange {
    pub path: String,
    pub rel: String,
    pub name: String,
    /// It is not there any more.
    pub gone: bool,
    /// Confetti has edits of its own, so taking the new one would lose them.
    pub mine: bool,
    pub added: usize,
    pub removed: usize,
}

/// Which open files have changed underneath us.
///
/// A stat per file, and a read only where the stat moved. The stamp is only
/// brought up to date when the contents turn out to be the same after all —
/// otherwise a change would be reported once and then forgotten about.
#[tauri::command]
fn disk_changes(state: State<'_, AppState>) -> Vec<DiskChange> {
    // Nothing to stat on another machine, and a stat per file over ssh every
    // few seconds is not something to do quietly in the background.
    if state.source.lock().unwrap().is_some() {
        return Vec::new();
    }
    let root = state.root.lock().unwrap().clone();
    let mut out = Vec::new();
    let mut files = state.files.lock().unwrap();
    for (path, of) in files.iter_mut() {
        let now = stamp_of(path);
        if now.is_some() && now == of.stamp {
            continue;
        }
        let rel = match &root {
            Some(r) => PathBuf::from(path)
                .strip_prefix(r)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.clone()),
            None => path.clone(),
        };
        let name = file_name_of(path);
        let Ok(disk) = std::fs::read_to_string(path) else {
            // Gone, or unreadable. Either way it is not what we have.
            if now.is_none() && of.stamp.is_some() {
                out.push(DiskChange {
                    path: path.clone(), rel, name, gone: true,
                    mine: of.text != of.on_disk, added: 0, removed: 0,
                });
            }
            continue;
        };
        if disk == of.on_disk {
            // Touched but not changed — nothing to say, and no need to read
            // it again next time round.
            of.stamp = now;
            continue;
        }
        let d = history::diff(&of.on_disk, &disk, 0);
        out.push(DiskChange {
            path: path.clone(),
            rel,
            name,
            gone: false,
            mine: of.text != of.on_disk,
            added: d.added,
            removed: d.removed,
        });
    }
    out
}

/// What the file on disk says now, against what Confetti is holding.
#[tauri::command]
fn disk_diff(state: State<'_, AppState>, file: String) -> Result<history::Diff, String> {
    let files = state.files.lock().unwrap();
    let of = files.get(&file).ok_or("That file isn't open.")?;
    let disk = std::fs::read_to_string(&file).map_err(|e| format!("Can't read it: {e}"))?;
    Ok(history::diff(&of.text, &disk, 3))
}

/// Take what is on disk, dropping what Confetti had.
///
/// Goes through the undo history like any other change, so if this turns out
/// to be the wrong answer Ctrl+Z gives you your version back.
#[tauri::command]
fn reload_file(state: State<'_, AppState>, file: String) -> Result<WsFile, String> {
    let disk = std::fs::read_to_string(&file).map_err(|e| format!("Can't read it: {e}"))?;
    // Not routed through replace_text: that one refuses a `*.default.conf`,
    // and reading a file is not editing it — a shipped file that changed
    // under an upgrade is exactly one you want to pick up. It also has to set
    // `on_disk` before the file is described again, or the result still says
    // there are unsaved changes.
    remember(&state, "reloading from disk", std::slice::from_ref(&file));
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(&file).ok_or("That file isn't open.")?;
        of.text = disk.clone();
        of.on_disk = disk;
        of.stamp = stamp_of(&file);
    }
    refresh_file(&state, &file)
}

/// Keep what Confetti has, and stop being asked about this file.
///
/// The file on disk is left exactly as it is: this only says "I know", and
/// the difference will show again the moment either side changes.
#[tauri::command]
fn keep_mine(state: State<'_, AppState>, file: String) -> Result<WsFile, String> {
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(&file).ok_or("That file isn't open.")?;
        of.stamp = stamp_of(&file);
        // What is on disk is now the thing we would be overwriting, so the
        // save button has to light up even if our text has not moved.
        if let Ok(disk) = std::fs::read_to_string(&file) {
            of.on_disk = disk;
        }
    }
    refresh_file(&state, &file)
}

#[tauri::command]
fn pack_status(state: State<'_, AppState>) -> PackStatus {
    state.status()
}

/// The matching criteria a `mask { }` accepts.
#[tauri::command]
fn vocabulary() -> Vec<types::Criterion> {
    types::CRITERIA.to_vec()
}

/// Point Confetti at an UnrealIRCd source folder so hints come from the
/// release you are actually configuring.
#[tauri::command]
fn pick_source_root(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<PackStatus, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Where is the UnrealIRCd source? (the folder containing src/ and doc/)")
        .blocking_pick_folder();
    let Some(fp) = picked else { return Ok(state.status()) };
    let dir = fp.into_path().map_err(|e| e.to_string())?;
    if !dir.join("src").join("conf.c").is_file() {
        return Err(format!(
            "{} doesn't look like an UnrealIRCd source folder — there's no src/conf.c in it.",
            dir.display()
        ));
    }
    let status = state.load_pack(&dir)?;
    {
        let mut s = load_settings();
        s.source_root = Some(dir.to_string_lossy().to_string());
        save_settings(&s);
    }
    Ok(status)
}

/// The full wiki text for one directive.
#[tauri::command]
fn hint_for(state: State<'_, AppState>, path: String, label: Option<String>) -> pack::Hint {
    state.with_pack(|pk| match pk {
        Some(p) => p.hint(&path, label.as_deref()),
        None => pack::Hint::default(),
    })
}

/// Re-render everything — used when documentation finishes loading.
#[tauri::command]
fn reopen(state: State<'_, AppState>) -> Result<Option<Workspace>, String> {
    let root = state.root.lock().unwrap().clone();
    match root {
        Some(r) => read_workspace(&state, &r).map(Some),
        None => Ok(None),
    }
}

/* ------------------------------------------------------------------ *
 * Folders, moving files, and the things a right-click asks for.
 * ------------------------------------------------------------------ */

/// One path segment, no traversal. Every folder command goes through this
/// before touching the disk, and `check_inside` catches anything it misses.
fn valid_segment(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Give it a name.".into());
    }
    if n.contains('/') || n.contains('\\') || n.starts_with('.') || n.contains("..") {
        return Err("Use a plain name, without slashes.".into());
    }
    if n.len() > 64 {
        return Err("That name is too long.".into());
    }
    Ok(n.to_string())
}

fn folder_path(root: &Path, dir: &str) -> Result<PathBuf, String> {
    if dir.is_empty() {
        return Err("That's the top level, not a folder.".into());
    }
    if dir.contains("..") {
        return Err("That path is outside the folder Confetti has open.".into());
    }
    Ok(root.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

#[tauri::command]
fn new_folder(state: State<'_, AppState>, dir: String, name: String) -> Result<Workspace, String> {
    let name = valid_segment(&name)?;
    if let Some(host) = source_of(&state) {
        let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        if remote::exists(&host, &rel)? {
            return Err(format!("{name} already exists."));
        }
        remote::mkdir(&host, &rel)?;
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let parent = if dir.is_empty() { root.clone() } else { folder_path(&root, &dir)? };
    let target = parent.join(&name);
    check_inside(&root, &target)?;
    if target.exists() {
        return Err(format!("{name} already exists."));
    }
    std::fs::create_dir_all(&target).map_err(|e| format!("Could not create {name}: {e}"))?;
    reload(&state)
}

#[tauri::command]
fn rename_folder(state: State<'_, AppState>, dir: String, name: String) -> Result<Workspace, String> {
    if let Some(host) = source_of(&state) {
        let name = valid_segment(&name)?;
        let (above, _) = dir.rsplit_once('/').unwrap_or(("", dir.as_str()));
        let to = if above.is_empty() { name } else { format!("{above}/{name}") };
        if to == dir {
            return remote_reload(&state, &host);
        }
        // Every unsaved file under it is about to move, and they are keyed by
        // path. Send them over first rather than losing them to the rename.
        let pending: Vec<(String, String)> = {
            let files = state.files.lock().unwrap();
            let base = format!("{}/{}/", host.root.trim_end_matches('/'), dir);
            files.iter()
                .filter(|(k, o)| k.starts_with(&base) && o.text != o.on_disk)
                .map(|(k, o)| (rel_on(&host, k), o.text.clone()))
                .collect()
        };
        for (rel, text) in &pending {
            remote::put_file(&host, rel, text)?;
        }
        remote::move_path(&host, &dir, &to)?;
        state.files.lock().unwrap().clear();
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let src = folder_path(&root, &dir)?;
    check_inside(&root, &src)?;
    let name = valid_segment(&name)?;
    let target = src.with_file_name(&name);
    if target == src {
        return reload(&state);
    }
    if target.exists() {
        return Err(format!("{name} already exists."));
    }
    // Unsaved edits are keyed by absolute path, and every one of them is about
    // to move. Write them out first rather than silently losing them.
    save_dirty_under(&state, &src)?;
    std::fs::rename(&src, &target).map_err(|e| format!("Could not rename: {e}"))?;
    state.files.lock().unwrap().clear();
    reload(&state)
}

/// A folder is never actually deleted: it is renamed to `name~`, out of the
/// way of the `.conf` walk but still on disk. Nothing here is unrecoverable.
#[tauri::command]
fn delete_folder(state: State<'_, AppState>, dir: String) -> Result<Workspace, String> {
    if let Some(host) = source_of(&state) {
        // Everything unsaved under it goes over first, then the folder is set
        // aside rather than deleted: nothing is really gone on one click.
        let pending: Vec<(String, String)> = {
            let files = state.files.lock().unwrap();
            let base = format!("{}/{}/", host.root.trim_end_matches('/'), dir);
            files.iter()
                .filter(|(k, o)| k.starts_with(&base) && o.text != o.on_disk)
                .map(|(k, o)| (rel_on(&host, k), o.text.clone()))
                .collect()
        };
        for (rel, text) in &pending {
            remote::put_file(&host, rel, text)?;
        }
        remote::retire_folder(&host, &dir)?;
        state.files.lock().unwrap().clear();
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let src = folder_path(&root, &dir)?;
    check_inside(&root, &src)?;
    if !src.is_dir() {
        return Err("That folder is already gone.".into());
    }
    save_dirty_under(&state, &src)?;
    let mut target = src.with_file_name(format!(
        "{}~",
        src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    ));
    let mut n = 2;
    while target.exists() {
        target = src.with_file_name(format!(
            "{}~{n}",
            src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
        n += 1;
        if n > 50 {
            return Err("Too many old copies of that folder already.".into());
        }
    }
    std::fs::rename(&src, &target).map_err(|e| format!("Could not remove it: {e}"))?;
    state.files.lock().unwrap().clear();
    reload(&state)
}

/// Flush anything unsaved that lives under `dir`, so a rename or a move cannot
/// throw work away.
fn save_dirty_under(state: &AppState, dir: &Path) -> Result<(), String> {
    let prefix = dir.to_string_lossy().to_string();
    let pending: Vec<(String, String)> = {
        let files = state.files.lock().unwrap();
        files
            .iter()
            .filter(|(k, v)| k.starts_with(&prefix) && v.text != v.on_disk)
            .map(|(k, v)| (k.clone(), v.text.clone()))
            .collect()
    };
    for (path, text) in pending {
        std::fs::write(&path, &text).map_err(|e| format!("Could not save {path} first: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
fn move_file(state: State<'_, AppState>, file: String, dir: String) -> Result<Workspace, String> {
    writable(&file)?;
    if let Some(host) = source_of(&state) {
        let rel = rel_on(&host, &file);
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        let to = if dir.is_empty() { name } else { format!("{dir}/{name}") };
        if to == rel {
            return remote_reload(&state, &host);
        }
        let pending = {
            let files = state.files.lock().unwrap();
            files.get(&file).filter(|o| o.text != o.on_disk).map(|o| o.text.clone())
        };
        remote::move_path(&host, &rel, &to)?;
        if let Some(t) = pending {
            remote::put_file(&host, &to, &t)?;
        }
        state.files.lock().unwrap().remove(&file);
        return remote_reload(&state, &host);
    }
    let root = root_of(&state)?;
    let src = PathBuf::from(&file);
    check_inside(&root, &src)?;
    let parent = if dir.is_empty() { root.clone() } else { folder_path(&root, &dir)? };
    if !parent.is_dir() {
        return Err("That folder doesn't exist.".into());
    }
    let name = src.file_name().ok_or("That isn't a file.")?;
    let target = parent.join(name);
    check_inside(&root, &target)?;
    if target == src {
        return reload(&state);
    }
    if target.exists() {
        return Err(format!("{} already exists there.", name.to_string_lossy()));
    }
    let pending = {
        let files = state.files.lock().unwrap();
        files.get(&file).filter(|o| o.text != o.on_disk).map(|o| o.text.clone())
    };
    std::fs::rename(&src, &target).map_err(|e| format!("Could not move it: {e}"))?;
    if let Some(t) = pending {
        let _ = std::fs::write(&target, t);
    }
    state.files.lock().unwrap().remove(&file);
    reload(&state)
}

/* ---------------- blocks: copy and paste ---------------- */

/// The exact text of one block, comments above it included — what "Copy" puts
/// on the clipboard, and what `paste_block` expects back.
#[tauri::command]
fn block_text(state: State<'_, AppState>, file: String, start: usize) -> Result<String, String> {
    let files = state.files.lock().unwrap();
    let of = files.get(&file).ok_or("That file isn't open.")?;
    let doc = Doc::parse(of.text.clone());
    let span = find_entry(&doc.root, start).ok_or("Couldn't find that block.")?.span.clone();
    let (from, to) = with_leading_comments(&of.text, &span);
    Ok(of.text[from..to].trim_end_matches('\n').to_string())
}

/// The byte range of a block plus the comment lines directly above it. Those
/// comments are almost always about the block, so they travel with it.
fn with_leading_comments(text: &str, span: &std::ops::Range<usize>) -> (usize, usize) {
    let mut from = line_start(text, span.start);
    loop {
        if from == 0 {
            break;
        }
        let prev = line_start(text, from - 1);
        let line = text[prev..from].trim();
        let is_comment = line.starts_with("//")
            || line.starts_with('#')
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.ends_with("*/");
        if is_comment {
            from = prev;
        } else {
            break;
        }
    }
    let to = match text[span.end..].find('\n') {
        Some(i) => span.end + i + 1,
        None => span.end,
    };
    (from, to)
}

/// Put text at the end of a file, having first checked it parses as config.
/// Pasting something that isn't config would make the file unreadable, and the
/// only honest moment to refuse is before it lands.
#[tauri::command]
fn paste_block(state: State<'_, AppState>, file: String, text: String) -> Result<WsFile, String> {
    writable(&file)?;
    let body = text.trim();
    if body.is_empty() {
        return Err("There's nothing on the clipboard to paste.".into());
    }
    let probe = Doc::parse(body.to_string());
    if probe.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err("That doesn't parse as configuration, so Confetti won't paste it.".into());
    }
    if probe.root.entries().next().is_none() {
        return Err("That has no config block in it.".into());
    }
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(&file).ok_or("That file isn't open.")?;
        let lead = if of.text.ends_with("\n\n") || of.text.is_empty() { "" } else { "\n" };
        of.text = format!("{}{lead}{}\n", of.text, body);
    }
    refresh_file(&state, &file)
}

/// Paste config text inside a block, rather than at the end of the file.
///
/// Copying a `permissions { }` from one operclass into another was impossible:
/// the only paste there was appended whole blocks to a file. The text is
/// re-indented to sit at its new depth, so a block pasted three levels in does
/// not arrive wearing the indentation of where it came from.
#[tauri::command]
fn paste_into(
    state: State<'_, AppState>,
    file: String,
    parent_start: usize,
    text: String,
) -> Result<WsFile, String> {
    let body = text.trim_end();
    if body.trim().is_empty() {
        return Err("There's nothing on the clipboard to paste.".into());
    }
    // It has to parse, or the file would be broken by the paste. Wrapped,
    // because a fragment like `parent netadmin;` is not a whole document.
    let probe = Doc::parse(format!("x {{\n{body}\n}}\n"));
    if probe.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err("That doesn't parse as configuration, so Confetti won't paste it.".into());
    }
    edit_file(&state, &file, "pasting", |text, doc| {
        let (range, indent, lead, tail) = {
            let parent = find_entry(&doc.root, parent_start).ok_or("Couldn't find that block.")?;
            block_insertion(text, parent)?
        };
        doc.edit(range, format!("{lead}{}{tail}", reindent(body, &indent)))?;
        Ok(())
    })
}

/// Config text moved to a new depth: its own shape kept, its outermost level
/// sitting at `indent`.
fn reindent(body: &str, indent: &str) -> String {
    let base = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(leading_whitespace)
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            out.push('\n');
            continue;
        }
        let cut = line.char_indices().nth(base.min(leading_whitespace(line))).map(|(i, _)| i).unwrap_or(0);
        out.push_str(indent);
        out.push_str(&line[cut..]);
        out.push('\n');
    }
    out
}

fn leading_whitespace(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

#[tauri::command]
fn copy_block(
    state: State<'_, AppState>,
    from: String,
    start: usize,
    to: String,
) -> Result<Workspace, String> {
    writable(&to)?;
    let block = {
        let files = state.files.lock().unwrap();
        let src = files.get(&from).ok_or("That file isn't open.")?;
        let doc = Doc::parse(src.text.clone());
        let span = find_entry(&doc.root, start).ok_or("Couldn't find that block.")?.span.clone();
        let (a, b) = with_leading_comments(&src.text, &span);
        src.text[a..b].to_string()
    };
    {
        let mut files = state.files.lock().unwrap();
        let dst = files.get_mut(&to).ok_or("The destination file isn't open.")?;
        let lead = if dst.text.ends_with("\n\n") || dst.text.is_empty() { "" } else { "\n" };
        dst.text = format!("{}{lead}{}\n", dst.text, block.trim_end_matches('\n'));
    }
    // Built from memory, not reloaded: the move only exists as an edit to the
    // two files' text so far, and re-reading the folder off the disk would
    // throw it straight back away.
    workspace_from_memory(&state)
}

/* ---------------- includes ---------------- */

/// Add `include "target";` to a file — after the last include already there,
/// so the includes stay together, or at the end if there are none.
#[tauri::command]
fn add_include(state: State<'_, AppState>, file: String, target: String) -> Result<WsFile, String> {
    writable(&file)?;
    let t = target.trim().replace('\\', "/");
    if t.is_empty() {
        return Err("Nothing to include.".into());
    }
    {
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(&file).ok_or("That file isn't open.")?;
        let doc = Doc::parse(of.text.clone());

        for e in doc.root.entries() {
            if let Some(v) = e.first(Kind::Value) {
                if doc.word_text(v) == t {
                    if let Some(n) = e.first(Kind::Name) {
                        if doc.word_text(n) == "include" {
                            return Err(format!("{t} is already included here."));
                        }
                    }
                }
            }
        }

        let mut after = None;
        for e in doc.root.entries() {
            let Some(n) = e.first(Kind::Name) else { continue };
            if doc.word_text(n) == "include" {
                after = Some(e.span.end);
            }
        }
        let line = format!("include \"{t}\";\n");
        let mut d2 = Doc::parse(of.text.clone());
        match after {
            Some(end) => {
                let at = match of.text[end..].find('\n') {
                    Some(i) => end + i + 1,
                    None => of.text.len(),
                };
                d2.edit(at..at, line)?;
                of.text = d2.render();
            }
            None => {
                let lead = if of.text.ends_with('\n') || of.text.is_empty() { "" } else { "\n" };
                of.text = format!("{}{lead}{line}", of.text);
            }
        }
    }
    refresh_file(&state, &file)
}

/* ---------------- the desktop around us ---------------- */

/// Show a file in Explorer or the desktop's file manager, selected.
#[tauri::command]
fn reveal_path(state: State<'_, AppState>, path: String) -> Result<(), String> {
    if state.source.lock().unwrap().is_some() {
        return Err("These files are on the server, not on this machine.".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| e.to_string())
}

/// Hand a file to whatever this machine opens `.conf` with.
#[tauri::command]
fn open_external(state: State<'_, AppState>, path: String) -> Result<(), String> {
    if state.source.lock().unwrap().is_some() {
        return Err("These files are on the server, not on this machine.".into());
    }
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|e| format!("Nothing on this machine is set up to open .conf files: {e}"))
}

/// Compare two open files line by line. Used to answer "what have I actually
/// changed from the shipped example?", which is otherwise a lot of squinting.
#[tauri::command]
fn diff_files(
    state: State<'_, AppState>,
    a: String,
    b: String,
) -> Result<history::Diff, String> {
    let files = state.files.lock().unwrap();
    let left = files.get(&a).ok_or("That file isn't open.")?;
    let right = files.get(&b).ok_or("The other file isn't open.")?;
    Ok(history::diff(&left.text, &right.text, 3))
}

#[tauri::command]
fn recent_folders() -> Vec<String> {
    load_settings().recents.into_iter().filter(|p| Path::new(p).is_dir()).collect()
}

fn remember_folder(root: &Path) {
    let mut s = load_settings();
    let key = root.to_string_lossy().to_string();
    s.recents.retain(|p| p != &key);
    s.recents.insert(0, key);
    s.recents.truncate(12);
    save_settings(&s);
}

/* ------------------------------------------------------------------ *
 * Tabs. One UnrealIRCd instance each.
 * ------------------------------------------------------------------ */

/// Start the tab bookkeeping off the first time anything asks about it. The
/// application opens with one tab whether or not anybody says the word.
fn ensure_tabs(state: &AppState) {
    let mut order = state.order.lock().unwrap();
    if order.is_empty() {
        order.push(1);
        *state.active.lock().unwrap() = 1;
        *state.next_id.lock().unwrap() = 2;
    }
}

/// Lift the live state out into a parked tab, leaving the fields empty.
fn park_active(state: &AppState) {
    let id = *state.active.lock().unwrap();
    let p = Parked {
        id,
        root: state.root.lock().unwrap().take(),
        files: std::mem::take(&mut *state.files.lock().unwrap()),
        pack: state.pack.lock().unwrap().take(),
        pack_root: state.pack_root.lock().unwrap().take(),
        history: std::mem::take(&mut *state.history.lock().unwrap()),
        source: state.source.lock().unwrap().take(),
    };
    state.parked.lock().unwrap().push(p);
}

/// Put a parked tab back into the live fields.
fn unpark(state: &AppState, id: u32) {
    let found = {
        let mut parked = state.parked.lock().unwrap();
        parked.iter().position(|p| p.id == id).map(|i| parked.remove(i))
    };
    let p = found.unwrap_or(Parked { id, ..Default::default() });
    *state.root.lock().unwrap() = p.root;
    *state.files.lock().unwrap() = p.files;
    *state.pack.lock().unwrap() = p.pack;
    *state.pack_root.lock().unwrap() = p.pack_root;
    *state.history.lock().unwrap() = p.history;
    *state.source.lock().unwrap() = p.source;
    *state.active.lock().unwrap() = id;
}

/// What to call a tab. A config folder is very often just `conf`, which says
/// nothing, so in that case the folder above it is the name worth showing.
fn tab_label(
    root: &Option<PathBuf>,
    source: &Option<remote::Host>,
    files: &std::collections::BTreeMap<String, OpenFile>,
) -> (String, String, usize, bool, bool) {
    let dirty = files.values().filter(|f| f.text != f.on_disk).count();
    if let Some(h) = source {
        return (h.name.clone(), format!("{} : {}", h.target, h.root), dirty, true, false);
    }
    match root {
        None => ("New tab".into(), "nothing open yet".into(), 0, false, true),
        Some(r) => {
            let last = r.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let name = if last.eq_ignore_ascii_case("conf") {
                r.parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(last)
            } else {
                last
            };
            (name, r.to_string_lossy().replace('\\', "/"), dirty, false, false)
        }
    }
}

#[tauri::command]
fn tabs(state: State<'_, AppState>) -> Vec<TabInfo> {
    ensure_tabs(&state);
    let order = state.order.lock().unwrap().clone();
    let active = *state.active.lock().unwrap();
    let parked = state.parked.lock().unwrap();

    order
        .iter()
        .map(|&id| {
            let (title, subtitle, dirty, remote, empty) = if id == active {
                tab_label(
                    &state.root.lock().unwrap(),
                    &state.source.lock().unwrap(),
                    &state.files.lock().unwrap(),
                )
            } else {
                match parked.iter().find(|p| p.id == id) {
                    Some(p) => tab_label(&p.root, &p.source, &p.files),
                    None => ("New tab".into(), "nothing open yet".into(), 0, false, true),
                }
            };
            TabInfo { id, title, subtitle, dirty, remote, active: id == active, empty }
        })
        .collect()
}

/// A new, empty tab, opened directly after the one you were on.
#[tauri::command]
fn new_tab(state: State<'_, AppState>) -> Vec<TabInfo> {
    ensure_tabs(&state);
    park_active(&state);
    let id = {
        let mut n = state.next_id.lock().unwrap();
        let id = *n;
        *n += 1;
        id
    };
    {
        let mut order = state.order.lock().unwrap();
        let active = *state.active.lock().unwrap();
        let at = order.iter().position(|&x| x == active).map(|i| i + 1).unwrap_or(order.len());
        order.insert(at, id);
    }
    unpark(&state, id);
    tabs(state)
}

/// Switch to another tab. Returns its workspace, rebuilt from memory — no file
/// is read again, so unsaved edits come back exactly as they were left.
/// Switch to another tab.
///
/// `rebuild` asks for the workspace back. A parked tab cannot change while it
/// is parked, so the interface keeps its own copy and asks for one only the
/// first time it visits — rebuilding means re-parsing every file, which is
/// several seconds on a real config and pure waste when nothing can have moved.
///
/// Returning `None` means "nothing for you": either you did not ask, or the tab
/// has nothing open in it. The caller knows which, and must not read it as
/// "this tab is empty now" — doing that wipes the pane.
#[tauri::command]
fn switch_tab(
    state: State<'_, AppState>,
    id: u32,
    rebuild: bool,
) -> Result<Option<Workspace>, String> {
    ensure_tabs(&state);
    if !state.order.lock().unwrap().contains(&id) {
        return Err("There's no such tab.".into());
    }
    // Already here: a second click while the first was still in flight. Do the
    // swap only when it is actually a different tab.
    if *state.active.lock().unwrap() != id {
        park_active(&state);
        unpark(&state, id);
    }
    if !rebuild || state.root.lock().unwrap().is_none() {
        return Ok(None);
    }
    workspace_from_memory(&state).map(Some)
}

/// Close a tab. Whether anything unsaved is going with it is the interface's
/// question to ask; by the time this is called the answer was yes.
#[tauri::command]
fn close_tab(state: State<'_, AppState>, id: u32) -> Result<Option<Workspace>, String> {
    ensure_tabs(&state);
    if state.order.lock().unwrap().len() < 2 {
        return Err("That's the only tab.".into());
    }
    let active = *state.active.lock().unwrap();

    let next = {
        let mut order = state.order.lock().unwrap();
        let Some(i) = order.iter().position(|&x| x == id) else {
            return Err("There's no such tab.".into());
        };
        order.remove(i);
        if id == active {
            // Fall back to the tab on the left, or the new first one.
            Some(order[i.saturating_sub(1).min(order.len() - 1)])
        } else {
            None
        }
    };

    state.parked.lock().unwrap().retain(|p| p.id != id);

    match next {
        None => Ok(None),
        Some(n) => {
            // The closed tab was the live one; its state goes rather than being
            // parked, and the neighbour takes over.
            unpark(&state, n);
            if state.root.lock().unwrap().is_none() {
                Ok(None)
            } else {
                workspace_from_memory(&state).map(Some)
            }
        }
    }
}

/// `rel` for a file, given whatever the workspace calls its base.
fn rel_from_key(key: &str, base: &str) -> String {
    let b = base.trim_end_matches(['/', '\\']);
    key.strip_prefix(b)
        .unwrap_or(key)
        .trim_start_matches(['/', '\\'])
        .replace('\\', "/")
}

/// Rebuild the workspace view from what is already in memory.
///
/// `read_workspace` reads every file off the disk, which is right when opening
/// a folder and wrong when returning to a tab: it would silently throw away
/// every edit you had not saved. This builds the same thing from the texts we
/// are already holding.
fn workspace_from_memory(state: &AppState) -> Result<Workspace, String> {
    let root = root_of(state)?;
    let host = state.source.lock().unwrap().clone();
    let base = match &host {
        Some(h) => h.root.trim_end_matches('/').to_string(),
        None => root.to_string_lossy().to_string(),
    };

    let store = state.files.lock().unwrap();
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut founds: Vec<workspace::Found> = Vec::new();
    for (key, of) in store.iter() {
        let rel = rel_from_key(key, &base);
        let (dir, name) = match rel.rsplit_once('/') {
            Some((d, n)) => (d.to_string(), n.to_string()),
            None => (String::new(), rel.clone()),
        };
        texts.push((rel.clone(), of.text.clone()));
        founds.push(workspace::Found {
            path: PathBuf::from(key), rel, dir, name, bytes: of.text.len(),
        });
    }

    let known: Vec<String> = founds.iter().map(|f| f.rel.clone()).collect();
    let cdir = workspace::conf_dir(&known);
    let raw = workspace::include_map(&texts);
    let mut included: std::collections::BTreeSet<String> = Default::default();
    let mut resolved: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> = Default::default();
    for (from, targets) in &raw {
        let mut ok = Vec::new();
        let mut miss = Vec::new();
        for t in targets {
            match workspace::resolve_include(&cdir, t, &known) {
                Some(r) => { included.insert(r.clone()); ok.push(r); }
                None => miss.push(t.clone()),
            }
        }
        resolved.insert(from.clone(), (ok, miss));
    }

    let files: Vec<WsFile> = state.with_pack(|pk| {
        founds
            .iter()
            .filter_map(|f| {
                let of = store.get(&f.path.to_string_lossy().to_string())?;
                let (inc, miss) = resolved.get(&f.rel).cloned().unwrap_or_default();
                Some(build_file(f, &of.text, &of.on_disk, inc, miss, included.contains(&f.rel), pk))
            })
            .collect()
    });

    let folders = match &host {
        Some(_) => {
            let mut d: Vec<String> =
                files.iter().filter(|f| !f.dir.is_empty()).map(|f| f.dir.clone()).collect();
            d.sort();
            d.dedup();
            d
        }
        None => workspace::folders(&root),
    };

    Ok(Workspace {
        root: match &host {
            Some(h) => format!("{} : {}", h.name, base),
            None => root.to_string_lossy().to_string(),
        },
        files,
        pack: state.status(),
        remote: host.as_ref().map(|h| format!("{} \u{2014} {}", h.name, h.target)),
        folders,
    })
}

/* ------------------------------------------------------------------ *
 * The server's own log, and moving what is in it into the config.
 * ------------------------------------------------------------------ */

/// Every log file the configuration says the server writes to.
///
/// Paths in a `log` block are relative to the install directory, which is the
/// parent of `conf/`. Anything absolute is taken as written.
#[tauri::command]
fn log_files(state: State<'_, AppState>) -> Result<Vec<logs::LogFile>, String> {
    let root = root_of(&state)?;
    let host = state.source.lock().unwrap().clone();

    // Rebuild just enough of the workspace to read its log blocks.
    let mut declared: Vec<(String, String, bool)> = Vec::new();
    {
        let files = state.files.lock().unwrap();
        let base = match &host {
            Some(h) => h.root.trim_end_matches('/').to_string(),
            None => root.to_string_lossy().to_string(),
        };
        for (key, of) in files.iter() {
            let rel = rel_from_key(key, &base);
            let doc = Doc::parse(of.text.clone());
            let entries: Vec<UiEntry> = state
                .with_pack(|pk| doc.root.entries().map(|n| build(&doc, n, 0, "", pk, false)).collect());
            logs::declared_files(&entries, &rel, &mut declared);
        }
    }

    // `logs/ircd.log` is where it goes when nobody said otherwise.
    if declared.is_empty() {
        declared.push(("logs/ircd.log".into(), "the usual place".into(), false));
    }
    declared.sort();
    declared.dedup();

    let install = match &host {
        Some(h) => {
            let b = h.root.trim_end_matches('/');
            b.strip_suffix("/conf").unwrap_or(b).to_string()
        }
        None => {
            let r = if root.file_name().map(|n| n == "conf").unwrap_or(false) {
                root.parent().unwrap_or(&root).to_path_buf()
            } else {
                root.clone()
            };
            r.to_string_lossy().to_string()
        }
    };

    let mut out = Vec::new();
    for (name, declared_in, json) in declared {
        let absolute = name.starts_with('/') || name.contains(":\\") || name.starts_with('\\');
        // `file "ircd.log";` is resolved by the server against LOGDIR, which is
        // the install's logs/ directory — see convert_to_absolute_path in
        // src/log.c. Joining it to the install root instead looks for a file
        // that is never there.
        let path = if absolute {
            name.clone()
        } else if host.is_some() {
            format!("{}/logs/{}", install.trim_end_matches('/'), name)
        } else {
            std::path::Path::new(&install).join("logs").join(&name).to_string_lossy().to_string()
        };

        let (exists, bytes) = match &host {
            // Asking the far end about every candidate would be a round trip
            // each; let the read find out instead.
            Some(_) => (true, 0),
            None => match std::fs::metadata(&path) {
                Ok(m) => (true, m.len()),
                Err(_) => (false, 0),
            },
        };
        out.push(logs::LogFile { name, path, exists, bytes, declared_in, json });
    }
    // Every example.*.conf names the same two files. One entry each.
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    out.retain(|f| seen.insert(f.path.clone()));
    // The JSON ones first: they are the only ones with anything to drag out.
    out.sort_by_key(|f| (!f.json, !f.exists));
    Ok(out)
}

/// Read the end of a log. `limit` is in lines.
#[tauri::command]
fn read_log(
    state: State<'_, AppState>,
    path: String,
    limit: usize,
) -> Result<logs::LogRead, String> {
    let limit = limit.clamp(1, 20_000);
    let host = state.source.lock().unwrap().clone();

    let text = match host {
        Some(h) => remote::tail(&h, &path, limit)?,
        None => {
            let meta = std::fs::metadata(&path)
                .map_err(|e| format!("Can't read {path}: {e}"))?;
            // Reading 400MB to show the last two thousand lines would be rude.
            const TAIL: u64 = 8 * 1024 * 1024;
            if meta.len() > TAIL {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                f.seek(SeekFrom::End(-(TAIL as i64))).map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                // The first line is almost certainly cut in half; drop it.
                let s = String::from_utf8_lossy(&buf).to_string();
                match s.find('\n') {
                    Some(i) => s[i + 1..].to_string(),
                    None => s,
                }
            } else {
                std::fs::read_to_string(&path).map_err(|e| format!("Can't read {path}: {e}"))?
            }
        }
    };

    Ok(logs::parse(&path, &text, limit))
}

/// Work out what dropping a fact onto a block would write, without writing it.
///
/// Refusing with a reason matters more than refusing: "a class block has
/// nowhere to put an address" teaches something, a dead drop target does not.
#[tauri::command]
fn plan_drop(
    state: State<'_, AppState>,
    file: String,
    start: usize,
    kind: String,
    value: String,
) -> Result<logs::DropPlan, String> {
    let files = state.files.lock().unwrap();
    let of = files.get(&file).ok_or("That file isn't open.")?;
    let doc = Doc::parse(of.text.clone());
    let node = find_entry(&doc.root, start).ok_or("Couldn't find that block.")?;

    let entry: UiEntry = state.with_pack(|pk| build(&doc, node, 0, "", pk, false));
    let mut target = &entry;
    let block_label = |e: &UiEntry| match &e.label {
        Some(l) if !l.is_empty() => format!("{} {}", e.name, l),
        _ => e.name.clone(),
    };
    let outer = block_label(&entry);

    // `oper bob { mask { ... } }` — the mask is where a fact goes, not the
    // oper block itself. Drop on the outside, land in the right place.
    if !types::is_matcher(&entry.path) {
        if let Some(m) = entry
            .children
            .iter()
            .find(|c| (c.name == "mask" || c.name == "match") && c.has_block)
        {
            target = m;
        }
    }

    if !target.has_block {
        return Err(format!("{outer} isn't a block, so nothing can go inside it."));
    }

    let (directive, as_block) = if types::is_matcher(&target.path) {
        match logs::criterion_for(&kind) {
            Some(c) => {
                let is_list = types::criterion(c).map(|x| x.kind == types::Kind::List).unwrap_or(false);
                (c.to_string(), is_list)
            }
            None => {
                return Err(format!(
                    "A matcher has no way to match on {}. Try a ban block instead.",
                    logs::label_for(&kind)
                ))
            }
        }
    } else {
        match logs::directive_for(&target.name, target.label.as_deref().unwrap_or(""), &kind) {
            Some(d) => (d.to_string(), false),
            None => {
                return Err(format!(
                    "{outer} has nowhere to put {}.",
                    logs::label_for(&kind)
                ))
            }
        }
    };

    let already = target
        .children
        .iter()
        .any(|c| c.name == directive && c.value.as_deref() == Some(value.as_str()));

    let indent = indent_at(&of.text, target.span[0]);
    let preview = if as_block {
        format!("{indent}\t{directive} {{ {value}; }}")
    } else {
        format!("{indent}\t{directive} {};", quote_if_needed(&value))
    };

    Ok(logs::DropPlan {
        file: file.clone(),
        block: block_label(target),
        directive,
        value,
        as_block,
        parent_start: target.span[0],
        preview,
        already,
    })
}

/// Quote a value if the tokenizer would otherwise read it as two things.
fn quote_if_needed(v: &str) -> String {
    let needs = v.is_empty()
        || v.chars().any(|c| c.is_whitespace() || c == ';' || c == '{' || c == '}' || c == '"');
    if needs {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

/* ------------------------------------------------------------------ *
 * The files UnrealIRCd ships, and making your own version of one.
 * ------------------------------------------------------------------ */

/// `*.default.conf` is installed by UnrealIRCd and replaced wholesale on the
/// next upgrade. Every one of them says so in its own header, in capitals.
/// Editing one is work that silently disappears, so nothing here writes to it.
fn is_default_conf(path: &str) -> bool {
    file_name_of(path).to_lowercase().ends_with(".default.conf")
}

fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// The gate every write goes through.
fn writable(path: &str) -> Result<(), String> {
    if !is_default_conf(path) {
        return Ok(());
    }
    let name = file_name_of(path);
    Err(format!(
        "{name} is installed by UnrealIRCd and replaced when you upgrade, so anything \
         you change here is lost the next time. Make your own {} instead — it is \
         included straight after this one, so it wins.",
        own_name_for(&name)
    ))
}

/// What to call your version. `.custom.conf` is the name UnrealIRCd's own
/// files suggest when they tell you to copy them.
fn own_name_for(default_name: &str) -> String {
    let stem = default_name.strip_suffix(".default.conf").unwrap_or(default_name);
    format!("{stem}.custom.conf")
}

/// Whether your version sits *beside* the shipped one or *instead of* it.
///
/// This is not a preference. Each shipped file says which it wants, and they
/// do not agree: the module lists and the operclasses are meant to be layered
/// on top, because a later block wins and `blacklist-module` and `parent`
/// exist for exactly that. snomasks.default.conf is meant to be copied and
/// swapped in, because its `log` blocks are additive — a second set alongside
/// the first would route every snomask twice.
#[derive(PartialEq, Clone, Copy)]
enum OwnKind {
    Beside,
    Instead,
}

fn own_kind(default_name: &str) -> OwnKind {
    match default_name {
        "snomasks.default.conf" => OwnKind::Instead,
        _ => OwnKind::Beside,
    }
}

fn own_template(default_name: &str) -> String {
    let own = own_name_for(default_name);
    match default_name {
        "modules.default.conf" => format!(
            "/* {own} — your own module choices.\n\
             \x20*\n\
             \x20* modules.default.conf is still included and still loads nearly\n\
             \x20* everything. This file is read straight after it, so switching a\n\
             \x20* module off here wins. That is UnrealIRCd's own recommended way\n\
             \x20* round, rather than copying the module list and maintaining it.\n\
             \x20*\n\
             \x20* https://www.unrealircd.org/docs/Blacklist-module_directive\n\
             \x20*/\n\n\
             /* Switch off a module you do not want:\n\n\
             \x20blacklist-module \"chanmodes/link\";\n\n\
             \x20*/\n\n\
             /* Load a third-party module you have installed:\n\n\
             \x20loadmodule \"third/showwebirc\";\n\n\
             \x20*/\n"
        ),
        "rpc.modules.default.conf" => format!(
            "/* {own} — your own JSON-RPC module choices.\n\
             \x20*\n\
             \x20* rpc.modules.default.conf is still included. This file is read\n\
             \x20* straight after it, so anything switched off here wins.\n\
             \x20*\n\
             \x20* https://www.unrealircd.org/docs/JSON-RPC\n\
             \x20*/\n\n\
             /* blacklist-module \"rpc/channel\"; */\n"
        ),
        "operclass.default.conf" => format!(
            "/* {own} — your own operclass blocks.\n\
             \x20*\n\
             \x20* operclass.default.conf is still included and still defines locop,\n\
             \x20* globop, admin, services-admin, netadmin and the -with-override\n\
             \x20* variants. This file is read straight after it, so you can build on\n\
             \x20* those with `parent` instead of copying a whole permissions tree.\n\
             \x20*\n\
             \x20* https://www.unrealircd.org/docs/Operclass_block\n\
             \x20* https://www.unrealircd.org/docs/Operclass_permissions\n\
             \x20*/\n\n\
             /* An operclass of your own, starting from one of the defaults:\n\n\
             operclass helper {{\n\
             \tparent locop;\n\
             \tpermissions {{\n\
             \t\tchannel {{ see; }}\n\
             \t}}\n\
             }}\n\n\
             \x20*/\n"
        ),
        "rpc-class.default.conf" => format!(
            "/* {own} — your own rpc-class blocks.\n\
             \x20*\n\
             \x20* rpc-class.default.conf is still included and still defines `full`\n\
             \x20* and `read-only`. This file is read after it, so you can inherit\n\
             \x20* from either with `parent` rather than listing every permission.\n\
             \x20*\n\
             \x20* https://www.unrealircd.org/docs/Rpc-class_block\n\
             \x20*/\n\n\
             /* rpc-class stats-only {{\n\
             \tparent read-only;\n\
             \tpermissions {{ stats; }}\n\
             }}\n\
             \x20*/\n"
        ),
        _ => format!(
            "/* {own} — your own version of {default_name}.\n\
             \x20*\n\
             \x20* The shipped file is still included; this one is read straight\n\
             \x20* after it, so what you put here wins.\n\
             \x20*/\n"
        ),
    }
}

/// Create your own version of a shipped file and wire it into unrealircd.conf.
#[tauri::command]
fn make_own(state: State<'_, AppState>, file: String) -> Result<Workspace, String> {
    let root = root_of(&state)?;
    if state.source.lock().unwrap().is_some() {
        return Err("This works on a local folder; the files are on the server.".into());
    }
    let src = PathBuf::from(&file);
    check_inside(&root, &src)?;
    let default_name = file_name_of(&file);
    if !is_default_conf(&file) {
        return Err(format!("{default_name} isn't one of UnrealIRCd's own files."));
    }

    let own_name = own_name_for(&default_name);
    let kind = own_kind(&default_name);
    let target = src.with_file_name(&own_name);
    if target.exists() {
        return Err(format!("{own_name} is already there — open that instead."));
    }

    let body = match kind {
        OwnKind::Beside => own_template(&default_name),
        // A copy to edit, with a line at the top saying where it came from.
        OwnKind::Instead => {
            let original = {
                let files = state.files.lock().unwrap();
                files.get(&file).map(|o| o.text.clone())
            }
            .unwrap_or_else(|| std::fs::read_to_string(&src).unwrap_or_default());
            format!(
                "/* {own_name} — your copy of {default_name}, which UnrealIRCd\n\
                 \x20* replaces on upgrade. unrealircd.conf includes this file instead\n\
                 \x20* of that one now, so edit away.\n\
                 \x20*/\n\n{original}"
            )
        }
    };

    std::fs::write(&target, &body).map_err(|e| format!("Could not create {own_name}: {e}"))?;

    // Wire it in. Which file to touch and how depends on whether yours sits
    // beside the shipped one or replaces it.
    if let Err(e) = wire_own(&state, &root, &default_name, &own_name, kind) {
        let _ = std::fs::remove_file(&target);
        return Err(e);
    }

    // The include line is an edit held in memory, like every other edit, and
    // is yours to save. reload() here would re-read the whole folder off the
    // disk and throw that line straight back away: the new file would appear
    // and nothing would include it.
    {
        let mut files = state.files.lock().unwrap();
        files.insert(
            target.to_string_lossy().to_string(),
            OpenFile {
                stamp: stamp_of(&target.to_string_lossy()),
                on_disk: body.clone(),
                text: body,
            },
        );
    }
    workspace_from_memory(&state)
}

/// Put the include line in unrealircd.conf, and say so if it cannot.
///
/// It has to be unrealircd.conf specifically. The shipped files are included
/// by the example configs too, and looking for "whichever open file includes
/// it" finds conf/examples/example.conf first — which is alphabetically
/// earlier, is not what the server reads, and is a file the user has never
/// opened. Wiring into that silently does nothing at all.
fn wire_own(
    state: &AppState,
    _root: &Path,
    default_name: &str,
    own_name: &str,
    kind: OwnKind,
) -> Result<(), String> {
    let key = {
        let files = state.files.lock().unwrap();
        files
            .keys()
            .find(|k| file_name_of(k).eq_ignore_ascii_case("unrealircd.conf"))
            .cloned()
    }
    .ok_or("There's no unrealircd.conf here to include it from.")?;

    // Where the shipped file is included from unrealircd.conf, if it is.
    let (value_span, span, last_include_end) = {
        let files = state.files.lock().unwrap();
        let of = files.get(&key).ok_or("That file isn't open.")?;
        let doc = Doc::parse(of.text.clone());
        let entries: Vec<UiEntry> = state
            .with_pack(|pk| doc.root.entries().map(|n| build(&doc, n, 0, "", pk, false)).collect());
        let mine = entries
            .iter()
            .find(|e| e.name == "include" && e.value.as_deref() == Some(default_name));
        let last = entries
            .iter()
            .filter(|e| e.name == "include")
            .map(|e| e.span[1])
            .max();
        (mine.and_then(|e| e.value_span), mine.map(|e| e.span), last)
    };

    if kind == OwnKind::Instead {
        let Some(vs) = value_span else {
            return Err(format!(
                "unrealircd.conf does not include {default_name}, so there is no                  include line to point at {own_name} instead. Add one by hand."
            ));
        };
        remember(state, &format!("including {own_name}"), std::slice::from_ref(&key));
        let mut files = state.files.lock().unwrap();
        let of = files.get_mut(&key).ok_or("That file isn't open.")?;
        let mut d = Doc::parse(of.text.clone());
        d.edit(vs[0]..vs[1], format!("\"{own_name}\""))?;
        of.text = d.render();
        return Ok(());
    }

    // Straight after the shipped file's own include, so yours is read second
    // and therefore wins. Failing that, after the last include there is.
    remember(state, &format!("including {own_name}"), std::slice::from_ref(&key));
    let mut files = state.files.lock().unwrap();
    let of = files.get_mut(&key).ok_or("That file isn't open.")?;
    let anchor = span.map(|s| s[1]).or(last_include_end);
    match anchor {
        Some(end) => {
            let at = match of.text[end..].find('\n') {
                Some(i) => end + i + 1,
                None => of.text.len(),
            };
            let mut d = Doc::parse(of.text.clone());
            d.edit(at..at, format!("include \"{own_name}\";\n"))?;
            of.text = d.render();
        }
        None => {
            let lead = if of.text.ends_with('\n') || of.text.is_empty() { "" } else { "\n" };
            of.text = format!("{}{lead}include \"{own_name}\";\n", of.text);
        }
    }
    Ok(())
}

/// One coloured run in the raw file view.
#[derive(Serialize, Clone)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    /// name | label | value | string | comment | preproc | brace | semi | extra
    pub kind: &'static str,
    /// How deep the block is, so a top-level name can read differently from a
    /// setting four levels in.
    pub depth: usize,
}

/// Colour the file using the parser that reads it, rather than a second set of
/// rules in the interface that could quietly disagree with the first. A word
/// shown as a block name here is a block name to conf.c too.
#[tauri::command]
fn highlight(state: State<'_, AppState>, file: String) -> Result<Vec<Token>, String> {
    let files = state.files.lock().unwrap();
    let of = files.get(&file).ok_or("That file isn't open.")?;
    let doc = Doc::parse(of.text.clone());
    let mut out = Vec::new();
    walk_tokens(&doc.root, 0, &mut out);
    Ok(out)
}

fn walk_tokens(n: &Node, depth: usize, out: &mut Vec<Token>) {
    // `oper bobsmith { .. }` — the word after the name is what the block is
    // called, and it reads quite differently from `set { ident 10; }` where
    // the word after the name is a setting. The difference is whether there is
    // a block, so ask the entry rather than counting words.
    let labelled = n.kind == Kind::Entry && n.children.iter().any(|c| c.kind == Kind::Block);
    let mut first_value = true;

    let inner = if n.kind == Kind::Block { depth + 1 } else { depth };
    for c in &n.children {
        let kind = match c.kind {
            Kind::Name => "name",
            Kind::Value => {
                let is_label = labelled && first_value;
                first_value = false;
                if c.quote.is_some() {
                    if is_label { "label" } else { "string" }
                } else if is_label {
                    "label"
                } else {
                    "value"
                }
            }
            Kind::Extra => "extra",
            Kind::LBrace | Kind::RBrace => "brace",
            Kind::Semi => "semi",
            Kind::LineComment | Kind::BlockComment => "comment",
            Kind::Preproc => "preproc",
            _ => "",
        };
        if !kind.is_empty() {
            out.push(Token { start: c.span.start, end: c.span.end, kind, depth: inner });
        }
        walk_tokens(c, inner, out);
    }
}

fn find_entry(n: &Node, start: usize) -> Option<&Node> {
    if n.kind == Kind::Entry && n.span.start == start {
        return Some(n);
    }
    n.children.iter().find_map(|c| find_entry(c, start))
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn indent_at(text: &str, at: usize) -> String {
    let ls = line_start(text, at);
    text[ls..at].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = AppState::default();
            let arg = std::env::args().skip(1).find(|a| !a.starts_with('-'));
            *state.pending.lock().unwrap() = arg;
            app.manage(state);

            // Documentation loads off the main thread so the window paints
            // straight away; the frontend reopens when it lands.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Some(root) = load_settings().source_root {
                    let st = handle.state::<AppState>();
                    let _ = st.load_pack(Path::new(&root));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_workspace,
            pick_folder,
            startup_workspace,
            reopen,
            set_value,
            set_bare_value,
            disk_changes,
            disk_diff,
            reload_file,
            keep_mine,
            add_child,
            add_block,
            comment_out,
            save_file,
            save_all,
            undo,
            redo,
            history_status,
            merge_preview,
            split_preview,
            apply_preview,
            mergeable,
            problems,
            delete_entry,
            block_types,
            child_directives,
            known_settings,
            hosts,
            save_host,
            forget_host,
            probe_host,
            open_remote,
            server_command,
            new_file,
            duplicate_file,
            rename_file,
            delete_file,
            move_block,
            get_config,
            set_config,
            pack_status,
            vocabulary,
            pick_source_root,
            hint_for,
            new_folder,
            rename_folder,
            delete_folder,
            move_file,
            block_text,
            paste_block,
            paste_into,
            copy_block,
            add_include,
            reveal_path,
            open_external,
            recent_folders,
            diff_files,
            tabs,
            new_tab,
            switch_tab,
            close_tab,
            log_files,
            read_log,
            plan_drop,
            make_own,
            highlight,
            delete_entries,
            comment_out_many,
            move_blocks
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r"C:\Users\Valerie\unrealircd-pr";

    fn count(e: &UiEntry, total: &mut usize, with: &mut usize, samples: &mut Vec<String>) {
        *total += 1;
        if let Some(h) = &e.hint {
            *with += 1;
            if samples.len() < 14 && !e.has_block {
                samples.push(format!("{:<38} {}", e.path, h));
            }
        }
        for c in &e.children {
            count(c, total, with, samples);
        }
    }

    /// The question that matters: open the config people actually start from,
    /// and see how much of it the app can explain.
    #[test]
    fn the_shipped_example_gets_explained() {
        let root = Path::new(SRC);
        if !root.exists() {
            eprintln!("no source tree, skipping");
            return;
        }
        let pk = pack::build(root, None).expect("pack");
        let text = std::fs::read_to_string(root.join("doc/conf/examples/example.conf")).unwrap();
        let doc = view("example.conf", &text, &text, Some(&pk));

        let (mut total, mut with, mut samples) = (0usize, 0usize, Vec::new());
        for e in &doc.entries {
            count(e, &mut total, &mut with, &mut samples);
        }
        println!("\n{with} of {total} entries in example.conf have an explanation\n");
        for s in &samples {
            println!("  {s}");
        }

        let unknown: Vec<&str> = doc.entries.iter().filter(|e| e.unknown).map(|e| e.name.as_str()).collect();
        println!("\ntop-level blocks not in this release: {unknown:?}");

        assert!(with * 2 > total, "fewer than half of the entries could be explained");
        assert!(unknown.is_empty(), "the shipped example should use only real blocks");
    }
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    #[test]
    fn text_lands_at_its_new_depth() {
        // Copied from the top level of a file...
        let copied = "permissions {\n\tchannel { override; }\n}";
        // ...and pasted two levels in.
        let got = reindent(copied, "\t\t");
        assert_eq!(got, "\t\tpermissions {\n\t\t\tchannel { override; }\n\t\t}\n");
    }

    #[test]
    fn a_deeper_copy_keeps_its_own_shape() {
        let copied = "\t\t\tparent netadmin;\n\t\t\tpermissions { chat; }";
        assert_eq!(reindent(copied, "\t"), "\tparent netadmin;\n\tpermissions { chat; }\n");
    }

    #[test]
    fn blank_lines_stay_blank_and_relative_shape_is_kept() {
        // The outermost line sets the base, so the second line keeps the one
        // space it was indented by.
        assert_eq!(reindent("a;\n\n b;", "  "), "  a;\n\n   b;\n");
    }
}

#[cfg(test)]
mod add_child_tests {
    use super::*;

    /// What `add_child` would write, without the app or a file on disk.
    fn add(text: &str, parent_start: usize, line: &str) -> String {
        let mut doc = Doc::parse(text.to_string());
        let (range, indent, lead, tail) = {
            let parent = find_entry(&doc.root, parent_start).expect("parent");
            block_insertion(&doc.render(), parent).expect("insertion point")
        };
        doc.edit(range, format!("{lead}{indent}{line}\n{tail}")).expect("edit");
        doc.render()
    }

    fn start_of(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap_or_else(|| panic!("no {needle} in {text}"))
    }

    #[test]
    fn a_block_already_spread_over_lines_gains_a_line() {
        let src = "oper lol {\n\tauto-login yes;\n}\n";
        let got = add(src, start_of(src, "oper"), "vhost x.example.org;");
        assert_eq!(got, "oper lol {\n\tauto-login yes;\n\tvhost x.example.org;\n}\n");
    }

    /// The one that was wrong. `mask { }` keeps its closing brace on the same
    /// line, so inserting at the start of that line put the new directive
    /// above the whole block — outside the thing you were adding to.
    #[test]
    fn a_block_written_on_one_line_is_opened_out() {
        let src = "oper lol {\n\tauto-login yes;\n\tmask { }\n}\n";
        let got = add(src, start_of(src, "mask"), "account { }");
        assert_eq!(got,
            "oper lol {\n\tauto-login yes;\n\tmask {\n\t\taccount { }\n\t}\n}\n",
            "\n--- got ---\n{got}");
        // And the thing that made it a bug: it is inside, not above.
        assert!(got.find("account").unwrap() > got.find("mask {").unwrap());
    }

    #[test]
    fn a_one_line_block_keeps_what_was_already_in_it() {
        let src = "operclass helper {\n\tpermissions {\n\t\tchat { globops; }\n\t}\n}\n";
        let got = add(src, start_of(src, "chat"), "wallops;");
        assert_eq!(got,
            "operclass helper {\n\tpermissions {\n\t\tchat {\n\t\t\tglobops;\n\t\t\twallops;\n\t\t}\n\t}\n}\n",
            "\n--- got ---\n{got}");
    }

    #[test]
    fn an_empty_block_on_two_lines_still_works() {
        let src = "oper lol {\n}\n";
        let got = add(src, start_of(src, "oper"), "class clients;");
        assert_eq!(got, "oper lol {\n\tclass clients;\n}\n");
    }

    /// Whatever it writes has to parse, and has to parse as the same shape.
    #[test]
    fn the_result_is_still_valid_configuration() {
        let src = "oper lol {\n\tmask { }\n}\n";
        let got = add(src, start_of(src, "mask"), "account { }");
        let doc = Doc::parse(got.clone());
        assert!(!doc.diagnostics.iter().any(|d| d.severity == Severity::Error),
                "{:?}\n{got}", doc.diagnostics);
        let oper = doc.root.entries().next().expect("an oper block");
        let mask = oper.block().unwrap().entries().next().expect("a mask inside it");
        assert_eq!(doc.word_text(mask.first(Kind::Name).unwrap()), "mask");
        let account = mask.block().unwrap().entries().next().expect("an account inside the mask");
        assert_eq!(doc.word_text(account.first(Kind::Name).unwrap()), "account");
    }

    /// A list item is content, not a directive name, so it has to survive
    /// being written: `account { "Bob Smith"; }` parses, `Bob Smith;` does not.
    #[test]
    fn a_list_item_that_needs_quoting_gets_it() {
        assert_eq!(quote_if_needed("bobsmith"), "bobsmith");
        assert_eq!(quote_if_needed("Bob Smith"), "\"Bob Smith\"");
        assert_eq!(quote_if_needed("weird;name"), "\"weird;name\"");
        // And an ordinary directive name is left exactly alone.
        for n in ["auto-login", "server-notice-show-event", "maxperip", "tls"] {
            assert_eq!(quote_if_needed(n), n);
        }
    }

    #[test]
    fn adding_a_name_to_a_list_writes_a_bare_line() {
        let src = "oper lol {
	mask {
		account { }
	}
}
";
        let got = add(src, start_of(src, "account"), &format!("{};", quote_if_needed("Bob Smith")));
        assert_eq!(got,
            "oper lol {
	mask {
		account {
			\"Bob Smith\";
		}
	}
}
",
            "
--- got ---
{got}");
        let doc = Doc::parse(got.clone());
        assert!(!doc.diagnostics.iter().any(|d| d.severity == Severity::Error), "{got}");
    }

    /// A criterion written bare has no value span, so the inspector had no
    /// field to offer. This is how it gets one.
    #[test]
    fn a_bare_directive_can_be_given_a_value() {
        let src = "oper lol {\n\tmask {\n\t\taccount;\n\t}\n}\n";
        let mut doc = Doc::parse(src.to_string());
        let at = {
            let e = find_entry(&doc.root, start_of(src, "account")).expect("entry");
            e.first(Kind::Name).unwrap().span.end
        };
        doc.edit(at..at, format!(" {}", quote_if_needed("bobsmith"))).unwrap();
        assert_eq!(doc.render(), "oper lol {\n\tmask {\n\t\taccount bobsmith;\n\t}\n}\n");
    }

    #[test]
    fn a_value_with_a_space_in_it_is_quoted_when_filled_in() {
        let src = "mask {\n\trealname;\n}\n";
        let mut doc = Doc::parse(src.to_string());
        let at = {
            let e = find_entry(&doc.root, start_of(src, "realname")).expect("entry");
            e.first(Kind::Name).unwrap().span.end
        };
        doc.edit(at..at, format!(" {}", quote_if_needed("a real name"))).unwrap();
        assert_eq!(doc.render(), "mask {\n\trealname \"a real name\";\n}\n");
        let d = Doc::parse(doc.render());
        assert!(!d.diagnostics.iter().any(|x| x.severity == Severity::Error));
    }

    /// Indentation is taken from the block being added to, not assumed.
    #[test]
    fn spaces_are_not_turned_into_tabs() {
        let src = "oper lol {\n    mask { }\n}\n";
        let got = add(src, start_of(src, "mask"), "account { }");
        assert!(got.contains("    mask {\n"), "\n{got}");
        assert!(got.contains("\n    }"), "the closing brace lines up again:\n{got}");
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;

    /// A throwaway folder with one config file in it.
    fn scratch(name: &str, body: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("confetti-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("test.conf");
        std::fs::write(&f, body).unwrap();
        (dir, f.to_string_lossy().to_string())
    }

    fn text_of_file(state: &AppState, path: &str) -> String {
        state.files.lock().unwrap().get(path).unwrap().text.clone()
    }

    /// Re-reading the folder must not throw away what you have not saved.
    ///
    /// Undo restores your text and then calls reload(), which used to replace
    /// every open file with whatever was on disk — so Ctrl+Z quietly meant
    /// "revert the whole workspace to disk" instead of "take back that one
    /// change". new_file, rename and delete all went through the same door.
    #[test]
    fn reading_the_folder_again_keeps_unsaved_work() {
        let (dir, path) = scratch("keep", "me {\n\tname a;\n}\n");
        let state = AppState::default();
        read_workspace(&state, &dir).unwrap();

        // An edit nobody has saved.
        {
            let mut files = state.files.lock().unwrap();
            files.get_mut(&path).unwrap().text = "me {\n\tname b;\n}\n".to_string();
        }
        read_workspace(&state, &dir).unwrap();
        assert_eq!(text_of_file(&state, &path), "me {\n\tname b;\n}\n",
                   "re-reading the folder discarded an unsaved edit");
        // And it still knows what is actually on disk.
        assert_eq!(state.files.lock().unwrap().get(&path).unwrap().on_disk,
                   "me {\n\tname a;\n}\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole cycle the "changed on disk" bar offers: you have edits, the
    /// file changes underneath, you take theirs, you change your mind.
    #[test]
    fn take_theirs_then_undo_gives_your_version_back() {
        let (dir, path) = scratch("undo", "me {\n\tname mine;\n}\n");
        let state = AppState::default();
        read_workspace(&state, &dir).unwrap();

        // You edit, without saving.
        let ours = "me {\n\tname ours;\n}\n".to_string();
        remember(&state, "editing", std::slice::from_ref(&path));
        state.files.lock().unwrap().get_mut(&path).unwrap().text = ours.clone();

        // Somebody else writes the file.
        let theirs = "me {\n\tname theirs;\n}\n";
        std::fs::write(&path, theirs).unwrap();

        // Confetti notices.
        let changes = {
            let mut open = state.files.lock().unwrap();
            let of = open.get_mut(&path).unwrap();
            let disk = std::fs::read_to_string(&path).unwrap();
            (disk != of.on_disk, of.text != of.on_disk)
        };
        assert_eq!(changes, (true, true), "the change and the clash are both real");

        // You take theirs.
        remember(&state, "reloading from disk", std::slice::from_ref(&path));
        {
            let mut open = state.files.lock().unwrap();
            let of = open.get_mut(&path).unwrap();
            of.text = theirs.to_string();
            of.on_disk = theirs.to_string();
        }
        assert_eq!(text_of_file(&state, &path), theirs);

        // Then change your mind. This is the step that silently did nothing.
        let snap = state.history.lock().unwrap().pop_undo().unwrap();
        assert_eq!(snap.label, "reloading from disk");
        swap_in(&state, snap);
        read_workspace(&state, &dir).unwrap();
        assert_eq!(text_of_file(&state, &path), ours, "undo did not give your version back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that has not been touched is not reported, however often we look.
    #[test]
    fn an_untouched_file_is_never_reported() {
        let (dir, path) = scratch("quiet", "me {\n\tname a;\n}\n");
        let state = AppState::default();
        read_workspace(&state, &dir).unwrap();
        let before = state.files.lock().unwrap().get(&path).unwrap().stamp.clone();
        assert!(before.is_some(), "a local file gets a stamp");
        // Same contents, new mtime — touched, not changed.
        std::fs::write(&path, "me {\n\tname a;\n}\n").unwrap();
        let disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(disk, state.files.lock().unwrap().get(&path).unwrap().on_disk,
                   "the contents are what decides, not the timestamp");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod value_and_block_tests {
    use super::*;

    /// A directive can have a value *and* a block: the log block's
    /// `file "ircd.log" { maxsize 100M; }` names a file and then says things
    /// about that file. Confetti assumed a second word was either a label on a
    /// block or a value on a plain line, never both — so the filename had no
    /// span and could not be edited, and the middle panel showed "1 setting"
    /// where the name of the file should be.
    fn dig<'a>(e: &'a UiEntry, path: &[&str]) -> &'a UiEntry {
        let mut cur = e;
        for want in path {
            cur = cur.children.iter().find(|c| &c.name == want)
                .unwrap_or_else(|| panic!("no {want} under {}", cur.name));
        }
        cur
    }

    fn parse(text: &str) -> Vec<UiEntry> {
        let doc = Doc::parse(text.to_string());
        assert!(!doc.diagnostics.iter().any(|d| d.severity == Severity::Error),
                "{:?}", doc.diagnostics);
        doc.root.entries().map(|e| build(&doc, e, 1, "", None, false)).collect()
    }

    #[test]
    fn a_value_with_a_block_keeps_both() {
        let src = "log {
	destination {
		file \"ircd.log\" { maxsize 100M; }
	}
}
";
        let top = parse(src);
        let f = dig(&top[0], &["destination", "file"]);
        assert!(f.has_block, "it has a block");
        assert_eq!(f.label.as_deref(), Some("ircd.log"), "and the file it names");
        assert_eq!(f.children.len(), 1, "and what it says about it");
        assert_eq!(f.children[0].name, "maxsize");
        assert_eq!(f.children[0].value.as_deref(), Some("100M"));
        // The thing that was missing: somewhere to click to change it.
        assert!(f.value_span.is_some(), "the file name has no span, so it cannot be edited");
        let sp = f.value_span.unwrap();
        assert_eq!(&src[sp[0]..sp[1]], "\"ircd.log\"", "and the span covers the name itself");
    }

    #[test]
    fn a_label_on_a_block_can_still_be_edited() {
        let src = "oper bobsmith {
	class clients;
}
";
        let top = parse(src);
        assert_eq!(top[0].label.as_deref(), Some("bobsmith"));
        let sp = top[0].value_span.expect("a block label needs a span too");
        assert_eq!(&src[sp[0]..sp[1]], "bobsmith");
    }

    #[test]
    fn a_block_with_no_second_word_has_nothing_to_edit() {
        let src = "log {
	destination { file \"x.log\"; }
}
";
        let top = parse(src);
        let d = dig(&top[0], &["destination"]);
        assert!(d.has_block);
        assert_eq!(d.label, None);
        assert_eq!(d.value_span, None, "there is no second word, so there is no span");
        // And the plain value inside it still behaves as a plain value.
        let f = dig(d, &["file"]);
        assert!(!f.has_block);
        assert_eq!(f.value.as_deref(), Some("x.log"));
        assert!(f.value_span.is_some());
    }
}
