/* Confetti — the server's log, and dragging what is in it into the config.
 *
 * The right-hand column shows either the config file or the log; they are the
 * same real estate because you never need both, and you do need the block list
 * beside whichever one you are reading.
 *
 * Every log line is mined for the things you might act on — an address, a
 * hostmask, an account, a channel — and each becomes a chip you can drag onto
 * a block. What lands there is worked out by the backend from the block's own
 * rules, so dropping an address on a matcher writes `ip`, dropping it on a ban
 * writes `mask`, and dropping it somewhere that cannot hold an address says so
 * instead of writing nonsense.
 */

state.logShown = false;
state.log = null;          // the last read
state.logPath = null;
state.logFiles = [];
state.logQ = "";
state.logLevels = new Set();
state.logSubsystems = new Set();
state.logLimit = 2000;

const LEVELS = ["debug", "info", "warn", "error", "fatal"];

/* Chips carry their fact in the drag. Text is used rather than a custom MIME
   type so a chip can also be dropped into any text field in the app. */
function factDrag(chip, fact) {
  chip.draggable = true;
  chip.addEventListener("dragstart", (ev) => {
    state.drag = null;                       // not a block drag
    state.dragFact = fact;
    ev.dataTransfer.setData("text/plain", fact.value);
    ev.dataTransfer.effectAllowed = "copy";
    document.body.classList.add("dragging-fact");
  });
  chip.addEventListener("dragend", () => {
    state.dragFact = null;
    document.body.classList.remove("dragging-fact");
    document.querySelectorAll(".dropok").forEach((n) => n.classList.remove("dropok"));
  });
}

/* ---------------- reading ---------------- */

async function findLogs() {
  try {
    state.logFiles = await invoke("log_files");
  } catch (e) {
    state.logFiles = [];
  }
  if (!state.logPath) {
    const first = state.logFiles.find((f) => f.exists) || state.logFiles[0];
    state.logPath = first ? first.path : null;
  }
}

async function loadLog(path) {
  if (path) state.logPath = path;
  if (!state.logPath) {
    await findLogs();
    if (!state.logPath) return renderLog();
  }
  state.logBusy = true;
  renderLog();
  try {
    state.log = await invoke("read_log", { path: state.logPath, limit: state.logLimit });
    state.logError = null;
  } catch (e) {
    state.log = null;
    state.logError = String(e);
  }
  state.logBusy = false;
  renderLog();
}

/* ---------------- what to show ---------------- */

function logRows() {
  if (!state.log) return [];
  const q = state.logQ.trim().toLowerCase();
  return state.log.entries.filter((e) => {
    if (state.logLevels.size && !state.logLevels.has(e.level)) return false;
    if (state.logSubsystems.size && !state.logSubsystems.has(e.subsystem)) return false;
    if (!q) return true;
    if (`${e.msg} ${e.event_id} ${e.subsystem}`.toLowerCase().includes(q)) return true;
    return e.facts.some((f) => f.value.toLowerCase().includes(q));
  });
}

function shortTime(ts) {
  // 2026-09-06T10:00:00.000Z -> 10:00:00
  const m = /T(\d\d:\d\d:\d\d)/.exec(ts || "");
  return m ? m[1] : (ts || "").slice(0, 8);
}

function renderLog() {
  const pane = $("logpane");
  if (!pane) return;
  pane.textContent = "";

  /* head: which file, and the filters */
  const head = el("div", "loghead");
  const pick = el("button", "minibtn", state.logPath
    ? state.logPath.split(/[\\/]/).pop()
    : "no log found");
  pick.title = state.logPath || "";
  pick.addEventListener("click", (ev) => logFileMenu(ev));
  head.appendChild(pick);

  head.appendChild(el("span", "spacer"));
  const reload = el("button", "minibtn", state.logBusy ? "reading…" : "reread");
  reload.addEventListener("click", () => loadLog());
  head.appendChild(reload);
  const close = el("button", "minibtn", "hide");
  close.title = "Back to the config file";
  close.addEventListener("click", () => showLog(false));
  head.appendChild(close);
  pane.appendChild(head);

  const filt = el("div", "logfilt");
  const q = el("input", "logq");
  q.placeholder = "filter — a nick, an address, an event";
  q.value = state.logQ;
  q.spellcheck = false;
  q.addEventListener("input", () => { state.logQ = q.value; renderLogBody(); });
  filt.appendChild(q);

  for (const lv of LEVELS) {
    if (!state.log || !state.log.entries.some((e) => e.level === lv)) continue;
    const b = el("button", "lvl " + lv + (state.logLevels.has(lv) ? " on" : ""), lv);
    b.addEventListener("click", () => {
      if (state.logLevels.has(lv)) state.logLevels.delete(lv); else state.logLevels.add(lv);
      renderLog();
    });
    filt.appendChild(b);
  }
  if (state.log && state.log.subsystems.length) {
    const s = el("select", "logsub");
    s.appendChild(Object.assign(document.createElement("option"),
      { value: "", textContent: `every subsystem (${state.log.subsystems.length})` }));
    for (const sub of state.log.subsystems) {
      s.appendChild(Object.assign(document.createElement("option"), { value: sub, textContent: sub }));
    }
    s.value = [...state.logSubsystems][0] || "";
    s.addEventListener("change", () => {
      state.logSubsystems = s.value ? new Set([s.value]) : new Set();
      renderLogBody();
    });
    filt.appendChild(s);
  }
  pane.appendChild(filt);

  const body = el("div", "logbody");
  body.id = "logbody";
  pane.appendChild(body);
  renderLogBody();
}

function renderLogBody() {
  const body = $("logbody");
  if (!body) return;
  body.textContent = "";

  if (state.logError) {
    body.appendChild(el("div", "tiny warnline", state.logError));
    const help = el("div", "tiny",
      "Confetti looks for the files your log { } blocks name. If the server "
      + "writes somewhere else, pick the file from the menu above.");
    body.appendChild(help);
    return;
  }
  if (state.logBusy) return body.appendChild(el("div", "tiny", "Reading…"));
  if (!state.log) return body.appendChild(el("div", "tiny", "No log loaded."));

  const rows = logRows();
  if (state.log.unparsed && !state.log.entries.length) {
    body.appendChild(el("div", "tiny",
      `${state.log.unparsed} lines, none of them JSON. This looks like a plain-text `
      + "log — the JSON one is the destination with a .json or json-formatted file."));
    return;
  }
  if (!rows.length) {
    body.appendChild(el("div", "tiny", "Nothing matches that filter."));
    return;
  }

  if (state.log.truncated) {
    body.appendChild(el("div", "tiny",
      `Showing the last ${state.log.entries.length} of ${state.log.total_lines} lines.`));
  }

  for (const e of rows) {
    const row = el("div", "logrow " + (e.level || "info"));

    const top = el("div", "logtop");
    top.appendChild(el("span", "logtime", shortTime(e.timestamp)));
    if (e.subsystem) top.appendChild(el("span", "logsys", e.subsystem));
    if (e.event_id) top.appendChild(el("span", "logev", e.event_id));
    row.appendChild(top);

    row.appendChild(el("div", "logmsg", e.msg));

    if (e.facts.length) {
      const chips = el("div", "chips");
      for (const f of e.facts) {
        const c = el("span", "chip k-" + f.kind);
        c.appendChild(el("span", "ckind", FACT_LABEL[f.kind] || f.kind));
        c.appendChild(el("span", "cval", f.value));
        c.title = `${f.from} — drag it onto a block, or right-click for somewhere to put it`;
        factDrag(c, f);
        c.addEventListener("contextmenu", (ev) => factMenu(ev, f));
        chips.appendChild(c);
      }
      row.appendChild(chips);
    }

    row.addEventListener("contextmenu", (ev) => {
      if (ev.target.closest(".chip")) return;
      menuAt(ev, [
        { heading: `${e.subsystem}.${e.event_id} — line ${e.line}` },
        { label: "Copy the message", run: () => putClip(e.msg, "a message") },
        { label: "Copy the whole entry", hint: "The raw JSON", run: () => putClip(e.raw, "a log entry") },
        "-",
        { label: "Only this subsystem", check: state.logSubsystems.has(e.subsystem),
          run: () => { state.logSubsystems = new Set([e.subsystem]); renderLog(); } },
        { label: "Only this event", run: () => { state.logQ = e.event_id; renderLog(); } },
        { label: "Clear the filters", run: () => {
          state.logQ = ""; state.logLevels.clear(); state.logSubsystems.clear(); renderLog(); } },
      ]);
    });
    body.appendChild(row);
  }
}

const FACT_LABEL = {
  ip: "address", mask: "hostmask", host: "host", nick: "nick", account: "account",
  channel: "channel", realname: "real name", certfp: "cert", country: "country",
  asn: "network", "security-group": "group", operclass: "operclass",
};

/* ---------------- putting a fact somewhere ---------------- */

/* Show exactly what would be written, then write it. Dropping something into a
   live server's configuration is not a gesture that should happen invisibly. */
async function dropFact(f, fact, entry) {
  let plan;
  try {
    plan = await invoke("plan_drop", {
      file: f.path, start: entry.span[0], kind: fact.kind, value: fact.value,
    });
  } catch (e) {
    return say({ title: "Not there", body: String(e), kind: "error" });
  }

  const back = el("div", "modal");
  const box = el("div", "sheet dlg");
  box.appendChild(el("div", "sheeth", `Add to ${plan.block}`));
  box.appendChild(el("div", "tiny", `${f.rel}, inside ${plan.block}`));
  const pre = el("pre", "ddetail droppreview");
  pre.textContent = plan.preview;
  box.appendChild(pre);
  if (plan.already) {
    box.appendChild(el("div", "tiny warnline", "That is already in this block."));
  }
  box.appendChild(el("div", "tiny",
    "Ctrl+Z takes it back, and nothing reaches the disk until you save."));

  const row = el("div", "irow");
  const no = el("button", "btn", "Cancel");
  const yes = el("button", "btn go", "Add it");
  row.append(no, el("span", "spacer"), yes);
  box.appendChild(row);
  back.appendChild(box);
  document.body.appendChild(back);
  yes.focus();

  const close = () => back.remove();
  no.addEventListener("click", close);
  back.addEventListener("mousedown", (ev) => { if (ev.target === back) close(); });
  const esc = (ev) => {
    if (ev.key === "Escape") { close(); document.removeEventListener("keydown", esc, true); }
    else if (ev.key === "Enter") { ev.preventDefault(); go(); }
  };
  document.addEventListener("keydown", esc, true);

  function go() {
    document.removeEventListener("keydown", esc, true);
    close();
    act(() => invoke("add_child", {
      file: plan.file, parentStart: plan.parent_start, name: plan.directive,
      value: plan.as_block ? plan.value : plan.value, asBlock: plan.as_block,
    }));
  }
  yes.addEventListener("click", go);
}

/* Right-click a chip: the same thing, but you pick the destination from a list
   rather than aiming at it. Faster when the block you want is scrolled away. */
function factMenu(ev, fact) {
  const targets = [];
  if (state.ws) {
    for (const f of state.ws.files) {
      for (const e of f.entries) {
        if (!e.has_block) continue;
        targets.push({
          label: `${e.name}${e.label ? " " + e.label : ""}`,
          hint: f.rel,
          run: () => dropFact(f, fact, e),
        });
      }
    }
  }
  menuAt(ev, [
    { heading: `${FACT_LABEL[fact.kind] || fact.kind}: ${fact.value}` },
    { label: "Copy it", run: () => putClip(fact.value, "a value") },
    { label: "Filter the log by it", run: () => { state.logQ = fact.value; renderLog(); } },
    "-",
    { label: "Put it in…", items: targets.length
        ? targets.slice(0, 40)
        : [{ label: "nothing open", disabled: true }] },
  ]);
}

/* Setting one value from a chip. Same promise as dropping onto a block: you
   see exactly what it will say before it says it. */
async function dropFactOnField(field, fact) {
  const { file, span, current } = field.__edit;
  const f = fileByPath(file);
  const ok = await confirmAsk({
    title: `Set it to ${fact.value}?`,
    body: `${current || "(empty)"}  →  ${fact.value}`,
    note: `${f ? f.rel : file} — Ctrl+Z takes it back.`,
    okLabel: "Set it",
  });
  if (!ok) return;
  act(() => invoke("set_value", { file, start: span[0], end: span[1], newText: fact.value }));
}

/* ---------------- drop targets in the config ---------------- */

/* Wired on the panes rather than on every row, so it survives a redraw and
   costs one listener instead of fifteen hundred. */
function armDropTargets() {
  for (const id of ["list", "tree", "insp"]) {
    const pane = $(id);
    if (!pane || pane.dataset.dropArmed) continue;
    pane.dataset.dropArmed = "1";

    pane.addEventListener("dragover", (ev) => {
      if (!state.dragFact) return;
      // A value you can edit takes the fact directly; anything else has to be
      // a block for it to go inside.
      const field = ev.target.closest(".can-edit");
      if (field && field.__edit) {
        ev.preventDefault();
        ev.dataTransfer.dropEffect = "copy";
        document.querySelectorAll(".dropok").forEach((n) => n.classList.remove("dropok"));
        field.classList.add("dropok");
        return;
      }
      const row = ev.target.closest(".ln, .mod, .kid");
      if (!row) return;
      ev.preventDefault();
      ev.dataTransfer.dropEffect = "copy";
      document.querySelectorAll(".dropok").forEach((n) => n.classList.remove("dropok"));
      row.classList.add("dropok");
    });
    pane.addEventListener("dragleave", (ev) => {
      const row = ev.target.closest(".ln, .mod, .kid");
      if (row) row.classList.remove("dropok");
    });
    pane.addEventListener("drop", (ev) => {
      if (!state.dragFact) return;
      ev.preventDefault();
      const field = ev.target.closest(".can-edit");
      if (field && field.__edit) {
        document.querySelectorAll(".dropok").forEach((n) => n.classList.remove("dropok"));
        dropFactOnField(field, state.dragFact);
        state.dragFact = null;
        document.body.classList.remove("dragging-fact");
        return;
      }
      const row = ev.target.closest(".ln, .mod, .kid");
      document.querySelectorAll(".dropok").forEach((n) => n.classList.remove("dropok"));
      if (!row) return;
      const hit = rowTarget(row);
      if (hit) dropFact(hit.f, state.dragFact, hit.e);
      state.dragFact = null;
      document.body.classList.remove("dragging-fact");
    });
  }
}

/* Which block a rendered row belongs to.
   Each renderer stamps the row with what it drew, so a drop reads it straight
   off. Clicking the row to make it select itself and then reading the
   selection would also work, and would move the selection and redraw the list
   out from under the drop while it was still happening. */
function rowTarget(row) {
  return row.__drop || null;
}

const logBaseBlockRow = blockRow;
blockRow = function (f, e) {
  const rows = logBaseBlockRow(f, e);
  if (rows[0]) rows[0].__drop = { f, e };
  return rows;
};

const logBaseModuleCard = moduleCard;
moduleCard = function (f, e) {
  const card = logBaseModuleCard(f, e);
  card.__drop = { f, e };
  return card;
};

const logBaseKidRows = kidRows;
kidRows = function (file, chain, depth, out) {
  const before = out.length;
  logBaseKidRows(file, chain, depth, out);
  const f = fileByPath(file);
  if (f && out[before]) out[before].__drop = { f, e: chain[chain.length - 1] };
};

/* ---------------- showing the pane ---------------- */

function showLog(on) {
  state.logShown = on;
  if (on) {
    state.fileShown = true;               // the column has to be open to hold it
    applyFileLayout();
    if (!state.log) loadLog();
  }
  applyLogLayout();
  if (!on) renderFileView();
}

function applyLogLayout() {
  const fp = $("filepane");
  const lp = $("logpane");
  if (!fp || !lp) return;
  fp.hidden = state.logShown;
  lp.hidden = !state.logShown;
  const btn = $("btn-log");
  if (btn) btn.textContent = state.logShown ? "Config" : "Log";
  if (state.logShown) renderLog();
}

async function logFileMenu(ev) {
  await findLogs();
  const items = state.logFiles.map((f) => ({
    label: f.name,
    check: f.path === state.logPath,
    hint: (f.json ? "JSON — " : "plain text — ") + (f.exists
      ? `${f.declared_in}${f.bytes ? " — " + Math.round(f.bytes / 1024) + " KB" : ""}`
      : "not there yet"),
    run: () => loadLog(f.path),
  }));
  menuAt(ev, [
    { heading: "log files this config names" },
    ...(items.length ? items : [{ label: "none declared", disabled: true }]),
    "-",
    { label: "How many lines", items: [500, 2000, 10000].map((n) => ({
      label: `the last ${n}`, check: state.logLimit === n,
      run: () => { state.logLimit = n; loadLog(); },
    })) },
  ]);
}

/* ---------------- wiring ---------------- */

const logBaseLoadWorkspace = loadWorkspace;
loadWorkspace = async function (ws) {
  await logBaseLoadWorkspace(ws);
  // A different instance has a different log.
  state.log = null;
  state.logPath = null;
  state.logError = null;
  await findLogs();
  if (state.logShown) loadLog();
  armDropTargets();
};

const logBaseRender = render;
render = function () {
  logBaseRender();
  armDropTargets();
};

$("btn-log").addEventListener("click", () => showLog(!state.logShown));
armDropTargets();
applyLogLayout();
