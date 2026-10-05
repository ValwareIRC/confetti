// Confetti — two views over one model.
//   Console   : every file at once, one dense line per block.
//   Workbench : one file, blocks as modules inside category regions.

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);

/* ------------------------------------------------------------------ *
 * Categories. These are the defaults; you can rename them, recolour
 * them, add your own, and move blocks between them. Stored per folder.
 * ------------------------------------------------------------------ */
const DEFAULT_CATEGORIES = [
  { id: "identity", name: "Identity", colour: "#F0A85C", icon: "identity",
    desc: "who this server says it is", blocks: ["me", "admin"] },
  { id: "in", name: "Doors & limits", colour: "#8FD37A", icon: "door",
    desc: "ports, and how hard people may push",
    blocks: ["listen", "class", "allow", "maxperip", "proxy", "webirc"] },
  { id: "staff", name: "Your staff", colour: "#B79BEE", icon: "staff",
    desc: "operators and what they may do", blocks: ["oper", "operclass", "drpass"] },
  { id: "servers", name: "Other servers", colour: "#7BC4E8", icon: "link",
    desc: "links, and who you trust", blocks: ["link", "ulines", "hideserver"] },
  { id: "order", name: "Keeping order", colour: "#EC8894", icon: "shield",
    desc: "spam, floods, bans and exemptions",
    blocks: ["ban", "except", "deny", "spamfilter", "badword", "blacklist", "require",
             "vhost", "security-group", "spamreport"] },
  { id: "settings", name: "Settings", colour: "#DFC055", icon: "sliders",
    desc: "the set { } block — how the server behaves", blocks: ["set"] },
  { id: "modules", name: "Modules", colour: "#7FD1C0", icon: "blocks",
    desc: "what the server loads, and what it must not",
    blocks: ["loadmodule", "blacklist-module"] },
  { id: "words", name: "Help & text", colour: "#E9A6C9", icon: "text",
    desc: "what people read — help, the motd, the channels you suggest",
    blocks: ["help", "tld", "files", "official-channels"] },
  { id: "aliases", name: "Command shortcuts", colour: "#C7D96B", icon: "shortcut",
    desc: "alias blocks — /NS, /CS and friends", blocks: ["alias"] },
  { id: "logging", name: "Logging", colour: "#8FA8D8", icon: "ledger",
    desc: "what gets written down, and where", blocks: ["log"] },
  { id: "keys", name: "Secrets & certificates", colour: "#D8926B", icon: "key",
    desc: "stored secrets, and which certificate answers for which name",
    blocks: ["secret", "sni"] },
  { id: "rpc", name: "The API", colour: "#B0B0E0", icon: "exchange",
    desc: "who may drive the server over RPC", blocks: ["rpc", "rpc-class", "rpc-user"] },
  { id: "reads", name: "Other files it reads", colour: "#9BD3B8", icon: "fileIn",
    desc: "the include lines, and nothing else", blocks: ["include"] },
  // Still here, and it should stay empty: what lands in it is a block from a
  // module this UnrealIRCd has never told Confetti about. That is worth
  // seeing, which it never was while it held every help file in the config.
  { id: "extras", name: "Everything else", colour: "#A3ACC2", icon: "dots",
    desc: "blocks Confetti has not met — usually from a third-party module",
    blocks: [] },
];

/* Categories a release ago that are not categories now, and where the blocks
   that were in them belong instead. `set` had a shelf to itself called House
   rules; giving it an honest name is the same move as giving loadmodule one. */
const RETIRED_CATEGORIES = { rules: "settings" };

/* The shelves that existed before Confetti started writing down which ones a
   config has already been shown. A saved config with no such note can only
   knowingly have had these open, so anything else in it arrived with an
   update and should start shut. */
const ORIGINAL_CATEGORIES = ["identity", "in", "staff", "servers", "order", "rules", "extras"];

const PLAIN = {
  me: "This server", admin: "Who to contact", listen: "A door in",
  class: "A group of connections", allow: "Who may connect", oper: "An operator",
  operclass: "A set of powers", drpass: "Password for /DIE and /RESTART",
  link: "Another server", ulines: "Servers we fully trust", set: "Server settings",
  "security-group": "A group of people", spamfilter: "A spam rule",
  blacklist: "A blocklist to check", badword: "A filtered word", vhost: "A host to wear",
  ban: "A ban", except: "An exemption", deny: "Something not allowed",
  include: "Also read this file", loadmodule: "Load this module",
  "blacklist-module": "Never load this module", log: "What to write down",
  alias: "A command shortcut", help: "Help text", tld: "Files shown to a region",
  "official-channels": "Channels we suggest", secret: "A stored secret",
  webirc: "A gateway we trust", sni: "Certificate for a hostname",
  proxy: "A reverse proxy we trust", files: "Where files live",
  maxperip: "How many from one address", rpc: "Driving the server remotely",
  "rpc-class": "A set of API powers", "rpc-user": "Who may use the API",
};

const state = {
  ws: null,
  file: null,            // rel path; null = everything (Console only)
  sel: null,             // { file, chain: [entry, ...] }
  open: new Set(),
  folders: new Set(),
  q: "",
  criteria: [],
  theme: "console",
  view: "console",
  cfg: null,             // { categories, assign, order, folded }
  drag: null,            // { file, start, key }
  dragCat: null,         // category id being reordered
  picked: new Set(),     // rel paths ticked in the tree, for doing several at once
  pickAnchor: null,      // where a Shift-click measures its run from
  treeOrder: [],         // the files as the tree currently lists them
  foldWb: {},            // Workbench shelves opened this session; never saved
};

/* ---------------- config ---------------- */

function blankCfg() {
  return {
    categories: JSON.parse(JSON.stringify(DEFAULT_CATEGORIES)),
    assign: {},   // blockKey -> category id
    folded: {},   // category id -> true
    // Every shelf here has been seen, so none of them is new.
    knownCats: DEFAULT_CATEGORIES.map((c) => c.id),
  };
}
/* Bring a config saved by an earlier version up to date.
 *
 * What you chose is yours and is kept: names, colours, icons, descriptions,
 * the order of the shelves, and every block you have dragged somewhere by
 * hand. Which block *type* lands where by default is not yours — it was never
 * settable — so that is refreshed, and categories added since are added. */
function reconcileCats(cfg) {
  for (const [gone, now] of Object.entries(RETIRED_CATEGORIES)) {
    if (!cfg.categories.some((c) => c.id === gone)) continue;
    cfg.categories = cfg.categories.filter((c) => c.id !== gone);
    for (const k of Object.keys(cfg.assign || {})) {
      if (cfg.assign[k] === gone) cfg.assign[k] = now;
    }
    if (cfg.folded) delete cfg.folded[gone];
  }

  const mine = new Map(cfg.categories.map((c) => [c.id, c]));
  for (const d of DEFAULT_CATEGORIES) {
    const c = mine.get(d.id);
    if (c) c.blocks = d.blocks.slice();
  }
  // Anything new goes in before the catch-all, which has to stay last:
  // catFor falls through to whatever is at the end.
  const added = DEFAULT_CATEGORIES.filter((d) => !mine.has(d.id))
    .map((d) => JSON.parse(JSON.stringify(d)));

  // A shelf you have never seen starts shut. Console remembers folding
  // between sessions and absent means open there, so without this every
  // category an update adds springs open the next time you start — six at
  // once, over a list you had arranged. Being already *in* the file is not
  // evidence you have seen it: it may have been written there by the same
  // update, on the first save after it appeared.
  if (!cfg.folded) cfg.folded = {};
  const seen = new Set(cfg.knownCats || ORIGINAL_CATEGORIES);
  for (const c of [...cfg.categories, ...added]) {
    if (!seen.has(c.id)) cfg.folded[c.id] = true;
  }
  const tail = cfg.categories.filter((c) => c.id === "extras");
  const head = cfg.categories.filter((c) => c.id !== "extras");
  cfg.categories = [...head, ...added.filter((d) => d.id !== "extras"), ...tail,
                    ...added.filter((d) => d.id === "extras")];
  cfg.knownCats = cfg.categories.map((c) => c.id);
  return cfg;
}

async function saveCfg() {
  // An older version wrote Workbench's fold state here. It is session-only
  // now, so clear it out rather than leave a key that quietly overrides the
  // "everything shut on startup" rule.
  if (state.cfg && state.cfg.foldedWb) delete state.cfg.foldedWb;
  try { await invoke("set_config", { value: state.cfg }); } catch (e) {}
}
const cats = () => (state.cfg ? state.cfg.categories : DEFAULT_CATEGORIES);
const catById = (id) => cats().find((c) => c.id === id) || cats()[cats().length - 1];

/* Which categories are folded, kept per view.
 *
 * A Console row is one line; a Workbench card is a block of colour a couple of
 * inches across. Seven categories of those is a wall you have to scroll past
 * before you find anything, so Workbench opens folded and you open the shelf
 * you want. Console keeps its own answer, because folding there is a different
 * decision about a different thing.
 *
 * Workbench stores only the exceptions: absent means folded, `false` means you
 * opened this one. That way a category added later starts folded too, without
 * anything having to go round and write it down. */
function foldMap() {
  if (!state.cfg) return {};
  // Workbench's fold state lives in `state`, not in `cfg`, so it is never
  // written to disk: every launch starts with every shelf shut, and what you
  // open stays open only for as long as you are looking at it. Console keeps
  // its own answer between sessions, because folding a dense list of lines is
  // a lasting decision rather than a way of getting your bearings.
  if (state.view === "workbench") return state.foldWb;
  if (!state.cfg.folded) state.cfg.folded = {};
  return state.cfg.folded;
}
function isFolded(id) {
  const m = foldMap();
  return state.view === "workbench" ? m[id] !== false : !!m[id];
}
function setFolded(id, folded) {
  const m = foldMap();
  if (state.view === "workbench") {
    if (folded) delete m[id]; else m[id] = false;
  } else if (folded) {
    m[id] = true;
  } else {
    delete m[id];
  }
  saveCfg();
}
function foldAllCats(folded) {
  for (const c of cats()) setFolded(c.id, folded);
  saveCfg();
}

function blockKey(fileRel, e) {
  return `${fileRel}:${e.name}:${e.label || ""}`;
}
function catFor(fileRel, e) {
  const k = blockKey(fileRel, e);
  if (state.cfg && state.cfg.assign[k]) return catById(state.cfg.assign[k]);
  return cats().find((c) => c.blocks.includes(e.name)) || cats()[cats().length - 1];
}

/* ---------------- helpers ---------------- */

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}
function lineOf(text, off) {
  let n = 1;
  for (let i = 0; i < off && i < text.length; i++) if (text[i] === "\n") n++;
  return n;
}
function fileOf(rel) { return state.ws.files.find((f) => f.rel === rel); }
function fileByPath(p) { return state.ws.files.find((f) => f.path === p); }

async function act(fn, whole) {
  try {
    const r = await fn();
    if (whole) { if (r) state.ws = r; state.sel = null; }
    else if (r && r.path) spliceFile(r);
    render();
  } catch (e) { oops(e); }
}
function spliceFile(f) {
  const i = state.ws.files.findIndex((x) => x.path === f.path);
  if (i >= 0) {
    f.includes = state.ws.files[i].includes;
    f.includes_unresolved = state.ws.files[i].includes_unresolved;
    f.included = state.ws.files[i].included;
    state.ws.files[i] = f;
  }
  // Keep the selection pointing at the same thing after a re-parse.
  if (state.sel && state.sel.file === f.path) {
    const chain = [];
    let list = f.entries;
    for (const step of state.sel.chain) {
      const again = list.find((x) => x.name === step.name && x.label === step.label)
                 || list.find((x) => x.span[0] === step.span[0]);
      if (!again) break;
      chain.push(again);
      list = again.children;
    }
    state.sel = chain.length ? { file: f.path, chain } : null;
  }
}

/* Starting a drag of a block.
 *
 * The setData call is not decoration. A dragstart handler that leaves the
 * dataTransfer empty makes the browser abandon the drag before it begins — no
 * drag image, no dragover, no drop — so these blocks looked draggable and
 * simply were not. Handing over the block's own text also means you can drag
 * one straight into any other editor and get real config out of it. */
function beginBlockDrag(ev, f, e) {
  state.drag = { file: f.path, start: e.span[0], key: blockKey(f.rel, e) };
  state.dragCat = null;
  state.dragFact = null;
  if (ev.dataTransfer) {
    ev.dataTransfer.setData("text/plain", f.text.slice(e.span[0], e.span[1]));
    ev.dataTransfer.effectAllowed = "copyMove";
  }
}

/* Which blocks a drop applies to: the one you picked up, and — once several
   are marked — the rest of them too. multi.js widens this; on its own it is
   just the one you are dragging. */
function draggedSet() {
  return state.drag ? [state.drag] : [];
}

function summarise(e) {
  const bits = [];
  for (const c of e.children) {
    if (bits.length >= 4) break;
    if (c.has_block) bits.push(`${c.name} {${c.children.length}}`);
    else if (c.bare) bits.push(c.name);
    else if (c.value != null) bits.push(`${c.name} ${c.value}`);
  }
  let s = bits.join(" · ");
  if (e.children.length > bits.length) s += ` · +${e.children.length - bits.length}`;
  return s || (e.value != null ? e.value : "");
}
function matches(e, q) {
  if (!q) return true;
  const hay = `${e.path} ${e.name} ${e.label || ""} ${e.value || ""}`.toLowerCase();
  return hay.includes(q) || e.children.some((c) => matches(c, q));
}
const sel = () => (state.sel ? state.sel.chain[state.sel.chain.length - 1] : null);
function isSelected(file, e) {
  const s = sel();
  return s && state.sel.file === file && s.span[0] === e.span[0];
}
function select(file, chain) { state.sel = { file, chain }; }

/* ---------------- inline editing ---------------- */

function editable(node, file, span, current, onDone, kind, choices, refKind) {
  // A value the server reads with CFG_YESNO has two states, so it gets a
  // switch. Asking anyone to type "yes" into a box that already knows it
  // wants yes or no is just a spelling test with consequences.
  if (kind === "yes-no") return yesNoSwitch(node, file, span, current);
  if (choices && choices.length > 1) return wordChoice(node, file, span, current, choices);
  if (refKind) return nameChoice(node, file, span, current, refKind);
  node.classList.add("can-edit");
  plainEdit(node, file, span, current, onDone);
}

/* A value that names another block: offer the ones that exist. */
function nameChoice(node, file, span, current, kind) {
  const names = definedNames(kind);
  if (!names.length) {
    node.classList.add("can-edit");
    return plainEdit(node, file, span, current);
  }
  node.classList.add("can-edit", "choice");
  node.title = `Names a ${kind} block`;
  node.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const set = (v) => act(() => invoke("set_value", {
      file, start: span[0], end: span[1], newText: v,
    }));
    menuAt(ev, [
      { heading: `${kind} blocks in this folder` },
      ...names.map((n) => ({
        label: n.name,
        hint: n.rel,
        check: n.name === (current || "").trim(),
        run: () => set(n.name),
      })),
      "-",
      { label: "Something else…",
        hint: "A block defined somewhere Confetti cannot see",
        run: async () => {
          const r = await ask({
            title: `Which ${kind}?`,
            fields: [{ name: "value", label: "Name", value: current || "", required: true }],
            okLabel: "Set it",
          });
          if (r) set(r.value);
        } },
    ]);
  });
}

/* Click it, type, Enter. What every value did before the typed controls
   existed, and what they fall back to when a value turns out not to be what
   its kind claimed. */
function plainEdit(node, file, span, current, onDone) {
  // What this field is, for anything dropped onto it.
  node.__edit = { file, span, current };
  node.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const input = el("input", "edit");
    input.value = current ?? "";
    input.spellcheck = false;
    node.replaceWith(input);
    input.focus();
    input.select();
    let done = false;
    const commit = () => {
      if (done) return;
      done = true;
      if (input.value === current) return render();
      act(() => invoke("set_value", { file, start: span[0], end: span[1], newText: input.value })
        .then((r) => { if (onDone) onDone(); return r; }));
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") commit();
      else if (e.key === "Escape") { done = true; render(); }
    });
    input.addEventListener("blur", commit);
  });
}

/* Directives whose value names another block, and the kind of block it names.
 *
 * Confetti has always known every named block in the workspace — that is how
 * the problems panel spots a class nobody defined. It just never offered them:
 * `parent` in an operclass was a blank box, in a file that already lists every
 * operclass there is. Kept in step with REFERENCES in analyse.rs. */
const NAMES_A_BLOCK = {
  class: "class",
  operclass: "operclass",
  "security-group": "security-group",
  "exclude-security-group": "security-group",
};

/* `parent` means whatever block it is written inside. */
function referencedBlock(path) {
  const parts = (path || "").split("::");
  const last = parts[parts.length - 1];
  if (last === "parent") return parts[0] || null;
  return NAMES_A_BLOCK[last] || null;
}

/* Every block of that kind defined anywhere in the workspace, by name. */
function definedNames(kind) {
  if (!state.ws || !kind) return [];
  const out = new Map();
  for (const f of state.ws.files) {
    for (const e of f.entries) {
      if (e.name !== kind || !e.label || !e.has_block) continue;
      if (!out.has(e.label)) out.set(e.label, f.rel);
    }
  }
  return [...out].map(([name, rel]) => ({ name, rel }));
}

/* A value the source compares against a fixed set of words.
 *
 * The list is what `conf.c` was seen comparing against, which cannot be proved
 * complete — an if-chain read from C might have a case the reader missed. So
 * it suggests rather than restricts: the words are one click away, and
 * "Something else…" is always there. A short list presented as exhaustive
 * would refuse configuration the server accepts. */
function wordChoice(node, file, span, current, choices) {
  node.classList.add("can-edit", "choice");
  node.title = "One of: " + choices.join(", ");
  node.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const set = (v) => act(() => invoke("set_value", {
      file, start: span[0], end: span[1], newText: v,
    }));
    menuAt(ev, [
      { heading: "the words this takes" },
      ...choices.map((w) => ({
        label: w,
        check: w === (current || "").trim(),
        run: () => set(w),
      })),
      "-",
      { label: "Something else…",
        hint: "The list is read from the server's source and may not be all of them",
        run: async () => {
          const r = await ask({
            title: "Set it to",
            fields: [{ name: "value", label: "Value", value: current || "", required: true }],
            okLabel: "Set it",
          });
          if (r) set(r.value);
        } },
    ]);
  });
}

function yesNoSwitch(node, file, span, current) {
  // `options { tls; serversonly; }` — a bare flag's whole text is its name,
  // and it means "on" by being there at all. Treating one as a yes/no value
  // drew it as a switch reading "no", and clicking that would have written
  // `yes` straight over the word `tls`.
  const word = String(current ?? "").trim().toLowerCase();
  if (word !== "yes" && word !== "no") {
    node.classList.add("can-edit");
    return plainEdit(node, file, span, current);
  }
  const on = word === "yes";
  node.classList.add("yesno", on ? "on" : "off");
  node.textContent = on ? "yes" : "no";
  node.title = `Click to make it ${on ? "no" : "yes"}`;
  node.addEventListener("click", (ev) => {
    ev.stopPropagation();
    act(() => invoke("set_value", {
      file, start: span[0], end: span[1], newText: on ? "no" : "yes",
    }));
  });
}

/* ---------------- file tree ---------------- */

function closeMenu() {
  document.querySelectorAll(".menu, .menu.sub").forEach((m) => m.remove());
}
// Late-bound on purpose: menubar.js wraps closeMenu to also drop the open state
// off the menu-bar button, and this must pick up the wrapper, not the original.
document.addEventListener("click", () => closeMenu());

async function newFile(dir) {
  const r = await ask({
    title: "New file",
    blurb: `It goes in ${dir || "the top level of the config folder"}. Nothing reads it until you `
         + `include it from unrealircd.conf.`,
    fields: [{
      name: "name", label: "Call it", value: "network.conf", required: true,
      hint: "Ending in .conf keeps it alongside the others.",
    }],
    okLabel: "Create it",
  });
  if (r) act(() => invoke("new_file", { dir: dir || "", name: r.name }), true);
}

/* Going back to looking at all of it. */
/* A fresh workspace opens with every shelf shut. */
function resetShelves() {
  state.foldWb = {};
}

function showEverythingFile() {
  state.file = null;
  state.sel = null;
  state.picked.clear();
  render();
}

function renderTree() {
  const t = $("tree");
  t.textContent = "";
  if (!state.ws) return;

  const head = el("div", "thead");
  head.appendChild(el("span", null, state.ws.root.replace(/\\/g, "/")));
  const plus = el("button", "tadd", "+");
  plus.title = "New file at the top level";
  plus.addEventListener("click", (e) => { e.stopPropagation(); newFile(""); });
  head.appendChild(plus);
  t.appendChild(head);

  // In both views: Workbench can show every file's cards at once, so there is
  // somewhere for this to go there as well.
  {
    const all = el("div", "trow folder" + (state.file === null ? " on" : ""));
    all.append(el("span", "g", "▾"), el("span", "nm", "everything"));
    all.appendChild(el("span", "n", String(state.ws.files.reduce((n, f) => n + f.blocks, 0))));
    all.addEventListener("click", showEverythingFile);
    t.appendChild(all);
  }

  const dirs = new Map();
  for (const f of state.ws.files) {
    if (!dirs.has(f.dir)) dirs.set(f.dir, []);
    dirs.get(f.dir).push(f);
  }
  // A folder you just made has no .conf in it yet. The backend reports every
  // directory so the tree can show it anyway — otherwise "New folder" looks
  // like it did nothing at all.
  for (const d of state.ws.folders || []) if (!dirs.has(d)) dirs.set(d, []);
  state.treeOrder = [];
  const dirNames = [...dirs.keys()].sort((a, b) => {
    const da = a === "" ? 0 : a.split("/").length;
    const db = b === "" ? 0 : b.split("/").length;
    return da - db || a.localeCompare(b);
  });

  for (const dir of dirNames) {
    let depth = 1;
    // Folding a folder hid its own files but left its subfolders on screen.
    if (dir !== "" && [...state.folders].some((fd) => dir !== fd && dir.startsWith(fd + "/"))) {
      continue;
    }
    if (dir !== "") {
      depth = dir.split("/").length + 1;
      const folded = state.folders.has(dir);
      const row = el("div", `trow folder i${Math.min(dir.split("/").length, 3)}`);
      row.append(el("span", "g", folded ? "▸" : "▾"), el("span", "nm", dir.split("/").pop() + "/"));
      row.appendChild(el("span", "n", String(dirs.get(dir).reduce((a, f) => a + f.blocks, 0))));
      const add = el("button", "tadd", "+");
      add.title = "New file in this folder";
      add.addEventListener("click", (e) => { e.stopPropagation(); newFile(dir); });
      row.appendChild(add);
      row.addEventListener("click", () => {
        if (folded) state.folders.delete(dir); else state.folders.add(dir);
        renderTree();
      });
      row.addEventListener("contextmenu", (e) => folderMenu(e, dir));
      row.__dir = dir;
      t.appendChild(row);
      if (folded) continue;
    }
    for (const f of dirs.get(dir)) {
      const row = el("div",
        `trow i${Math.min(depth, 3)}${state.file === f.rel ? " on" : ""}${f.blocks ? "" : " blank"}`);
      row.append(el("span", "g", f.included ? "↳" : "·"), el("span", "nm", f.name));
      if (f.dirty) row.appendChild(el("span", "dirty", "●"));
      row.appendChild(el("span", "n", f.blocks ? String(f.blocks) : "—"));
      row.title = f.rel + (f.included ? " — included by another file" : "");
      if (state.picked.has(f.rel)) row.classList.add("picked");
      row.__file = f;
      state.treeOrder.push(f.rel);

      // Ctrl picks one more, Shift picks the run between here and the last one.
      // A plain click is still just "open this", which is what it always was.
      row.addEventListener("click", (ev) => {
        if (ev.ctrlKey || ev.metaKey) {
          if (state.picked.has(f.rel)) state.picked.delete(f.rel);
          else state.picked.add(f.rel);
          state.pickAnchor = f.rel;
          return renderTree();
        }
        if (ev.shiftKey && state.pickAnchor) {
          const a = state.treeOrder.indexOf(state.pickAnchor);
          const b = state.treeOrder.indexOf(f.rel);
          if (a >= 0 && b >= 0) {
            for (let i = Math.min(a, b); i <= Math.max(a, b); i++) state.picked.add(state.treeOrder[i]);
            return renderTree();
          }
        }
        state.picked.clear();
        state.pickAnchor = f.rel;
        state.file = f.rel;
        state.sel = null;
        render();
      });
      row.addEventListener("contextmenu", (e) => {
        // Right-clicking outside the selection drops it, the way every file
        // manager behaves.
        if (!state.picked.has(f.rel)) { state.picked.clear(); renderTree(); }
        fileMenu(e, f);
      });

      // Dropping a block here moves it into this file.
      row.addEventListener("dragover", (e) => {
        if (!state.drag || state.drag.file === f.path) return;
        e.preventDefault();
        row.classList.add("drop");
      });
      row.addEventListener("dragleave", () => row.classList.remove("drop"));
      row.addEventListener("drop", (e) => {
        e.preventDefault();
        row.classList.remove("drop");
        if (!state.drag || state.drag.file === f.path) return;
        // Several blocks at once go in one call per source file: the offsets
        // in a list all describe the file before any of them are cut.
        const by = new Map();
        for (const d of draggedSet()) {
          if (d.file === f.path) continue;
          if (!by.has(d.file)) by.set(d.file, []);
          by.get(d.file).push(d.start);
        }
        state.drag = null;
        act(async () => {
          let last = null;
          for (const [from, starts] of by) {
            last = await invoke("move_blocks", { from, starts, to: f.path });
          }
          if (typeof clearMarks === "function") clearMarks();
          return last;
        }, true);
      });
      t.appendChild(row);
    }
  }
}

/* ---------------- Console view ---------------- */

function keyOf(file, e) { return file + "#" + e.span[0]; }

function kidRows(file, chain, depth, out) {
  const e = chain[chain.length - 1];
  const row = el("div", "kid" + (depth > 2 ? " d" + Math.min(depth, 5) : ""));
  const k = keyOf(file, e);
  row.dataset.k = k;
  const open = state.open.has(k);

  row.appendChild(el("span", "tw", e.has_block ? (open ? "▾" : "▸") : ""));
  row.appendChild(el("span", "k", e.bare ? "" : e.name));

  // A directive can have a value *and* a block: `file "ircd.log" { maxsize
  // 100M; }` names a file and then says things about it. Show both — the name
  // first, since that is the part you came to change, then what is inside.
  if (e.has_block && e.label != null) {
    const lv = el("span", "v lv", e.label);
    if (e.value_span) editable(lv, file, e.value_span, e.label, null, e.kind);
    row.appendChild(lv);
    row.appendChild(el("span", "v none", summarise(e)));
  } else {
    const v = el("span", "v");
    if (e.has_block) { v.textContent = summarise(e); v.classList.add("none"); }
    else if (e.bare) { v.textContent = e.name; }
    else if (e.value != null) { v.textContent = e.value; }
    else { v.textContent = "on"; v.classList.add("none"); }
    if (!e.has_block && e.value_span) {
      editable(v, file, e.value_span, e.bare ? e.name : e.value, null,
               e.bare ? null : e.kind, e.bare ? null : e.choices,
               e.bare ? null : referencedBlock(e.path));
    }
    row.appendChild(v);
  }

  if (isSelected(file, e)) row.classList.add("on");
  row.addEventListener("click", () => {
    select(file, chain);
    if (e.has_block) { if (open) state.open.delete(k); else state.open.add(k); }
    render();
  });
  out.push(row);

  if (e.has_block && open) {
    for (const c of e.children) kidRows(file, [...chain, c], depth + 1, out);
    out.push(addRow(file, e, depth + 1));
  }
}

function addRow(file, parent, depth) {
  const row = el("div", "kid addk" + (depth > 2 ? " d" + Math.min(depth, 5) : ""));
  row.appendChild(el("span", "tw", "+"));
  const listy = parent.criterion && parent.criterion.kind === "list";
  const b = el("span", "k dim",
    listy ? `add one more to ${nameOf(parent)}` : `add to ${nameOf(parent)}`);
  b.style.minWidth = "0";
  row.appendChild(b);
  row.title = `Add a directive inside ${parent.path}`;
  row.addEventListener("click", (ev) => {
    ev.stopPropagation();
    addDirectiveTo(fileByPath(file), parent);
  });
  // Hovering one lights up the block it would go into, so the answer is on
  // screen before you click rather than in the file afterwards.
  row.addEventListener("mouseenter", () => markTarget(file, parent, true));
  row.addEventListener("mouseleave", () => markTarget(file, parent, false));
  return row;
}

/* `permissions`, or `operclass helper` where the block has a name. */
function nameOf(e) {
  return e.label ? `${e.name} ${e.label}` : e.name;
}

/* Outline the row for a block, so an "add" row can point at its target. */
function markTarget(file, parent, on) {
  const k = keyOf(file, parent);
  for (const n of document.querySelectorAll("#list .ln, #list .kid")) {
    if (n.dataset.k === k) n.classList.toggle("target", on);
  }
}

function blockRow(f, e) {
  const row = el("div", "ln");
  const k = keyOf(f.path, e);
  row.dataset.k = k;
  const open = state.open.has(k);
  const cat = catFor(f.rel, e);

  const rail = el("span", "rail");
  rail.style.background = cat.colour;
  row.appendChild(rail);
  row.appendChild(el("span", "tw", e.children.length ? (open ? "▾" : "▸") : ""));
  row.appendChild(el("span", "ty", e.name));
  if (e.label) row.appendChild(el("span", "lb", e.label));
  row.appendChild(el("span", "sum", summarise(e)));
  if (e.unknown) row.appendChild(el("span", "flag", "unknown block"));
  if (e.refused) {
    const f = el("span", "flag bad", "the server refuses this");
    f.title = e.refused;
    row.appendChild(f);
  }
  row.appendChild(el("span", "fl", (state.file === null ? f.name : "") + ":" + e.line));

  if (isSelected(f.path, e)) row.classList.add("on");
  row.draggable = true;
  row.addEventListener("dragstart", (ev) => beginBlockDrag(ev, f, e));
  row.addEventListener("dragend", () => { state.drag = null; });
  row.addEventListener("click", () => {
    select(f.path, [e]);
    if (e.children.length) { if (open) state.open.delete(k); else state.open.add(k); }
    render();
  });

  const out = [row];
  if (open) {
    for (const c of e.children) kidRows(f.path, [e, c], 2, out);
    out.push(addRow(f.path, e, 2));
  }
  return out;
}

/* The files the middle column gathers from: the one you picked, or all of
   them when you have not picked one. Both views ask here, and visible.js
   narrows it, so "everything" means the same thing wherever you are. */
function gatherFiles() {
  if (!state.ws) return [];
  return state.file === null
    ? state.ws.files
    : state.ws.files.filter((f) => f.rel === state.file);
}

function renderConsole(list) {
  const files = gatherFiles();

  const buckets = new Map(cats().map((c) => [c.id, []]));
  let shown = 0;
  for (const f of files) {
    for (const e of f.entries) {
      if (!matches(e, state.q)) continue;
      const c = catFor(f.rel, e);
      if (!buckets.has(c.id)) buckets.set(c.id, []);
      buckets.get(c.id).push([f, e]);
      shown++;
    }
  }
  if (!shown) return emptyState(list);

  for (const cat of cats()) {
    const rows = buckets.get(cat.id) || [];
    // A category you have just made has nothing in it, and you fill it by
    // dragging blocks onto it — so hiding it until it has something is a
    // category that can never be filled. While filtering it stays hidden,
    // because then an empty heading only means "no matches here".
    if (!rows.length && state.q) continue;
    list.appendChild(groupHeader(cat, rows.length));
    if (isFolded(cat.id)) continue;
    if (!rows.length) {
      list.appendChild(el("div", "grpempty",
        "Nothing filed here yet \u2014 drag a block onto this heading, or "
        + "right-click one and use \u201cPut it under\u201d."));
      continue;
    }
    for (const [f, e] of rows) for (const r of blockRow(f, e)) list.appendChild(r);
  }
}

/* "listen \u00d73 \u00b7 me \u00b7 admin" \u2014 what is in there, in the order it appears,
   without turning into a paragraph. */
function nameList(items) {
  const seen = new Map();
  for (const e of items) seen.set(e.name, (seen.get(e.name) || 0) + 1);
  const bits = [...seen].map(([n, c]) => (c > 1 ? `${n} \u00d7${c}` : n));
  const shown = bits.slice(0, 6).join("  \u00b7  ");
  return bits.length > 6 ? `${shown}  \u00b7  +${bits.length - 6} more` : shown;
}

/* Dragging a category by its heading or its shelf tab reorders it. The order
   is yours and it is the order both views read, so it is the same gesture in
   each. */
function beginCatDrag(ev, cat) {
  state.dragCat = cat.id;
  state.drag = null;
  state.dragFact = null;
  if (ev.dataTransfer) {
    ev.dataTransfer.setData("text/plain", cat.name);
    ev.dataTransfer.effectAllowed = "move";
  }
  ev.stopPropagation();
}

/* Drop one category onto another: the dragged one takes the target's place. */
function dropCatOn(cat) {
  const list = cats();
  const from = list.findIndex((c) => c.id === state.dragCat);
  const to = list.findIndex((c) => c.id === cat.id);
  state.dragCat = null;
  if (from < 0 || to < 0 || from === to) return false;
  const [moved] = list.splice(from, 1);
  list.splice(to, 0, moved);
  saveCfg();
  render();
  return true;
}

function groupHeader(cat, count) {
  const h = el("div", "grp");
  const sw = el("span", "sw");
  sw.style.background = cat.colour;
  const folded = isFolded(cat.id);
  const gi = el("span", "gi");
  gi.appendChild(catIcon(cat));
  h.append(el("span", "tw", folded ? "▸" : "▾"), sw, gi,
           el("span", "gn", cat.name));
  if (cat.desc) h.appendChild(el("span", "gd", cat.desc));
  h.append(el("span", "rule"), el("span", "n", String(count)));
  h.addEventListener("click", () => {
    setFolded(cat.id, !folded);
    render();
  });
  h.draggable = true;
  h.addEventListener("dragstart", (e) => beginCatDrag(e, cat));
  h.addEventListener("dragend", () => { state.dragCat = null; });
  h.addEventListener("dragover", (e) => {
    if (state.drag || (state.dragCat && state.dragCat !== cat.id)) {
      e.preventDefault();
      h.classList.add("drop");
    }
  });
  h.addEventListener("dragleave", () => h.classList.remove("drop"));
  h.addEventListener("drop", (e) => {
    e.preventDefault();
    h.classList.remove("drop");
    if (state.dragCat) { dropCatOn(cat); return; }
    if (!state.drag) return;
    for (const d of draggedSet()) state.cfg.assign[d.key] = cat.id;
    state.drag = null;
    saveCfg(); render();
  });
  return h;
}

function emptyState(list) {
  const e = el("div", "empty");
  e.append(el("b", null, state.q ? "Nothing matches" : "No blocks here"),
           el("p", null, state.q ? `Nothing matches “${state.q}”.` : "This file may be all comments."));
  list.appendChild(e);
}

/* ---------------- Workbench view ---------------- */

function moduleCard(f, e) {
  const cat = catFor(f.rel, e);
  const card = el("div", "mod");
  card.draggable = true;
  if (isSelected(f.path, e)) card.classList.add("on");

  const bar = el("div", "mbar");
  bar.style.background = cat.colour;
  bar.appendChild(el("span", "mty", e.name));
  if (e.unknown) bar.appendChild(el("span", "mflag", "?"));
  card.appendChild(bar);

  const body = el("div", "mbody");
  if (e.label) body.appendChild(el("div", "mlb", e.label));
  else body.appendChild(el("div", "mlb", PLAIN[e.name] || e.name));

  let n = 0;
  for (const c of e.children) {
    if (n >= 4) break;
    const kv = el("div", "mkv");
    // A bare entry — admin { "Bob Smith"; } — has no name/value pair to show. Its
    // text is the whole of it, so give it the full width and no key.
    const text = c.has_block
      ? (c.label != null ? `${c.label}  { ${c.children.length} }` : `{ ${c.children.length} }`)
      : c.bare ? c.name : (c.value ?? "on");
    if (!c.bare) kv.appendChild(el("span", "mk", c.name));
    const v = el("span", "mv" + (c.bare ? " bare" : ""), text);
    v.title = text;
    if (!c.has_block && c.value_span) {
      editable(v, f.path, c.value_span, c.bare ? c.name : c.value, null,
               c.bare ? null : c.kind, c.bare ? null : c.choices,
               c.bare ? null : referencedBlock(c.path));
    }
    kv.appendChild(v);
    body.appendChild(kv);
    n++;
  }
  if (e.children.length > n) body.appendChild(el("div", "mmore", `+${e.children.length - n} more`));
  card.appendChild(body);

  card.addEventListener("dragstart", (ev) => beginBlockDrag(ev, f, e));
  card.addEventListener("dragend", () => { state.drag = null; });
  card.addEventListener("click", () => { select(f.path, [e]); render(); });
  return card;
}

function renderWorkbench(list) {
  const files = gatherFiles();
  if (!files.length) return emptyState(list);

  // Cards are [file, entry] pairs so the board can hold more than one file at
  // a time — "everything" is a real place to be in this view too, not a state
  // it quietly redirects out of.
  const board = el("div", "board");
  const buckets = new Map(cats().map((c) => [c.id, []]));
  for (const f of files) {
    for (const e of f.entries) {
      if (!matches(e, state.q)) continue;
      const c = catFor(f.rel, e);
      if (!buckets.has(c.id)) buckets.set(c.id, []);
      buckets.get(c.id).push([f, e]);
    }
  }

  for (const cat of cats()) {
    const items = buckets.get(cat.id) || [];
    const folded = isFolded(cat.id);
    // An empty shelf is still a shelf: it is the thing you drag the first card
    // onto. Only a filter hides one, where empty means "nothing matched".
    if (!items.length && state.q) continue;

    const region = el("div", "region" + (items.length ? "" : " bare"));
    region.style.setProperty("--rc", cat.colour);

    const tab = el("div", "rtab");
    tab.style.background = cat.colour;
    const ric = el("span", "ric");
    ric.appendChild(catIcon(cat));
    tab.append(ric, el("span", null, cat.name),
               el("span", "rn", String(items.length)));
    tab.addEventListener("click", () => {
      setFolded(cat.id, !folded);
      render();
    });
    // Console got this by wrapping groupHeader; Workbench builds its own tab,
    // so it was the one surface where a category had no menu.
    tab.addEventListener("contextmenu", (ev) => categoryMenu(ev, cat));
    tab.draggable = true;
    tab.title = "Drag to reorder";
    tab.addEventListener("dragstart", (ev) => beginCatDrag(ev, cat));
    tab.addEventListener("dragend", () => { state.dragCat = null; });
    region.appendChild(tab);
    if (cat.desc) region.appendChild(el("div", "rdesc", cat.desc));

    if (!items.length) {
      if (!folded) {
        region.appendChild(el("div", "rfold",
          "Nothing filed here yet \u2014 drag a card onto this shelf."));
      }
    } else if (!folded) {
      const mods = el("div", "mods");
      for (const [bf, e] of items) mods.appendChild(moduleCard(bf, e));
      region.appendChild(mods);
    } else {
      region.appendChild(el("div", "rfold", nameList(items.map(([, e]) => e))));
    }

    region.addEventListener("dragover", (e) => {
      if (state.drag || (state.dragCat && state.dragCat !== cat.id)) {
        e.preventDefault();
        region.classList.add("drop");
      }
    });
    region.addEventListener("dragleave", () => region.classList.remove("drop"));
    region.addEventListener("drop", (e) => {
      e.preventDefault();
      region.classList.remove("drop");
      if (state.dragCat) { dropCatOn(cat); return; }
      if (!state.drag) return;
      for (const d of draggedSet()) state.cfg.assign[d.key] = cat.id;
      state.drag = null;
      saveCfg(); render();
    });
    board.appendChild(region);
  }

  if (!board.children.length) return emptyState(list);
  const n = files.reduce((a, x) => a + x.blocks, 0);
  list.appendChild(el("div", "bhead", files.length === 1
    ? `${files[0].rel} — ${files[0].blocks} blocks`
    : `everything — ${n} blocks across ${files.length} files`));
  list.appendChild(board);
}

/* ---------------- inspector ---------------- */

function criterionRow(file, e) {
  const wrap = el("div", "frow");
  const row = el("div", "irow");
  const sel2 = el("select", "sel2");
  for (const [label, prefix] of [["Match when", ""], ["But not when", "exclude-"]]) {
    const g = document.createElement("optgroup");
    g.label = label;
    for (const c of state.criteria) {
      const o = document.createElement("option");
      o.value = prefix + c.name; o.textContent = prefix + c.name;
      g.appendChild(o);
    }
    sel2.appendChild(g);
  }
  if (![...sel2.querySelectorAll("option")].some((o) => o.value === e.name)) {
    const o = document.createElement("option");
    o.value = e.name; o.textContent = e.name + " — not a known criterion";
    sel2.insertBefore(o, sel2.firstChild);
  }
  sel2.value = e.name;
  sel2.addEventListener("change", () => {
    if (!e.name_span) return;
    act(() => invoke("set_value", {
      file, start: e.name_span[0], end: e.name_span[1], newText: sel2.value }));
  });
  row.appendChild(sel2);

  const kind = e.criterion ? e.criterion.kind : e.kind;
  const f = fileByPath(file);
  const isList = kind === "list" || (e.has_block && !e.children.some((c) => !c.bare));

  if (isList || e.has_block) {
    const n = e.children.length;
    row.appendChild(el("span", "tiny", n ? `${n} ${n === 1 ? "entry" : "entries"}` : "empty"));
  } else if (kind === "yes-no") {
    const on = (e.value || "").toLowerCase() === "yes";
    const t = el("button", "yesno" + (on ? " on" : ""), on ? "yes" : "no");
    t.addEventListener("click", () => {
      if (!e.value_span) return;
      act(() => invoke("set_value", {
        file, start: e.value_span[0], end: e.value_span[1], newText: on ? "no" : "yes" }));
    });
    row.appendChild(t);
  } else {
    // A criterion added from the panel is written bare — `ip;` — so there is
    // no value span to hang a field on. Offer one anyway: without it there is
    // no way at all to say what the thing should match.
    const has = e.value != null;
    const v = el("span", "fld" + (has ? "" : " none"), has ? e.value : "say what it matches");
    if (has && e.value_span) editable(v, file, e.value_span, e.value, null, kind);
    else if (e.name_span) {
      v.classList.add("can-edit");
      v.title = "Set it";
      v.addEventListener("click", () => setCriterionValue(f, e));
    }
    row.appendChild(v);
  }
  const del = el("button", "del", "×");
  del.title = "Comment it out";
  del.addEventListener("click", () => act(() => invoke("comment_out", { file, start: e.span[0] })));
  row.appendChild(del);
  wrap.appendChild(row);

  // A list criterion holds names, one per line. It used to render as the
  // words "0 entries" and nothing else — no way to see them, add one, or
  // take one away.
  if (isList) {
    const chips = el("div", "chips");
    for (const c of e.children) {
      const chip = el("span", "chip");
      const t = el("span", "chipt", c.bare ? c.name : `${c.name} ${c.value ?? ""}`.trim());
      const span = c.bare ? c.name_span : c.value_span;
      if (span) editable(t, file, span, c.bare ? c.name : c.value);
      chip.appendChild(t);
      const x = el("button", "chipx", "×");
      x.title = "Remove it";
      x.addEventListener("click", () =>
        act(() => invoke("delete_entry", { file, start: c.span[0] })));
      chip.appendChild(x);
      chips.appendChild(chip);
    }
    const plus = el("button", "chip add", "+ add one");
    plus.addEventListener("click", () => addListItemTo(f, e));
    chips.appendChild(plus);
    wrap.appendChild(chips);
  }

  const bare = e.name.startsWith("exclude-") ? e.name.slice(8) : e.name;
  const c = state.criteria.find((x) => x.name === bare);
  if (c) wrap.appendChild(el("div", "tiny", e.name.startsWith("exclude-")
    ? `Excluded even if they matched: ${c.about.toLowerCase()}` : c.about));
  return wrap;
}

/* What the inspector shows when nothing is selected: the file you are looking
   at, what is in it, and how the server reaches it. */
function renderFileSummary(p) {
  const f = state.file ? fileOf(state.file) : null;
  if (!f) {
    const n = state.ws.files.reduce((a, x) => a + x.blocks, 0);
    const h = el("div", "ihead");
    h.append(el("div", "t", "Everything"), el("div", "p", state.ws.root.replace(/\\/g, "/")));
    p.appendChild(h);
    p.appendChild(el("div", "tiny",
      `${state.ws.files.length} files, ${n} blocks. Pick one on the left, or a block to see what it does.`));
    return;
  }

  const head = el("div", "ihead");
  head.append(el("div", "t", f.name), el("div", "p", f.rel));
  p.appendChild(head);

  const kinds = new Map();
  for (const e of f.entries) kinds.set(e.name, (kinds.get(e.name) || 0) + 1);
  const rows = [...kinds].sort((a, b) => b[1] - a[1]);

  if (rows.length) {
    p.appendChild(el("div", "lab", "what is in it"));
    const tbl = el("div", "isum");
    for (const [name, n] of rows.slice(0, 14)) {
      const r = el("div", "irow2");
      r.appendChild(el("span", "ikey", name));
      r.appendChild(el("span", "fld", String(n)));
      r.addEventListener("click", () => {
        const first = f.entries.find((e) => e.name === name);
        if (first) { select(f.path, [first]); state.open.add(keyOf(f.path, first)); render(); }
      });
      tbl.appendChild(r);
    }
    if (rows.length > 14) tbl.appendChild(el("div", "tiny", `+${rows.length - 14} more kinds`));
    p.appendChild(tbl);
  } else {
    p.appendChild(el("div", "tiny", "No blocks in this one yet."));
  }

  // Reachability is the thing people get wrong: a file the server never
  // includes is edited happily and changes nothing at all.
  p.appendChild(el("div", "lab", "how the server reaches it"));
  if (f.name === "unrealircd.conf") {
    p.appendChild(el("div", "tiny", "This is the one UnrealIRCd boots from."));
  } else if (f.included) {
    p.appendChild(el("div", "tiny", "Another file includes it, so the server reads it."));
  } else {
    const w = el("div", "tiny warnline",
      "Nothing includes this file. The server never reads it \u2014 add an "
      + "include line to unrealircd.conf if you meant it to.");
    p.appendChild(w);
  }
  if (f.includes.length) {
    p.appendChild(el("div", "lab", "it pulls in"));
    for (const inc of f.includes) {
      const b = el("button", "ilink", inc);
      b.addEventListener("click", () => { state.file = inc; state.sel = null; render(); });
      p.appendChild(b);
    }
  }
  if (f.includes_unresolved.length) {
    p.appendChild(el("div", "tiny",
      `Could not follow: ${f.includes_unresolved.join(", ")}`));
  }

  const add = el("button", "btn wide", "Add a block to this file\u2026");
  add.addEventListener("click", () => addBlockTo(f));
  p.appendChild(add);
}

/* What the second word on a block is, in words. `oper bobsmith { }` names an
   oper; `file "ircd.log" { maxsize 100M; }` names a file and then says things
   about that file. */
function labelWord(e) {
  const shapes = {
    oper: "oper name", operclass: "class name", class: "class name",
    link: "server name", "security-group": "group name", "rpc-class": "class name",
    file: "file", log: "file", listen: "address", vhost: "host",
    alias: "command", help: "topic", tld: "for", secret: "name", sni: "host",
  };
  return shapes[e.name] || "name";
}

function renderInspector() {
  const p = $("insp");
  p.textContent = "";
  if (!state.ws) return p.appendChild(el("div", "tiny", "Nothing open yet."));
  if (!state.sel) return renderFileSummary(p);

  const file = state.sel.file;
  const chain = state.sel.chain;
  const e = chain[chain.length - 1];
  const f = fileByPath(file);

  // Breadcrumb — click a step to go back up to it.
  if (chain.length > 1) {
    const bc = el("div", "crumb");
    chain.forEach((step, i) => {
      if (i) bc.appendChild(el("span", "csep", "›"));
      const b = el("button", "cstep" + (i === chain.length - 1 ? " on" : ""), step.name);
      b.addEventListener("click", () => { state.sel = { file, chain: chain.slice(0, i + 1) }; render(); });
      bc.appendChild(b);
    });
    p.appendChild(bc);
  }

  const head = el("div", "ihead");
  const title = PLAIN[e.name] || e.name;
  head.append(el("div", "t", e.label ? `${title} · ${e.label}` : title), el("div", "p", e.path));
  p.appendChild(head);
  if (f) p.appendChild(el("div", "tiny", `${f.rel} · line ${lineOf(f.text, e.span[0])}`));

  if (e.hint || e.hint_source) {
    const h = el("div", "hint", e.hint || "No description in the wiki for this one.");
    const bits = [];
    if (e.hint_about) bits.push(`about ${e.hint_about}, the group this sits in`);
    if (e.hint_source) bits.push(e.hint_source);
    if (bits.length) h.appendChild(el("span", "src", bits.join(" · ")));
    p.appendChild(h);
  }

  if (e.has_block && e.label != null && e.value_span) {
    const r = el("div", "frow");
    r.appendChild(el("span", "lab", labelWord(e)));
    const lv = el("span", "fld", e.label);
    editable(lv, file, e.value_span, e.label, null, e.kind);
    r.appendChild(lv);
    p.appendChild(r);
  }

  if (!e.has_block && e.value_span) {
    const r = el("div", "frow");
    r.appendChild(el("span", "lab", e.bare ? "this line" : e.name));
    const v = el("span", "fld", e.value ?? e.name);
    editable(v, file, e.value_span, e.bare ? e.name : e.value, null,
             e.bare ? null : e.kind, e.bare ? null : e.choices,
             e.bare ? null : referencedBlock(e.path));
    r.appendChild(v);
    p.appendChild(r);
    const del = el("button", "btn wide", "Comment this out");
    del.addEventListener("click", () => act(() => invoke("comment_out", { file, start: e.span[0] })));
    p.appendChild(del);
  }

  if (e.has_block) {
    p.appendChild(el("div", "divider"));
    if (e.matcher) {
      p.appendChild(el("span", "lab", "matches"));
      for (const c of e.children) {
        if (c.bare && !c.criterion) {
          const r = el("div", "irow");
          const v = el("span", "fld", c.name);
          if (c.value_span) editable(v, file, c.value_span, c.name);
          r.appendChild(v);
          const del = el("button", "del", "×");
          del.addEventListener("click", () => act(() => invoke("comment_out", { file, start: c.span[0] })));
          r.appendChild(del);
          p.appendChild(r);
        } else p.appendChild(criterionRow(file, c));
      }
      // This used to be a dropdown that added the criterion bare, whatever
      // it needed — so `ip` arrived with no address and no way to give it one.
      const add = el("button", "btn wide", "+ add a criterion…");
      add.addEventListener("click", () => addCriterionTo(fileByPath(file), e));
      p.appendChild(add);
    } else {
      p.appendChild(el("span", "lab", `${e.children.length} inside`));
      for (const c of e.children) {
        const r = el("div", "irow");
        const k = el("span", "ikey", c.bare ? "—" : c.name);
        r.appendChild(k);
        const v = el("span", "fld", c.has_block
          ? (c.label != null ? `${c.label}  { ${c.children.length} } ›` : `{ ${c.children.length} } ›`)
          : (c.bare ? c.name : (c.value ?? "on")));
        v.style.marginLeft = "auto";
        if (c.has_block) {
          v.classList.add("into");
          v.title = "Open this one";
          v.addEventListener("click", (ev) => {
            ev.stopPropagation();
            state.sel = { file, chain: [...chain, c] };
            render();
          });
        } else if (c.value_span) {
          editable(v, file, c.value_span, c.bare ? c.name : c.value, null,
                   c.bare ? null : c.kind, c.bare ? null : c.choices,
                   c.bare ? null : referencedBlock(c.path));
        }
        r.appendChild(v);
        if (c.refused) { r.classList.add("refused"); r.title = c.refused; }
        const del = el("button", "del", "×");
        del.title = "Comment it out";
        del.addEventListener("click", () => act(() => invoke("comment_out", { file, start: c.span[0] })));
        r.appendChild(del);
        p.appendChild(r);
        // The server refuses this one, and says what to do instead. Better
        // read here than as a line in a log after it will not start.
        if (c.refused) {
          const why = el("div", "refusal", c.refused);
          p.appendChild(why);
        }
      }
      const listy = e.criterion && e.criterion.kind === "list";
      const add = el("button", "btn wide",
        listy ? `+ add one more to ${nameOf(e)}` : `+ add to ${nameOf(e)}`);
      add.title = listy
        ? `Add another entry to the ${e.name} list`
        : `Add a directive inside ${e.path}`;
      add.addEventListener("click", () => addDirectiveTo(fileByPath(file), e));
      p.appendChild(add);
    }
  }
}

/* ---------------- categories editor ---------------- */

function openCategories() {
  closeMenu();
  const back = el("div", "modal");
  const box = el("div", "sheet");
  box.appendChild(el("div", "sheeth", "Categories"));
  box.appendChild(el("div", "tiny",
    "These group your blocks in both views. Drag a block onto a heading to move it here."));

  const list = el("div", "catlist");
  const redraw = () => {
    list.textContent = "";
    for (const c of cats()) {
      const row = el("div", "catrow");
      const sw = el("input", "cswatch");
      sw.type = "color"; sw.value = c.colour;
      sw.addEventListener("input", () => { c.colour = sw.value; saveCfg(); render(); });
      // A grid of the icons Confetti draws, rather than a text field that
      // invited a character from whatever emoji table happened to be around.
      // Named `pick` and not `icon`: a local called icon would hide the
      // function that draws them.
      const pick = el("button", "cicon");
      const paint = () => { pick.textContent = ""; pick.appendChild(catIcon(c)); };
      paint();
      pick.title = "Pick an icon";
      pick.addEventListener("click", (ev) => {
        ev.stopPropagation();
        const pop = el("div", "icopop");
        for (const n of CATEGORY_ICONS) {
          const b = el("button", "icopick" + (c.icon === n ? " on" : ""));
          b.appendChild(icon(n));
          b.title = n;
          b.addEventListener("click", () => {
            c.icon = n; pop.remove(); paint(); saveCfg(); render();
          });
          pop.appendChild(b);
        }
        pick.appendChild(pop);
        const away = (e) => {
          if (pop.contains(e.target)) return;
          pop.remove();
          document.removeEventListener("mousedown", away, true);
        };
        setTimeout(() => document.addEventListener("mousedown", away, true), 0);
      });
      const name = el("input", "cname");
      name.value = c.name;
      name.addEventListener("change", () => { c.name = name.value; saveCfg(); render(); });
      const desc = el("input", "cdesc");
      desc.value = c.desc || "";
      desc.placeholder = "what this group is for";
      desc.addEventListener("change", () => { c.desc = desc.value; saveCfg(); render(); });
      const del = el("button", "del", "×");
      del.title = "Remove this category — its blocks go to Everything else";
      del.addEventListener("click", () => {
        if (cats().length <= 1) return;
        state.cfg.categories = cats().filter((x) => x.id !== c.id);
        for (const k of Object.keys(state.cfg.assign)) {
          if (state.cfg.assign[k] === c.id) delete state.cfg.assign[k];
        }
        saveCfg(); redraw(); render();
      });
      row.append(sw, pick, name, desc, del);
      list.appendChild(row);
    }
  };
  redraw();
  box.appendChild(list);

  const add = el("button", "btn wide", "+ New category");
  add.addEventListener("click", async () => {
    const r = await ask({
      title: "New category",
      blurb: "A shelf of your own. Drag blocks onto it, or use a block’s right-click menu.",
      fields: [
        { name: "name", label: "Call it", placeholder: "Spam and abuse", required: true },
        { name: "desc", label: "What goes in it", placeholder: "optional — shown under the heading" },
      ],
      okLabel: "Add it",
    });
    if (!r) return;
    const name = r.name;
    state.cfg.categories.splice(cats().length - 1, 0, {
      id: "c" + Math.abs(Date.now ? name.length * 7919 + cats().length : cats().length) + cats().length,
      name, colour: "#8FD37A", icon: "◆", desc: r.desc || "", blocks: [],
    });
    saveCfg(); redraw(); render();
  });
  box.appendChild(add);

  const reset = el("button", "btn wide", "Reset to the defaults");
  reset.addEventListener("click", async () => {
    const ok = await confirmAsk({
      title: "Reset the categories?",
      body: "The default set comes back.",
      note: "Blocks you have already filed stay where you put them.",
      okLabel: "Reset them",
    });
    if (!ok) return;
    state.cfg.categories = JSON.parse(JSON.stringify(DEFAULT_CATEGORIES));
    saveCfg(); redraw(); render();
  });
  box.appendChild(reset);

  const done = el("button", "btn go wide", "Done");
  done.addEventListener("click", () => back.remove());
  box.appendChild(done);

  back.appendChild(box);
  back.addEventListener("click", (e) => { if (e.target === back) back.remove(); });
  document.body.appendChild(back);
}

/* ---------------- chrome ---------------- */

function renderChrome() {
  const ws = state.ws;
  const dirty = ws ? ws.files.filter((f) => f.dirty) : [];
  $("btn-save").disabled = !dirty.length;
  $("btn-save").textContent = dirty.length > 1 ? `Save ${dirty.length}` : "Save";
  $("btn-add").disabled = !ws || state.file === null;
  $("btn-view").textContent = state.view === "console" ? "View: Console" : "View: Workbench";
  $("btn-theme").textContent = state.theme === "console" ? "Theme: Console" : "Theme: Cream";

  $("sdot").className = "dot" + (!ws ? " off" : dirty.length ? " warn" : "");
  $("sroot").textContent = ws ? ws.root.replace(/\\/g, "/") : "nothing open";
  if (ws) {
    const blocks = ws.files.reduce((n, f) => n + f.blocks, 0);
    const errs = ws.files.reduce(
      (n, f) => n + f.diagnostics.filter((d) => d.severity === "error").length, 0);
    $("scount").textContent = `${ws.files.length} files · ${blocks} blocks`
      + (errs ? ` · ${errs} error${errs === 1 ? "" : "s"}` : "")
      + (dirty.length ? ` · ${dirty.length} unsaved` : "");
  } else $("scount").textContent = "";

  const pk = ws && ws.pack;
  $("sdocs").textContent = pk && pk.loaded ? `${pk.note} · ${pk.with_prose} explained` : "no docs";
}

function render() {
  if (state.ws && !state.cfg) state.cfg = blankCfg();
  renderChrome();
  renderTree();
  const list = $("list");
  list.textContent = "";
  list.className = "list " + state.view;
  if (!state.ws) {
    const e = el("div", "empty");
    e.append(el("b", null, "Nothing open"),
      el("p", null, "Open an UnrealIRCd folder — the install, or its conf/ — and every .conf under it appears here."));
    const b = el("button", "btn go", "Open folder");
    b.addEventListener("click", openFolder);
    e.appendChild(b);
    list.appendChild(e);
  } else if (state.view === "console") renderConsole(list);
  else renderWorkbench(list);
  renderInspector();
}

/* ---------------- theme + layout ---------------- */

function setTheme(name) {
  state.theme = name;
  document.documentElement.setAttribute("data-theme", name);
  try { localStorage.setItem("confetti.theme", name); } catch (e) {}
}
const body = document.querySelector(".body");
let layout = { tree: 224, insp: 290 };
try { layout = Object.assign(layout, JSON.parse(localStorage.getItem("confetti.layout") || "{}")); }
catch (e) {}
// Sizing all four columns is file.js's job — it is the only place that knows
// about the file pane too, and they have to be clamped together or they starve
// the list between them.
function applyLayout() {
  if (typeof applyFileLayout === "function") applyFileLayout();
}
function saveLayout() {
  try { localStorage.setItem("confetti.layout", JSON.stringify(layout)); } catch (e) {}
}
function draggable(handle, which) {
  handle.addEventListener("mousedown", (ev) => {
    ev.preventDefault();
    handle.classList.add("dragging");
    document.body.classList.add("resizing");
    const rect = body.getBoundingClientRect();
    const move = (e) => {
      if (which === "tree") layout.tree = e.clientX - rect.left;
      else layout.insp = rect.right - e.clientX;
      applyLayout();
    };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      handle.classList.remove("dragging");
      document.body.classList.remove("resizing");
      saveLayout();
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });
}

/* ---------------- wiring ---------------- */

async function openFolder() {
  try {
    const ws = await invoke("pick_folder");
    if (ws) await loadWorkspace(ws);
  } catch (e) { oops(e); }
}
// The obvious file to land on: the one UnrealIRCd actually boots from, or
// failing that whichever holds the most.
function mainFile(ws) {
  return ws.files.find((f) => f.name === "unrealircd.conf")
      || ws.files.slice().sort((a, b) => b.blocks - a.blocks)[0]
      || ws.files[0];
}

async function loadWorkspace(ws) {
  state.ws = ws;
  state.sel = null;
  state.open = new Set();
  try {
    const cfg = await invoke("get_config");
    const was = cfg && cfg.categories ? JSON.stringify(cfg) : null;
    state.cfg = was ? reconcileCats(cfg) : blankCfg();
    // Write the note about which shelves have been seen, or the next launch
    // would decide they are new all over again.
    if (was && JSON.stringify(state.cfg) !== was) saveCfg();
    // An older version saved Workbench's fold state. It is session-only now,
    // so a leftover key would quietly defeat "every shelf shut on startup".
    delete state.cfg.foldedWb;
    state.foldWb = {};
    if (!state.cfg.assign) state.cfg.assign = {};
    if (!state.cfg.folded) state.cfg.folded = {};
  } catch (e) { state.cfg = blankCfg(); }
  // Both views can show everything now, so neither is forced into a file.
  // A folder with one file in it is the exception: there is nothing else.
  if (ws.files.length === 1) state.file = ws.files[0].rel;
  else if (!state.file || !fileOf(state.file)) state.file = null;
  render();
}

$("btn-open").addEventListener("click", openFolder);
$("btn-view").addEventListener("click", () => {
  state.view = state.view === "console" ? "workbench" : "console";
  try { localStorage.setItem("confetti.view", state.view); } catch (e) {}
  render();
});
$("btn-theme").addEventListener("click", () => {
  setTheme(state.theme === "console" ? "cream" : "console");
  renderChrome();
});
$("btn-cats").addEventListener("click", openCategories);
$("btn-save").addEventListener("click", async () => {
  try {
    await invoke("save_all");
    const ws = await invoke("reopen");
    if (ws) { state.ws = ws; render(); }
  } catch (e) { oops(e); }
});
$("btn-add").addEventListener("click", () => {
  if (!state.ws) return;
  const f = state.file ? fileOf(state.file) : mainFile(state.ws);
  if (f) addBlockTo(f);
});
$("btn-docs").addEventListener("click", async () => {
  try {
    await invoke("pick_source_root");
    const ws = await invoke("reopen");
    if (ws) { state.ws = ws; render(); }
  } catch (e) { oops(e); }
});
$("q").addEventListener("input", (e) => {
  state.q = e.target.value.trim().toLowerCase();
  render();
});
document.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") { e.preventDefault(); $("q").focus(); }
  if (e.key === "Escape") {
    if (document.activeElement === $("q")) $("q").blur();
    document.querySelectorAll(".modal").forEach((m) => m.remove());
    closeMenu();
  }
});
/* Clicking the empty space under the tree means the same as clicking
   "everything" — there is nothing else it could sensibly mean, and leaving it
   inert makes the pane feel like it has a dead zone at the bottom. */
$("tree").addEventListener("click", (e) => {
  if (e.target !== $("tree")) return;      // a row handled it already
  if (!state.ws) return;
  showEverythingFile();
});

window.addEventListener("resize", applyLayout);
draggable($("s1"), "tree");
draggable($("s2"), "insp");

try { setTheme(localStorage.getItem("confetti.theme") || "console"); } catch (e) { setTheme("console"); }
try { state.view = localStorage.getItem("confetti.view") || "console"; } catch (e) {}
applyLayout();
render();

(async () => {
  try { state.criteria = await invoke("vocabulary"); } catch (e) { state.criteria = []; }
  try {
    const ws = await invoke("startup_workspace");
    if (ws) await loadWorkspace(ws);
  } catch (e) { console.error(e); }
  for (let i = 0; i < 16; i++) {
    if (state.ws && state.ws.pack && state.ws.pack.loaded) break;
    await new Promise((r) => setTimeout(r, 800));
    try {
      const pk = await invoke("pack_status");
      if (pk.loaded && state.ws) {
        const ws = await invoke("reopen");
        if (ws) { state.ws = ws; render(); }
        break;
      }
    } catch (e) { break; }
  }
})();
