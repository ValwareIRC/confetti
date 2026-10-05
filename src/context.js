/* Confetti — right-click, everywhere.
 *
 * Before this file, six surfaces had a menu and everything else fell through to
 * WebView2's own — Back, Refresh, Save as, Print, Inspect — which is a browser
 * menu in the middle of a desktop application. Nothing falls through now: a
 * global handler catches anything the specific menus did not claim and offers
 * something that belongs to the surface you actually clicked.
 *
 * The one thing the browser menu was genuinely providing is Cut/Copy/Paste in
 * text fields, so that is the first thing routed and it is a real edit menu.
 */

/* ---------------- clipboard ---------------- */

/* An internal copy alongside the system clipboard. Reading the system
   clipboard can be refused by the webview; copying and pasting a block within
   Confetti has to work regardless, so both are kept and the internal one wins
   when it is fresher. */
state.clip = null;

async function putClip(text, what) {
  state.clip = { text, what, at: Date.now() };
  try { await navigator.clipboard.writeText(text); } catch (e) { /* internal only */ }
}
async function getClip() {
  try {
    const t = await navigator.clipboard.readText();
    if (t && t.trim()) return t;
  } catch (e) { /* fall through to ours */ }
  return state.clip ? state.clip.text : "";
}
function clipLabel() {
  return state.clip ? `Paste ${state.clip.what}` : "Paste a block here";
}

/* ---------------- shared item builders ---------------- */

function foldersOf() {
  if (!state.ws) return [];
  const seen = new Set(state.ws.folders || []);
  for (const f of state.ws.files) if (f.dir) seen.add(f.dir);
  return [...seen].sort();
}

/* "Move to" / "Copy to" / "Include from" all want the same list of somewhere
   else, so they are built once. */
function fileTargets(exceptPath, run) {
  return state.ws.files
    .filter((x) => x.path !== exceptPath)
    .slice(0, 20)
    .map((x) => ({ label: x.rel, run: () => run(x) }));
}

function folderTargets(exceptDir, run) {
  const items = [];
  if (exceptDir !== "") items.push({ label: "the top level", run: () => run("") });
  for (const d of foldersOf()) {
    if (d === exceptDir) continue;
    items.push({ label: d + "/", run: () => run(d) });
  }
  return items.length ? items : [{ label: "nowhere else yet", disabled: true, hint: "Make a folder first" }];
}

function foldAllFolders(on) {
  if (on) for (const d of foldersOf()) state.folders.add(d);
  else state.folders.clear();
  renderTree();
}

/* ---------------- text fields ---------------- */

/* execCommand is deprecated and still the only thing that will cut or paste
   into a focused field from a menu without a permission prompt. The async
   Clipboard API is used where it works and this is the fallback. */
function inputMenu(ev, el0) {
  const isInput = el0.tagName === "INPUT" || el0.tagName === "TEXTAREA";
  const some = isInput ? el0.selectionStart !== el0.selectionEnd : !window.getSelection().isCollapsed;
  const empty = isInput ? !el0.value : !el0.textContent;
  const ro = isInput && (el0.readOnly || el0.disabled);

  menuAt(ev, [
    { label: "Cut", key: "Ctrl X", disabled: !some || ro,
      hint: ro ? "This field can't be edited" : some ? "" : "Select something first",
      run: () => { el0.focus(); document.execCommand("cut"); } },
    { label: "Copy", key: "Ctrl C", disabled: !some,
      hint: some ? "" : "Select something first",
      run: () => { el0.focus(); document.execCommand("copy"); } },
    { label: "Paste", key: "Ctrl V", disabled: ro,
      run: async () => {
        el0.focus();
        const t = await getClip();
        if (!document.execCommand("paste") && t) {
          // execCommand paste is blocked in most webviews; do it by hand.
          if (isInput) {
            const a = el0.selectionStart, b = el0.selectionEnd;
            el0.value = el0.value.slice(0, a) + t + el0.value.slice(b);
            el0.selectionStart = el0.selectionEnd = a + t.length;
            el0.dispatchEvent(new Event("input", { bubbles: true }));
          }
        }
      } },
    "-",
    { label: "Select all", key: "Ctrl A", disabled: empty,
      run: () => { el0.focus(); if (isInput) el0.select(); else document.execCommand("selectAll"); } },
    { label: "Clear it", disabled: empty || ro, danger: true,
      run: () => { el0.value = ""; el0.dispatchEvent(new Event("input", { bubbles: true })); el0.focus(); } },
  ]);
}

/* ---------------- the tree ---------------- */

/* Right-clicking the tree's background, its header, or the "everything" row.
   This is the menu that used to be Back / Refresh / Print. */
function treeMenu(ev) {
  const anyFolded = foldersOf().some((d) => state.folders.has(d));
  menuAt(ev, [
    { heading: state.ws ? state.ws.root.replace(/\\/g, "/") : "nothing open" },
    { label: "Collapse all folders", disabled: !foldersOf().length,
      run: () => foldAllFolders(true) },
    { label: "Expand all folders", disabled: !anyFolded,
      run: () => foldAllFolders(false) },
    "-",
    { label: "New file at the top level…", disabled: !state.ws, run: () => newFile("") },
    { label: "New folder at the top level…", disabled: !state.ws, run: () => newFolder("") },
    "-",
    { label: "Show everything", check: state.file === null,
      hint: "One list across every file",
      disabled: state.view !== "console",
      run: () => { state.file = null; state.sel = null; render(); } },
    "-",
    { label: "Reveal this folder", disabled: !state.ws || !!(state.ws && state.ws.remote),
      hint: state.ws && state.ws.remote ? "The files are on the server" : "Open it in your file manager",
      run: () => reveal(state.ws.root) },
    { label: "Reread everything from disk", disabled: !state.ws, danger: true,
      run: () => rereadFromDisk() },
    { label: "Open a different folder…", run: () => doIt("btn-open") },
  ]);
}

function folderMenu(ev, dir) {
  const inside = state.ws.files.filter((f) => f.dir === dir || f.dir.startsWith(dir + "/"));
  const folded = state.folders.has(dir);
  const blocks = inside.reduce((n, f) => n + f.blocks, 0);

  menuAt(ev, [
    { heading: `${dir}/  —  ${inside.length} file${inside.length === 1 ? "" : "s"}, ${blocks} blocks` },
    { label: folded ? "Expand it" : "Collapse it",
      run: () => { if (folded) state.folders.delete(dir); else state.folders.add(dir); renderTree(); } },
    { label: "Collapse everything else",
      hint: "Leaves just this one open",
      run: () => { foldAllFolders(true); state.folders.delete(dir); renderTree(); } },
    { label: "Collapse all folders", run: () => foldAllFolders(true) },
    { label: "Expand all folders", run: () => foldAllFolders(false) },
    "-",
    { label: "New file in here…", run: () => newFile(dir) },
    { label: "New folder in here…", run: () => newFolder(dir) },
    "-",
    { label: "Copy its path", run: () => putClip(dir, "a path") },
    { label: "Reveal it", disabled: !!state.ws.remote,
      hint: state.ws.remote ? "The files are on the server" : "",
      run: () => reveal(state.ws.root.replace(/\\/g, "/") + "/" + dir) },
    "-",
    { label: "Rename it…", run: () => renameFolder(dir) },
    { label: "Remove it…", danger: true,
      hint: `${inside.length} file${inside.length === 1 ? "" : "s"} go with it`,
      run: () => removeFolder(dir, inside.length) },
  ]);
}

/* ---------------- folder actions ---------------- */

async function newFolder(dir) {
  const r = await ask({
    title: "New folder",
    blurb: dir ? `Inside ${dir}/.` : "At the top level of the config folder.",
    fields: [{ name: "name", label: "Call it", placeholder: "networks", required: true }],
    okLabel: "Create it",
  });
  if (r) act(() => invoke("new_folder", { dir, name: r.name }), true);
}

async function renameFolder(dir) {
  const short = dir.split("/").pop();
  const r = await ask({
    title: `Rename ${short}/`,
    blurb: "Every include line that names a file in here will need the same change.",
    fields: [{ name: "name", label: "Folder name", value: short, required: true }],
    okLabel: "Rename it",
  });
  if (r && r.name !== short) act(() => invoke("rename_folder", { dir, name: r.name }), true);
}

async function removeFolder(dir, n) {
  const ok = await confirmAsk({
    title: `Remove ${dir}/?`,
    body: n
      ? `${n} file${n === 1 ? "" : "s"} go with it, and anything that includes them stops working.`
      : "There is nothing in it.",
    note: "It is renamed rather than deleted — it stays on disk as a ~ folder beside this one.",
    okLabel: "Remove it",
    danger: true,
  });
  if (ok) act(() => invoke("delete_folder", { dir }), true);
}

async function rereadFromDisk() {
  const dirty = state.ws.files.filter((x) => x.dirty).length;
  const ok = await confirmAsk({
    title: "Reread everything from disk?",
    body: dirty
      ? `${dirty} file${dirty === 1 ? " has" : "s have"} changes that are not saved. They go.`
      : "Nothing here has changed, so nothing is lost.",
    okLabel: "Reread it", danger: dirty > 0,
  });
  if (!ok) return;
  try {
    const ws = await invoke("reopen");
    if (ws) { state.ws = ws; state.sel = null; render(); await refreshProblems(); }
  } catch (e) { oops(e); }
}

async function reveal(path) {
  try { await invoke("reveal_path", { path }); } catch (e) { oops(e); }
}
async function openExternally(path) {
  try { await invoke("open_external", { path }); } catch (e) { oops(e); }
}

/* ---------------- files ---------------- */

/* Replaces the five-item menu main.js had. Everything a file can do is here,
   grouped, with the config-specific things — is anything including this? — put
   where you would look for them. */
function fileMenu(ev, f) {
  const remote = !!(state.ws && state.ws.remote);
  const many = state.picked.size > 1 && state.picked.has(f.rel);
  if (many) return manyFilesMenu(ev);

  const includers = state.ws.files.filter((x) => (x.includes || []).includes(f.rel));
  const main = state.ws.files.find((x) => x.name === "unrealircd.conf");

  menuAt(ev, [
    { heading: `${f.rel}  —  ${f.blocks} block${f.blocks === 1 ? "" : "s"}` },
    { label: "Open it", check: state.file === f.rel,
      run: () => { state.file = f.rel; state.sel = null; render(); } },
    { label: "Show it beside the list",
      hint: "The file itself, foldable",
      run: () => { state.file = f.rel; state.fileShown = true; applyFileLayout(); render(); } },
    "-",
    { label: "Copy", items: [
      { label: "Its name", run: () => putClip(f.name, "a name") },
      { label: "Its path in the config", run: () => putClip(f.rel, "a path") },
      { label: "Its full path on disk", run: () => putClip(f.path, "a path") },
      { label: "Everything in it", hint: `${f.text.length} bytes`,
        run: () => putClip(f.text, "a file") },
      { label: "An include line for it",
        hint: `include "${f.rel}";`,
        run: () => putClip(`include "${f.rel}";`, "an include line") },
    ] },
    { label: "Reveal it", disabled: remote,
      hint: remote ? "The files are on the server" : "Show it in your file manager",
      run: () => reveal(f.path) },
    { label: "Open in your editor", disabled: remote,
      hint: remote ? "The files are on the server" : "Whatever this machine opens .conf with",
      run: () => openExternally(f.path) },
    "-",
    { label: "Save just this one", disabled: !f.dirty,
      hint: f.dirty ? "" : "Nothing has changed in it",
      run: () => saveOne(f) },
    { label: clipLabel(), hint: "Adds a block to the end of this file",
      run: () => pasteInto(f) },
    "-",
    { heading: "how the server reaches it" },
    ...includeItems(f, includers, main),
    "-",
    { label: "Compare it with…", items: fileTargets(f.path, (x) => compareFiles(f, x)) },
    "-",
    { label: "New file beside it…", run: () => newFile(f.dir) },
    { label: "New folder beside it…", run: () => newFolder(f.dir) },
    { label: "Duplicate it", run: () => act(() => invoke("duplicate_file", { file: f.path }), true) },
    { label: "Rename it…", run: () => renameFile(f) },
    { label: "Move it to…", items: folderTargets(f.dir, (d) => moveFile(f, d)) },
    "-",
    { label: "Delete it…", danger: true,
      hint: "A ~ copy is kept beside it",
      run: () => deleteFile(f) },
  ]);
}

/* The include story for one file, said plainly. A file nothing includes is
   edited happily and changes nothing, which is the single most confusing thing
   that can happen to somebody configuring this server. */
function includeItems(f, includers, main) {
  if (f.name === "unrealircd.conf") {
    return [{ label: "This is the file it boots from", disabled: true, hint: "Nothing needs to include it" }];
  }
  const out = [];
  if (includers.length) {
    out.push({ label: `Included by ${includers.length} file${includers.length === 1 ? "" : "s"}`,
               items: includers.map((x) => ({
                 label: x.rel,
                 run: () => { state.file = x.rel; state.sel = null; render(); },
               })) });
  } else {
    out.push({ label: "Nothing includes it", disabled: true,
               hint: "The server never reads this file as things stand" });
  }
  const targets = state.ws.files.filter((x) => x.path !== f.path && !(x.includes || []).includes(f.rel));
  out.push({
    label: "Include it from…",
    disabled: !targets.length,
    hint: targets.length ? "" : "Every open file already includes it",
    items: [
      ...(main && targets.some((x) => x.path === main.path)
        ? [{ label: "unrealircd.conf", hint: "the one it boots from",
             run: () => addInclude(main, f) }, "-"]
        : []),
      ...targets.filter((x) => !main || x.path !== main.path).slice(0, 20)
        .map((x) => ({ label: x.rel, run: () => addInclude(x, f) })),
    ],
  });
  return out;
}

async function addInclude(into, f) {
  try {
    const updated = await invoke("add_include", { file: into.path, target: f.rel });
    spliceFile(updated);
    render();
    await refreshProblems();
  } catch (e) { oops(e); }
}

async function saveOne(f) {
  try {
    const updated = await invoke("save_file", { file: f.path });
    spliceFile(updated);
    render();
    await refreshProblems();
  } catch (e) { oops(e); }
}

async function renameFile(f) {
  const r = await ask({
    title: `Rename ${f.name}`,
    blurb: "Anything that includes this file by name needs the same change, or the server stops finding it.",
    fields: [{ name: "name", label: "File name", value: f.name, required: true }],
    okLabel: "Rename it",
  });
  if (r && r.name !== f.name) act(() => invoke("rename_file", { file: f.path, name: r.name }), true);
}

async function moveFile(f, dir) {
  act(() => invoke("move_file", { file: f.path, dir }), true);
}

async function deleteFile(f) {
  const ok = await confirmAsk({
    title: `Delete ${f.rel}?`,
    body: `It holds ${f.blocks} block${f.blocks === 1 ? "" : "s"}.`,
    note: `A copy is kept beside it as ${f.name}~ so you can get it back.`,
    okLabel: "Delete it", danger: true,
  });
  if (ok) act(() => invoke("delete_file", { file: f.path }), true);
}

async function compareFiles(a, b) {
  try {
    const d = await invoke("diff_files", { a: a.path, b: b.path });
    showDiff(`${a.rel}  →  ${b.rel}`, { diff: d }, null);
  } catch (e) { oops(e); }
}

async function pasteInto(f) {
  const text = await getClip();
  if (!text.trim()) return say({ title: "Nothing to paste", body: "The clipboard is empty." });
  try {
    const updated = await invoke("paste_block", { file: f.path, text });
    spliceFile(updated);
    render();
    await refreshProblems();
  } catch (e) { oops(e); }
}

/* A second copy of a whole block, to edit rather than retype. */
async function duplicateBlock(f, e) {
  try {
    const text = await invoke("block_text", { file: f.path, start: e.span[0] });
    const updated = await invoke("paste_block", { file: f.path, text });
    spliceFile(updated);
    render();
    await refreshProblems();
  } catch (err) { oops(err); }
}

/* ---------------- several files at once ---------------- */

function manyFilesMenu(ev) {
  const picked = [...state.picked].map((rel) => fileOf(rel)).filter(Boolean);
  const blocks = picked.reduce((n, f) => n + f.blocks, 0);
  menuAt(ev, [
    { heading: `${picked.length} files  —  ${blocks} blocks` },
    { label: "Copy their paths", run: () => putClip(picked.map((f) => f.rel).join("\n"), "some paths") },
    { label: "Copy include lines for them",
      run: () => putClip(picked.map((f) => `include "${f.rel}";`).join("\n"), "some include lines") },
    "-",
    { label: "Move them to…", items: folderTargets(null, async (d) => {
      for (const f of picked) {
        try { await invoke("move_file", { file: f.path, dir: d }); } catch (e) { return oops(e); }
      }
      state.picked.clear();
      act(() => invoke("reopen"), true);
    }) },
    { label: "Save the changed ones",
      disabled: !picked.some((f) => f.dirty),
      run: async () => {
        for (const f of picked.filter((x) => x.dirty)) {
          try { await invoke("save_file", { file: f.path }); } catch (e) { return oops(e); }
        }
        act(() => invoke("reopen"), true);
      } },
    "-",
    { label: "Delete them…", danger: true, run: async () => {
      const ok = await confirmAsk({
        title: `Delete ${picked.length} files?`,
        body: `${blocks} block${blocks === 1 ? "" : "s"} in total.`,
        note: "A ~ copy of each is kept beside it.",
        okLabel: `Delete ${picked.length}`, danger: true,
      });
      if (!ok) return;
      for (const f of picked) {
        try { await invoke("delete_file", { file: f.path }); } catch (e) { return oops(e); }
      }
      state.picked.clear();
      act(() => invoke("reopen"), true);
    } },
    "-",
    { label: "Let them go", run: () => { state.picked.clear(); renderTree(); } },
  ]);
}

/* ---------------- the remaining surfaces ---------------- */

function listMenu(ev) {
  const f = state.file ? fileOf(state.file) : null;
  menuAt(ev, [
    { heading: f ? f.rel : "everything" },
    { label: "Add a block…", key: "Ctrl B", disabled: !f,
      hint: f ? "" : "Pick a file first",
      run: () => addBlockTo(f) },
    { label: clipLabel(), disabled: !f, run: () => pasteInto(f) },
    "-",
    { label: "Console", check: state.view === "console",
      run: () => { if (state.view !== "console") doIt("btn-view"); } },
    { label: "Workbench", check: state.view === "workbench",
      run: () => { if (state.view !== "workbench") doIt("btn-view"); } },
    "-",
    { label: "Fold every category",
      run: () => { foldAllCats(true); render(); } },
    { label: "Unfold every category",
      run: () => { foldAllCats(false); render(); } },
    { label: "Categories…", run: () => doIt("btn-cats") },
    "-",
    { label: "What Confetti found", check: state.showProblems,
      run: () => $("btn-problems").click() },
  ]);
}

function inspectorMenu(ev) {
  const s = picked();
  menuAt(ev, [
    { heading: s ? s.e.path : "nothing selected" },
    { label: "Copy its path", disabled: !s, run: () => putClip(s.e.path, "a path") },
    { label: "Copy its value", disabled: !s || s.e.value == null,
      run: () => putClip(s.e.value, "a value") },
    { label: s && s.e.has_block ? `Copy the whole ${s.e.name} block` : "Copy this line",
      disabled: !s, run: () => copyBlock(s.f, s.e) },
    { label: s ? clipLabel() + " inside it" : "Paste",
      disabled: !s || !s.e.has_block || isShipped(s.f),
      run: () => pasteIntoBlock(s.f, s.e) },
    { label: "What is this?", disabled: !s, run: () => explain(s.e) },
    "-",
    { label: "Show it in the file", disabled: !s,
      run: () => { state.fileShown = true; applyFileLayout(); jumpTo(s.f.rel, s.e.span[0]); } },
    { label: "Clear the selection", disabled: !s,
      run: () => { state.sel = null; render(); } },
  ]);
}

function filePaneMenu(ev) {
  const f = currentFile();
  menuAt(ev, [
    { heading: f ? f.rel : "no file" },
    { label: "Fold every block", disabled: !f, run: () => $("btn-foldall").click() },
    { label: "Unfold everything", disabled: !f, run: () => $("btn-unfoldall").click() },
    "-",
    { label: "Copy the whole file", disabled: !f, run: () => putClip(f.text, "a file") },
    { label: clipLabel(), disabled: !f, run: () => pasteInto(f) },
    "-",
    { label: "Save just this one", disabled: !f || !f.dirty, run: () => saveOne(f) },
    { label: "Hide this pane", run: () => doIt("btn-file") },
  ]);
}

function statusMenu(ev) {
  const dirty = state.ws ? state.ws.files.filter((f) => f.dirty) : [];
  menuAt(ev, [
    { heading: state.ws ? state.ws.root.replace(/\\/g, "/") : "nothing open" },
    { label: "Copy the folder path", disabled: !state.ws,
      run: () => putClip(state.ws.root, "a path") },
    { label: dirty.length === 1 ? "Save the one changed file"
           : dirty.length ? `Save all ${dirty.length} changed files` : "Save",
      disabled: !dirty.length,
      hint: dirty.length ? "" : "Nothing has changed",
      run: () => doIt("btn-save") },
    { label: "What Confetti found", check: state.showProblems,
      run: () => $("btn-problems").click() },
    "-",
    { label: "Documentation…",
      hint: state.ws && state.ws.pack.loaded ? state.ws.pack.note : "none loaded",
      run: () => doIt("btn-docs") },
  ]);
}

async function explain(e) {
  try {
    const h = (await invoke("hint_for", { path: e.path, label: e.label || null })) || {};
    let body;
    if (h.text) {
      body = h.about
        ? `This is part of ${h.about}.

${h.text}`
        : h.text;
    } else if (h.known === false) {
      body = "This release's source has no such directive. It may belong to a "
           + "module that isn't loaded, or it may be a typo — the server will "
           + "refuse to start on an unknown directive.";
    } else {
      body = "Confetti has no prose for this one. Point it at the UnrealIRCd "
           + "source under Help › Point Confetti at the UnrealIRCd source, and "
           + "it will read the wiki that ships with it.";
    }
    say({ title: e.path, body, detail: h.source || undefined });
  } catch (err) { oops(err); }
}

/* ---------------- routing ---------------- */

/* Anything the specific menus did not claim ends up here. A specific handler
   marks the event as handled by calling preventDefault, which menuAt does, so
   there is no flag to keep in sync. */
document.addEventListener("contextmenu", (ev) => {
  if (ev.defaultPrevented) return;
  const t = ev.target instanceof Element ? ev.target : document.body;
  const field = t.closest("input, textarea, [contenteditable=true]");
  if (field) return inputMenu(ev, field);
  if (t.closest(".menu")) { ev.preventDefault(); return; }
  if (t.closest(".modal")) { ev.preventDefault(); return; }
  if (!state.ws) { ev.preventDefault(); return treeMenu(ev); }
  if (t.closest(".tree")) return treeMenu(ev);
  if (t.closest(".filepane")) return filePaneMenu(ev);
  if (t.closest(".insp")) return inspectorMenu(ev);
  if (t.closest(".status")) return statusMenu(ev);
  if (t.closest(".top")) { ev.preventDefault(); return; }
  if (t.closest(".listwrap")) return listMenu(ev);
  ev.preventDefault();
});

/* The keyboard's own menu key, and Shift+F10, on whatever is selected. */
document.addEventListener("keydown", (ev) => {
  if (ev.key !== "ContextMenu" && !(ev.shiftKey && ev.key === "F10")) return;
  const on = document.querySelector(".ln.on, .kid.on, .mod.on, .trow.on");
  const target = on || document.querySelector(".listwrap") || document.body;
  const r = target.getBoundingClientRect();
  ev.preventDefault();
  target.dispatchEvent(new MouseEvent("contextmenu", {
    bubbles: true, cancelable: true,
    clientX: Math.round(r.left + 24), clientY: Math.round(r.top + r.height / 2),
  }));
});

/* ---------------- blocks: copy, paste, and "what is this?" ---------------- */

async function copyBlock(f, e) {
  try {
    const t = await invoke("block_text", { file: f.path, start: e.span[0] });
    await putClip(t, `the ${e.name} block`);
  } catch (err) { oops(err); }
}

const ctxBaseBlockMenu = blockMenu;
blockMenu = function (ev, f, e) {
  ctxBaseBlockMenu(ev, f, e);
  const m = document.querySelector(".menu:not(.sub)");
  if (!m) return;
  menuAppend(m, [
    "-",
    { label: "Copy it", hint: "The whole block, with the comments above it",
      run: () => copyBlock(f, e) },
    { label: "Copy it into…", items: fileTargets(f.path, (x) =>
        act(() => invoke("copy_block", { from: f.path, start: e.span[0], to: x.path }), true)) },
    { label: clipLabel() + " inside it", hint: `Adds it inside ${e.name}`,
      disabled: isShipped(f) || !e.has_block,
      run: () => pasteIntoBlock(f, e) },
    { label: clipLabel() + " after it", hint: "Adds it to the end of this file",
      disabled: isShipped(f),
      run: () => pasteInto(f) },
    { label: "Duplicate it", hint: "A copy at the end of this file",
      disabled: isShipped(f),
      run: () => duplicateBlock(f, e) },
    "-",
    { label: "What is this?", hint: "What the server does with it",
      run: () => explain(e) },
  ]);
};

const ctxBaseDirectiveMenu = directiveMenu;
directiveMenu = function (ev, f, chain) {
  ctxBaseDirectiveMenu(ev, f, chain);
  const m = document.querySelector(".menu:not(.sub)");
  if (!m) return;
  const e = chain[chain.length - 1];
  const parent = chain.length > 1 ? chain[chain.length - 2] : null;
  const ro = isShipped(f);
  menuAppend(m, [
    "-",
    { label: e.has_block ? `Copy the whole ${e.name} block` : "Copy this line",
      hint: "With the comments above it",
      run: () => copyBlock(f, e) },
    { label: e.has_block ? clipLabel() + " inside it" : "Paste next to it",
      hint: e.has_block ? `Adds it inside ${e.name}` : `Adds it inside ${parent ? parent.name : "the block"}`,
      disabled: ro || (!e.has_block && !parent),
      run: () => pasteIntoBlock(f, e.has_block ? e : parent) },
    { label: "Duplicate it", hint: "A copy right below this one",
      disabled: ro || !parent,
      run: () => duplicateEntry(f, parent, e) },
    "-",
    { label: "What is this?", hint: "What the server does with it",
      run: () => explain(e) },
  ]);
};

/* Paste inside a block rather than at the end of the file. */
async function pasteIntoBlock(f, parent) {
  if (!parent) return;
  const text = await getClip();
  if (!text.trim()) return say({ title: "Nothing to paste", body: "The clipboard is empty." });
  try {
    const updated = await invoke("paste_into", {
      file: f.path, parentStart: parent.span[0], text,
    });
    spliceFile(updated);
    render();
    await refreshProblems();
  } catch (e) { oops(e); }
}

/* Copy a line or block and paste it straight back in, so a second one can be
   edited instead of typed from nothing. */
async function duplicateEntry(f, parent, e) {
  try {
    const text = await invoke("block_text", { file: f.path, start: e.span[0] });
    const updated = await invoke("paste_into", {
      file: f.path, parentStart: parent.span[0], text,
    });
    spliceFile(updated);
    render();
    await refreshProblems();
  } catch (err) { oops(err); }
}
