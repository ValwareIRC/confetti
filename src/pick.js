/* Confetti — choosing what to add, and removing what you don't want.
 *
 * Nothing here asks you to remember a name. The block list comes from
 * `_ConfigCommands[]` in the server's own source plus the blocks modules add,
 * and the directive list from the same scrape — so what you are offered is
 * exactly what this release accepts, and nothing else. Typing a name is still
 * allowed, for a module Confetti has never met; it is just not the default.
 */

/* A searchable list of choices. Returns via the callback; Escape cancels. */
function chooser(opts) {
  const back = el("div", "modal");
  const box = el("div", "sheet");
  box.appendChild(el("div", "sheeth", opts.title));
  if (opts.blurb) box.appendChild(el("div", "tiny", opts.blurb));

  const search = el("input", "search wide");
  search.placeholder = opts.placeholder || "type to narrow it down";
  search.spellcheck = false;
  box.appendChild(search);

  const list = el("div", "picklist");
  box.appendChild(list);

  const footer = el("div", "irow");
  const custom = el("button", "btn", opts.customLabel || "Something else…");
  if (opts.customLabel === false) custom.hidden = true;
  custom.addEventListener("click", async () => {
    const r = await ask({
      title: opts.customLabel || "Something else",
      blurb: opts.customPrompt || "Name it exactly as the config spells it.",
      fields: [{ name: "name", label: "Name", required: true, placeholder: "no braces, no semicolon" }],
      okLabel: "Use it",
    });
    if (!r) return;
    back.remove();
    opts.onPick({ name: r.name, custom: true });
  });
  const cancel = el("button", "btn", "Cancel");
  cancel.addEventListener("click", () => back.remove());
  footer.append(custom, el("span", "spacer"), cancel);
  box.appendChild(footer);

  let shown = [];
  let cursor = 0;

  function draw() {
    const q = search.value.trim().toLowerCase();
    shown = opts.items.filter((it) =>
      !q || it.name.toLowerCase().includes(q) || (it.hint || "").toLowerCase().includes(q));
    if (cursor >= shown.length) cursor = Math.max(0, shown.length - 1);
    list.textContent = "";
    if (!shown.length) {
      list.appendChild(el("div", "tiny", `Nothing matches “${q}”. Use “Something else…” if it's a module Confetti hasn't met.`));
      return;
    }
    shown.forEach((it, i) => {
      const row = el("button", "pick" + (i === cursor ? " on" : ""));
      const nm = el("span", "pname", it.name);
      row.appendChild(nm);
      if (it.badge) row.appendChild(el("span", "pbadge", it.badge));
      if (it.hint) row.appendChild(el("span", "phint", it.hint));
      row.addEventListener("click", () => { back.remove(); opts.onPick(it); });
      row.addEventListener("mouseenter", () => { cursor = i; });
      list.appendChild(row);
    });
    const on = list.querySelector(".pick.on");
    if (on) on.scrollIntoView({ block: "nearest" });
  }

  search.addEventListener("input", () => { cursor = 0; draw(); });
  search.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); cursor = Math.min(cursor + 1, shown.length - 1); draw(); }
    else if (e.key === "ArrowUp") { e.preventDefault(); cursor = Math.max(cursor - 1, 0); draw(); }
    else if (e.key === "Enter" && shown[cursor]) { e.preventDefault(); back.remove(); opts.onPick(shown[cursor]); }
    else if (e.key === "Escape") { back.remove(); }
  });

  draw();
  back.appendChild(box);
  back.addEventListener("click", (e) => { if (e.target === back) back.remove(); });
  document.body.appendChild(back);
  search.focus();
}

/* ---------------- adding a block ---------------- */

async function addBlockTo(f) {
  let types = [];
  try { types = await invoke("block_types"); } catch (e) { /* fall back to typing */ }

  chooser({
    title: `Add a block to ${f.name}`,
    blurb: "These are the blocks this UnrealIRCd accepts, read from its own source.",
    placeholder: "oper, listen, security-group…",
    customLabel: "A block Confetti doesn't know…",
    customPrompt: "Block name, exactly as the config spells it",
    items: types.map((t) => ({
      name: t.name,
      hint: t.hint,
      badge: t.subtypes.length ? `${t.subtypes.length} kinds` : undefined,
      meta: t,
    })),
    onPick: async (choice) => {
      const t = choice.meta;
      let label = "";
      if (t && t.subtypes && t.subtypes.length) {
        // A fixed vocabulary — offer it rather than asking them to recall it.
        return chooser({
          title: `What kind of ${t.name} block?`,
          blurb: t.label_hint || "",
          items: t.subtypes.map((s) => ({ name: s })),
          customLabel: "Something else…",
          onPick: (sub) => reallyAddBlock(f, t.name, sub.name),
        });
      }
      if (t && t.label_hint) {
        // A name that isn't given is still allowed — the problems panel says so
        // the moment the block exists, which teaches better than a refusal.
        const r = await ask({
          title: `New ${t.name} block`,
          blurb: t.hint || "",
          fields: [{ name: "label", label: "Name it", placeholder: t.label_hint, hint: t.label_hint }],
          okLabel: "Add it",
        });
        if (!r) return;
        label = r.label;
      }
      reallyAddBlock(f, choice.name, label);
    },
  });
}

function reallyAddBlock(f, name, label) {
  act(() => invoke("add_block", { file: f.path, name: name.trim(), label: (label || "").trim() }));
}

/* ---------------- adding a directive ---------------- */

async function addDirectiveTo(f, parent) {
  if (parent.matcher) return addCriterionTo(f, parent);
  if (parent.criterion && parent.criterion.kind === "list") return addListItemTo(f, parent);

  let choices = [];
  try {
    choices = await invoke("child_directives", { path: parent.path });
  } catch (e) { /* fall back to typing */ }

  // Keyed on what the backend actually sends, which is Kind serialised
  // kebab-case. This table used to have "on/off" in it — a key that exists
  // nowhere — so every boolean fell straight through to a free text box.
  const KIND_HINT = {
    number: "a plain number",
    duration: "1d, 2h, 90s — or plain seconds",
    size: "10k, 2M — or plain bytes",
    rate: "3:60 — three per sixty seconds",
    port: "6697, or a range like 6667-6669",
    ip: "an address, or a range like 127.0.0.0/8",
    mask: "a hostmask, e.g. *@*.example.org",
    modes: "+iwx",
    password: "the password",
    text: "",
  };

  const finish = async (name, isBlock, kind, matcher, choices) => {
    if (matcher) return addMatcherTo(f, parent, name);
    if (isBlock || kind === "list") {
      return act(() => invoke("add_child", {
        file: f.path, parentStart: parent.span[0], name, value: "", asBlock: true,
      }));
    }
    // A yes/no arrives switched on, and is one click to flip. There is nothing
    // to ask: both possible answers are already on screen as a switch.
    if (kind === "yes-no") {
      return act(() => invoke("add_child", {
        file: f.path, parentStart: parent.span[0], name, value: "yes", asBlock: false,
      }));
    }
    // Naming another block? Offer the ones that exist, from the workspace
    // rather than from the source: `parent` in an operclass should list your
    // operclasses, not ask you to remember them.
    const refKind = referencedBlock(`${parent.path}::${name}`);
    if (refKind) {
      const names = definedNames(refKind);
      if (names.length) {
        const r = await ask({
          title: `${name} inside ${parent.name}`,
          blurb: `Which ${refKind} block it points at.`,
          fields: [{
            name: "value", label: refKind, choices: names.map((n) => n.name),
            hint: "Cancel and use the menu on the value if you need one Confetti cannot see.",
          }],
          okLabel: "Add it",
        });
        if (!r) return;
        return act(() => invoke("add_child", {
          file: f.path, parentStart: parent.span[0], name, value: r.value, asBlock: false,
        }));
      }
    }

    // Where the source enumerates the words, pick from them rather than
    // typing one. Still a select with the current text alongside, because the
    // list is read from an if-chain and may not be every case.
    const words = (choices || []).filter(Boolean);
    const r = await ask({
      title: `${name} inside ${parent.name}`,
      blurb: words.length > 1 ? "The server compares this against a fixed set of words." : "",
      fields: [{
        name: "value", label: "Set it to",
        choices: words.length > 1 ? words : undefined,
        placeholder: KIND_HINT[kind] || "leave blank for a bare flag",
        hint: words.length > 1
          ? "Read from the server's own source. Cancel and use the menu on the value if you need something not listed."
          : (kind && kind !== "text" ? `This one takes ${KIND_HINT[kind] || kind}.` : ""),
      }],
      okLabel: "Add it",
    });
    if (!r) return;
    act(() => invoke("add_child", {
      file: f.path, parentStart: parent.span[0], name, value: r.value, asBlock: false,
    }));
  };

  if (!choices.length) {
    const r = await ask({
      title: `Add to ${parent.name}`,
      blurb: "Confetti has no list for this block — point it at the UnrealIRCd source under Docs and it will.",
      fields: [{ name: "name", label: "Directive", required: true }],
      okLabel: "Next",
    });
    if (r) finish(r.name, false, null);
    return;
  }

  // Anything already in the block is worth marking, since most directives may
  // only appear once.
  const present = new Set(parent.children.map((c) => c.name));
  chooser({
    title: `Add to ${parent.path}`,
    blurb: "What this release accepts inside that block.",
    placeholder: "start typing…",
    customLabel: "A directive Confetti doesn't know…",
    customPrompt: `Directive name to put inside ${parent.name} { }`,
    items: choices.map((c) => ({
      name: c.name,
      hint: c.hint,
      badge: present.has(c.name) ? "already here"
           : referencedBlock(`${parent.path}::${c.name}`) ? "names a block"
           : (c.choices && c.choices.length > 1) ? `one of ${c.choices.length}`
           : c.matcher ? "a mask"
           : c.is_block ? "a block"
           : c.kind !== "text" ? c.kind : undefined,
      meta: c,
    })),
    onPick: (choice) => {
      const c = choice.meta;
      finish(choice.name, c ? c.is_block : false, c ? c.kind : null,
             c ? c.matcher : false, c ? c.choices : null);
    },
  });
}

/* A matcher can be written two ways and UnrealIRCd takes both:
 *
 *     mask *@*.example.org;                 one host, one line
 *     mask { ip 1.2.3.4; account bob; }     several things at once
 *
 * Offering only the first is what made `mask` inside an oper block look like a
 * plain text field. Nothing is documented below `oper::mask`, so the scrape
 * cannot tell the block form exists; `is_matcher` can.
 */
const VALUE_FORM = (name) => ({
  name: "One hostmask",
  hint: `${name} *@*.example.org;   — matches on the host alone`,
  meta: "value",
});
const BLOCK_FORM = (name) => ({
  name: "A block of criteria",
  hint: `${name} { ip 1.2.3.4; account bob; }   — several things at once`,
  meta: "block",
});

function addMatcherTo(f, parent, name) {
  chooser({
    title: `${name} inside ${parent.name}`,
    blurb: "This one can be written either way.",
    placeholder: "",
    customLabel: false,
    // `mask` is usually one host; `except` is usually a block. Both take
    // either, so both are offered — the likelier one just goes first.
    items: (name === "except" ? [BLOCK_FORM(name), VALUE_FORM(name)]
                              : [VALUE_FORM(name), BLOCK_FORM(name)]),
    onPick: async (choice) => {
      if (choice.meta === "block") {
        return act(() => invoke("add_child", {
          file: f.path, parentStart: parent.span[0], name, value: "", asBlock: true,
        }));
      }
      const r = await ask({
        title: `${name} inside ${parent.name}`,
        blurb: "Anyone whose host matches this.",
        fields: [{
          name: "value", label: "Match", required: true,
          placeholder: "*@*.example.org",
          hint: "A hostmask: user@host, with * for anything.",
        }],
        okLabel: "Add it",
      });
      if (!r) return;
      act(() => invoke("add_child", {
        file: f.path, parentStart: parent.span[0], name, value: r.value, asBlock: false,
      }));
    },
  });
}

/* `account { bobsmith; }`, `country { GB; NL; }` — a list of things, where
   every line is one of them. There is no directive to choose here; the only
   question is what to add. */
async function addListItemTo(f, parent) {
  const c = parent.criterion;
  const r = await ask({
    title: `Add to ${parent.name}`,
    blurb: c ? c.about : "",
    fields: [{
      name: "item", label: "One more", required: true,
      placeholder: LIST_PLACEHOLDER[parent.name.replace(/^exclude-/, "")] || "",
      hint: parent.children.length
        ? `Already here: ${parent.children.map((x) => x.bare ? x.name : x.value).join(", ")}`
        : "Any one of these matching is a match.",
    }],
    okLabel: "Add it",
  });
  if (!r) return;
  act(() => invoke("add_child", {
    file: f.path, parentStart: parent.span[0], name: r.item.trim(), value: "", asBlock: false,
  }));
}

/* A criterion with no value yet. The panel writes `ip;` when you pick it, so
   this is how you say what it should match — there was no way at all before. */
async function setCriterionValue(f, e) {
  const c = e.criterion;
  const kind = c ? c.kind : e.kind;
  const r = await ask({
    title: e.name,
    blurb: c ? c.about : "",
    fields: [{
      name: "value", label: "Match", required: true,
      placeholder: KIND_PLACEHOLDER[kind] || LIST_PLACEHOLDER[e.name.replace(/^exclude-/, "")] || "",
      hint: KIND_PLACEHOLDER[kind] ? `For example: ${KIND_PLACEHOLDER[kind]}` : "",
    }],
    okLabel: "Set it",
  });
  if (!r) return;
  act(() => invoke("set_bare_value", { file: f.path, start: e.span[0], value: r.value }));
}

const LIST_PLACEHOLDER = {
  account: "bobsmith",
  country: "GB",
  "security-group": "known-users",
  certfp: "a certificate fingerprint",
  ip: "127.0.0.0/8",
  mask: "*@*.example.org",
  realname: "*some words*",
};

/* ---------------- adding to a mask ---------------- */

/* Inside a matcher every line is one of eighteen criteria, each with its own
   shape: some are yes or no, some are a list of things, one is a hostmask.
   Offering a free text box here is how you end up with a mask block full of
   words the server will refuse to start on. */
async function addCriterionTo(f, parent) {
  const present = new Set(parent.children.map((c) => c.name));
  const items = [];
  for (const c of state.criteria) {
    items.push({
      name: c.name, hint: c.about, meta: c,
      badge: present.has(c.name) ? "already here" : KIND_BADGE[c.kind] || c.kind,
    });
  }
  // The same list again, negated. `exclude-tls` is as real a criterion as
  // `tls`, and having to know that you may prefix it is not discoverable.
  for (const c of state.criteria) {
    const n = "exclude-" + c.name;
    items.push({
      name: n, hint: "Never match when: " + c.about.toLowerCase(), meta: c,
      badge: present.has(n) ? "already here" : "an exception",
    });
  }

  chooser({
    title: `What should ${parent.name} match on?`,
    blurb: "These are the criteria UnrealIRCd understands inside a matcher.",
    placeholder: "mask, ip, account, country…",
    customLabel: "A criterion Confetti doesn't know…",
    customPrompt: `Criterion name to put inside ${parent.name} { }`,
    items,
    onPick: async (choice) => {
      const c = choice.meta;
      const kind = c ? c.kind : "text";

      // A list criterion is a block you then fill; a yes/no is complete the
      // moment it exists. Neither should ask you to type anything.
      if (kind === "list") {
        return act(() => invoke("add_child", {
          file: f.path, parentStart: parent.span[0], name: choice.name,
          value: "", asBlock: true,
        }));
      }
      if (kind === "yes-no") {
        return act(() => invoke("add_child", {
          file: f.path, parentStart: parent.span[0], name: choice.name,
          value: "yes", asBlock: false,
        }));
      }

      const r = await ask({
        title: `${choice.name}`,
        blurb: c ? c.about : "",
        fields: [{
          name: "value", label: "Match", required: true,
          placeholder: KIND_PLACEHOLDER[kind] || "",
          hint: KIND_PLACEHOLDER[kind] ? `For example: ${KIND_PLACEHOLDER[kind]}` : "",
        }],
        okLabel: "Add it",
      });
      if (!r) return;
      act(() => invoke("add_child", {
        file: f.path, parentStart: parent.span[0], name: choice.name,
        value: r.value, asBlock: false,
      }));
    },
  });
}

const KIND_BADGE = {
  "yes-no": "yes or no",
  list: "a list",
  mask: "a hostmask",
  ip: "an address",
  number: "a number",
  duration: "a length of time",
};

const KIND_PLACEHOLDER = {
  mask: "*!*@*.example.net",
  ip: "127.0.0.0/8",
  number: "100",
  duration: "1h",
  text: "",
};

/* ---------------- removing ---------------- */

function describeEntry(e) {
  const what = e.label ? `${e.name} ${e.label}` : e.name;
  const n = e.children.length;
  return n ? `${what} — ${n} setting${n === 1 ? "" : "s"} inside` : what;
}

function commentOut(f, e) {
  act(() => invoke("comment_out", { file: f.path, start: e.span[0] }));
}

async function deleteEntry(f, e) {
  const ok = await confirmAsk({
    title: `Delete ${describeEntry(e)}?`,
    body: `It goes from ${f.rel}.`,
    note: "Ctrl+Z brings it back, and nothing reaches the disk until you save.",
    okLabel: "Delete it",
    danger: true,
  });
  if (!ok) return;
  act(() => invoke("delete_entry", { file: f.path, start: e.span[0] }));
}

/* ---------------- replace the prompt-driven paths ---------------- */

/* Both menus get a proper delete beside the comment-out. */
const pickBaseBlockMenu = blockMenu;
blockMenu = function (ev, f, e) {
  pickBaseBlockMenu(ev, f, e);
  const m = document.querySelector(".menu");
  if (!m) return;
  menuAppend(m, [
    { label: "Delete it", danger: true,
      hint: "Gone from the file. Ctrl+Z brings it back.",
      run: () => deleteEntry(f, e) },
    { label: "Add another block to this file…", run: () => addBlockTo(f) },
  ]);
};

const pickBaseDirectiveMenu = directiveMenu;
directiveMenu = function (ev, f, chain) {
  pickBaseDirectiveMenu(ev, f, chain);
  const m = document.querySelector(".menu");
  if (!m) return;
  const e = chain[chain.length - 1];
  menuAppend(m, [
    ...(e.has_block
      ? [{ label: `Add something inside ${e.name}…`, run: () => addDirectiveTo(f, e) }]
      : []),
    { label: "Delete it", danger: true,
      hint: "Gone from the file. Ctrl+Z brings it back.",
      run: () => deleteEntry(f, e) },
  ]);
};

/* And the inspector gets the same two, so they are not menu-only. */
const pickBaseRenderInspector = renderInspector;
renderInspector = function () {
  pickBaseRenderInspector();
  if (!state.sel || !state.ws) return;
  const f = fileByPath(state.sel.file);
  if (!f) return;
  const e = state.sel.chain[state.sel.chain.length - 1];
  const p = $("insp");

  // The inspector already draws "+ add to <block>" above; a second button
  // saying the same thing in different words is how you end up unsure which
  // block you are adding to.
  const row = el("div", "irow");
  const c = el("button", "btn", "Comment out");
  c.title = "It stays in the file, switched off";
  c.addEventListener("click", () => commentOut(f, e));
  const d = el("button", "btn danger", "Delete");
  d.addEventListener("click", () => deleteEntry(f, e));
  row.append(c, el("span", "spacer"), d);
  p.appendChild(row);
};

