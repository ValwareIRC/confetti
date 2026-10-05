/* Confetti — the menu bar.
 *
 * Thirteen buttons across the top was thirteen things to read every time you
 * looked up. They are five now, in the order people actually reach for them,
 * and every item says what it does in words rather than in an icon.
 *
 * Nothing here reimplements an action. The real buttons still exist, offstage,
 * with all their wiring and their enabled/disabled state; a menu item reads
 * that state and clicks the button. So there is exactly one implementation of
 * every action, and the keyboard shortcuts, the status bar and the menu can
 * never disagree about whether something is available.
 */

const canDo = (id) => { const b = $(id); return !!b && !b.disabled; };
const doIt  = (id) => { const b = $(id); if (b && !b.disabled) b.click(); };

/* The block or directive the menus act on, if anything is selected. */
function picked() {
  if (!state.sel || !state.ws) return null;
  const f = fileByPath(state.sel.file);
  if (!f) return null;
  const chain = state.sel.chain;
  return { f, chain, e: chain[chain.length - 1], parent: chain.length > 1 ? chain[chain.length - 2] : null };
}
function here() {
  if (!state.ws) return null;
  return state.file ? fileOf(state.file) : mainFile(state.ws);
}

/* An item that is real but not available right now stays in the menu, greyed,
   with the reason attached. "Why can't I?" is a worse question than a menu that
   has already answered it. */
function need(cond, why, item) {
  if (cond) return item;
  return Object.assign({}, item, { disabled: true, hint: why, run: undefined });
}

const MENUS = [
  {
    name: "File",
    items: () => {
      const f = here();
      const many = state.tabs.length > 1;
      return [
        { label: "Open a folder…", key: "Ctrl O",
          hint: "An UnrealIRCd install, or its conf/ — replaces what this tab is showing",
          run: () => doIt("btn-open") },
        { label: "Open a folder in a new tab…", key: "Ctrl ⇧ O",
          hint: "Leaves this instance where it is",
          run: () => openFolderInNewTab() },
        { label: "Recently opened", items: recentItems() },
        "-",
        { label: "Connect to a server…",
          hint: "Edit the config over your own ssh",
          run: () => doIt("btn-servers") },
        { label: "Connect in a new tab…",
          run: () => connectInNewTab() },
        "-",
        { label: "New tab", key: "Ctrl T",
          hint: "Another UnrealIRCd, side by side with this one",
          run: () => newTab() },
        need(many, "It's the only one", {
          label: "Close this tab", key: "Ctrl W", danger: true,
          run: () => closeTab(state.tab),
        }),
        ...(many ? [{ label: "Go to", items: state.tabs.map((t) => ({
          label: t.title, check: t.active, hint: t.subtitle,
          run: () => switchTo(t.id),
        })) }] : []),
        "-",
        { heading: f ? f.rel : "no file" },
        need(!!state.ws, "Open a folder first", {
          label: "New file…", run: () => newFile(f ? f.dir : ""),
        }),
        need(!!f, "Open a folder first", {
          label: "Duplicate it",
          run: () => act(() => invoke("duplicate_file", { file: f.path }), true),
        }),
        need(!!f, "Open a folder first", {
          label: "Rename it…",
          run: async () => {
            const r = await ask({
              title: `Rename ${f.name}`,
              blurb: "Anything that includes this file by name needs the same change.",
              fields: [{ name: "name", label: "File name", value: f.name, required: true }],
              okLabel: "Rename it",
            });
            if (r && r.name !== f.name) act(() => invoke("rename_file", { file: f.path, name: r.name }), true);
          },
        }),
        need(!!f, "Open a folder first", {
          label: "Delete it…", danger: true,
          run: async () => {
            const ok = await confirmAsk({
              title: `Delete ${f.rel}?`,
              body: `It holds ${f.blocks} block${f.blocks === 1 ? "" : "s"}.`,
              note: `A copy is kept beside it as ${f.name}~ so you can get it back.`,
              okLabel: "Delete it", danger: true,
            });
            if (ok) act(() => invoke("delete_file", { file: f.path }), true);
          },
        }),
        "-",
        need(canDo("btn-save"), "Nothing has changed", {
          label: $("btn-save") ? $("btn-save").textContent : "Save", key: "Ctrl S",
          hint: "Write the changed files to disk",
          run: () => doIt("btn-save"),
        }),
        need(!!state.ws, "Open a folder first", {
          label: "Reread from disk",
          hint: "Throws away anything unsaved",
          danger: true,
          run: async () => {
            const dirty = state.ws.files.filter((x) => x.dirty).length;
            const ok = await confirmAsk({
              title: "Reread the folder from disk?",
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
          },
        }),
      ];
    },
  },

  {
    name: "Edit",
    items: () => {
      const s = picked();
      const f = here();
      const h = state.history || {};
      return [
        need(canDo("btn-undo"), "Nothing to undo", {
          label: h.undo ? `Undo ${h.undo}` : "Undo", key: "Ctrl Z", run: () => doIt("btn-undo"),
        }),
        need(canDo("btn-redo"), "Nothing to redo", {
          label: h.redo ? `Redo ${h.redo}` : "Redo", key: "Ctrl ⇧ Z", run: () => doIt("btn-redo"),
        }),
        "-",
        need(canDo("btn-add"), state.ws ? "Pick a file first" : "Open a folder first", {
          label: "Add a block…", key: "Ctrl B",
          hint: "Pick from what this release accepts",
          run: () => doIt("btn-add"),
        }),
        need(!!(s && s.e.has_block), "Select a block first", {
          label: s ? `Add something inside ${s.e.name}…` : "Add a directive…",
          run: () => addDirectiveTo(s.f, s.e),
        }),
        "-",
        { heading: s ? `${s.e.path} — ${s.f.rel}:${s.e.line}` : "nothing selected" },
        need(!!s, "Select something first", {
          label: "Show it in the file",
          run: () => { state.fileShown = true; applyFileLayout(); jumpTo(s.f.rel, s.e.span[0]); },
        }),
        need(!!s, "Select something first", {
          label: "Copy its path", run: () => copyText(s.e.path),
        }),
        need(!!s, "Select something first", {
          label: "Comment it out", danger: true,
          hint: "It stays in the file, switched off",
          run: () => commentOut(s.f, s.e),
        }),
        need(!!s, "Select something first", {
          label: "Delete it", danger: true,
          hint: "Ctrl+Z brings it back",
          run: () => deleteEntry(s.f, s.e),
        }),
        "-",
        { label: "Filter the list", key: "Ctrl K", run: () => $("q").focus() },
      ];
    },
  },

  {
    name: "View",
    items: () => [
      { label: "Console", check: state.view === "console",
        hint: "Every file at once, grouped how you filed them",
        run: () => { if (state.view !== "console") doIt("btn-view"); } },
      { label: "Workbench", check: state.view === "workbench",
        hint: "One file's blocks, as cards you can rearrange",
        run: () => { if (state.view !== "workbench") doIt("btn-view"); } },
      "-",
      { label: "The config file itself", check: state.fileShown && !state.logShown,
        hint: "Side by side, and foldable",
        run: () => { if (state.logShown) showLog(false); else doIt("btn-file"); } },
      { label: "The server's log", check: !!state.logShown,
        hint: "Drag an address, an account or a channel out of it into a block",
        run: () => showLog(!state.logShown) },
      { label: "What Confetti found", check: state.showProblems,
        hint: "Dangling references, clashing ports, leftover example values",
        run: () => $("btn-problems").click() },
      "-",
      need(!!state.ws, "Open a folder first", {
        label: "Viewers", key: "Ctrl Shift V",
        hint: "What the whole config adds up to, rather than what one line says",
        items: VIEWERS.map((v) => ({
          label: v.name, hint: v.blurb, run: () => openViewer(v.id),
        })),
      }),
      "-",
      { label: "Theme", items: [
        { label: "Console", check: state.theme === "console", run: () => setThemeFromMenu("console") },
        { label: "Cream", check: state.theme === "cream", run: () => setThemeFromMenu("cream") },
      ] },
      "-",
      "-",
      { label: "Every file here", check: visScope() === SCOPE_ALL,
        hint: "Even the ones the server never reads",
        run: () => setScope(SCOPE_ALL) },
      { label: "Only what the server reads", check: visScope() === SCOPE_READ,
        hint: "Follows the include lines out from unrealircd.conf",
        run: () => setScope(SCOPE_READ) },
      need(!!state.ws, "Open a folder first", {
        label: "Show every hidden file",
        run: showEverything,
      }),
      "-",
      { label: "Categories…", hint: "Name them, colour them, reorder them",
        run: () => doIt("btn-cats") },
      need(!!state.cfg, "Open a folder first", {
        label: "Fold every category",
        run: () => { foldAllCats(true); render(); },
      }),
      need(!!state.cfg, "Open a folder first", {
        label: "Unfold every category",
        run: () => { foldAllCats(false); render(); },
      }),
    ],
  },

  {
    name: "Server",
    items: () => {
      const remote = !!(state.ws && state.ws.remote);
      const why = "Open a folder over ssh first — File › Connect to a server";
      return [
        { heading: remote ? state.ws.remote : "editing local files" },
        need(remote, why, {
          label: "Check the config",
          hint: "Runs unrealircd configtest — the only real answer",
          run: () => doIt("btn-configtest"),
        }),
        need(remote, why, {
          label: "Rehash it…",
          hint: "Reload the config on the running server",
          run: () => doIt("btn-rehash"),
        }),
        "-",
        { label: "Servers…", run: () => doIt("btn-servers") },
      ];
    },
  },

  {
    name: "Help",
    items: () => {
      const pk = state.ws && state.ws.pack;
      return [
        { heading: pk && pk.loaded ? pk.note : "no docs loaded" },
        { label: "Point Confetti at the UnrealIRCd source…",
          hint: pk && pk.loaded
            ? `${pk.with_prose} directives explained from its own wiki`
            : "It reads conf.c and doc/unrealircd_wiki.zim for hints",
          run: () => doIt("btn-docs") },
        "-",
        { label: "Keyboard shortcuts", run: showKeys },
        { label: "About Confetti", run: showAbout },
      ];
    },
  },
];

/* Folders opened before. The list is kept by the backend and pruned of
   anything that is no longer there, so it never offers a dead path. */
let recents = [];
async function loadRecents() {
  try { recents = await invoke("recent_folders"); } catch (e) { recents = []; }
}
function recentItems() {
  const here = state.ws ? state.ws.root : null;
  const list = recents.filter((p) => p !== here).slice(0, 10);
  if (!list.length) {
    return [{ label: "nothing yet", disabled: true, hint: "Folders you open are remembered here" }];
  }
  return list.map((p) => ({
    label: p.split(/[\\/]/).filter(Boolean).slice(-2).join("/"),
    hint: p.replace(/\\/g, "/"),
    run: async () => {
      try {
        const ws = await invoke("open_workspace", { path: p });
        if (ws) await loadWorkspace(ws);
      } catch (e) { oops(e); }
    },
  }));
}
loadRecents();

function setThemeFromMenu(name) {
  if (state.theme !== name) { setTheme(name); renderChrome(); }
}

function showKeys() {
  say({
    title: "Keyboard",
    body: [
      "Ctrl K   jump to the filter",
      "Ctrl Z   undo          Ctrl ⇧ Z   redo",
      "Ctrl S   save          Ctrl O      open a folder",
      "Ctrl B   add a block",
      "Escape   close whatever is open, or leave the filter",
      "",
      "Right-click anything — a block, a directive, a category heading, a line",
      "in the file — for what you can do to it.",
    ].join("\n"),
  });
}

function showAbout() {
  say({
    title: "Confetti",
    body: "A config editor for UnrealIRCd.\n\n"
        + "It never rewrites a byte you did not change: your comments, your "
        + "blank lines and your indentation come back exactly as you left "
        + "them. What it offers you comes from the server's own source — the "
        + "blocks in conf.c and the prose in doc/unrealircd_wiki.zim — so it "
        + "describes the release you are actually running, not a release "
        + "someone documented once.",
  });
}

/* ---------------- the bar ---------------- */

let openMenu = null;

function menuUnder(btn, items) {
  const r = btn.getBoundingClientRect();
  menuAt({
    clientX: r.left, clientY: r.bottom + 2,
    preventDefault() {}, stopPropagation() {},
  }, items);
  const m = document.querySelector(".menu:not(.sub)");
  if (m) m.classList.add("dropdown");
  btn.classList.add("open");
  openMenu = btn;
}

function buildBar() {
  const bar = $("mbar");
  bar.textContent = "";
  for (const def of MENUS) {
    const b = el("button", "mtop", def.name);
    b.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const wasOpen = openMenu === b;
      closeMenu();
      if (!wasOpen) menuUnder(b, def.items());
    });
    // Once one is open, sliding along the bar opens the next — the way a menu
    // bar has behaved since 1984.
    b.addEventListener("mouseenter", () => {
      if (openMenu && openMenu !== b) { closeMenu(); menuUnder(b, def.items()); }
    });
    bar.appendChild(b);
  }
}

const barBaseCloseMenu = closeMenu;
closeMenu = function () {
  barBaseCloseMenu();
  if (openMenu) { openMenu.classList.remove("open"); openMenu = null; }
};

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && openMenu) { e.preventDefault(); closeMenu(); return; }
  if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return;
  const k = e.key.toLowerCase();
  if (k === "s") { e.preventDefault(); doIt("btn-save"); }
  else if (k === "o") { e.preventDefault(); doIt("btn-open"); }
  else if (k === "b") { e.preventDefault(); doIt("btn-add"); }
});

buildBar();
