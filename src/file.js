/* Confetti — the config file itself, beside the blocks.
 *
 * The blocks view and the file view are two readings of the same bytes, and
 * this is the half that never lies: it is the text on disk, with nothing
 * interpreted. Selecting a block here highlights it there and the other way
 * round, and any block can be folded away in the text just as it can in the
 * list.
 */

state.fileShown = true;
state.folds = new Set();   // keys of folded regions, per file

try {
  state.fileShown = localStorage.getItem("confetti.fileShown") !== "0";
} catch (e) { /* defaults are fine */ }

const fileLayout = { file: 420 };
try {
  Object.assign(fileLayout, JSON.parse(localStorage.getItem("confetti.fileLayout") || "{}"));
} catch (e) { /* defaults are fine */ }

function saveFileLayout() {
  try {
    localStorage.setItem("confetti.fileLayout", JSON.stringify(fileLayout));
    localStorage.setItem("confetti.fileShown", state.fileShown ? "1" : "0");
  } catch (e) { /* private mode */ }
}

/* Line numbers are 1-based; offsets are bytes. One pass builds the map. */
function lineIndex(text) {
  const starts = [0];
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "\n") starts.push(i + 1);
  }
  return starts;
}
function lineAt(starts, offset) {
  let lo = 0, hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid] <= offset) lo = mid; else hi = mid - 1;
  }
  return lo; // 0-based
}

/* Every block that spans more than one line can be folded, at any depth. */
function foldRegions(f, starts) {
  const out = [];
  (function walk(list) {
    for (const e of list) {
      if (e.has_block) {
        const from = lineAt(starts, e.span[0]);
        const to = lineAt(starts, e.span[1] - 1);
        if (to > from) {
          out.push({ key: `${f.rel}#${e.span[0]}`, from, to, entry: e });
        }
        walk(e.children);
      }
    }
  })(f.entries);
  return out;
}

function currentFile() {
  if (!state.ws) return null;
  if (state.sel) {
    const byPath = fileByPath(state.sel.file);
    if (byPath) return byPath;
  }
  if (state.file) return fileOf(state.file);
  // Nothing picked yet: show the file the server actually boots from, not
  // whichever happens to sort first.
  return mainFile(state.ws);
}

/* Colouring comes from the backend, which gets it from the same parser that
   reads the file for everything else. It arrives a moment after the text does;
   until then the text is drawn plain rather than guessed at. */
state.tokens = null;        // { path, list } for the file on screen

async function loadTokens(f) {
  if (state.tokens && state.tokens.path === f.path && state.tokens.text === f.text) return;
  try {
    const list = await invoke("highlight", { file: f.path });
    state.tokens = { path: f.path, text: f.text, list };
  } catch (e) {
    state.tokens = { path: f.path, text: f.text, list: [] };
    return;
  }
  renderFileView();
}

/* The runs that touch this line, in order, clipped to it. */
function tokensOn(from, to) {
  const all = state.tokens ? state.tokens.list : null;
  if (!all || !all.length) return null;
  // The list is in file order, so a scan from where the last line left off
  // would be cheaper — but a binary search keeps this independent of the order
  // lines are drawn in, which folding makes unpredictable.
  let lo = 0, hi = all.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (all[mid].end <= from) lo = mid + 1; else hi = mid;
  }
  const out = [];
  for (let i = lo; i < all.length && all[i].start < to; i++) {
    const t = all[i];
    if (t.end <= from) continue;
    out.push({ start: Math.max(t.start, from), end: Math.min(t.end, to), kind: t.kind, depth: t.depth });
  }
  return out;
}

function paintLine(node, line, offset) {
  if (line === "") { node.textContent = " "; return; }
  const runs = tokensOn(offset, offset + line.length);
  if (!runs || !runs.length) { node.textContent = line; return; }
  let at = 0;
  for (const r of runs) {
    const a = r.start - offset;
    const b = r.end - offset;
    if (a > at) node.appendChild(document.createTextNode(line.slice(at, a)));
    const s = el("span", "t-" + r.kind + (r.depth > 0 ? " d" + Math.min(r.depth, 4) : ""),
                 line.slice(a, b));
    node.appendChild(s);
    at = b;
  }
  if (at < line.length) node.appendChild(document.createTextNode(line.slice(at)));
}

function renderFileView() {
  const pane = $("filepane");
  const body = $("filebody");
  if (!pane || !body) return;
  body.textContent = "";

  const f = currentFile();
  $("fhname").textContent = f ? f.rel + (f.dirty ? " ·" : "") : "the file";
  if (!f) {
    body.appendChild(el("div", "tiny", "Nothing open."));
    return;
  }

  loadTokens(f);
  const text = f.text;
  const starts = lineIndex(text);
  const lines = text.split("\n");
  const regions = foldRegions(f, starts);

  // Which lines are hidden, and which line carries the fold marker.
  const hidden = new Set();
  const marker = new Map();  // line -> region
  for (const r of regions) {
    marker.set(r.from, r);
    if (state.folds.has(r.key)) {
      for (let i = r.from + 1; i <= r.to; i++) hidden.add(i);
    }
  }

  // The selected block's line range, so it can be picked out.
  let selFrom = -1, selTo = -2;
  if (state.sel && state.sel.file === f.path) {
    const e = state.sel.chain[state.sel.chain.length - 1];
    selFrom = lineAt(starts, e.span[0]);
    selTo = lineAt(starts, e.span[1] - 1);
  }

  const frag = document.createDocumentFragment();
  for (let i = 0; i < lines.length; i++) {
    if (hidden.has(i)) continue;
    const row = el("div", "fl" + (i >= selFrom && i <= selTo ? " hl" : ""));

    const r = marker.get(i);
    const tw = el("span", "ftw", r ? (state.folds.has(r.key) ? "▸" : "▾") : "");
    if (r) {
      tw.classList.add("can");
      tw.title = state.folds.has(r.key)
        ? `Unfold ${r.entry.name}`
        : `Fold ${r.entry.name} away`;
      tw.addEventListener("click", (ev) => {
        ev.stopPropagation();
        if (state.folds.has(r.key)) state.folds.delete(r.key);
        else state.folds.add(r.key);
        renderFileView();
      });
    }
    row.appendChild(tw);
    row.appendChild(el("span", "ln", String(i + 1)));

    const tx = el("span", "ftext");
    paintLine(tx, lines[i], starts[i]);
    row.appendChild(tx);

    if (r && state.folds.has(r.key)) {
      const n = r.to - r.from;
      row.appendChild(el("span", "ffold", `… ${n} line${n === 1 ? "" : "s"} }`));
    }

    // Clicking a line selects whatever block owns it.
    row.addEventListener("click", () => selectAtOffset(f, starts[i]));
    frag.appendChild(row);
  }
  body.appendChild(frag);

  const first = body.querySelector(".fl.hl");
  if (first) first.scrollIntoView({ block: "nearest" });
}

/* The innermost entry containing this byte, and the chain down to it. */
function selectAtOffset(f, offset) {
  const chain = [];
  let list = f.entries;
  for (;;) {
    const hit = list.find((e) => e.span[0] <= offset && offset < e.span[1]);
    if (!hit) break;
    chain.push(hit);
    if (!hit.children.length) break;
    list = hit.children;
  }
  if (!chain.length) return;
  state.file = f.rel;
  state.sel = { file: f.path, chain };
  // Open the block in the list too, so both halves agree.
  let key = null;
  for (const e of chain) {
    key = keyOf(f.path, e);
    if (e.has_block) state.open.add(key);
  }
  render();
}

/* ---------------- layout ---------------- */

const COL = {
  list: 360,                      // the middle column never goes below this
  tree: [150, 420],
  insp: [200, 520],
  file: [260, 900],
  gaps: 15,                       // three 5px splitters
};

/* Fit all four columns into the window at once.
 *
 * There used to be a clamp per splitter, each ignorant of the others, so a wide
 * inspector plus a wide file pane could leave the list a couple of hundred
 * pixels wide — the one column you are actually reading. Everything is sized
 * here instead, and the middle column has a floor: when the window is too
 * narrow the space comes back off the file pane first, then the inspector, then
 * the tree, in the order you are least likely to miss it. */
function applyFileLayout() {
  const b = document.querySelector(".body");
  if (!b) return;
  const w = b.clientWidth || window.innerWidth;

  layout.tree = Math.min(Math.max(layout.tree, COL.tree[0]), COL.tree[1]);
  layout.insp = Math.min(Math.max(layout.insp, COL.insp[0]), COL.insp[1]);
  fileLayout.file = Math.min(Math.max(fileLayout.file, COL.file[0]), COL.file[1]);

  let file = state.fileShown ? fileLayout.file : 0;
  let over = layout.tree + layout.insp + file + COL.gaps + COL.list - w;
  const take = (have, floor) => { const t = Math.max(0, Math.min(over, have - floor)); over -= t; return have - t; };
  if (over > 0 && file) file = take(file, COL.file[0]);
  if (over > 0) layout.insp = take(layout.insp, COL.insp[0]);
  if (over > 0) layout.tree = take(layout.tree, COL.tree[0]);
  // Still short on a very narrow window: the file pane is the one that goes.
  if (over > 0 && file) file = take(file, 0);

  b.style.setProperty("--tree-w", layout.tree + "px");
  b.style.setProperty("--insp-w", layout.insp + "px");
  b.style.setProperty("--file-w", file + "px");
  b.classList.toggle("nofile", !state.fileShown || file === 0);
  const btn = $("btn-file");
  if (btn) btn.textContent = state.fileShown ? "Hide file" : "Show file";
}


function makeFileDraggable() {
  const handle = $("s3");
  const b = document.querySelector(".body");
  if (!handle || !b) return;
  handle.addEventListener("mousedown", (ev) => {
    ev.preventDefault();
    handle.classList.add("dragging");
    document.body.classList.add("resizing");
    const rect = b.getBoundingClientRect();
    const move = (e) => {
      state.fileShown = true;
      fileLayout.file = rect.right - e.clientX;
      applyFileLayout();
    };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      handle.classList.remove("dragging");
      document.body.classList.remove("resizing");
      if (fileLayout.file < 120) {
        fileLayout.file = 420;
        state.fileShown = false;
        applyFileLayout();
      }
      saveFileLayout();
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });
  handle.addEventListener("dblclick", toggleFilePane);
}

function toggleFilePane() {
  state.fileShown = !state.fileShown;
  applyFileLayout();
  saveFileLayout();
  if (state.fileShown) renderFileView();
}

/* ---------------- wiring ---------------- */

$("btn-file").addEventListener("click", toggleFilePane);
$("btn-foldall").addEventListener("click", () => {
  const f = currentFile();
  if (!f) return;
  for (const r of foldRegions(f, lineIndex(f.text))) state.folds.add(r.key);
  renderFileView();
});
$("btn-unfoldall").addEventListener("click", () => {
  state.folds.clear();
  renderFileView();
});

// Redraw the file whenever anything else redraws.
const baseRender = render;
render = function () {
  baseRender();
  if (state.fileShown) renderFileView();
};

window.addEventListener("resize", applyFileLayout);
makeFileDraggable();
applyFileLayout();
renderFileView();

