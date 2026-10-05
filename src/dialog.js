/* Confetti — dialogs.
 *
 * Nothing here uses the browser's alert/confirm/prompt: those render as
 * "tauri.localhost says", which is somebody else's chrome in the middle of
 * your application. These are in-app, keyboard-driven, and follow the house
 * rules — Escape cancels, Enter accepts, focus lands where you will type, and
 * nothing dismisses itself on a timer.
 */

function dialogShell(title, blurb) {
  const back = el("div", "modal");
  const box = el("div", "sheet dlg");
  box.appendChild(el("div", "sheeth", title));
  if (blurb) box.appendChild(el("div", "tiny", blurb));
  back.appendChild(box);
  back.addEventListener("mousedown", (e) => {
    if (e.target === back) back.dispatchEvent(new CustomEvent("dismiss"));
  });
  return { back, box };
}

function mountDialog(back, onKey) {
  document.body.appendChild(back);
  const esc = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      back.dispatchEvent(new CustomEvent("dismiss"));
    } else if (onKey) {
      onKey(e);
    }
  };
  document.addEventListener("keydown", esc, true);
  back.addEventListener("dismiss", () => {
    document.removeEventListener("keydown", esc, true);
  });
  return () => {
    document.removeEventListener("keydown", esc, true);
    back.remove();
  };
}

/* Ask for one or more values.
   fields: [{ name, label, placeholder, value, hint, required, choices }]
   Resolves to an object of values, or null if cancelled. */
function ask(opts) {
  return new Promise((resolve) => {
    const { back, box } = dialogShell(opts.title, opts.blurb);
    const inputs = {};

    for (const f of opts.fields) {
      const row = el("div", "dfield");
      row.appendChild(el("label", "dlabel", f.label));
      let input;
      if (f.choices) {
        input = el("select", "dinput");
        for (const c of f.choices) {
          const o = document.createElement("option");
          o.value = typeof c === "string" ? c : c.value;
          o.textContent = typeof c === "string" ? c : c.label;
          input.appendChild(o);
        }
        if (f.value) input.value = f.value;
      } else {
        input = el("input", "dinput");
        input.value = f.value || "";
        input.placeholder = f.placeholder || "";
        input.spellcheck = false;
      }
      row.appendChild(input);
      if (f.hint) row.appendChild(el("div", "dhint", f.hint));
      box.appendChild(row);
      inputs[f.name] = input;
    }

    const err = el("div", "derr");
    err.hidden = true;
    box.appendChild(err);

    const done = mountDialog(back, (e) => {
      if (e.key === "Enter" && e.target.tagName !== "TEXTAREA") {
        e.preventDefault();
        accept();
      }
    });
    const cancel = () => { done(); resolve(null); };
    back.addEventListener("dismiss", cancel);

    function accept() {
      const out = {};
      for (const f of opts.fields) {
        out[f.name] = inputs[f.name].value.trim();
        if (f.required && !out[f.name]) {
          err.hidden = false;
          err.textContent = `${f.label} is needed.`;
          inputs[f.name].focus();
          return;
        }
      }
      done();
      resolve(out);
    }

    const row = el("div", "irow");
    const no = el("button", "btn", opts.cancelLabel || "Cancel");
    no.addEventListener("click", cancel);
    const yes = el("button", "btn go", opts.okLabel || "OK");
    yes.addEventListener("click", accept);
    row.append(no, el("span", "spacer"), yes);
    box.appendChild(row);

    const first = inputs[opts.fields[0].name];
    if (first) { first.focus(); if (first.select) first.select(); }
  });
}

/* A yes/no question. Resolves true or false. */
function confirmAsk(opts) {
  return new Promise((resolve) => {
    const { back, box } = dialogShell(opts.title);
    box.appendChild(el("div", "dbody", opts.body));
    if (opts.note) box.appendChild(el("div", "tiny", opts.note));

    const done = mountDialog(back, (e) => {
      if (e.key === "Enter") { e.preventDefault(); done(); resolve(true); }
    });
    const no = () => { done(); resolve(false); };
    back.addEventListener("dismiss", no);

    const row = el("div", "irow");
    const cancel = el("button", "btn", opts.cancelLabel || "Cancel");
    cancel.addEventListener("click", no);
    const ok = el("button", "btn " + (opts.danger ? "danger" : "go"), opts.okLabel || "Yes");
    ok.addEventListener("click", () => { done(); resolve(true); });
    row.append(cancel, el("span", "spacer"), ok);
    box.appendChild(row);
    ok.focus();
  });
}

/* Tell them something. Persists until dismissed — never a timed toast. */
function say(opts) {
  return new Promise((resolve) => {
    const { back, box } = dialogShell(opts.title || "Confetti");
    const body = el("div", "dbody" + (opts.kind === "error" ? " bad" : ""));
    body.textContent = opts.body;
    box.appendChild(body);
    if (opts.detail) {
      const d = el("pre", "ddetail");
      d.textContent = opts.detail;
      box.appendChild(d);
    }
    const done = mountDialog(back, (e) => {
      if (e.key === "Enter") { e.preventDefault(); done(); resolve(); }
    });
    const close = () => { done(); resolve(); };
    back.addEventListener("dismiss", close);
    const ok = el("button", "btn go wide", opts.okLabel || "Close");
    ok.addEventListener("click", close);
    box.appendChild(ok);
    ok.focus();
  });
}

/* Every "that didn't work" path goes through here, so failures read the same
   way wherever they come from. */
function oops(e, what) {
  const text = (e && e.message) ? e.message : String(e);
  return say({
    title: what || "That didn't work",
    body: text,
    kind: "error",
  });
}
