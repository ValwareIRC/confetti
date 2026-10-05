/* Confetti — connecting to a server over ssh.
 *
 * Authentication is left entirely to your own ssh: your agent, your keys, your
 * ~/.ssh/config aliases. Confetti never asks for a password and never stores
 * one. If `ssh yourbox` works in a terminal, putting `yourbox` in here works.
 */

async function openServers() {
  closeMenu();
  const back = el("div", "modal");
  const box = el("div", "sheet");
  box.appendChild(el("div", "sheeth", "Servers"));
  box.appendChild(el("div", "tiny",
    "Confetti uses your own ssh — your agent, your keys, your ~/.ssh/config. " +
    "If you can already ssh to a machine, name it here and it will just work. " +
    "No password is asked for and none is stored."));

  const list = el("div", "hostlist");
  box.appendChild(list);

  async function redraw() {
    list.textContent = "";
    let saved = [];
    try { saved = await invoke("hosts"); } catch (e) { /* none yet */ }
    if (!saved.length) {
      list.appendChild(el("div", "tiny", "No servers yet. Add one below."));
    }
    for (const h of saved) {
      const row = el("div", "hostrow");
      const nm = el("span", "nm");
      nm.appendChild(el("span", "hname", h.name));
      nm.appendChild(el("span", "htarget",
        `${h.target}${h.port ? ":" + h.port : ""}  ${h.root}`));
      row.appendChild(nm);

      const test = el("button", "btn", "Test");
      test.addEventListener("click", async () => {
        test.textContent = "…";
        try {
          const what = await invoke("probe_host", { host: h });
          test.textContent = "reached";
          test.title = what;
        } catch (e) {
          test.textContent = "failed";
          test.classList.add("bad");
          oops(e);
        }
      });
      row.appendChild(test);

      const go = el("button", "btn go", "Open");
      go.addEventListener("click", async () => {
        go.textContent = "opening…";
        try {
          const ws = await invoke("open_remote", { host: h });
          back.remove();
          await loadWorkspace(ws);
        } catch (e) {
          go.textContent = "Open";
          oops(e);
        }
      });
      row.appendChild(go);

      const del = el("button", "del", "×");
      del.title = "Forget this server";
      del.addEventListener("click", async () => {
        const ok = await confirmAsk({
          title: `Forget ${h.name}?`,
          body: "Confetti stops offering it here.",
          note: "Nothing on the server itself is touched.",
          okLabel: "Forget it",
          danger: true,
        });
        if (!ok) return;
        await invoke("forget_host", { name: h.name });
        redraw();
      });
      row.appendChild(del);
      list.appendChild(row);
    }
  }
  await redraw();

  box.appendChild(el("div", "divider"));
  box.appendChild(el("span", "lab", "add a server"));

  const form = el("div", "hostform");
  const field = (ph, val) => {
    const i = el("input", "cdesc");
    i.placeholder = ph;
    if (val) i.value = val;
    return i;
  };
  const fName = field("a name for it — “live”, “test box”");
  const fTarget = field("irc@irc.example.org  (or an ssh_config alias)");
  const fPort = field("port, if not 22");
  const fRoot = field("/home/irc/unrealircd/conf", "/home/irc/unrealircd/conf");
  const fKey = field("key file, if your agent doesn't have it");
  form.append(fName, fTarget, fPort, fRoot, fKey);
  box.appendChild(form);

  const mk = () => ({
    name: fName.value.trim(),
    target: fTarget.value.trim(),
    port: fPort.value.trim() ? parseInt(fPort.value.trim(), 10) : null,
    root: fRoot.value.trim(),
    key: fKey.value.trim() || null,
  });

  const status = el("div", "tiny");
  box.appendChild(status);

  const row = el("div", "irow");
  const test = el("button", "btn", "Test it");
  test.addEventListener("click", async () => {
    const h = mk();
    if (!h.target) { status.textContent = "Give it something to connect to first."; return; }
    status.textContent = "connecting…";
    try {
      const what = await invoke("probe_host", { host: h });
      status.textContent = `Reached it — ${what}`;
    } catch (e) {
      status.textContent = String(e);
    }
  });
  const save = el("button", "btn", "Save it");
  save.addEventListener("click", async () => {
    try {
      await invoke("save_host", { host: mk() });
      fName.value = ""; fTarget.value = ""; fPort.value = ""; fKey.value = "";
      status.textContent = "Saved.";
      redraw();
    } catch (e) { status.textContent = String(e); }
  });
  const open = el("button", "btn go", "Save and open");
  open.addEventListener("click", async () => {
    const h = mk();
    status.textContent = "connecting…";
    try {
      await invoke("save_host", { host: h });
      const ws = await invoke("open_remote", { host: h });
      back.remove();
      await loadWorkspace(ws);
    } catch (e) { status.textContent = String(e); }
  });
  row.append(test, save, el("span", "spacer"), open);
  box.appendChild(row);

  const done = el("button", "btn wide", "Close");
  done.addEventListener("click", () => back.remove());
  box.appendChild(done);

  back.appendChild(box);
  back.addEventListener("click", (e) => { if (e.target === back) back.remove(); });
  document.body.appendChild(back);
}

/* Run one of UnrealIRCd's own commands on the server it came from. This is the
   only real answer about whether a config is valid — everything Confetti
   computes is a model of conf.c, and this is conf.c. */
async function serverCommand(verb) {
  const back = el("div", "modal");
  const box = el("div", "sheet wide");
  box.appendChild(el("div", "sheeth", `${verb} on the server`));
  const out = el("pre", "serverout", "running…");
  box.appendChild(out);
  const close = el("button", "btn wide", "Close");
  close.addEventListener("click", () => back.remove());
  box.appendChild(close);
  back.appendChild(box);
  document.body.appendChild(back);

  try {
    const r = await invoke("server_command", { verb });
    out.textContent = (r.stdout + (r.stderr ? "\n" + r.stderr : "")).trim()
      || `(no output, exit ${r.code})`;
    out.className = "serverout " + (r.code === 0 ? "ok" : "bad");
  } catch (e) {
    out.textContent = String(e);
    out.className = "serverout bad";
  }
}

$("btn-servers").addEventListener("click", openServers);
$("btn-configtest").addEventListener("click", () => serverCommand("configtest"));
$("btn-rehash").addEventListener("click", async () => {
  const ok = await confirmAsk({
    title: "Rehash the running server?",
    body: "It rereads its configuration from disk and applies it live.",
    note: "Only what you have saved counts — unsaved edits stay here until you save them.",
    okLabel: "Rehash it",
  });
  if (ok) serverCommand("rehash");
});
