/* Confetti — deciding which files the middle column is allowed to talk about.
 *
 * Two things, working together.
 *
 * The scope is the honest answer to "what does the server actually read?".
 * UnrealIRCd starts at unrealircd.conf and follows its include lines; anything
 * it never reaches — the shipped example.*.conf, a file not wired up yet, an
 * old copy — is not configuration, it is a file lying around. A folder where
 * the server reads nine of thirty-four .conf files is mostly noise, and the
 * noise looks exactly like the real thing.
 *
 * The eye is your own opinion on top of that, and it overrides the scope in
 * both directions: hide something the server does read because you are not
 * thinking about it today, or show something it does not because you are
 * drafting it. That second direction is the point — it is how you say "what
 * the server reads, and also this one file".
 */

const SCOPE_ALL = "all";
const SCOPE_READ = "reachable";

let reachCache = null;

/* What the server reads is the useful default: a config folder is mostly
   files the server never opens — the shipped example.*.conf especially — and
   they look exactly like the real thing until you check. Somewhere without a
   unrealircd.conf has nothing to walk out from, so that falls back to showing
   everything rather than showing nothing at all. */
function visScope() {
  if (!state.cfg) return SCOPE_ALL;
  if (state.cfg.scope) return state.cfg.scope;
  return visRoots().length ? SCOPE_READ : SCOPE_ALL;
}

function visMap() {
  if (!state.cfg) return {};
  if (!state.cfg.vis) state.cfg.vis = {};
  return state.cfg.vis;
}

/* Where the walk starts. unrealircd.conf is what the server itself is given,
   so it is the default, but the list is editable — a second root is how you
   follow an alternative entry point without leaving the scope behind. */
function visRoots() {
  if (!state.cfg || !state.ws) return [];
  if (!state.cfg.scopeRoots || !state.cfg.scopeRoots.length) {
    const main = state.ws.files.find((f) => f.name === "unrealircd.conf");
    state.cfg.scopeRoots = main ? [main.rel] : [];
  }
  return state.cfg.scopeRoots;
}

/* Walk the include graph. `includes` is already resolved to rel paths by the
   backend, so this is a plain breadth-first walk — and it follows your edits,
   because a file becomes reachable the moment its include line parses. */
function reachable() {
  if (reachCache) return reachCache;
  const seen = new Set();
  const queue = [...visRoots()];
  while (queue.length) {
    const rel = queue.shift();
    if (!rel || seen.has(rel)) continue;
    seen.add(rel);
    const f = fileOf(rel);
    if (!f) continue;
    for (const inc of f.includes || []) if (!seen.has(inc)) queue.push(inc);
  }
  reachCache = seen;
  return seen;
}

/* A folder is hidden if it, or anything above it, has been hidden. */
function folderHidden(dir) {
  const v = visMap();
  let d = dir;
  for (;;) {
    if (v["dir:" + d] === "hide") return true;
    const cut = d.lastIndexOf("/");
    if (cut < 0) return false;
    d = d.slice(0, cut);
  }
}

/* "shown" | "hidden" | "unread". The last is a different thing from the middle
   one: the server never reads it, which you did not decide and should not have
   to remember. */
function visibility(f) {
  const v = visMap();
  if (v[f.rel] === "hide") return "hidden";
  if (v[f.rel] === "show") return "shown";
  if (f.dir && folderHidden(f.dir)) return "hidden";
  if (visScope() === SCOPE_READ && !reachable().has(f.rel)) return "unread";
  return "shown";
}
const isShown = (f) => visibility(f) === "shown";

function setVis(key, how) {
  const v = visMap();
  if (how === null) delete v[key]; else v[key] = how;
  saveCfg();
  render();
}

function showEverything() {
  if (!state.cfg) return;
  state.cfg.vis = {};
  state.cfg.scope = SCOPE_ALL;
  saveCfg();
  render();
}

function setScope(s) {
  if (!state.cfg) return;
  state.cfg.scope = s;
  visRoots();
  saveCfg();
  render();
}

function visCounts() {
  if (!state.ws) return { shown: 0, total: 0, byHand: 0 };
  let shown = 0, byHand = 0;
  for (const f of state.ws.files) {
    const v = visibility(f);
    if (v === "shown") shown++;
    else if (v === "hidden") byHand++;
  }
  return { shown, total: state.ws.files.length, byHand };
}

/* ---------------- the middle column ---------------- */

/* Both views ask gatherFiles() what to draw, so hiding is one wrapper here
   rather than a rule each renderer has to remember. Picking a single file is
   an explicit request to see that one, so it is never filtered away. */
const visBaseGatherFiles = gatherFiles;
gatherFiles = function () {
  const files = visBaseGatherFiles();
  return state.file === null ? files.filter(isShown) : files;
};

/* Nothing visible is a different message from nothing there. */
const visBaseEmptyState = emptyState;
emptyState = function (list) {
  if (state.file === null && state.ws && state.ws.files.length && !gatherFiles().length) {
    return list.appendChild(nothingVisible());
  }
  return visBaseEmptyState(list);
};

function nothingVisible() {
  const e = el("div", "empty");
  e.appendChild(el("b", null, "Nothing to show"));
  e.appendChild(el("p", null, visScope() === SCOPE_READ
    ? "The scope is set to what the server reads, and nothing here is reachable "
      + "from unrealircd.conf. Open an eye in the tree to bring a file back."
    : "Every file is hidden. Open an eye in the tree to bring one back."));
  const b = el("button", "btn go", "Show everything");
  b.addEventListener("click", showEverything);
  e.appendChild(b);
  return e;
}

/* ---------------- the eye ---------------- */

function eyeButton(how, what, onToggle) {
  const b = el("button", "eye " + how, how === "shown" ? "◉" : "○");
  b.title = how === "shown"
    ? `${what} is in the list — click to hide it`
    : how === "unread"
      ? `The server never reads ${what} — click to show it anyway`
      : `${what} is hidden — click to show it`;
  b.addEventListener("click", (ev) => { ev.stopPropagation(); onToggle(); });
  return b;
}

/* Clicking a file's eye cycles it back to its default rather than straight to
   the opposite, so a file the scope excluded returns to being excluded instead
   of becoming a permanent exception you have to remember about. */
function cycleFile(f) {
  const cur = visMap()[f.rel];
  const v = visibility(f);
  if (v === "shown") setVis(f.rel, cur === "show" ? null : "hide");
  else if (cur === "hide") setVis(f.rel, null);
  else setVis(f.rel, "show");
}

/* ---------------- tree ---------------- */

const visBaseRenderTree = renderTree;
renderTree = function () {
  visBaseRenderTree();
  if (!state.ws || !state.cfg) return;
  const t = $("tree");
  if (!t) return;

  const head = t.querySelector(".thead");
  if (head) head.insertAdjacentElement("afterend", scopeBar());

  for (const row of t.querySelectorAll(".trow")) {
    if (row.__dir !== undefined) {
      const hidden = folderHidden(row.__dir);
      row.classList.toggle("dim", hidden);
      row.appendChild(eyeButton(hidden ? "hidden" : "shown", row.__dir + "/",
        () => setVis("dir:" + row.__dir, hidden ? null : "hide")));
    } else if (row.__file) {
      const f = row.__file;
      const v = visibility(f);
      row.classList.toggle("dim", v !== "shown");
      row.classList.toggle("unread", v === "unread");
      row.appendChild(eyeButton(v, f.name, () => cycleFile(f)));
    }
  }
};

function scopeBar() {
  const on = visScope() === SCOPE_READ;
  const c = visCounts();
  const bar = el("div", "scopebar");

  const b = el("button", "scopebtn" + (on ? " on" : ""),
    on ? "what the server reads" : "every file here");
  b.title = on
    ? "Following the include lines from " + (visRoots().join(", ") || "nothing")
    : "Every .conf under this folder, whether the server reads it or not";
  b.addEventListener("click", () => setScope(on ? SCOPE_ALL : SCOPE_READ));
  b.addEventListener("contextmenu", (ev) => scopeMenu(ev));
  bar.appendChild(b);

  const n = el("span", "scopen", `${c.shown} of ${c.total}`);
  n.title = c.byHand
    ? `${c.byHand} hidden by hand — click to show everything`
    : "Click to show everything";
  n.addEventListener("click", showEverything);
  bar.appendChild(n);
  return bar;
}

function scopeMenu(ev) {
  const roots = visRoots();
  menuAt(ev, [
    { heading: "what the middle column shows" },
    { label: "Every file here", check: visScope() === SCOPE_ALL,
      run: () => setScope(SCOPE_ALL) },
    { label: "Only what the server reads", check: visScope() === SCOPE_READ,
      hint: "Follows the include lines from where it starts",
      run: () => setScope(SCOPE_READ) },
    "-",
    { heading: "it starts at" },
    ...roots.map((r) => ({
      label: r, check: true,
      hint: roots.length > 1 ? "click to stop starting here" : "the file the server is given",
      disabled: roots.length < 2,
      run: () => {
        state.cfg.scopeRoots = roots.filter((x) => x !== r);
        saveCfg(); render();
      },
    })),
    { label: "Start somewhere else too…", items: (state.ws ? state.ws.files : [])
        .filter((f) => !roots.includes(f.rel))
        .slice(0, 30)
        .map((f) => ({
          label: f.rel,
          run: () => { state.cfg.scopeRoots = [...roots, f.rel]; saveCfg(); render(); },
        })) },
    "-",
    { label: "Show everything", run: showEverything },
  ]);
}

/* ---------------- menus ---------------- */

const visBaseFileMenu = fileMenu;
fileMenu = function (ev, f) {
  visBaseFileMenu(ev, f);
  const m = document.querySelector(".menu:not(.sub)");
  if (!m) return;
  const v = visibility(f);
  menuAppend(m, [
    "-",
    { label: v === "shown" ? "Hide it from the list" : "Show it in the list",
      check: v === "shown",
      hint: v === "unread" ? "The server never reads this file" : "",
      run: () => cycleFile(f) },
    { label: "Hide everything else",
      hint: "Just this one file in the middle column",
      run: () => {
        const vm = visMap();
        for (const x of state.ws.files) vm[x.rel] = x.rel === f.rel ? "show" : "hide";
        saveCfg();
        render();
      } },
    { label: "Show everything", run: showEverything },
  ]);
};

const visBaseFolderMenu = folderMenu;
folderMenu = function (ev, dir) {
  visBaseFolderMenu(ev, dir);
  const m = document.querySelector(".menu:not(.sub)");
  if (!m) return;
  const hidden = folderHidden(dir);
  menuAppend(m, [
    "-",
    { label: hidden ? "Show this folder's files" : "Hide this folder's files",
      check: !hidden,
      run: () => setVis("dir:" + dir, hidden ? null : "hide") },
    { label: "Show everything", run: showEverything },
  ]);
};

/* ---------------- keeping it honest ---------------- */

/* Reachability is derived from include lines, so the one edit that changes the
   answer is the one you make while looking at it. Recomputing once per draw is
   a walk over a few dozen files and never wrong; caching it across draws would
   be neither. */
const visBaseRender = render;
render = function () {
  reachCache = null;
  visBaseRender();
};

/* Opening a different instance, or a different folder, starts from shut. */
const visBaseLoadWorkspace2 = loadWorkspace;
loadWorkspace = async function (ws) {
  resetShelves();
  await visBaseLoadWorkspace2(ws);
};



