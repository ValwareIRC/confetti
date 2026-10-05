/* Confetti — one tab per UnrealIRCd instance.
 *
 * A tab is a whole instance: its own folder or ssh host, its own unsaved
 * edits, its own undo history, and its own documentation pack — two servers
 * may well be two different releases, and a hint from the wrong one is worse
 * than no hint.
 *
 * Switching does not re-read anything. The backend parks the tab you are
 * leaving and hands back the one you are going to, rebuilt from memory, so
 * work you had not saved is exactly where you left it.
 *
 * The bar stays out of the way until there are two of them.
 */

state.tab = null;          // id of the tab on screen
state.tabs = [];           // what the bar is showing
state.uiTabs = {};         // id -> the view state parked for that tab

const SHOW_TABS_FROM = 2;

/* What belongs to the tab rather than to the application. Theme, view and the
   width of the panes are yours, not the instance's, so they stay put. */
function parkUi() {
  if (state.tab == null) return;
  state.uiTabs[state.tab] = {
    // A parked tab cannot change, so its workspace stays valid and switching
    // back needs neither a re-parse nor a re-analysis.
    ws: state.ws,
    problems: state.problems,
    file: state.file,
    sel: state.sel,
    open: state.open,
    folders: state.folders,
    picked: state.picked,
    pickAnchor: state.pickAnchor,
    q: state.q,
    cfg: state.cfg,
    showProblems: state.showProblems,
  };
}

function restoreUi(id) {
  const u = state.uiTabs[id];
  state.open = (u && u.open) || new Set();
  state.folders = (u && u.folders) || new Set();
  state.picked = (u && u.picked) || new Set();
  state.pickAnchor = (u && u.pickAnchor) || null;
  state.q = (u && u.q) || "";
  state.showProblems = !!(u && u.showProblems);
  const q = $("q");
  if (q) q.value = state.q;
  if (u && u.file && state.ws && fileOf(u.file)) {
    state.file = u.file;
    state.sel = u.sel;
  }
}

/* Every answer about the tabs is stamped, because they arrive out of order.
   Opening a folder asks for the list, and if you press Ctrl+T while that
   question is still in the air its reply — describing the world before your
   new tab existed — would land afterwards and undo it. The bar would then
   show two tabs while the code believed in one, which is exactly the state in
   which Ctrl+Tab quietly does nothing. */
let tabsSeq = 0;

function setTabs(list) {
  tabsSeq++;
  state.tabs = list;
  const on = list.find((t) => t.active);
  if (on) state.tab = on.id;
  renderTabs();
}

async function refreshTabs() {
  const mine = tabsSeq;
  let got = [];
  try { got = await invoke("tabs"); } catch (e) { got = []; }
  if (mine !== tabsSeq) return;   // something newer happened while we were asking
  setTabs(got);
}

function renderTabs() {
  const bar = $("tabs");
  if (!bar) return;
  bar.textContent = "";
  bar.hidden = state.tabs.length < SHOW_TABS_FROM;
  if (bar.hidden) return;

  for (const t of state.tabs) {
    const b = el("div", "tab" + (t.active ? " on" : "") + (t.empty ? " blank" : ""));
    b.title = t.subtitle + (t.dirty ? `\n${t.dirty} unsaved` : "");

    b.appendChild(el("span", "tdot" + (t.remote ? " remote" : "")));
    b.appendChild(el("span", "tname", t.title));
    if (t.dirty) {
      const d = el("span", "tdirty", "●");
      d.title = `${t.dirty} file${t.dirty === 1 ? "" : "s"} not saved`;
      b.appendChild(d);
    }

    const x = el("button", "tx", "×");
    x.title = "Close this instance";
    x.addEventListener("click", (ev) => { ev.stopPropagation(); closeTab(t.id); });
    b.appendChild(x);

    b.addEventListener("click", () => switchTo(t.id));
    b.addEventListener("auxclick", (ev) => { if (ev.button === 1) closeTab(t.id); });
    b.addEventListener("contextmenu", (ev) => tabMenu(ev, t));
    bar.appendChild(b);
  }

  const add = el("button", "tplus", "+");
  add.title = "Another UnrealIRCd  (Ctrl+T)";
  add.addEventListener("click", () => newTab());
  bar.appendChild(add);
}

/* ---------------- moving between them ---------------- */

/* One switch at a time. Clicking a tab twice — which is exactly what you do
   when the first click feels slow — used to start a second switch that arrived
   after the first had landed, found itself already there, and read the "nothing
   for you" reply as "this tab is empty", blanking a tab that was perfectly
   fine. */
let switching = false;

async function switchTo(id) {
  if (switching || id === state.tab) return;
  switching = true;
  parkUi();
  try {
    const cache = state.uiTabs[id];
    const reuse = !!(cache && cache.ws);
    const ws = await invoke("switch_tab", { id, rebuild: !reuse });
    state.tab = id;

    if (reuse) {
      // Straight back to what we were holding: no parse, no analysis, one draw.
      state.ws = cache.ws;
      state.cfg = cache.cfg || state.cfg;
      state.problems = cache.problems || null;
      restoreUi(id);
      render();
      if (state.showProblems) renderProblems();
    } else if (ws) {
      await loadWorkspace(ws);
      restoreUi(id);
      render();
    } else {
      // Genuinely nothing opened in this one yet.
      state.ws = null;
      state.sel = null;
      state.file = null;
      state.cfg = null;
      state.problems = null;
      restoreUi(id);
      render();
    }
  } catch (e) {
    oops(e);
  } finally {
    switching = false;
  }
  await refreshTabs();
}

async function newTab() {
  parkUi();
  try {
    setTabs(await invoke("new_tab"));
    state.ws = null;
    state.sel = null;
    state.file = null;
    state.cfg = null;
    state.open = new Set();
    state.folders = new Set();
    state.picked = new Set();
    state.q = "";
    if ($("q")) $("q").value = "";
    render();
  } catch (e) { oops(e); }
}

async function closeTab(id) {
  const t = state.tabs.find((x) => x.id === id);
  if (state.tabs.length < 2) return;
  if (t && t.dirty) {
    const ok = await confirmAsk({
      title: `Close ${t.title}?`,
      body: `${t.dirty} file${t.dirty === 1 ? " has" : "s have"} changes that are not saved.`,
      note: "Closing the tab throws them away — there is no undo across a close.",
      okLabel: "Close it anyway",
      danger: true,
    });
    if (!ok) return;
  }
  delete state.uiTabs[id];
  try {
    const ws = await invoke("close_tab", { id });
    if (ws) {
      await loadWorkspace(ws);
      await refreshTabs();
      restoreUi(state.tab);
      render();
      return;
    }
    if (id === state.tab) {
      state.ws = null; state.sel = null; state.file = null; state.cfg = null;
      render();
    }
  } catch (e) { oops(e); }
  await refreshTabs();
}

/* Open something in a tab of its own: the tab appears first, then you are
   asked what goes in it — so if you change your mind you are left with an
   empty tab rather than having clobbered the one you were on. */
async function openFolderInNewTab() {
  await newTab();
  await openFolder();
  await refreshTabs();
}

async function connectInNewTab() {
  await newTab();
  await openServers();
  await refreshTabs();
}

/* ---------------- the tab's own menu ---------------- */

function tabMenu(ev, t) {
  const others = state.tabs.filter((x) => x.id !== t.id);
  menuAt(ev, [
    { heading: t.subtitle },
    { label: "Show it", check: t.active, disabled: t.active, run: () => switchTo(t.id) },
    { label: "Save its changes", disabled: !t.dirty,
      hint: t.dirty ? `${t.dirty} file${t.dirty === 1 ? "" : "s"} waiting` : "Nothing has changed",
      run: async () => { await switchTo(t.id); doIt("btn-save"); } },
    "-",
    { label: "Copy where it is", run: () => putClip(t.subtitle, "a path") },
    "-",
    { label: "New tab", key: "Ctrl T", run: () => newTab() },
    { label: "Close it", key: "Ctrl W", danger: true,
      disabled: state.tabs.length < 2,
      hint: state.tabs.length < 2 ? "It's the only one" : "",
      run: () => closeTab(t.id) },
    { label: "Close the others", danger: true, disabled: !others.length,
      run: async () => { for (const o of others) await closeTab(o.id); } },
  ]);
}

/* ---------------- keep the bar honest ---------------- */

/* Titles carry the unsaved dot, so the bar has to be redrawn whenever anything
   might have changed one. */
const tabsBaseRenderChrome = renderChrome;
renderChrome = function () {
  tabsBaseRenderChrome();
  if (!state.tabs.length) return;
  const on = state.tabs.find((t) => t.id === state.tab);
  const dirty = state.ws ? state.ws.files.filter((f) => f.dirty).length : 0;
  if (on && on.dirty !== dirty) {
    on.dirty = dirty;
    renderTabs();
  }
};

const tabsBaseLoadWorkspace = loadWorkspace;
loadWorkspace = async function (ws) {
  await tabsBaseLoadWorkspace(ws);
  await refreshTabs();
};

document.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey)) return;
  const k = e.key.toLowerCase();
  if (k === "t" && !e.shiftKey) { e.preventDefault(); newTab(); }
  else if (k === "w") { e.preventDefault(); if (state.tab != null) closeTab(state.tab); }
  else if (k === "o" && e.shiftKey) { e.preventDefault(); openFolderInNewTab(); }
  else if (e.key === "Tab" && state.tabs.length > 1) {
    e.preventDefault();
    const i = state.tabs.findIndex((t) => t.id === state.tab);
    const n = state.tabs.length;
    switchTo(state.tabs[(i + (e.shiftKey ? -1 : 1) + n) % n].id);
  }
});

refreshTabs();
