/* Confetti — viewers.
 *
 * Every one of these answers a question the files cannot: not "what does this
 * line say" but "what does the whole thing add up to". An operclass inherits
 * from a parent in another file; a setting you never wrote still has a value;
 * an allow block only matters if no earlier one matched first. Reading is not
 * enough for any of that, so these work it out and show it.
 *
 * They are read-only. Nothing here edits your config — every row is a way
 * back to the line it came from.
 */

const VIEWERS = [
  { id: "operclass-tree", icon: "tree", name: "Operclass family tree",
    blurb: "Authority, not nesting: a class sits above the class it inherits from, because inheriting means having everything that one has and more. Click a box for what it ends up able to do.",
    build: viewOperclassTree },
  { id: "who-can", icon: "grid", name: "Who can do what",
    blurb: "One column per operclass your opers actually use, one row per permission. What you are usually after is where two columns differ.",
    build: viewWhoCan },
  { id: "operclass-diff", icon: "balance", name: "Compare two operclasses",
    blurb: "What one can do that the other cannot",
    build: viewOperclassDiff },
  { id: "settings", icon: "sliders", name: "Effective settings",
    blurb: "Every set directive, yours or the server's default, and where it came from",
    build: viewSettings },
  { id: "funnel", icon: "funnel", name: "Connection funnel",
    blurb: "Put an address in and watch it fall through your rules",
    build: viewFunnel },
  { id: "includes", icon: "web", name: "Include graph",
    blurb: "What the server actually reads, and what is sitting there unread",
    build: viewIncludes },
  { id: "ports", icon: "plug", name: "Port map",
    blurb: "Every listener on one board, with the clashes marked",
    build: viewPorts },
  { id: "bans", icon: "noentry", name: "Bans and exceptions",
    blurb: "Every rule that keeps someone out, and every one that lets them past",
    build: viewBans },
  { id: "groups", icon: "people", name: "Security groups",
    blurb: "What each group catches, and everywhere it is then used",
    build: viewGroups },
  { id: "roster", icon: "list", name: "Roster",
    blurb: "Your staff as a chain of command, ranked by their operclasses — or any block type as a plain table.",
    build: viewRoster },
  { id: "stock", icon: "box", name: "Changed from stock",
    blurb: "What you have altered from the config UnrealIRCd ships",
    build: viewStock },
  { id: "running", icon: "signal", name: "Config vs the running server",
    blurb: "What the files say against what the server is actually doing",
    build: viewRunning },
  { id: "logblocks", icon: "receipt", name: "Log, by block",
    blurb: "The server's own complaints, filed against the block each one is about",
    build: viewLogBlocks },
];

/* ---------------- the shell ---------------- */

let viewerOpen = null;

function openViewer(id) {
  closeMenu();
  const back = el("div", "modal");
  const box = el("div", "sheet viewer");

  const head = el("div", "vhead");
  const title = el("div", "sheeth", "Viewers");
  const close = el("button", "btn", "Close");
  close.addEventListener("click", () => back.remove());
  head.append(title, el("span", "spacer"), close);
  box.appendChild(head);

  const main = el("div", "vmain");
  const rail = el("div", "vrail");
  const body = el("div", "vbody");
  main.append(rail, body);
  box.appendChild(main);

  const show = async (v) => {
    viewerOpen = v.id;
    for (const b of rail.querySelectorAll(".vpick")) b.classList.toggle("on", b.dataset.id === v.id);
    title.textContent = v.name;
    body.textContent = "";
    body.appendChild(el("div", "vblurb", v.blurb));
    const holder = el("div", "vhold");
    body.appendChild(holder);
    try {
      await v.build(holder, back);
    } catch (e) {
      holder.appendChild(el("div", "vempty", "That view couldn't be built: " + e));
    }
  };

  for (const v of VIEWERS) {
    const b = el("button", "vpick");
    b.dataset.id = v.id;
    const vi = el("span", "vicon");
    vi.appendChild(icon(v.icon));
    b.append(vi, el("span", null, v.name));
    b.addEventListener("click", () => show(v));
    rail.appendChild(b);
  }

  back.appendChild(box);
  back.addEventListener("click", (e) => { if (e.target === back) back.remove(); });
  document.addEventListener("keydown", function esc(e) {
    if (e.key === "Escape" && document.body.contains(back)) { back.remove(); }
    if (!document.body.contains(back)) document.removeEventListener("keydown", esc);
  });
  document.body.appendChild(back);
  show(VIEWERS.find((v) => v.id === (id || viewerOpen)) || VIEWERS[0]);
}

/* ---------------- small helpers ---------------- */

/* The files these views work from: whatever the rest of the app is showing.
   With the scope set to "only what the server reads", the examples/ folder is
   not part of your config and should not turn up in a roster of your opers. */
function vFiles() {
  if (!state.ws) return [];
  return state.ws.files.filter((f) => typeof isShown !== "function" || isShown(f));
}

/* Every block of a kind, across those files. */
function vBlocks(name) {
  const out = [];
  for (const f of vFiles()) {
    for (const e of f.entries) if (e.name === name) out.push({ f, e });
  }
  return out;
}
function vKid(e, name) { return (e.children || []).find((c) => c.name === name); }
function vKids(e, name) { return (e.children || []).filter((c) => c.name === name); }
function vVal(e, name) {
  const c = vKid(e, name);
  if (!c) return null;
  return c.bare ? c.name : (c.value ?? "");
}

/* A row that takes you back to the line it came from. */
function vWhere(f, e, label) {
  const a = el("button", "vwhere", label || `${f.name}:${e.line}`);
  a.title = "Show it in the file";
  a.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const modal = a.closest(".modal");
    if (modal) modal.remove();
    state.fileShown = true;
    applyFileLayout();
    jumpTo(f.rel, e.span[0]);
    select(f.path, [e]);
    render();
  });
  return a;
}

function vSection(parent, title, note) {
  const s = el("div", "vsec");
  s.appendChild(el("div", "vsech", title));
  if (note) s.appendChild(el("div", "tiny", note));
  parent.appendChild(s);
  return s;
}

function vTable(cols) {
  const t = el("div", "vtable");
  const h = el("div", "vtr vth");
  for (const c of cols) h.appendChild(el("span", "vtd", c));
  t.appendChild(h);
  t.addRow = (cells, cls) => {
    const r = el("div", "vtr" + (cls ? " " + cls : ""));
    for (const c of cells) {
      const d = el("span", "vtd");
      if (c instanceof Node) d.appendChild(c);
      else d.textContent = c == null ? "" : String(c);
      r.appendChild(d);
    }
    t.appendChild(r);
    return r;
  };
  return t;
}

function vEmpty(parent, text) {
  parent.appendChild(el("div", "vempty", text));
}

function vChip(text, cls) { return el("span", "vchip" + (cls ? " " + cls : ""), text); }

/* `*.example.org` against `irc.example.org`. The server's own matching is
   richer than this; where a rule uses something we cannot judge from the file
   alone, the viewer says so rather than guessing. */
function vGlob(pattern, text) {
  if (pattern == null || text == null) return false;
  const rx = new RegExp("^" + String(pattern)
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")
    .replace(/\*/g, ".*")
    .replace(/\?/g, ".") + "$", "i");
  return rx.test(text);
}

/* ---------------- operclasses ---------------- */

/* Every operclass in the workspace, by name. */
function operclassMap() {
  const m = new Map();
  for (const { f, e } of vBlocks("operclass")) {
    if (!e.label) continue;
    m.set(e.label, {
      name: e.label, f, e,
      parent: vVal(e, "parent"),
      own: permPaths(vKid(e, "permissions")),
    });
  }
  return m;
}

/* `permissions { channel { override { mode; } } }` flattened to the paths the
   server checks: channel, channel:override, channel:override:mode. A branch
   granted on its own grants everything under it, so the branch itself is a
   permission too. */
function permPaths(block) {
  const out = [];
  const walk = (node, prefix) => {
    for (const c of (node.children || [])) {
      const name = c.bare ? c.name : c.name;
      if (!name) continue;
      const path = prefix ? `${prefix}:${name}` : name;
      out.push(path);
      if (c.has_block) walk(c, path);
    }
  };
  if (block) walk(block, "");
  return out;
}

/* Everything a class ends up with, each permission tagged with the class that
   granted it. Walks up the parent chain, and stops on a loop rather than
   spinning — UnrealIRCd itself refuses a config with one. */
function effectivePerms(map, name) {
  const got = new Map();
  const chain = [];
  const seen = new Set();
  let cur = name;
  let loop = null;
  while (cur) {
    if (seen.has(cur)) { loop = cur; break; }
    seen.add(cur);
    const oc = map.get(cur);
    if (!oc) { chain.push({ name: cur, missing: true }); break; }
    chain.push({ name: cur, oc });
    for (const p of oc.own) if (!got.has(p)) got.set(p, cur);
    cur = oc.parent;
  }
  return { got, chain, loop };
}

/* A permission granted here that an ancestor already granted. */
function redundantPerms(map, name) {
  const oc = map.get(name);
  if (!oc || !oc.parent) return [];
  const up = effectivePerms(map, oc.parent);
  return oc.own.filter((p) => up.got.has(p));
}

/* ---------------- the chart ---------------- */

/* A drawn tree: operclasses as boxes, parents above children, and the opers
   that use each class hanging underneath it. Everything on one canvas, so the
   shape of who-inherits-from-whom is something you see rather than something
   you reconstruct from an indented list. */

const NODE = { classW: 176, classH: 50, operW: 148, operH: 32, hgap: 16, vgap: 40 };

/* Lay a forest out: children first, each parent centred over its own. */
function layOut(roots) {
  // How far right each row has been filled. A node goes where its children
  // put it, unless that would land on something already there — in which case
  // it moves right and takes its children with it, so the elbows stay square.
  const free = [];
  const shift = (n, dx) => {
    n.x += dx;
    free[n.depth] = Math.max(free[n.depth] || 0, n.x + n.w + NODE.hgap);
    for (const k of n.kids) shift(k, dx);
  };
  const rows = [];
  const place = (n, depth) => {
    n.depth = depth;
    rows[depth] = Math.max(rows[depth] || 0, n.h);
    for (const k of n.kids) place(k, depth + 1);
    let x;
    if (n.kids.length) {
      const first = n.kids[0];
      const last = n.kids[n.kids.length - 1];
      x = (first.x + last.x + last.w) / 2 - n.w / 2;
    } else {
      x = free[depth] || 0;
    }
    const min = free[depth] || 0;
    if (x < min) {
      for (const k of n.kids) shift(k, min - x);
      x = min;
    }
    n.x = x;
    free[depth] = x + n.w + NODE.hgap;
  };
  for (const r of roots) place(r, 0);
  const cursor = Math.max(0, ...free.map((v) => v || 0));

  // Rows are as tall as the tallest thing on them.
  const tops = [];
  let y = 0;
  for (let d = 0; d < rows.length; d++) {
    tops[d] = y;
    y += (rows[d] || 0) + NODE.vgap;
  }
  const all = [];
  const walk = (n) => { n.y = tops[n.depth]; all.push(n); n.kids.forEach(walk); };
  roots.forEach(walk);
  return { nodes: all, width: Math.max(cursor, 1), height: Math.max(y, 1) };
}

function svgEl(name, attrs) {
  const n = document.createElementNS("http://www.w3.org/2000/svg", name);
  for (const k in attrs) n.setAttribute(k, attrs[k]);
  return n;
}

/* An elbow from the bottom of a parent to the top of a child. */
function elbow(p, c) {
  const x1 = p.x + p.w / 2, y1 = p.y + p.h;
  const x2 = c.x + c.w / 2, y2 = c.y;
  const mid = y1 + (y2 - y1) / 2;
  const r = Math.min(9, Math.abs(x2 - x1) / 2, (y2 - y1) / 2);
  if (Math.abs(x2 - x1) < 1) return `M${x1} ${y1} L${x2} ${y2}`;
  const dir = x2 > x1 ? 1 : -1;
  return `M${x1} ${y1} L${x1} ${mid - r}` +
         ` Q${x1} ${mid} ${x1 + dir * r} ${mid}` +
         ` L${x2 - dir * r} ${mid}` +
         ` Q${x2} ${mid} ${x2} ${mid + r}` +
         ` L${x2} ${y2}`;
}

/* A chart you can drag and zoom, because a config with twenty operclasses
   will not fit however carefully it is laid out. */
function chart(host, roots, onPick, chosenId, drawInto) {
  const laid = layOut(roots);
  const pad = 24;
  const box = el("div", "chartbox");
  const vbW = laid.width + pad * 2;
  const vbH = laid.height + pad * 2;
  const svg = svgEl("svg", {
    class: "chart",
    viewBox: `${-pad} ${-pad} ${vbW} ${vbH}`,
    preserveAspectRatio: "xMidYMin meet",
  });
  // Let the drawing be as tall as it needs, within reason, instead of a fixed
  // slab with a field of empty underneath it.
  svg.style.aspectRatio = `${vbW} / ${vbH}`;
  const root = svgEl("g", {});
  svg.appendChild(root);

  const lines = svgEl("g", { class: "edges" });
  root.appendChild(lines);
  for (const n of laid.nodes) {
    for (const k of n.kids) {
      lines.appendChild(svgEl("path", { d: elbow(n, k), class: "edge " + k.kind }));
    }
  }

  for (const n of laid.nodes) {
    const g = svgEl("g", {
      class: `node ${n.kind}${n.id === chosenId ? " on" : ""}`,
      transform: `translate(${n.x} ${n.y})`,
      tabindex: "0",
    });
    if (drawInto) {
      drawInto(n, g);
    } else {
      g.appendChild(svgEl("rect", { width: n.w, height: n.h, rx: 8 }));
      const t = svgEl("text", { x: n.w / 2, y: 21, class: "nlabel" });
      t.textContent = n.label;
      g.appendChild(t);
      if (n.sub) {
        const s2 = svgEl("text", { x: n.w / 2, y: 37, class: "nsub" });
        s2.textContent = n.sub;
        g.appendChild(s2);
      }
    }
    g.addEventListener("click", () => onPick(n));
    g.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onPick(n); } });
    const title = svgEl("title", {});
    title.textContent = n.title || n.label;
    g.appendChild(title);
    root.appendChild(g);
  }

  // Drag to pan, wheel to zoom, and a way back when you get lost.
  let view = { x: -pad, y: -pad, w: laid.width + pad * 2, h: laid.height + pad * 2 };
  const home = { ...view };
  const apply = () => svg.setAttribute("viewBox", `${view.x} ${view.y} ${view.w} ${view.h}`);
  svg.addEventListener("wheel", (e) => {
    e.preventDefault();
    const r = svg.getBoundingClientRect();
    const fx = (e.clientX - r.left) / r.width, fy = (e.clientY - r.top) / r.height;
    const k = e.deltaY > 0 ? 1.12 : 1 / 1.12;
    const nw = Math.min(home.w * 4, Math.max(home.w / 8, view.w * k));
    const nh = view.h * (nw / view.w);
    view.x += (view.w - nw) * fx;
    view.y += (view.h - nh) * fy;
    view.w = nw; view.h = nh;
    apply();
  }, { passive: false });
  let drag = null;
  svg.addEventListener("pointerdown", (e) => {
    if (e.target.closest(".node")) return;
    drag = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
    svg.setPointerCapture(e.pointerId);
    svg.classList.add("grabbing");
  });
  svg.addEventListener("pointermove", (e) => {
    if (!drag) return;
    const r = svg.getBoundingClientRect();
    view.x = drag.vx - (e.clientX - drag.x) * (view.w / r.width);
    view.y = drag.vy - (e.clientY - drag.y) * (view.h / r.height);
    apply();
  });
  const stop = () => { drag = null; svg.classList.remove("grabbing"); };
  svg.addEventListener("pointerup", stop);
  svg.addEventListener("pointercancel", stop);

  const tools = el("div", "charttools");
  const fit = el("button", "minibtn", "Fit");
  fit.addEventListener("click", () => { view = { ...home }; apply(); });
  tools.append(el("span", "tiny", "drag to move · wheel to zoom"), el("span", "spacer"), fit);

  box.append(tools, svg);
  host.appendChild(box);
  return box;
}

/* Rank operclasses by authority rather than by nesting.
 *
 * `operclass X { parent Y; }` means X starts from everything Y may do and adds
 * to it — so X is the stronger of the two, and X is in charge of Y. Drawing
 * the config's nesting literally put netadmin-with-override *underneath*
 * netadmin, which says the opposite of what it means. The chart is the
 * inheritance turned upside down: whoever inherits sits above whoever they
 * inherited from.
 *
 * Returns, for each class: who derives from it (its superiors), the one the
 * chart hangs it under, and how far down it sits.
 */
function classRanks(map) {
  const derived = new Map();   // Y -> [X, ...] where X { parent Y }
  for (const oc of map.values()) {
    if (!oc.parent || !map.has(oc.parent)) continue;
    if (!derived.has(oc.parent)) derived.set(oc.parent, []);
    derived.get(oc.parent).push(oc.name);
  }
  const depth = new Map();
  const depthOf = (n, seen) => {
    if (depth.has(n)) return depth.get(n);
    if (seen.has(n)) return 0;            // a loop; the detail panel says so
    seen.add(n);
    const up = derived.get(n) || [];
    const d = up.length ? 1 + Math.max(...up.map((x) => depthOf(x, seen))) : 0;
    depth.set(n, d);
    return d;
  };
  for (const n of map.keys()) depthOf(n, new Set());

  // Two classes can share a parent, which makes this a lattice rather than a
  // tree. The deepest superior holds it in the layout; the others get a line
  // drawn in afterwards so nothing is hidden and nothing is duplicated.
  const holder = new Map();
  const extra = [];
  for (const [y, xs] of derived) {
    const sorted = [...xs].sort((a, b) => depth.get(b) - depth.get(a) || a.localeCompare(b));
    holder.set(y, sorted[0]);
    for (const x of sorted.slice(1)) extra.push([x, y]);
  }
  const roots = [...map.keys()].filter((n) => !holder.has(n)).sort();
  const under = new Map();
  for (const [y, x] of holder) {
    if (!under.has(x)) under.set(x, []);
    under.get(x).push(y);
  }
  return { derived, depth, under, extra, roots };
}

/* ---------------- operclass family tree ---------------- */

function viewOperclassTree(host) {
  const map = operclassMap();
  if (!map.size) return vEmpty(host, "No operclass blocks in this workspace.");
  const rank = classRanks(map);

  let chosen = rank.roots[0] || [...map.keys()][0];

  const build = (name) => {
    const oc = map.get(name);
    const eff = effectivePerms(map, name);
    return {
      kind: "class", id: name, label: name,
      sub: `${eff.got.size} permissions`,
      title: `${name} — ${oc.f.rel}:${oc.e.line}`,
      w: NODE.classW, h: NODE.classH,
      kids: (rank.under.get(name) || []).sort().map(build),
      oc,
    };
  };

  const wrap = el("div", "chartwrap");
  host.appendChild(wrap);
  const detail = el("div", "vdetail");

  const redraw = () => {
    wrap.textContent = "";
    const c = chart(wrap, rank.roots.map(build), (n) => { chosen = n.id; redraw(); }, chosen);
    wrap.appendChild(el("div", "tiny",
      "Higher up means more authority: a class sits above the class it inherits from, " +
      "because inheriting means having everything that one has and more."));
    wrap.appendChild(detail);
    drawDetail();
    return c;
  };

  const drawDetail = () => {
    detail.textContent = "";
    const oc = map.get(chosen);
    if (!oc) return;
    detail.appendChild(el("div", "vdh", oc.name));
    const line = el("div", "vsub");
    line.append(vWhere(oc.f, oc.e, `${oc.f.rel}:${oc.e.line}`));
    detail.appendChild(line);

    const eff = effectivePerms(map, oc.name);
    if (eff.loop) {
      detail.appendChild(el("div", "vwarn",
        `The parent chain comes back to ${eff.loop}. UnrealIRCd refuses to start on a loop like this.`));
    }
    const missing = eff.chain.find((c) => c.missing);
    if (missing) {
      detail.appendChild(el("div", "vwarn",
        `Its parent ${missing.name} is not defined anywhere Confetti can see.`));
    }

    const over = rank.derived.get(oc.name) || [];
    const under = oc.parent && map.has(oc.parent) ? oc.parent : null;
    const rel = el("div", "vsub");
    rel.appendChild(el("span", null, over.length
      ? `In charge of it: ${over.join(", ")}. ` : "Nothing is above it. "));
    rel.appendChild(el("span", null, under
      ? `It is in charge of ${under}, and has everything ${under} has.`
      : "It answers to nothing — every permission below is written out here."));
    detail.appendChild(rel);

    const dup = redundantPerms(map, oc.name);
    if (dup.length) {
      detail.appendChild(el("div", "vnote",
        `${dup.length} of these are already granted further up, so writing them here changes nothing: ${dup.join(", ")}`));
    }
    const own = eff.chain[0] && eff.chain[0].oc ? eff.chain[0].oc.own.length : 0;
    detail.appendChild(el("div", "vsech",
      `${eff.got.size} permissions — ${own} written here, ${eff.got.size - own} inherited`));
    detail.appendChild(permTree(eff.got, oc.name));
  };
  redraw();
}

/* Permissions are a tree in the server too — channel:override:mode is three
   levels, not one word. Showing them as a flat alphabetical list threw that
   away and made a hundred rows of soup. */
function permTree(got, here) {
  const root = { kids: new Map() };
  for (const [path, from] of got) {
    let n = root;
    for (const part of path.split(":")) {
      if (!n.kids.has(part)) n.kids.set(part, { kids: new Map(), from: null });
      n = n.kids.get(part);
    }
    n.from = from;
  }
  const box = el("div", "ptree");
  const draw = (node, depth, into) => {
    for (const [name, kid] of [...node.kids.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      const row = el("div", "prow");
      row.style.paddingLeft = 4 + depth * 15 + "px";
      row.appendChild(el("span", "pdot" + (kid.from === here ? " own" : kid.from ? "" : " part")));
      row.appendChild(el("span", "pname2", name));
      if (kid.from && kid.from !== here) row.appendChild(el("span", "pfrom", kid.from));
      if (kid.from === here) row.appendChild(el("span", "pfrom own", "written here"));
      into.appendChild(row);
      draw(kid, depth + 1, into);
    }
  };
  draw(root, 0, box);
  return box;
}

/* ---------------- who can do what ---------------- */

/* A grid, not a list. Rows are the permission tree; columns are the classes
   your opers actually use, with the people in each column named at the top.
   What you want from this view is the difference between two columns, and a
   flat alphabetical list of every permission with a "granted by" beside it
   never showed that. */
function viewWhoCan(host) {
  const map = operclassMap();
  const opers = vBlocks("oper");
  if (!opers.length) return vEmpty(host, "No oper blocks in this workspace.");

  const cols = [];
  const seen = new Map();
  for (const { f, e } of opers) {
    const cls = vVal(e, "operclass") || "no operclass";
    if (!seen.has(cls)) {
      const c = { cls, opers: [], eff: map.has(cls) ? effectivePerms(map, cls) : { got: new Map() } };
      seen.set(cls, c);
      cols.push(c);
    }
    seen.get(cls).opers.push({ f, e, name: e.label || "(unnamed)" });
  }
  cols.sort((a, b) => b.eff.got.size - a.eff.got.size);

  const bar = el("div", "vbar");
  const only = el("input");
  only.type = "checkbox";
  only.checked = cols.length > 1;
  const lab = el("label", "vcheck");
  lab.append(only, el("span", null, "only where they differ"));
  const search = el("input", "search");
  search.placeholder = "narrow it down — kill, override, server-ban";
  search.spellcheck = false;
  bar.append(search, el("span", "spacer"), lab);
  host.appendChild(bar);

  const out = el("div");
  host.appendChild(out);

  const draw = () => {
    out.textContent = "";
    const q = search.value.trim().toLowerCase();

    // Every permission anybody has, as a tree.
    const all = new Set();
    for (const c of cols) for (const p of c.eff.got.keys()) all.add(p);
    const paths = [...all].filter((p) => !q || p.toLowerCase().includes(q)).sort();

    const rows = paths.filter((p) => {
      if (!only.checked || cols.length < 2) return true;
      const first = cols[0].eff.got.has(p);
      return cols.some((c) => c.eff.got.has(p) !== first);
    });

    const grid = el("div", "grid");
    grid.style.setProperty("--cols", String(cols.length));

    const head = el("div", "grow ghead");
    head.appendChild(el("span", "gname", `${rows.length} permissions`));
    for (const c of cols) {
      const h = el("span", "gcell gcolhead");
      h.appendChild(el("b", null, c.cls));
      const who = el("span", "gwho", c.opers.map((o) => o.name).join(", "));
      who.title = c.opers.map((o) => `${o.name} — ${o.f.rel}:${o.e.line}`).join("\n");
      h.appendChild(who);
      grid.appendChild(h);
      head.appendChild(h);
    }
    grid.textContent = "";
    grid.appendChild(head);

    let lastTop = null;
    for (const p of rows) {
      const parts = p.split(":");
      if (parts[0] !== lastTop) {
        lastTop = parts[0];
        const sep = el("div", "grow gsep");
        sep.appendChild(el("span", "gname", lastTop));
        for (const c of cols) sep.appendChild(el("span", "gcell"));
        grid.appendChild(sep);
      }
      const row = el("div", "grow");
      const nm = el("span", "gname");
      nm.style.paddingLeft = 6 + (parts.length - 1) * 13 + "px";
      nm.textContent = parts[parts.length - 1];
      nm.title = p;
      row.appendChild(nm);
      for (const c of cols) {
        const cell = el("span", "gcell");
        const from = c.eff.got.get(p);
        if (from) {
          const mark = el("span", "mark" + (from === c.cls ? " own" : ""));
          mark.textContent = from === c.cls ? "●" : "○";
          mark.title = from === c.cls ? `written in ${c.cls}` : `inherited from ${from}`;
          cell.appendChild(mark);
        } else {
          cell.appendChild(el("span", "mark no", "·"));
        }
        row.appendChild(cell);
      }
      grid.appendChild(row);
    }
    out.appendChild(grid);
    if (!rows.length) {
      vEmpty(out, only.checked
        ? "Every one of these classes has exactly the same permissions."
        : `Nothing matches “${q}”.`);
    }
    out.appendChild(el("div", "tiny",
      "● written in that class · ○ inherited from a parent · · not held"));
  };
  search.addEventListener("input", draw);
  only.addEventListener("change", draw);
  draw();
}

function viewOperclassDiff(host) {
  const map = operclassMap();
  const names = [...map.keys()].sort();
  if (names.length < 2) return vEmpty(host, "You need two operclasses to compare.");

  const bar = el("div", "vbar");
  const mk = (val) => {
    const s = el("select", "vsel");
    for (const n of names) {
      const o = document.createElement("option");
      o.value = n; o.textContent = n;
      s.appendChild(o);
    }
    s.value = val;
    return s;
  };
  // Two ends of the list rather than two neighbours, which are usually a
  // class and its own -with-override twin and differ by almost nothing.
  const a = mk(names[0]);
  const b = mk(names[names.length - 1]);
  bar.append(a, el("span", "vsep", "vs"), b);
  host.appendChild(bar);

  const out = el("div");
  host.appendChild(out);

  const draw = () => {
    out.textContent = "";
    const ea = effectivePerms(map, a.value).got;
    const eb = effectivePerms(map, b.value).got;
    const only = (x, y) => [...x.keys()].filter((p) => !y.has(p)).sort();
    const both = [...ea.keys()].filter((p) => eb.has(p)).sort();

    const cols = el("div", "vcols");
    const col = (title, items, cls) => {
      const c = el("div", "vcol");
      const h = el("div", "vsech");
      h.append(el("span", null, title), vChip(String(items.length), "n"));
      c.appendChild(h);
      if (!items.length) vEmpty(c, "nothing");
      for (const p of items) c.appendChild(el("div", "vitem " + cls, p));
      return c;
    };
    cols.append(
      col(`only ${a.value}`, only(ea, eb), "vplus"),
      col("both", both, "vsame"),
      col(`only ${b.value}`, only(eb, ea), "vplus"));
    out.appendChild(cols);
  };
  a.addEventListener("change", draw);
  b.addEventListener("change", draw);
  draw();
}

/* ---------------- effective settings ---------------- */

async function viewSettings(host) {
  // Every leaf under every set block, with where it was written.
  const written = new Map();
  for (const { f, e } of vBlocks("set")) {
    const walk = (node, prefix) => {
      for (const c of (node.children || [])) {
        const path = `${prefix}::${c.name}`;
        const kids = c.children || [];
        // A block whose contents are all bare words is a list of values, not
        // a set of named settings underneath it.
        const isList = c.has_block && kids.length && kids.every((k) => k.bare);
        if (c.has_block && kids.length && !isList) walk(c, path);
        else if (isList) {
          written.set(path, {
            value: kids.length === 1 ? "1 value" : `${kids.length} values`, f, e: c,
          });
        } else {
          written.set(path, { value: c.bare ? "(on)" : (c.value ?? "(on)"), f, e: c });
        }
      }
    };
    walk(e, "set");
  }

  let known = [];
  try { known = await invoke("known_settings"); } catch (err) { /* no pack yet */ }
  const byPath = new Map(known.map((k) => [k.path, k]));

  const bar = el("div", "vbar");
  const showAll = el("input");
  showAll.type = "checkbox";
  const lab = el("label", "vcheck");
  lab.append(showAll, el("span", null, "also show settings you have not written"));
  const search = el("input", "search");
  search.placeholder = "narrow it down";
  search.spellcheck = false;
  bar.append(search, el("span", "spacer"), lab);
  host.appendChild(bar);

  const out = el("div");
  host.appendChild(out);

  const draw = () => {
    out.textContent = "";
    const q = search.value.trim().toLowerCase();
    const paths = new Set(written.keys());
    if (showAll.checked) for (const k of known) paths.add(k.path);

    const rows = [...paths].filter((p) => !q || p.toLowerCase().includes(q)).sort();
    const t = vTable(["setting", "in effect", "default", "where"]);
    let changed = 0;
    for (const p of rows) {
      const w = written.get(p);
      const k = byPath.get(p);
      // The source is the authority. The wiki fills the gaps, and says so —
      // the two do disagree, and when they do the running server follows conf.c.
      const def = k && k.default != null ? k.default : null;
      const docDef = !def && k && k.doc_default != null ? k.doc_default : null;
      const shown = def != null ? el("span", null, def)
        : docDef != null ? (() => { const s2 = el("span"); s2.append(el("span", null, docDef), vChip("docs", "dim")); return s2; })()
        : el("span", "vsub", "—");
      if (w) {
        const same = def != null && String(w.value) === String(def);
        // Only where the default is actually known. Counting "we don't know"
        // as "you changed it" would have called 44 of 46 settings changed.
        if (def != null && !same) changed++;
        t.addRow([p.replace(/^set::/, ""), w.value, shown,
                  vWhere(w.f, w.e)], same ? "vsame" : def == null ? "" : "vown");
      } else {
        t.addRow([p.replace(/^set::/, ""),
                  def != null ? def : docDef != null ? docDef : "—",
                  shown, "not written"], "vdim");
      }
    }
    const known_n = [...written.keys()].filter((p) => byPath.has(p) && byPath.get(p).default != null).length;
    out.appendChild(el("div", "vsech",
      `${written.size} settings written — of the ${known_n} whose default Confetti knows, ` +
      `${changed} ${changed === 1 ? "differs" : "differ"} from it`));
    out.appendChild(t);
    if (!known.length) {
      out.appendChild(el("div", "tiny",
        "Defaults come from the UnrealIRCd source. Point Confetti at it under Docs and they appear here."));
    }
  };
  search.addEventListener("input", draw);
  showAll.addEventListener("change", draw);
  draw();
}

/* ---------------- connection funnel ---------------- */

/* What a matcher block or a plain mask would test. Anything that depends on
   the connection rather than the address is reported rather than guessed. */
function vMatchTest(entry, who) {
  const undecided = [];
  const test = (name, value) => {
    switch (name) {
      case "mask": case "match": case "ip": case "hostname": case "host":
        return vGlob(value, who.host) || vGlob(value, who.ip) ||
               vGlob(value, `${who.user}@${who.host}`) || vGlob(value, `${who.user}@${who.ip}`);
      default:
        undecided.push(name);
        return null;
    }
  };
  let result = null;
  if (!entry.has_block) {
    result = test(entry.name === "mask" || entry.name === "match" ? "mask" : entry.name,
                  entry.bare ? entry.name : entry.value);
  } else {
    // A block of criteria: any one of them matching is a match.
    for (const c of entry.children) {
      const vals = c.has_block ? c.children.map((x) => x.bare ? x.name : x.value)
                               : [c.bare ? c.name : c.value];
      for (const v of vals) {
        const r = test(c.name, v);
        if (r === true) result = true;
        else if (r === false && result === null) result = false;
      }
    }
  }
  return { result, undecided };
}

function viewFunnel(host) {
  const bar = el("div", "vbar");
  const input = el("input", "search wide");
  input.placeholder = "someone@somewhere — try bob@1.2.3.4 or bob@host.example.org";
  input.spellcheck = false;
  input.value = "someone@198.51.100.7";
  bar.appendChild(input);
  host.appendChild(bar);

  const out = el("div");
  host.appendChild(out);

  const draw = () => {
    out.textContent = "";
    const raw = input.value.trim();
    const at = raw.lastIndexOf("@");
    const who = {
      user: at > 0 ? raw.slice(0, at) : "someone",
      host: at >= 0 ? raw.slice(at + 1) : raw,
    };
    who.ip = /^[0-9.]+$|:/.test(who.host) ? who.host : "";
    if (!who.host) return vEmpty(out, "Put an address in the box.");

    const step = (title, note) => {
      const s = el("div", "vstep");
      s.appendChild(el("div", "vsech", title));
      if (note) s.appendChild(el("div", "tiny", note));
      out.appendChild(s);
      return s;
    };

    let stopped = false;

    // 1. Rules that keep people out, in the order the server would apply them.
    for (const kind of ["deny", "ban"]) {
      const s = step(`${kind} blocks`, `${vBlocks(kind).length} of them`);
      let hit = 0;
      for (const { f, e } of vBlocks(kind)) {
        const m = vKid(e, "mask") || vKid(e, "match");
        if (!m) continue;
        const r = vMatchTest(m, who);
        if (r.result === true) {
          hit++;
          stopped = true;
          const row = el("div", "vhit");
          row.append(vChip("caught", "bad"),
                     el("span", null, `${kind} ${e.label || ""}`.trim()),
                     el("span", "vsub", vVal(e, "reason") || ""),
                     vWhere(f, e));
          s.appendChild(row);
        } else if (r.undecided.length) {
          s.appendChild(el("div", "vmaybe",
            `${kind} ${e.label || ""} tests ${r.undecided.join(", ")} — that depends on the connection, not the address`));
        }
      }
      if (!hit) s.appendChild(el("div", "vpass", "nothing here catches it"));
    }

    // 2. The first allow block that matches wins; the rest never run.
    const s = step("allow blocks", "the first one that matches is the one that counts");
    let won = null;
    for (const { f, e } of vBlocks("allow")) {
      if (e.label) continue; // allow channel { } and friends are a different thing
      const m = vKid(e, "mask") || vKid(e, "match") || vKid(e, "ip") || vKid(e, "hostname");
      const r = m ? vMatchTest(m, who) : { result: null, undecided: ["nothing to match on"] };
      const row = el("div", won ? "vskip" : (r.result === true ? "vhit" : "vmiss"));
      row.append(
        vChip(won ? "never reached" : r.result === true ? "matched" : r.result === false ? "no" : "maybe",
              won ? "dim" : r.result === true ? "good" : ""),
        el("span", null, m ? (m.has_block ? `${m.name} { … }` : `${m.name} ${m.bare ? m.name : m.value}`) : "no mask"),
        el("span", "vsub", `class ${vVal(e, "class") || "—"}`),
        vWhere(f, e));
      s.appendChild(row);
      if (!won && r.result === true) won = { f, e };
    }
    if (!won) s.appendChild(el("div", "vwarn",
      "Nothing matched, so this connection would be refused — there is no default allow."));

    // 3. And what that class then permits.
    if (won) {
      const cls = vVal(won.e, "class");
      const cb = vBlocks("class").find((c) => c.e.label === cls);
      const s2 = step(`class ${cls || "—"}`, cb ? "" : "that class is not defined anywhere Confetti can see");
      if (cb) {
        const t = vTable(["limit", "value"]);
        for (const c of cb.e.children) t.addRow([c.name, c.bare ? "on" : (c.value ?? "on")]);
        s2.appendChild(t);
        s2.appendChild(vWhere(cb.f, cb.e));
      }
      const per = vVal(won.e, "maxperip") || vVal(won.e, "global-maxperip");
      if (per) s2.appendChild(el("div", "tiny", `At most ${per} connections from the same address.`));
    }

    if (stopped) {
      out.insertBefore(el("div", "vwarn",
        "A ban or deny rule already caught this address — the allow blocks below would never be reached."), out.firstChild);
    }
  };
  input.addEventListener("input", draw);
  draw();
}

/* ---------------- include graph ---------------- */

function viewIncludes(host) {
  // Every file, deliberately: this is the view that tells you which ones the
  // server never reaches, so hiding them here would defeat it.
  const files = state.ws ? state.ws.files : [];
  if (!files.length) return vEmpty(host, "Nothing open.");
  const byRel = new Map(files.map((f) => [f.rel, f]));
  const root = files.find((f) => f.name === "unrealircd.conf") || files[0];

  // Breadth-first from the root: everything reachable, and how deep.
  const depth = new Map([[root.rel, 0]]);
  const via = new Map();
  const queue = [root.rel];
  while (queue.length) {
    const rel = queue.shift();
    const f = byRel.get(rel);
    if (!f) continue;
    for (const inc of (f.includes || [])) {
      if (depth.has(inc)) continue;
      depth.set(inc, depth.get(rel) + 1);
      via.set(inc, rel);
      queue.push(inc);
    }
  }

  // Included more than once — usually harmless, occasionally the reason a
  // block is defined twice.
  const times = new Map();
  for (const f of files) {
    for (const inc of (f.includes || [])) times.set(inc, (times.get(inc) || 0) + 1);
  }

  const s = vSection(host, `Reached from ${root.name}`,
    `${depth.size} of ${files.length} files. The rest are never read by the server.`);
  const draw = (rel, into) => {
    const f = byRel.get(rel);
    if (!f) return;
    const row = el("div", "vinc");
    row.style.paddingLeft = 4 + depth.get(rel) * 20 + "px";
    row.append(el("span", "vtwig", depth.get(rel) ? "└" : "●"),
               vWhere(f, f.entries[0] || { span: [0], line: 1 }, f.rel),
               el("span", "vsub", `${f.blocks} blocks`));
    if ((times.get(rel) || 0) > 1) row.appendChild(vChip(`included ${times.get(rel)}×`, "warn"));
    for (const u of (f.includes_unresolved || [])) {
      row.appendChild(vChip(`can't follow ${u}`, "dim"));
    }
    into.appendChild(row);
    for (const inc of (f.includes || [])) {
      if (via.get(inc) === rel) draw(inc, into);
    }
  };
  draw(root.rel, s);

  const orphans = files.filter((f) => !depth.has(f.rel));
  const s2 = vSection(host, `${orphans.length} files the server never reads`,
    "Sitting in the folder, included by nothing. Editing one of these changes nothing.");
  if (!orphans.length) vEmpty(s2, "None — everything here is reached.");
  for (const f of orphans) {
    const row = el("div", "vinc vdim");
    row.append(el("span", "vtwig", "·"), vWhere(f, f.entries[0] || { span: [0], line: 1 }, f.rel),
               el("span", "vsub", `${f.blocks} blocks`));
    s2.appendChild(row);
  }
}

/* ---------------- port map ---------------- */

function viewPorts(host) {
  const rows = [];
  for (const { f, e } of vBlocks("listen")) {
    const opts = vKid(e, "options");
    const flags = opts ? (opts.children || []).map((c) => c.name) : [];
    const ports = vKids(e, "port").map((c) => c.bare ? c.name : c.value);
    // `listen { file "rpc.socket"; }` is a unix socket. It has no port, and
    // saying "?" made it look like Confetti had failed to read it.
    const sock = vVal(e, "file");
    const ip = sock ? "unix socket"
      : vVal(e, "ip") || (e.label && e.label.includes(":") ? e.label.split(":")[0] : e.label) || "*";
    const list = sock ? [sock]
      : ports.length ? ports
      : (e.label && e.label.includes(":") ? [e.label.split(":").pop()] : ["not given"]);
    for (const p of list) {
      // A range like 6667-6669 is several listeners.
      const m = /^(\d+)\s*-\s*(\d+)$/.exec(String(p).trim());
      const nums = m ? Array.from({ length: Math.max(0, +m[2] - +m[1] + 1) }, (_, i) => +m[1] + i)
                     : [String(p).trim()];
      for (const n of nums) rows.push({ f, e, ip, port: String(n), flags, sock: !!sock });
    }
  }
  if (!rows.length) return vEmpty(host, "No listen blocks in this workspace.");

  const clash = new Map();
  for (const r of rows) {
    const k = `${r.ip}:${r.port}`;
    clash.set(k, (clash.get(k) || 0) + 1);
  }

  const s = vSection(host, `${rows.length} listeners`,
    "One row per port the server would bind. Two rows on the same address and port is a clash.");
  const t = vTable(["address", "port", "how", "where"]);
  rows.sort((a, b) => (a.port - b.port) || a.ip.localeCompare(b.ip));
  for (const r of rows) {
    const how = el("span");
    const tls = r.flags.includes("tls") || r.flags.includes("ssl");
    // A unix socket never leaves the machine, so "plain" would be alarming
    // about the one listener that is not exposed at all.
    how.appendChild(r.sock ? vChip("local only", "dim")
                           : vChip(tls ? "TLS" : "plain", tls ? "good" : "warn"));
    for (const fl of r.flags) if (fl !== "tls" && fl !== "ssl") how.appendChild(vChip(fl, "dim"));
    const dup = !r.sock && clash.get(`${r.ip}:${r.port}`) > 1;
    t.addRow([r.ip, r.port, how, vWhere(r.f, r.e)], dup ? "vbad" : "");
  }
  s.appendChild(t);

  const dups = [...clash.entries()].filter(([, n]) => n > 1);
  if (dups.length) {
    s.appendChild(el("div", "vwarn",
      `${dups.length} address and port pairs are claimed more than once: ${dups.map(([k]) => k).join(", ")}`));
  }
  const anyPlain = rows.some((r) => !r.sock && !r.flags.includes("tls") && !r.flags.includes("ssl"));
  if (anyPlain) {
    s.appendChild(el("div", "tiny",
      "Plain ports are normal — most networks keep one — but everything on them is readable in transit."));
  }
}

/* ---------------- bans and exceptions ---------------- */

function viewBans(host) {
  const kinds = ["ban", "deny", "except", "tkl", "spamfilter"];
  let total = 0;
  for (const kind of kinds) {
    const blocks = vBlocks(kind);
    total += blocks.length;
    if (!blocks.length) continue;
    const s = vSection(host, `${blocks.length} ${kind} block${blocks.length === 1 ? "" : "s"}`);
    const t = vTable(["kind", "what it matches", "then what", "where"]);
    for (const { f, e } of blocks) {
      const m = vKid(e, "mask") || vKid(e, "match");
      const what = !m ? "—"
        : m.has_block
          ? (m.children || []).map((c) => `${c.name} ${c.has_block ? "{…}" : (c.bare ? "" : c.value)}`).join(", ")
          : (m.bare ? m.name : m.value);
      const then = vVal(e, "action") || vVal(e, "reason") || vVal(e, "type") || "";
      t.addRow([e.label ? `${kind} ${e.label}` : kind, what, then, vWhere(f, e)]);
    }
    s.appendChild(t);
  }
  if (!total) return vEmpty(host, "No ban, deny, except, tkl or spamfilter blocks here.");

  // An exception that no rule could ever apply to is worth knowing about.
  const bans = [...vBlocks("ban"), ...vBlocks("deny")];
  const excepts = vBlocks("except");
  const idle = [];
  for (const { f, e } of excepts) {
    const type = vVal(e, "type");
    if (!type) continue;
    const covers = bans.some(({ e: b }) => (b.label || "").includes(type) || vVal(b, "type") === type);
    if (!covers) idle.push({ f, e, type });
  }
  if (idle.length) {
    const s = vSection(host, "Exceptions with nothing to except from",
      "Each of these exempts people from a kind of ban you have not configured. Harmless, but it is not doing anything.");
    for (const i of idle) {
      const row = el("div", "vinc");
      row.append(vChip(i.type, "dim"), el("span", null, i.e.label || "except"), vWhere(i.f, i.e));
      s.appendChild(row);
    }
  }
}

/* ---------------- security groups ---------------- */

function viewGroups(host) {
  const groups = vBlocks("security-group");
  const built = ["known-users", "unknown-users", "webirc-users", "websocket-users", "tls-and-known-users"];
  if (!groups.length) {
    vEmpty(host, "No security-group blocks of your own. UnrealIRCd still has its built-in ones: " + built.join(", "));
  }

  // Where each group is used, from anywhere in the workspace.
  const uses = new Map();
  const note = (name, f, e, how) => {
    if (!name) return;
    if (!uses.has(name)) uses.set(name, []);
    uses.get(name).push({ f, e, how });
  };
  const walk = (f, e, trail) => {
    if (e.name === "security-group" || e.name === "exclude-security-group") {
      note(e.bare ? e.name : e.value, f, e, trail.join(" › ") || "somewhere");
    }
    for (const c of (e.children || [])) walk(f, c, [...trail, e.label ? `${e.name} ${e.label}` : e.name]);
  };
  for (const f of vFiles()) for (const e of f.entries) walk(f, e, []);

  for (const { f, e } of groups) {
    const s = vSection(host, e.label || "(unnamed)");
    const t = vTable(["criterion", "value"]);
    for (const c of (e.children || [])) {
      t.addRow([c.name, c.has_block
        ? (c.children || []).map((x) => x.bare ? x.name : `${x.name} ${x.value}`).join(", ")
        : (c.bare ? "yes" : (c.value ?? ""))]);
    }
    s.appendChild(t);
    s.appendChild(vWhere(f, e));
    const u = uses.get(e.label) || [];
    s.appendChild(el("div", u.length ? "vsub" : "vwarn",
      u.length ? `Used in ${u.length} place${u.length === 1 ? "" : "s"}` : "Defined, but nothing uses it."));
    for (const x of u) {
      const row = el("div", "vinc");
      row.append(el("span", "vsub", x.how), vWhere(x.f, x.e));
      s.appendChild(row);
    }
  }

  const usedBuiltins = built.filter((b) => uses.has(b));
  if (usedBuiltins.length) {
    const s = vSection(host, "Built-in groups you use",
      "UnrealIRCd defines these itself, so there is nothing to see in your files.");
    for (const b of usedBuiltins) {
      const s2 = el("div", "vinc");
      s2.append(vChip(b, "dim"), el("span", "vsub", `${uses.get(b).length} uses`));
      s.appendChild(s2);
      for (const x of uses.get(b)) {
        const row = el("div", "vinc vdim");
        row.append(el("span", "vsub", x.how), vWhere(x.f, x.e));
        s.appendChild(row);
      }
    }
  }
}

/* ---------------- roster ---------------- */

/* Your staff, arranged by who is in charge of whom.
 *
 * The rank comes from the operclasses: if one oper's class inherits from
 * another's, the first is in charge of the second, because inheriting means
 * having everything that class has and more. People on the same class are
 * peers and sit side by side. Where two classes are unrelated in the config —
 * which is most of them, since locop, globop, admin and netadmin are each
 * written out in full rather than derived from one another — there is no line
 * to draw, and drawing one would be a guess.
 */
function operFamilyTree(host) {
  const map = operclassMap();
  const opers = vBlocks("oper");
  if (!opers.length) return vEmpty(host, "No oper blocks in this workspace.");

  const byClass = new Map();
  const homeless = [];
  for (const { f, e } of opers) {
    const cls = vVal(e, "operclass");
    if (!cls || !map.has(cls)) { homeless.push({ f, e, cls }); continue; }
    if (!byClass.has(cls)) byClass.set(cls, []);
    byClass.get(cls).push({ f, e });
  }

  const rank = classRanks(map);

  // A rank is only worth a row if somebody is on it. Where an unused class
  // sits between two used ones, the line skips it and says so, rather than
  // leaving a gap nobody is standing in.
  const groupFor = new Map();
  const mk = (cls) => {
    if (groupFor.has(cls)) return groupFor.get(cls);
    const people = (byClass.get(cls) || []).sort((a, b) =>
      (a.e.label || "").localeCompare(b.e.label || ""));
    const g = {
      kind: "rank", id: "rank:" + cls, cls, people, skipped: [],
      w: 0, h: NODE.operH + 18, kids: [], label: cls,
    };
    groupFor.set(cls, g);
    return g;
  };

  // Walk down from each root, carrying any unused class along as a note.
  const seen = new Set();
  const build = (cls, skipped) => {
    const used = byClass.has(cls);
    const below = (rank.under.get(cls) || []).sort();
    if (!used) {
      // Nobody is on this class: hand its subordinates to whoever is above.
      return below.flatMap((c) => build(c, [...skipped, cls]));
    }
    if (seen.has(cls)) return [];
    seen.add(cls);
    const g = mk(cls);
    g.skipped = skipped;
    g.kids = below.flatMap((c) => build(c, []));
    const wide = g.people.reduce((a, p) => a + NODE.operW + 8, -8);
    g.w = Math.max(NODE.operW, wide);
    return [g];
  };
  const roots = rank.roots.sort().flatMap((c) => build(c, []));

  if (homeless.length) {
    const g = {
      kind: "rank", id: "rank:none", cls: null, skipped: [],
      people: homeless, w: Math.max(NODE.operW, homeless.length * (NODE.operW + 8) - 8),
      h: NODE.operH + 18, kids: [], label: "no operclass",
    };
    roots.push(g);
  }
  if (!roots.length) return vEmpty(host, "None of your opers name an operclass Confetti can see.");

  const wrap = el("div", "chartwrap");
  host.appendChild(wrap);
  const detail = el("div", "vdetail");
  let chosen = roots[0].id;

  const redraw = () => {
    wrap.textContent = "";
    chart(wrap, roots, (n) => { chosen = n.id; redraw(); }, chosen, drawRank);
    wrap.appendChild(el("div", "tiny",
      "Higher up means more authority. A dotted line means the classes in between " +
      "have nobody on them. Two people with no line between them are not ranked " +
      "against each other by the config at all."));
    wrap.appendChild(detail);
    drawDetail();
  };

  /* A rank is drawn as its people, side by side, under one bracket. */
  const rootIds = new Set(roots.map((r) => r.id));
  const drawRank = (g, group) => {
    const y = 18;
    const up = rootIds.has(g.id) ? "" : ` M${g.w / 2} 9 L${g.w / 2} 0`;
    if (g.people.length > 1) {
      group.appendChild(svgEl("path", {
        class: "bracket",
        d: `M${NODE.operW / 2} ${y} L${NODE.operW / 2} 9 L${g.w - NODE.operW / 2} 9` +
           ` L${g.w - NODE.operW / 2} ${y}${up}`,
      }));
    } else if (up) {
      group.appendChild(svgEl("path", { class: "bracket", d: `M${g.w / 2} ${y} L${g.w / 2} 0` }));
    }
    g.people.forEach((p, i) => {
      const x = i * (NODE.operW + 8);
      const b = svgEl("g", { class: "person", transform: `translate(${x} ${y})` });
      b.appendChild(svgEl("rect", { width: NODE.operW, height: NODE.operH, rx: 7 }));
      const t = svgEl("text", { x: NODE.operW / 2, y: 14, class: "nlabel" });
      t.textContent = p.e.label || "(unnamed)";
      b.appendChild(t);
      const mask = vVal(p.e, "mask") || vVal(p.e, "match");
      const sub = svgEl("text", { x: NODE.operW / 2, y: 26, class: "nsub" });
      sub.textContent = !mask ? "from anywhere"
        : /^\*[@!]?\*?$/.test(mask) ? "from anywhere" : mask;
      b.appendChild(sub);
      const title = svgEl("title", {});
      title.textContent = `${p.e.label || "oper"} — ${p.f.rel}:${p.e.line}`;
      b.appendChild(title);
      b.addEventListener("click", (ev) => {
        ev.stopPropagation();
        const modal = wrap.closest(".modal");
        if (modal) modal.remove();
        state.fileShown = true;
        applyFileLayout();
        jumpTo(p.f.rel, p.e.span[0]);
        select(p.f.path, [p.e]);
        render();
      });
      group.appendChild(b);
    });
    const label = svgEl("text", { x: g.w / 2, y: y + NODE.operH + 12, class: "rankname" });
    label.textContent = g.cls || "no operclass";
    group.appendChild(label);
  };

  const drawDetail = () => {
    detail.textContent = "";
    const g = [...groupFor.values()].concat(roots).find((x) => x.id === chosen);
    if (!g) return;
    detail.appendChild(el("div", "vdh", g.cls || "Opers with no operclass"));
    if (!g.cls) {
      detail.appendChild(el("div", "vwarn",
        "An oper block with no operclass can /OPER but may do nothing at all."));
    }
    if (g.skipped.length) {
      detail.appendChild(el("div", "vnote",
        `Nobody is on ${g.skipped.join(" or ")}, so the line above skips ${g.skipped.length === 1 ? "it" : "them"}.`));
    }
    const t = vTable(["oper", "may be used from", "class", "where"]);
    for (const p of g.people) {
      t.addRow([p.e.label || "(unnamed)",
                vVal(p.e, "mask") || vVal(p.e, "match") || "anywhere",
                vVal(p.e, "class") || "—",
                vWhere(p.f, p.e)]);
    }
    detail.appendChild(t);
    if (g.cls) {
      const eff = effectivePerms(map, g.cls);
      detail.appendChild(el("div", "vsech", `${eff.got.size} permissions each`));
      detail.appendChild(permTree(eff.got, g.cls));
    }
  };
  redraw();
}

function viewRoster(host) {
  const kinds = new Map();
  for (const f of vFiles()) {
    for (const e of f.entries) kinds.set(e.name, (kinds.get(e.name) || 0) + 1);
  }
  const names = [...kinds.keys()].sort((a, b) => kinds.get(b) - kinds.get(a) || a.localeCompare(b));
  if (!names.length) return vEmpty(host, "Nothing open.");

  const bar = el("div", "vbar");
  const sel = el("select", "vsel");
  for (const n of names) {
    const o = document.createElement("option");
    o.value = n; o.textContent = `${n}  (${kinds.get(n)})`;
    sel.appendChild(o);
  }
  sel.value = names.includes("oper") ? "oper" : names[0];
  bar.appendChild(sel);

  // Only opers have a chain of command to draw, so the choice only appears
  // when you are looking at them.
  let mode = "tree";
  const seg = el("div", "seg");
  const mkBtn = (id, text) => {
    const b = el("button", "segb" + (mode === id ? " on" : ""), text);
    b.addEventListener("click", () => { mode = id; draw(); });
    return b;
  };
  bar.append(el("span", "spacer"), seg);
  host.appendChild(bar);

  const out = el("div");
  host.appendChild(out);

  const drawTable = () => {
    const blocks = vBlocks(sel.value);
    const seen = new Map();
    for (const { e } of blocks) {
      for (const c of (e.children || [])) seen.set(c.name, (seen.get(c.name) || 0) + 1);
    }
    const cols = [...seen.keys()].sort((a, b) => seen.get(b) - seen.get(a)).slice(0, 8);
    const t = vTable(["name", ...cols, "where"]);
    for (const { f, e } of blocks) {
      const cells = cols.map((c) => {
        const k = vKid(e, c);
        if (!k) return "—";
        if (k.has_block) return `{ ${k.children.length} }`;
        return k.bare ? "on" : (k.value ?? "on");
      });
      t.addRow([e.label || "(unnamed)", ...cells, vWhere(f, e)]);
    }
    out.appendChild(t);
    if (!blocks.length) vEmpty(out, "None of those here.");
  };

  const draw = () => {
    out.textContent = "";
    seg.textContent = "";
    const isOper = sel.value === "oper";
    if (isOper) {
      seg.append(mkBtn("tree", "Family tree"), mkBtn("table", "Table"));
    } else {
      mode = "table";
    }
    if (isOper && mode === "tree") operFamilyTree(out);
    else drawTable();
  };
  sel.addEventListener("change", () => { mode = sel.value === "oper" ? "tree" : "table"; draw(); });
  draw();
}

/* ---------------- changed from stock ---------------- */

async function viewStock(host) {
  const files = state.ws ? state.ws.files : [];
  // The shipped originals live under examples/ in every UnrealIRCd install.
  const stock = new Map();
  for (const f of files) {
    if (/(^|\/)examples\//.test(f.rel)) stock.set(f.name.replace(/^example\./, ""), f);
  }
  const pairs = [];
  const ex = files.find((f) => /(^|\/)examples\/example\.conf$/.test(f.rel));
  const main = files.find((f) => f.name === "unrealircd.conf");
  if (ex && main) pairs.push([main, ex]);
  for (const f of files) {
    if (/(^|\/)examples\//.test(f.rel)) continue;
    const s = stock.get(f.name);
    if (s && s !== f) pairs.push([f, s]);
  }

  if (!pairs.length) {
    return vEmpty(host, "Confetti can't find the shipped examples/ folder next to your config, so it has nothing to compare against.");
  }

  const s = vSection(host, "Your files against the ones UnrealIRCd ships",
    "Added lines are yours. Removed lines are stock lines you took out.");
  for (const [mine, theirs] of pairs) {
    const box = el("div", "vsec");
    const h = el("div", "vsech");
    h.append(el("span", null, mine.rel), el("span", "vsep", "vs"), el("span", "vsub", theirs.rel));
    box.appendChild(h);
    try {
      const d = await invoke("diff_files", { a: theirs.path, b: mine.path });
      const added = d.lines.filter((l) => l.change === "added").length;
      const removed = d.lines.filter((l) => l.change === "removed").length;
      box.appendChild(el("div", "vsub", `${added} lines added, ${removed} removed`));
      const dd = el("div", "diff");
      for (const l of d.lines) {
        if (l.change === "same") continue;
        const row = el("div", "dl " + l.change);
        row.append(el("span", "dn", l.old_line ? String(l.old_line) : ""),
                   el("span", "dm", l.change === "added" ? "+" : "−"),
                   el("span", "dt", l.text));
        dd.appendChild(row);
      }
      box.appendChild(dd);
    } catch (e) {
      box.appendChild(el("div", "vwarn", String(e)));
    }
    s.appendChild(box);
  }
}

/* ---------------- config vs the running server ---------------- */

async function viewRunning(host) {
  if (!(state.ws && state.ws.remote)) {
    return vEmpty(host,
      "This one asks the server, so it needs one. Open a config over ssh first — File › Connect to a server.");
  }
  const s = vSection(host, "Asking the server…");
  let out;
  try {
    out = await invoke("server_command", { verb: "module-status" });
  } catch (e) {
    s.textContent = "";
    return vEmpty(s, String(e));
  }
  s.textContent = "";

  // The module list the server prints, against the loadmodule lines you wrote.
  const running = new Set();
  for (const line of (out.text || "").split("\n")) {
    const m = /([a-z0-9_\-/.]+)\.so|\b(?:Module\s+)?([a-z0-9_\-]+\/[a-z0-9_\-]+)\b/i.exec(line);
    if (m) running.add((m[1] || m[2]).replace(/\.so$/, ""));
  }
  const declared = new Map();
  for (const f of vFiles()) {
    for (const e of f.entries) {
      if (e.name !== "loadmodule") continue;
      const v = (e.value || e.label || "").replace(/^third\//, "third/").replace(/\.so$/, "");
      declared.set(v, { f, e });
    }
  }

  const box = vSection(host, "Modules",
    `${running.size} loaded on the server, ${declared.size} loadmodule lines in your config.`);
  const t = vTable(["module", "in your config", "loaded now"]);
  const all = [...new Set([...running, ...declared.keys()])].sort();
  for (const m of all) {
    const inConf = declared.has(m);
    const isUp = running.has(m);
    t.addRow([inConf ? vWhere(declared.get(m).f, declared.get(m).e, m) : m,
              inConf ? "yes" : "no", isUp ? "yes" : "no"],
             inConf === isUp ? "" : "vbad");
  }
  box.appendChild(t);
  if (!running.size) {
    box.appendChild(el("div", "tiny",
      "Confetti couldn't read a module list out of the server's reply. Raw output below."));
    const pre = el("div", "diff");
    pre.appendChild(el("div", "dl", (out.text || "").slice(0, 4000)));
    box.appendChild(pre);
  }

  const st = vSection(host, "What the server says about itself");
  try {
    const status = await invoke("server_command", { verb: "status" });
    const pre = el("div", "diff");
    for (const line of (status.text || "").split("\n")) {
      pre.appendChild(el("div", "dl", line));
    }
    st.appendChild(pre);
  } catch (e) {
    st.appendChild(el("div", "vwarn", String(e)));
  }
}

/* ---------------- log, by block ---------------- */

/* Does this log line name that block? A whole word, and long enough to mean
   something — an alias called `os` otherwise matches the word "host". */
function vNames(msg, key) {
  if (!msg || !key || key.length < 3) return false;
  const rx = new RegExp("(^|[^A-Za-z0-9_.-])" +
    key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") +
    "([^A-Za-z0-9_-]|$)");
  return rx.test(msg);
}

async function viewLogBlocks(host) {
  let files = [];
  try { files = await invoke("log_files"); } catch (e) { /* none */ }
  const usable = files.filter((f) => f.exists);
  if (!usable.length) {
    return vEmpty(host, "No log file Confetti can read. The log block in your config says where it goes.");
  }

  const bar = el("div", "vbar");
  const sel = el("select", "vsel");
  for (const f of usable) {
    const o = document.createElement("option");
    o.value = f.path; o.textContent = `${f.name}  (${Math.round(f.bytes / 1024)} kB)`;
    sel.appendChild(o);
  }
  bar.appendChild(sel);
  host.appendChild(bar);
  const out = el("div");
  host.appendChild(out);

  // Everything in the workspace that a log line might be about.
  const named = [];
  for (const f of vFiles()) {
    for (const e of f.entries) {
      if (e.label) named.push({ f, e, key: e.label, what: `${e.name} ${e.label}` });
      if (e.name === "listen") {
        for (const p of vKids(e, "port")) named.push({ f, e, key: String(p.value ?? p.name), what: `listen port ${p.value ?? p.name}` });
      }
    }
  }

  const draw = async () => {
    out.textContent = "";
    out.appendChild(el("div", "tiny", "reading…"));
    let read;
    try { read = await invoke("read_log", { path: sel.value, limit: 4000 }); }
    catch (e) { out.textContent = ""; return vEmpty(out, String(e)); }
    out.textContent = "";

    const groups = new Map();
    const loose = [];
    for (const en of read.entries) {
      // The server names the file and line when it complains about config.
      const cite = /([A-Za-z0-9_.\-/\\]+\.conf):(\d+)/.exec(en.msg || "");
      let key = null, label = null, jump = null;
      if (cite) {
        const f = (state.ws ? state.ws.files : []).find((x) => x.rel.endsWith(cite[1]) || x.name === cite[1]);
        key = `${cite[1]}:${cite[2]}`;
        label = key;
        if (f) jump = { f, e: { span: [0], line: +cite[2] } };
      } else {
        const hit = named.find((n) => vNames(en.msg, n.key));
        if (hit) { key = hit.what; label = hit.what; jump = hit; }
      }
      if (!key) { loose.push(en); continue; }
      if (!groups.has(key)) groups.set(key, { label, jump, entries: [] });
      groups.get(key).entries.push(en);
    }

    const ordered = [...groups.values()].sort((a, b) => b.entries.length - a.entries.length);
    out.appendChild(el("div", "vsech", groups.size === 1
      ? `${read.entries.length} entries — one of them is about something in your config`
      : `${read.entries.length} entries — ${groups.size} of them are about something in your config`));
    for (const g of ordered) {
      const s = el("div", "vsec");
      const h = el("div", "vsech");
      h.append(el("span", null, g.label), vChip(String(g.entries.length), "n"));
      if (g.jump && g.jump.f) {
        const b = el("button", "vwhere", "show it");
        b.addEventListener("click", () => {
          const modal = b.closest(".modal");
          if (modal) modal.remove();
          state.fileShown = true;
          applyFileLayout();
          jumpTo(g.jump.f.rel, g.jump.e.span ? g.jump.e.span[0] : 0);
          render();
        });
        h.appendChild(b);
      }
      s.appendChild(h);
      for (const en of g.entries.slice(0, 8)) {
        const row = el("div", "vlogrow " + (en.level || ""));
        row.append(vChip(en.level || "info", en.level === "error" ? "bad" : en.level === "warn" ? "warn" : "dim"),
                   el("span", "vsub", en.timestamp || ""),
                   el("span", null, en.msg || ""));
        s.appendChild(row);
      }
      if (g.entries.length > 8) s.appendChild(el("div", "tiny", `${g.entries.length - 8} more like it`));
      out.appendChild(s);
    }
    if (!groups.size) {
      vEmpty(out, "Nothing in this log names a block in your config — which is the good outcome.");
    }
    // Whatever is left, filed by the part of the server that said it. A log
    // with nothing to pin on a block is still worth reading.
    if (loose.length) {
      const bySub = new Map();
      for (const en of loose) {
        const k = en.subsystem || "other";
        if (!bySub.has(k)) bySub.set(k, []);
        bySub.get(k).push(en);
      }
      const s = el("div", "vsec");
      s.appendChild(el("div", "vsech", "everything else, by what said it"));
      for (const [sub_, list] of [...bySub.entries()].sort((a, b) => b[1].length - a[1].length)) {
        const g = el("div", "vsec");
        const h = el("div", "vsech");
        h.append(el("span", null, sub_), vChip(String(list.length), "n"));
        g.appendChild(h);
        for (const en of list.slice(0, 5)) {
          const row = el("div", "vlogrow " + (en.level || ""));
          row.append(vChip(en.level || "info",
                           en.level === "error" ? "bad" : en.level === "warn" ? "warn" : "dim"),
                     el("span", "vsub", en.timestamp || ""),
                     el("span", null, en.msg || ""));
          g.appendChild(row);
        }
        if (list.length > 5) g.appendChild(el("div", "tiny", `${list.length - 5} more`));
        s.appendChild(g);
      }
      out.appendChild(s);
    }

  };
  sel.addEventListener("change", draw);
  draw();
}

/* Ctrl+Shift+V opens whichever viewer you had open last. */
document.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey) || !e.shiftKey) return;
  if (e.key.toLowerCase() !== "v") return;
  if (!state.ws) return;
  e.preventDefault();
  if (document.querySelector(".sheet.viewer")) return;
  openViewer(viewerOpen);
});
