/* Confetti — right-click menus.
 *
 * One builder, attached everywhere. Every menu says what it will do in words,
 * marks anything destructive, and never offers an action that would be refused
 * — if merging this kind of block would change what the server does, the item
 * is there but disabled and carries the reason, because "why can't I?" is a
 * worse experience than "here's why".
 */

/* Submenus close on a timer rather than on the first mouseleave, because
   travelling from "Theme" to "Cream" means crossing rows that are not "Theme".
 *
 * The timer alone is not enough. Every row is built by this same function, so
 * the rows *inside* a submenu were carrying the sibling behaviour too: hovering
 * "Cream" scheduled the closure of the submenu "Cream" lives in, and nothing
 * cancelled it, so the thing you were pointing at deleted itself out from under
 * you a moment later. A row only counts as a sibling if it is not itself in the
 * submenu — and once the pointer is inside, no timer runs at all. */
let subTimer = null;
let subOwner = null;

const inSub = (n) => !!(n && n.closest && n.closest(".menu.sub"));

function closeSubs() {
  document.querySelectorAll(".menu.sub").forEach((s) => s.remove());
  if (subOwner) { subOwner.classList.remove("subopen"); subOwner = null; }
}
function holdSub() {
  if (subTimer) { clearTimeout(subTimer); subTimer = null; }
}
function dropSub() {
  holdSub();
  subTimer = setTimeout(closeSubs, 450);
}

/* What a row does when the pointer arrives on it: hold the submenu open if the
   row is part of it, otherwise start closing it. */
function hoverRow(b) {
  if (inSub(b)) holdSub();
  else dropSub();
}

/* Build a menu from a list of items. An item is "-" for a rule, {heading} for a
   label, or {label, hint, key, check, danger, disabled, items, run}. Nested
   `items` make a submenu. */
function buildMenu(items) {
  const m = el("div", "menu");
  // If anything in this menu is a toggle, every row gets the tick gutter, so
  // the labels line up instead of stepping in and out.
  const gutter = items.some((it) => it && it.check !== undefined);

  for (const it of items) {
    if (it === "-") { m.appendChild(el("div", "msep")); continue; }
    if (it.heading) { m.appendChild(el("div", "mhead", it.heading)); continue; }

    const b = el("button", "mi" + (it.danger ? " danger" : "") + (it.disabled ? " off" : ""));
    const line = el("span", "mline");
    if (gutter) line.appendChild(el("span", "mtick", it.check ? "\u2713" : ""));
    line.appendChild(el("span", "mlabel", it.label));
    if (it.key) line.appendChild(el("span", "mkey", it.key));
    if (it.items) line.appendChild(el("span", "marrow", "\u203a"));
    b.appendChild(line);
    if (it.hint) { b.appendChild(el("span", "mhint", it.hint)); b.classList.add("tall"); }

    if (it.items) {
      b.classList.add("hassub");
      const openSub = () => {
        holdSub();
        if (b.classList.contains("subopen")) return;
        closeSubs();
        const sub = buildMenu(it.items);
        sub.classList.add("sub");
        sub.dataset.owner = "1";
        document.body.appendChild(sub);
        const r = b.getBoundingClientRect();
        const sr = sub.getBoundingClientRect();
        // Overlap the parent row by a few pixels so the pointer never crosses
        // a gap on its way in — a gap is a mouseleave, and a mouseleave is a
        // closed menu.
        const x = r.right + sr.width > window.innerWidth - 8 ? r.left - sr.width + 6 : r.right - 6;
        sub.style.left = Math.max(4, x) + "px";
        sub.style.top = Math.max(4, Math.min(r.top - 5, window.innerHeight - sr.height - 8)) + "px";
        sub.addEventListener("mouseenter", holdSub);
        sub.addEventListener("mouseleave", dropSub);
        b.classList.add("subopen");
        subOwner = b;
      };
      b.addEventListener("mouseenter", openSub);
      b.addEventListener("mouseleave", () => { if (!inSub(b)) dropSub(); });
      b.addEventListener("click", (e) => { e.stopPropagation(); openSub(); });
    } else if (it.disabled) {
      b.addEventListener("mouseenter", () => hoverRow(b));
      b.addEventListener("click", (e) => e.stopPropagation());
    } else {
      b.addEventListener("mouseenter", () => hoverRow(b));
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        closeMenu();
        it.run();
      });
    }
    m.appendChild(b);
  }
  return m;
}

/* Add rows to a menu that is already up. Files loaded later extend the block
   and directive menus this way rather than hand-rolling buttons, so an added
   row gets the same markup, hints, submenus and keyboard behaviour as a
   built-in one. */
function menuAppend(m, items) {
  const tmp = buildMenu(items);
  while (tmp.firstChild) m.appendChild(tmp.firstChild);
}

/* Open one at a point \u2014 a right-click, or under a menu-bar button. */
function menuAt(ev, items) {
  if (ev.preventDefault) ev.preventDefault();
  if (ev.stopPropagation) ev.stopPropagation();
  closeMenu();

  const m = buildMenu(items);
  document.body.appendChild(m);

  // Keep it on screen, and never let a long menu run off the bottom.
  const r = m.getBoundingClientRect();
  const x = Math.min(ev.clientX, window.innerWidth - r.width - 8);
  const y = Math.min(ev.clientY, window.innerHeight - r.height - 8);
  m.style.left = Math.max(4, x) + "px";
  m.style.top = Math.max(4, y) + "px";
  return m;
}

const subBaseCloseMenu = closeMenu;
closeMenu = function () {
  holdSub();
  closeSubs();
  subBaseCloseMenu();
};

function copyText(s) {
  try {
    navigator.clipboard.writeText(s);
  } catch (e) { /* clipboard refused; nothing else to do */ }
}

/* Where else could this block live? Every other file in the workspace. */
function moveTargets(f, e) {
  return state.ws.files
    .filter((x) => x.path !== f.path)
    .slice(0, 12)
    .map((x) => ({
      label: `   ${x.rel}`,
      run: () => act(() => invoke("move_block", {
        from: f.path, start: e.span[0], to: x.path,
      }), true),
    }));
}

/* The menu for a whole block, in either view. */
function blockMenu(ev, f, e) {
  const sameKind = f.entries.filter((x) => x.name === e.name && x.has_block).length;
  const refuse = state.mergeRefusals ? state.mergeRefusals[e.name] : null;
  const items = [];

  items.push({ heading: `${e.name}${e.label ? " " + e.label : ""} — ${f.rel}:${e.line}` });

  items.push({
    label: "Show it in the file",
    run: () => { state.fileShown = true; applyFileLayout(); jumpTo(f.rel, e.span[0]); },
  });
  items.push({ label: "Copy its path", run: () => copyText(e.path) });
  items.push("-");

  if (e.has_block) {
    if (sameKind > 1) {
      items.push({
        label: `Fold the ${sameKind} ${e.name} blocks into one`,
        disabled: !!refuse,
        hint: refuse || undefined,
        run: () => mergeBlocks(f, e.name),
      });
    }
    items.push({
      label: "Add a directive in here…",
      hint: "Pick from what this release accepts inside it",
      run: () => addDirectiveTo(f, e),
    });
  }

  // Recategorise without dragging — dragging is fine but not everyone wants to.
  items.push("-", { heading: "Put it under" });
  for (const c of cats()) {
    const here = catFor(f.rel, e).id === c.id;
    items.push({
      label: `   ${here ? "• " : "   "}${c.name}`,
      run: () => {
        state.cfg.assign[blockKey(f.rel, e)] = c.id;
        saveCfg();
        render();
      },
    });
  }

  if (state.ws.files.length > 1) {
    items.push("-", { heading: "Move to file" }, ...moveTargets(f, e));
  }

  items.push("-", {
    label: "Comment it out",
    hint: "It stays in the file, switched off",
    danger: true,
    run: () => act(() => invoke("comment_out", { file: f.path, start: e.span[0] })),
  });
  menuAt(ev, items);
}

/* The menu for a directive inside a block. */
function directiveMenu(ev, f, chain) {
  const e = chain[chain.length - 1];
  const parent = chain.length > 1 ? chain[chain.length - 2] : null;
  const items = [{ heading: `${e.path} — ${f.rel}:${e.line}` }];

  items.push({
    label: "Show it in the file",
    run: () => { state.fileShown = true; applyFileLayout(); jumpTo(f.rel, e.span[0]); },
  });
  items.push({ label: "Copy its path", run: () => copyText(e.path) });
  if (e.value != null) {
    items.push({ label: "Copy its value", run: () => copyText(e.value) });
  }

  if (parent && e.has_block) {
    const refuse = state.mergeRefusals ? state.mergeRefusals[parent.name] : null;
    items.push("-", {
      label: `Move it into its own ${parent.name} block`,
      disabled: !!refuse,
      hint: refuse || undefined,
      run: () => splitOut(f, parent, e),
    });
  }

  items.push("-", {
    label: "Comment it out",
    hint: "It stays in the file, switched off",
    danger: true,
    run: () => act(() => invoke("comment_out", { file: f.path, start: e.span[0] })),
  });
  menuAt(ev, items);
}

/* The menu on a category heading or region. */
function categoryMenu(ev, cat) {
  menuAt(ev, [
    { heading: cat.name },
    {
      label: isFolded(cat.id) ? "Unfold it" : "Fold it away",
      run: () => { setFolded(cat.id, !isFolded(cat.id)); render(); },
    },
    {
      label: "Fold every category",
      run: () => {
        foldAllCats(true);
        render();
      },
    },
    {
      label: "Unfold every category",
      run: () => {
        foldAllCats(false);
        render();
      },
    },
    "-",
    {
      label: "Rename it…",
      run: async () => {
        const r = await ask({
          title: "Rename category",
          fields: [
            { name: "name", label: "Call it", value: cat.name, required: true },
            { name: "desc", label: "What goes in it", value: cat.desc || "",
              placeholder: "optional — shown under the heading" },
          ],
          okLabel: "Rename it",
        });
        if (r) { cat.name = r.name; cat.desc = r.desc; saveCfg(); render(); }
      },
    },
    { label: "Edit all categories…", run: openCategories },
  ]);
}

/* ---------------- attach to the existing renderers ---------------- */

const menuBaseBlockRow = blockRow;
blockRow = function (f, e) {
  const rows = menuBaseBlockRow(f, e);
  rows[0].addEventListener("contextmenu", (ev) => {
    select(f.path, [e]);
    renderInspector();
    blockMenu(ev, f, e);
  });
  return rows;
};

const menuBaseKidRows = kidRows;
kidRows = function (file, chain, depth, out) {
  const before = out.length;
  menuBaseKidRows(file, chain, depth, out);
  const f = fileByPath(file);
  if (f && out[before]) {
    out[before].addEventListener("contextmenu", (ev) => {
      select(file, chain);
      renderInspector();
      directiveMenu(ev, f, chain);
    });
  }
};

const menuBaseGroupHeader = groupHeader;
groupHeader = function (cat, count) {
  const h = menuBaseGroupHeader(cat, count);
  h.addEventListener("contextmenu", (ev) => categoryMenu(ev, cat));
  return h;
};

const menuBaseModuleCard = moduleCard;
moduleCard = function (f, e) {
  const card = menuBaseModuleCard(f, e);
  card.addEventListener("contextmenu", (ev) => {
    select(f.path, [e]);
    renderInspector();
    blockMenu(ev, f, e);
  });
  return card;
};

/* Right-clicking a line in the file picks the block that owns it, then offers
   the same menu — so the two views behave the same way. */
/* Bound once, on the pane rather than per render, so it survives every redraw.
   It used to be registered with { once: true }, which meant a second
   right-click on the same line fell through to the browser menu. Missing a
   line is left unhandled on purpose — the pane's own menu picks it up. */
$("filebody").addEventListener("contextmenu", (ev) => {
  const f = currentFile();
  if (!f) return;
  const row = ev.target.closest(".fl");
  if (!row) return;
  const n = row.querySelector(".ln");
  if (!n) return;
  const lineNo = parseInt(n.textContent, 10) - 1;
  const starts = lineIndex(f.text);
  if (starts[lineNo] === undefined) return;
  selectAtOffset(f, starts[lineNo]);
  if (!state.sel) return;
  const chain = state.sel.chain;
  if (chain.length === 1) blockMenu(ev, f, chain[0]);
  else directiveMenu(ev, f, chain);
});

/* Ask the backend once per block type why merging is refused, so the menu can
   explain rather than just refuse. */
(async () => {
  state.mergeRefusals = {};
  const kinds = ["set", "allow", "deny", "class", "security-group", "oper", "link", "listen"];
  for (const k of kinds) {
    try {
      state.mergeRefusals[k] = await invoke("mergeable", { name: k });
    } catch (e) { /* leave it unknown */ }
  }
})();
