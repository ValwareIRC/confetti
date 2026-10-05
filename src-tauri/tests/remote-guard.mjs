/* Nothing that changes files may reach a local filesystem call while the open
 * config is on another machine.
 *
 * Asking for a new folder on a server used to run mkdir against
 * `/home/ircd/unrealircd/conf/...` on Windows. That resolves to
 * `C:\home\ircd\unrealircd\conf\...`, so the commands that create rather than
 * read would quietly have made directories on this machine instead of saying
 * they could not.
 *
 * This reads lib.rs and checks, for every one of those commands, that the
 * "is it remote" branch comes before the first std::fs call in the body — so
 * a new command cannot be added without one, and an existing one cannot have
 * its guard moved below the part that touches this disk.
 *
 * Run: node src-tauri/tests/remote-guard.mjs
 */
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const src = readFileSync(new URL("../src/lib.rs", import.meta.url), "utf8");
const lines = src.split("\n");

/* Every command that creates, moves, renames or removes a file or folder. */
const COMMANDS = [
  "new_folder", "new_file", "rename_file", "rename_folder",
  "delete_file", "delete_folder", "move_file", "duplicate_file",
];

/* The body of a top-level fn, from its signature to the closing brace in
 * column zero. */
function body(name) {
  const start = lines.findIndex((l) => l.startsWith(`fn ${name}(`) || l.startsWith(`async fn ${name}(`));
  assert.notEqual(start, -1, `lib.rs has no fn ${name}`);
  let end = start + 1;
  while (end < lines.length && lines[end] !== "}") end++;
  return lines.slice(start, end + 1);
}

const failures = [];
for (const name of COMMANDS) {
  const b = body(name);
  const guard = b.findIndex((l) => l.includes("source_of(&state)"));
  const touch = b.findIndex((l) => l.includes("std::fs::"));
  if (guard === -1) {
    failures.push(`${name}: never asks whether the config is on another machine`);
    continue;
  }
  if (touch !== -1 && guard > touch) {
    failures.push(
      `${name}: touches this machine's disk at line +${touch} before checking at +${guard}`);
  }
  // The remote branch has to actually leave the function, or it would fall
  // through into the local code and do the operation twice.
  const after = b.slice(guard);
  const ret = after.findIndex((l) => l.includes("return remote_reload("));
  assert.notEqual(ret, -1, `${name}: the remote branch does not return`);
}
assert.deepEqual(failures, [], "\n  " + failures.join("\n  "));

/* And the remote helpers must not be reachable with a path that escapes the
 * config folder — that check lives in remote.rs, so make sure it is called. */
const remote = readFileSync(new URL("../src/remote.rs", import.meta.url), "utf8");
for (const fn of ["mkdir", "move_path", "copy_path", "put_file", "delete_file", "retire_folder", "exists"]) {
  const i = remote.indexOf(`pub fn ${fn}(`);
  assert.notEqual(i, -1, `remote.rs has no ${fn}`);
  const end = remote.indexOf("\n}", i);
  const b = remote.slice(i, end);
  assert.ok(b.includes("at(h,"), `remote::${fn} builds a path without checking it`);
}

console.log(`remote guard: ${COMMANDS.length} commands checked, all guarded`);
