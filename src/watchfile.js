/* Confetti — noticing somebody else editing the same files.
 *
 * A config folder is not private. You rehash from a shell, vim is open in
 * another window, a deploy script rewrites something. Confetti holds every
 * file in memory, so without this it would happily save its own stale copy
 * over whatever you did elsewhere — and never mention it.
 *
 * This is a strip along the top, not a dialog. Nothing here is your mistake
 * and nothing is urgent enough to take the keyboard away from you.
 */

state.disk = [];
state.diskIgnored = new Set();

async function checkDisk() {
  if (!state.ws || state.ws.remote) return;
  let found = [];
  try { found = await invoke("disk_changes"); } catch (e) { return; }
  // A file that changed again is worth mentioning again, even if you ignored
  // it the first time.
  const key = (c) => `${c.path}#${c.added}:${c.removed}:${c.gone}`;
  for (const k of [...state.diskIgnored]) {
    if (!found.some((c) => key(c) === k)) state.diskIgnored.delete(k);
  }
  state.disk = found.filter((c) => !state.diskIgnored.has(key(c)));
  drawDiskBar();
}

function drawDiskBar() {
  let bar = $("diskbar");
  if (!bar) {
    bar = el("div", "diskbar");
    bar.id = "diskbar";
    const body = document.querySelector(".body");
    body.parentNode.insertBefore(bar, body);
  }
  bar.textContent = "";
  bar.hidden = !state.disk.length;
  if (!state.disk.length) return;

  for (const c of state.disk) {
    const row = el("div", "diskrow" + (c.mine ? " clash" : ""));
    const di = el("span", "diskicon");
    di.appendChild(icon(c.mine ? "alert" : "refresh"));
    row.appendChild(di);

    const what = el("span", "diskwhat");
    what.appendChild(el("b", null, c.name));
    if (c.gone) {
      what.appendChild(el("span", null, " is no longer on disk."));
    } else {
      const n = [];
      if (c.added) n.push(`${c.added} line${c.added === 1 ? "" : "s"} added`);
      if (c.removed) n.push(`${c.removed} removed`);
      what.appendChild(el("span", null,
        ` changed on disk${n.length ? " — " + n.join(", ") : ""}.`));
    }
    if (c.mine) {
      what.appendChild(el("span", "diskmine",
        " You have unsaved changes here, so one of the two has to go."));
    }
    row.appendChild(what);
    row.appendChild(el("span", "spacer"));

    if (!c.gone) {
      const see = el("button", "minibtn", "See the difference");
      see.addEventListener("click", () => showDiskDiff(c));
      row.appendChild(see);

      const take = el("button", "minibtn go", c.mine ? "Take theirs" : "Reload it");
      take.title = c.mine
        ? "Drop your unsaved changes and take the file as it is on disk. Ctrl+Z brings yours back."
        : "Read it again from disk";
      take.addEventListener("click", () => takeTheirs(c));
      row.appendChild(take);
    }

    const keep = el("button", "minibtn", c.gone ? "Keep it open" : "Keep mine");
    keep.title = c.gone
      ? "Confetti still has the contents. Saving writes the file back."
      : "Leave the file on disk alone for now. Saving will overwrite it.";
    keep.addEventListener("click", () => keepMine(c));
    row.appendChild(keep);

    const x = el("button", "minibtn", "×");
    x.title = "Say nothing more about this one";
    x.addEventListener("click", () => {
      state.diskIgnored.add(`${c.path}#${c.added}:${c.removed}:${c.gone}`);
      state.disk = state.disk.filter((y) => y !== c);
      drawDiskBar();
    });
    row.appendChild(x);
    bar.appendChild(row);
  }
}

async function takeTheirs(c) {
  try {
    const updated = await invoke("reload_file", { file: c.path });
    spliceFile(updated);
    state.disk = state.disk.filter((y) => y.path !== c.path);
    drawDiskBar();
    render();
    await refreshProblems();
  } catch (e) { oops(e); }
}

async function keepMine(c) {
  try {
    const updated = await invoke("keep_mine", { file: c.path });
    spliceFile(updated);
    state.disk = state.disk.filter((y) => y.path !== c.path);
    drawDiskBar();
    render();
  } catch (e) { oops(e); }
}

async function showDiskDiff(c) {
  try {
    const d = await invoke("disk_diff", { file: c.path });
    showDiff(`${c.rel} — yours, against what is on disk now`, { diff: d }, null);
  } catch (e) { oops(e); }
}

/* When to look. Coming back to the window is the moment that matters — you
   have just been editing somewhere else — and a slow tick covers a script
   rewriting a file while you sit here. */
window.addEventListener("focus", () => { checkDisk(); });
setInterval(() => { if (document.hasFocus()) checkDisk(); }, 5000);
