/* Confetti — problems, history, and the block operations that need a preview.
 *
 * Loaded after main.js. It wraps three of main.js's renderers rather than
 * editing them, so the two files stay separable.
 */

state.problems = null;
state.showProblems = false;
state.history = { undo: null, redo: null };

const dismissed = () => (state.cfg && state.cfg.dismissed) || {};

async function refreshProblems() {
  try {
    state.problems = await invoke("problems");
  } catch (e) {
    state.problems = null;
  }
  try {
    state.history = await invoke("history_status");
  } catch (e) { /* no history yet */ }
  renderChrome();
  if (state.showProblems) renderProblems();
}

function liveFindings() {
  if (!state.problems) return [];
  const d = dismissed();
  return state.problems.findings.filter((f) => !d[f.id]);
}

/* The badge counts what is actually wrong. Notes — an unused class, a
   duplicate the server merges anyway — are informational and never counted,
   because a number that can never reach zero is just background anxiety. */
function problemCounts() {
  const live = liveFindings();
  return {
    errors: live.filter((f) => f.level === "error").length,
    warnings: live.filter((f) => f.level === "warning").length,
    notes: live.filter((f) => f.level === "note").length,
  };
}

function renderProblems() {
  let panel = $("problems");
  if (!panel) {
    panel = el("div", "problems");
    panel.id = "problems";
    document.querySelector(".listwrap").appendChild(panel);
  }
  panel.textContent = "";
  panel.style.display = state.showProblems ? "" : "none";
  if (!state.showProblems) return;

  const live = liveFindings();
  const head = el("div", "phead");
  head.appendChild(el("span", null, live.length ? `${live.length} to look at` : "Nothing wrong"));
  head.appendChild(el("span", "spacer"));
  const hide = el("button", "btn", "Close");
  hide.addEventListener("click", () => {
    state.showProblems = false;
    renderProblems();
    renderChrome();
  });
  head.appendChild(hide);
  panel.appendChild(head);

  if (!live.length) {
    panel.appendChild(el("div", "tiny", state.problems
      ? "No dangling references, no clashing ports, nothing left at an example value."
      : "Nothing checked yet."));
    return;
  }

  const order = { error: 0, warning: 1, note: 2 };
  const sorted = live.slice().sort((a, b) => order[a.level] - order[b.level]);
  for (const f of sorted) {
    const row = el("div", "prob " + f.level);
    const top = el("div", "irow");
    top.appendChild(el("span", "plevel " + f.level,
      f.level === "error" ? "will break" : f.level === "warning" ? "check this" : "tidy-up"));
    top.appendChild(el("span", "ploc", `${f.file}:${f.line}`));
    top.appendChild(el("span", "spacer"));
    const go = el("button", "btn", "Show me");
    go.addEventListener("click", () => jumpTo(f.file, f.start));
    top.appendChild(go);
    const x = el("button", "del", "×");
    x.title = "Dismiss — it stays dismissed for this folder";
    x.addEventListener("click", () => {
      if (!state.cfg.dismissed) state.cfg.dismissed = {};
      state.cfg.dismissed[f.id] = true;
      saveCfg();
      renderProblems();
      renderChrome();
    });
    top.appendChild(x);
    row.appendChild(top);
    row.appendChild(el("div", "pmsg", f.message));
    if (f.other) row.appendChild(el("div", "tiny", `the other one is at ${f.other}`));
    panel.appendChild(row);
  }
}

/* Go to a thing by file and byte offset. No animated scroll — nothing moves
   unless you moved it. */
function jumpTo(fileRel, start) {
  if (!state.ws) return;
  const f = state.ws.files.find((x) => x.rel === fileRel) || fileByPath(fileRel);
  if (!f) return;
  state.file = f.rel;
  const hit = findBySpan(f.entries, start);
  if (hit) {
    state.sel = { file: f.path, chain: [hit] };
    state.open.add(keyOf(f.path, hit));
  }
  render();
  const on = document.querySelector(".ln.on, .kid.on");
  if (on) on.scrollIntoView({ block: "center" });
}

/* ---------------- diff preview ---------------- */

function showDiff(title, preview, onApply) {
  const back = el("div", "modal");
  const box = el("div", "sheet wide");
  box.appendChild(el("div", "sheeth", title));

  if (preview.plan && preview.plan.conflicts.length) {
    const warn = el("div", "conflicts");
    const n = preview.plan.conflicts.length;
    warn.appendChild(el("div", "clabel",
      `${n} directive${n === 1 ? "" : "s"} would appear more than once`));
    for (const c of preview.plan.conflicts) {
      const r = el("div", "conflict");
      r.appendChild(el("span", "cname", c.name));
      r.appendChild(el("span", "cvals", c.values.join("   →   ")));
      r.appendChild(el("span", "ceffect", c.effect));
      warn.appendChild(r);
    }
    box.appendChild(warn);
  }

  const d = el("div", "diff");
  for (const l of preview.diff.lines) {
    const row = el("div", "dl " + l.change);
    row.appendChild(el("span", "dn", l.old_line ? String(l.old_line) : ""));
    row.appendChild(el("span", "dm",
      l.change === "added" ? "+" : l.change === "removed" ? "−" : " "));
    row.appendChild(el("span", "dt", l.text));
    d.appendChild(row);
  }
  box.appendChild(d);
  box.appendChild(el("div", "tiny",
    `${preview.diff.added} line${preview.diff.added === 1 ? "" : "s"} added, `
    + `${preview.diff.removed} removed.`
    + (onApply ? " Nothing is written until you say so, and Undo takes it back." : "")));

  const row = el("div", "irow");
  if (onApply) {
    const cancel = el("button", "btn", "Leave it alone");
    cancel.addEventListener("click", () => back.remove());
    row.appendChild(cancel);
    row.appendChild(el("span", "spacer"));
    const ok = el("button", "btn go", "Do it");
    ok.addEventListener("click", async () => {
      back.remove();
      await onApply();
    });
    row.appendChild(ok);
  } else {
    // Comparing two files: there is nothing to apply, only something to read.
    const ok = el("button", "btn go wide", "Close");
    ok.addEventListener("click", () => back.remove());
    row.appendChild(ok);
  }
  box.appendChild(row);

  back.appendChild(box);
  back.addEventListener("click", (e) => { if (e.target === back) back.remove(); });
  document.body.appendChild(back);
}

async function mergeBlocks(f, kind) {
  const starts = f.entries.filter((e) => e.name === kind && e.has_block).map((e) => e.span[0]);
  if (starts.length < 2) return;
  try {
    const preview = await invoke("merge_preview", { file: f.path, starts, keep: "all" });
    showDiff(`Fold ${starts.length} ${kind} blocks into one`, preview, async () => {
      await act(() => invoke("apply_preview", {
        file: f.path,
        text: preview.text,
        label: `merged ${starts.length} ${kind} blocks`,
      }));
    });
  } catch (e) {
    oops(e);
  }
}

async function splitOut(f, parent, child) {
  try {
    const preview = await invoke("split_preview", {
      file: f.path, blockStart: parent.span[0], childStarts: [child.span[0]],
    });
    showDiff(`Move ${child.name} into a ${parent.name} block of its own`, preview, async () => {
      await act(() => invoke("apply_preview", {
        file: f.path, text: preview.text, label: `split ${child.name} out`,
      }));
    });
  } catch (e) {
    oops(e);
  }
}

/* ---------------- wrap the existing renderers ---------------- */

const baseRenderInspector = renderInspector;
renderInspector = function () {
  baseRenderInspector();
  if (!state.sel || !state.ws) return;
  const p = $("insp");
  const chain = state.sel.chain;
  const f = fileByPath(state.sel.file);
  if (!f) return;
  const e = chain[chain.length - 1];

  // Fold several blocks of one kind together — the three set blocks case.
  if (chain.length === 1 && e.has_block) {
    const same = f.entries.filter((x) => x.name === e.name && x.has_block).length;
    if (same > 1) {
      const b = el("button", "btn wide", `Fold the ${same} ${e.name} blocks into one`);
      b.addEventListener("click", () => mergeBlocks(f, e.name));
      p.appendChild(b);
    }
  }
  // Or pull one directive out into a block of its own.
  if (chain.length === 2 && chain[0].has_block && e.has_block) {
    const b = el("button", "btn wide", `Move ${e.name} into its own ${chain[0].name} block`);
    b.addEventListener("click", () => splitOut(f, chain[0], e));
    p.appendChild(b);
  }
};

const baseRenderChrome = renderChrome;
renderChrome = function () {
  baseRenderChrome();
  const h = state.history || {};
  const u = $("btn-undo");
  const r = $("btn-redo");
  if (u) {
    u.disabled = !h.undo;
    u.title = h.undo ? `Undo ${h.undo}` : "Nothing to undo";
  }
  if (r) {
    r.disabled = !h.redo;
    r.title = h.redo ? `Redo ${h.redo}` : "Nothing to redo";
  }
  const pb = $("btn-problems");
  if (pb) {
    const c = problemCounts();
    const bits = [];
    if (c.errors) bits.push(`${c.errors} will break`);
    if (c.warnings) bits.push(`${c.warnings} to check`);
    pb.textContent = bits.length ? bits.join(" · ") : "no problems";
    pb.className = "statbtn" + (c.errors ? " bad" : c.warnings ? " warn" : "");
  }
};

const baseAct = act;
act = async function (fn, whole) {
  await baseAct(fn, whole);
  await refreshProblems();
};

const baseLoadWorkspace = loadWorkspace;
loadWorkspace = async function (ws) {
  await baseLoadWorkspace(ws);
  // Open on the shape rather than on 174 lines of detail: every category
  // folded, each still saying what is inside it.
  // Console only: Workbench needs no flag for this, because folded is simply
  // what absent means there.
  if (state.cfg && !state.cfg.everOpened) {
    if (!state.cfg.folded) state.cfg.folded = {};
    for (const c of cats()) state.cfg.folded[c.id] = true;
    state.cfg.everOpened = true;
    saveCfg();
    render();
  }
  await refreshProblems();
};

$("btn-undo").addEventListener("click", async () => {
  try {
    const ws = await invoke("undo");
    if (ws) {
      state.ws = ws;
      state.sel = null;
      render();
      await refreshProblems();
    }
  } catch (e) { oops(e); }
});
$("btn-redo").addEventListener("click", async () => {
  try {
    const ws = await invoke("redo");
    if (ws) {
      state.ws = ws;
      state.sel = null;
      render();
      await refreshProblems();
    }
  } catch (e) { oops(e); }
});
$("btn-problems").addEventListener("click", () => {
  state.showProblems = !state.showProblems;
  renderProblems();
});

document.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey)) return;
  const k = e.key.toLowerCase();
  if (k === "z" && !e.shiftKey) {
    e.preventDefault();
    $("btn-undo").click();
  } else if ((k === "z" && e.shiftKey) || k === "y") {
    e.preventDefault();
    $("btn-redo").click();
  }
});
