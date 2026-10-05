/* Confetti — picking out several blocks and doing one thing to all of them.
 *
 * Ctrl-click adds a block to the selection, Shift-click takes the run between
 * the last one and this. From there the right-click menu acts on the lot:
 * filing them under a category, moving them to another file, commenting them
 * out, deleting them, copying them.
 *
 * Every multi-block edit is one call, not a loop of single ones. Deleting
 * three blocks by three separate calls only works for the first: the other two
 * offsets describe the file as it was before the first cut landed. The backend
 * takes the whole list, cuts them out of one parse and renders once.
 */

state.marked = new Map();   // "path + NUL + start" -> { file, start, rel, key, name, label }
state.markAnchor = null;

const markId = (f, e) => `${f.path}\u0000${e.span[0]}`;

function isMarked(f, e) { return state.marked.has(markId(f, e)); }
function markCount() { return state.marked.size; }

function mark(f, e, on) {
  const id = markId(f, e);
  if (on === false || (on === undefined && state.marked.has(id))) state.marked.delete(id);
  else {
    state.marked.set(id, {
      file: f.path, start: e.span[0], rel: f.rel,
      key: blockKey(f.rel, e), name: e.name, label: e.label || "",
    });
  }
}
function clearMarks() {
  if (!state.marked.size) return false;
  state.marked.clear();
  state.markAnchor = null;
  return true;
}

/* The blocks on screen, in the order they are drawn, so Shift can take a run
   of them without caring which file or category they landed in. */
function drawnBlocks() {
  return [...document.querySelectorAll("#list .ln, #list .mod")]
    .map((n) => n.__drop)
    .filter(Boolean);
}

/* One click on a block, before the ordinary select-and-open behaviour. Returns
   true when it was a marking click and the normal one should not happen. */
function markClick(ev, f, e) {
  if (ev.ctrlKey || ev.metaKey) {
    mark(f, e);
    state.markAnchor = markId(f, e);
    render();
    return true;
  }
  if (ev.shiftKey && state.markAnchor) {
    const all = drawnBlocks();
    const a = all.findIndex((x) => markId(x.f, x.e) === state.markAnchor);
    const b = all.findIndex((x) => markId(x.f, x.e) === markId(f, e));
    if (a >= 0 && b >= 0) {
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) mark(all[i].f, all[i].e, true);
      render();
      return true;
    }
  }
  // A plain click puts the selection down and carries on as before.
  if (state.marked.size) clearMarks();
  state.markAnchor = markId(f, e);
  return false;
}

/* ---------------- what you can do to several at once ---------------- */

function markedList() { return [...state.marked.values()]; }

/* Grouped by file, because every edit is per file. */
function markedByFile() {
  const by = new Map();
  for (const m of markedList()) {
    if (!by.has(m.file)) by.set(m.file, []);
    by.get(m.file).push(m.start);
  }
  return by;
}

function markSummary() {
  const list = markedList();
  const kinds = new Map();
  for (const m of list) kinds.set(m.name, (kinds.get(m.name) || 0) + 1);
  const bits = [...kinds].map(([n, c]) => (c > 1 ? `${n} ×${c}` : n));
  const files = new Set(list.map((m) => m.rel)).size;
  return `${list.length} blocks — ${bits.slice(0, 4).join(", ")}`
       + (bits.length > 4 ? `, +${bits.length - 4} more` : "")
       + (files > 1 ? ` — across ${files} files` : "");
}

async function markedApply(fn, label) {
  try {
    for (const [file, starts] of markedByFile()) {
      const r = await fn(file, starts);
      if (r && r.path) spliceFile(r);
      else if (r && r.files) state.ws = r;
    }
    clearMarks();
    render();
    await refreshProblems();
  } catch (e) {
    oops(e, label);
  }
}

function markedMenu(ev) {
  const list = markedList();
  const files = state.ws.files;
  const oneFile = new Set(list.map((m) => m.file)).size === 1;

  menuAt(ev, [
    { heading: markSummary() },
    { label: "Put them all under", items: cats().map((c) => ({
      label: c.name, hint: c.desc || "",
      run: () => {
        for (const m of list) state.cfg.assign[m.key] = c.id;
        saveCfg();
        clearMarks();
        render();
      },
    })) },
    { label: "Move them to", items: files.slice(0, 25).map((x) => ({
      label: x.rel,
      disabled: oneFile && x.path === list[0].file,
      run: () => markedApply(
        (file, starts) => invoke("move_blocks", { from: file, starts, to: x.path }),
        "Moving them"),
    })) },
    "-",
    { label: "Copy them", hint: "All of them, as config text", run: () => copyMarked() },
    "-",
    { label: `Comment out all ${list.length}`, danger: true,
      hint: "They stay in the file, switched off",
      run: () => markedApply(
        (file, starts) => invoke("comment_out_many", { file, starts }), "Commenting them out") },
    { label: `Delete all ${list.length}`, danger: true,
      run: async () => {
        const ok = await confirmAsk({
          title: `Delete ${list.length} blocks?`,
          body: markSummary(),
          note: "Ctrl+Z takes it back, and nothing reaches the disk until you save.",
          okLabel: `Delete ${list.length}`, danger: true,
        });
        if (ok) markedApply((file, starts) => invoke("delete_entries", { file, starts }), "Deleting them");
      } },
    "-",
    { label: "Let them go", key: "Esc", run: () => { clearMarks(); render(); } },
  ]);
}

async function copyMarked() {
  const parts = [];
  for (const m of markedList()) {
    try { parts.push(await invoke("block_text", { file: m.file, start: m.start })); }
    catch (e) { /* skip the one that moved */ }
  }
  if (parts.length) await putClip(parts.join("\n\n"), `${parts.length} blocks`);
}

/* ---------------- hooking into the rows ---------------- */

const multiBaseBlockRow = blockRow;
blockRow = function (f, e) {
  const rows = multiBaseBlockRow(f, e);
  const row = rows[0];
  if (!row) return rows;
  if (isMarked(f, e)) row.classList.add("marked");
  row.addEventListener("click", (ev) => { if (markClick(ev, f, e)) ev.stopImmediatePropagation(); }, true);
  row.addEventListener("contextmenu", (ev) => {
    if (markCount() > 1 && isMarked(f, e)) { ev.stopImmediatePropagation(); markedMenu(ev); }
  }, true);
  return rows;
};

const multiBaseModuleCard = moduleCard;
moduleCard = function (f, e) {
  const card = multiBaseModuleCard(f, e);
  if (isMarked(f, e)) card.classList.add("marked");
  card.addEventListener("click", (ev) => { if (markClick(ev, f, e)) ev.stopImmediatePropagation(); }, true);
  card.addEventListener("contextmenu", (ev) => {
    if (markCount() > 1 && isMarked(f, e)) { ev.stopImmediatePropagation(); markedMenu(ev); }
  }, true);
  return card;
};

/* Dragging one of a marked set drags all of them; dragging anything else puts
   the selection down first, so you never move something you had forgotten was
   still picked. */
const multiBaseBeginBlockDrag = beginBlockDrag;
beginBlockDrag = function (ev, f, e) {
  const together = markCount() > 1 && isMarked(f, e);
  if (!together && markCount()) clearMarks();
  multiBaseBeginBlockDrag(ev, f, e);
  state.dragMarked = together;
  if (together && ev.dataTransfer) {
    ev.dataTransfer.setData("text/plain", `${markCount()} blocks`);
  }
};

/* And every drop target asks here what it is dropping. */
const multiBaseDraggedSet = draggedSet;
draggedSet = function () {
  if (state.dragMarked && markCount()) return markedList();
  return multiBaseDraggedSet();
};

const multiBaseRenderInspector = renderInspector;
renderInspector = function () {
  if (markCount() > 1) {
    const p = $("insp");
    p.textContent = "";
    const h = el("div", "ihead");
    h.append(el("div", "t", `${markCount()} blocks picked`), el("div", "p", markSummary()));
    p.appendChild(h);
    p.appendChild(el("div", "tiny",
      "Right-click any of them to move, file, comment out or delete the lot. "
      + "Escape puts them down."));
    const b = el("button", "btn wide", "Let them go");
    b.addEventListener("click", () => { clearMarks(); render(); });
    p.appendChild(b);
    return;
  }
  multiBaseRenderInspector();
};

/* Escape drops the selection before anything else claims it. */
document.addEventListener("keydown", (ev) => {
  if (ev.key !== "Escape" || !markCount()) return;
  if (document.querySelector(".modal, .menu")) return;
  ev.stopPropagation();
  clearMarks();
  render();
}, true);
