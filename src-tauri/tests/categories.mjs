/* Bringing a saved set of categories forward, tested against the real code in
 * main.js rather than a copy of it.
 *
 * Run: node src-tauri/tests/categories.mjs
 */
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const src = readFileSync(new URL("../../src/main.js", import.meta.url), "utf8");

/* Lift the three pieces out of main.js and run them for real. */
function lift(from, to) {
  const a = src.indexOf(from);
  assert.notEqual(a, -1, `main.js no longer contains ${JSON.stringify(from)}`);
  const b = src.indexOf(to, a);
  assert.notEqual(b, -1, `no ${JSON.stringify(to)} after it`);
  return src.slice(a, b + to.length);
}
const code = [
  lift("const DEFAULT_CATEGORIES = [", "\n];"),
  lift("const RETIRED_CATEGORIES =", ";"),
  lift("const ORIGINAL_CATEGORIES =", ";"),
  lift("function reconcileCats(cfg) {", "\n  return cfg;\n}"),
  "export { DEFAULT_CATEGORIES, RETIRED_CATEGORIES, ORIGINAL_CATEGORIES, reconcileCats };",
].join("\n");
const mod = await import("data:text/javascript," + encodeURIComponent(code));
const { DEFAULT_CATEGORIES, reconcileCats } = mod;

const ids = (cfg) => cfg.categories.map((c) => c.id);
const catOf = (cfg, block) =>
  (cfg.categories.find((c) => c.blocks.includes(block)) || { id: "extras" }).id;

/* --- the defaults themselves --- */
{
  const fresh = { categories: JSON.parse(JSON.stringify(DEFAULT_CATEGORIES)), assign: {}, folded: {} };
  assert.equal(catOf(fresh, "set"), "settings", "set has a shelf of its own");
  assert.equal(catOf(fresh, "loadmodule"), "modules", "loadmodule has a shelf of its own");
  assert.equal(catOf(fresh, "blacklist-module"), "modules");
  assert.equal(ids(fresh).at(-1), "extras", "the catch-all stays last, or everything lands in it");
  assert.ok(!ids(fresh).includes("rules"), "House rules is gone");
  // Nothing may be in two places, or catFor picks whichever came first.
  const seen = new Set();
  for (const c of DEFAULT_CATEGORIES) {
    for (const b of c.blocks) {
      assert.ok(!seen.has(b), `${b} is filed under two categories`);
      seen.add(b);
    }
  }
}

/* --- a config saved before any of this existed --- */
{
  const old = {
    categories: [
      { id: "identity", name: "Identity", colour: "#F0A85C", icon: "x", desc: "d", blocks: ["me", "admin"] },
      { id: "rules", name: "House rules", colour: "#DFC055", icon: "y", desc: "the set block", blocks: ["set"] },
      { id: "extras", name: "Everything else", colour: "#A3ACC2", icon: "z", desc: "d", blocks: [] },
    ],
    assign: { "conf/x.conf#12": "rules", "conf/y.conf#40": "identity" },
    folded: { rules: true },
  };
  const cfg = reconcileCats(JSON.parse(JSON.stringify(old)));

  assert.ok(!ids(cfg).includes("rules"), "House rules is retired");
  assert.equal(catOf(cfg, "set"), "settings", "set moves to its own shelf");
  assert.equal(catOf(cfg, "loadmodule"), "modules", "modules shelf is added");
  assert.equal(ids(cfg).at(-1), "extras", "the catch-all is still last");
  assert.equal(cfg.assign["conf/x.conf#12"], "settings",
    "a block dragged onto House rules follows it, rather than falling into Everything else");
  assert.equal(cfg.assign["conf/y.conf#40"], "identity", "other assignments are untouched");
  assert.equal(cfg.folded.rules, undefined, "the fold state of a gone shelf goes with it");
  // Console remembers folding, and absent means open there. A shelf you have
  // never seen must not be open the first time you start the program.
  for (const id of ["settings", "modules", "words", "aliases", "logging", "keys", "rpc", "reads"]) {
    assert.equal(cfg.folded[id], true, `${id} is new here, so it starts shut`);
  }
  assert.equal(cfg.folded.identity, undefined, "a shelf you had open stays open");
  // Every default is present exactly once.
  for (const d of DEFAULT_CATEGORIES) {
    assert.equal(ids(cfg).filter((i) => i === d.id).length, 1, `${d.id} appears once`);
  }
}

/* --- what you customised is yours --- */
{
  const mine = {
    categories: [
      { id: "extras", name: "Odds and ends", colour: "#111111", icon: "!", desc: "mine", blocks: [] },
      { id: "identity", name: "WHO WE ARE", colour: "#abcdef", icon: "@", desc: "my words", blocks: ["me"] },
      { id: "mine", name: "My own shelf", colour: "#222222", icon: "#", desc: "custom", blocks: [] },
    ],
    assign: {}, folded: {},
  };
  const cfg = reconcileCats(JSON.parse(JSON.stringify(mine)));
  const id = cfg.categories.find((c) => c.id === "identity");
  assert.equal(id.name, "WHO WE ARE", "your name is kept");
  assert.equal(id.colour, "#abcdef", "your colour is kept");
  assert.equal(id.icon, "@", "your icon is kept");
  assert.equal(id.desc, "my words", "your description is kept");
  assert.deepEqual(id.blocks, ["me", "admin"],
    "which block types land there is refreshed — it was never yours to set");
  assert.ok(ids(cfg).includes("mine"), "a category you invented survives");
  assert.equal(ids(cfg).at(-1), "extras",
    "the catch-all is moved back to last even if you had dragged it up");
  assert.equal(cfg.categories.find((c) => c.id === "extras").name, "Odds and ends",
    "and it keeps the name you gave it");
}

/* --- already in the file, but never seen --- */
{
  // What if the update that added the shelves also saved them? They are
  // present, so "is this new" cannot be answered by looking for it.
  const saved = {
    categories: JSON.parse(JSON.stringify(DEFAULT_CATEGORIES)),
    assign: {},
    folded: { order: true },
    // No knownCats note: written before Confetti kept one.
  };
  const cfg = reconcileCats(JSON.parse(JSON.stringify(saved)));
  for (const id of ["settings", "modules", "words", "aliases", "logging", "keys", "rpc", "reads"]) {
    assert.equal(cfg.folded[id], true,
      `${id} is in the file but has never been seen, so it starts shut`);
  }
  assert.equal(cfg.folded.identity, undefined, "one of the originals is left as it was");
  assert.equal(cfg.folded.order, true, "and one you had folded stays folded");
  assert.deepEqual(cfg.knownCats, cfg.categories.map((c) => c.id),
    "and now they have been seen, so this does not happen twice");

  // Next launch: what you opened stays open.
  const opened = JSON.parse(JSON.stringify(cfg));
  delete opened.folded.words;
  const again = reconcileCats(opened);
  assert.equal(again.folded.words, undefined, "a shelf you opened stays open next time");
}

/* --- running it twice changes nothing the second time --- */
{
  const once = reconcileCats({ categories: JSON.parse(JSON.stringify(DEFAULT_CATEGORIES)), assign: {}, folded: {} });
  const twice = reconcileCats(JSON.parse(JSON.stringify(once)));
  assert.deepEqual(twice, once, "reconciling is idempotent");
}

/* --- every block type this release accepts has a home --- */
{
  // The list is what the scrape found in UnrealIRCd 6.2.7's own source, so a
  // block type going missing from a category here is a real regression.
  const EVERY_BLOCK = [
    "admin", "alias", "allow", "ban", "blacklist", "blacklist-module", "class",
    "deny", "drpass", "except", "files", "help", "hideserver", "include", "link",
    "listen", "loadmodule", "log", "maxperip", "me", "official-channels", "oper",
    "operclass", "proxy", "require", "rpc", "rpc-class", "rpc-user", "secret",
    "security-group", "set", "sni", "spamfilter", "spamreport", "tld", "ulines",
    "vhost", "webirc",
  ];
  const home = new Map();
  for (const c of DEFAULT_CATEGORIES) for (const b of c.blocks) home.set(b, c.id);
  const orphans = EVERY_BLOCK.filter((b) => !home.has(b));
  assert.deepEqual(orphans, [],
    `these would land in Everything else: ${orphans.join(", ")}`);
  assert.deepEqual(DEFAULT_CATEGORIES.find((c) => c.id === "extras").blocks, [],
    "the catch-all is a catch-all, not a category with its own list");
  // The biggest things in a config each get a shelf rather than sharing one.
  assert.equal(home.get("help"), "words");
  assert.equal(home.get("loadmodule"), "modules");
  assert.equal(home.get("set"), "settings");
  assert.equal(home.get("log"), "logging");
  assert.equal(home.get("alias"), "aliases");
  assert.equal(home.get("include"), "reads");
}

console.log("categories: all checks passed");
