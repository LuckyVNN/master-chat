"use strict";

// Tauri's freezePrototype option would inject this into every webview and break
// Google's and Facebook's pages, so the shell freezes its own prototype instead.
Object.freeze(Object.prototype);

// The low-level IPC bridge Tauri gives its own pages; the @tauri-apps/api
// package wraps the same call. Using it directly keeps the app bundler-free.
const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => [...document.querySelectorAll(sel)];
let services = [];
let state = { accounts: [], active: null, locked: false, settings: {}, version: "" };
let picked = null;
let editing = null;
let editColor = "";
let drag = null;
let justDragged = false;

const COLORS = ["#e5484d", "#f76b15", "#ffb224", "#30a46c", "#12a594", "#0090ff", "#3e63dd", "#8e4ec6", "#d6409f", "#6f6e77"];

const serviceOf = (id) => services.find((s) => s.id === id);
const colorOf = (a) => a.color || serviceOf(a.service).color;

function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "style") node.style.cssText = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else if (k in node) node[k] = v;
    else node.setAttribute(k, v);
  }
  node.append(...children);
  return node;
}

// Logos are Simple Icons glyphs (CC0) bundled in ui/icons, drawn white on a color.
function glyph(serviceId) {
  const g = el("span", { className: "glyph" });
  g.style.setProperty("--icon", `url(icons/${serviceId}.svg)`);
  return g;
}
function logo(serviceId, color, className) {
  return el("span", { className, style: `--c:${color}` }, [glyph(serviceId)]);
}

function render() {
  document.body.classList.toggle("labels", Boolean(state.settings.show_labels));
  $("#dnd").classList.toggle("on", Boolean(state.settings.dnd_until));
  $("#dnd").title = state.settings.dnd_until ? "Do Not Disturb is on" : "Do Not Disturb";
  for (const id of ["#add", "#dnd", "#manage"]) $(id).hidden = state.locked;
  renderSidebar();
  if (state.locked) {
    showOnly("lock-panel");
    setTimeout(() => $("#unlock-pin").focus());
  } else if (!$("#lock-panel").hidden) {
    showOnly(state.accounts.length ? null : "welcome");
  }
  if (!$("#manage-panel").hidden) renderSettings();
}

function renderSidebar() {
  // Re-rendering mid-drag would drop the icon being dragged; drag end renders.
  if (drag) return;
  $("#accounts").replaceChildren(
    ...state.accounts.map((a) => {
      const svc = serviceOf(a.service);
      const btn = el(
        "button",
        {
          className: "acct",
          title: `${a.label} · ${svc.name}`,
          style: `--c:${colorOf(a)}`,
          "aria-label": `${a.label}, ${svc.name}`,
          onclick: () => justDragged || select(a.id),
          oncontextmenu: (e) => {
            e.preventDefault();
            invoke("show_account_menu", { id: a.id });
          },
        },
        [glyph(svc.id)]
      );
      if (a.unread > 0) btn.append(el("span", { className: "badge", textContent: a.unread > 99 ? "99+" : a.unread }));
      if (!a.notify) {
        btn.append(
          el("span", { className: "muted-mark", title: "Notifications off", innerHTML: '<svg viewBox="0 0 24 24" width="11" height="11"><path fill="currentColor" d="M3 3l18 18-1.4 1.4-3-3H4v-1l2-2v-5c0-1 .2-2 .7-2.8L1.6 4.4zM18 14.2 9.3 5.5A6 6 0 0 1 18 10.6zM10 20h4a2 2 0 0 1-4 0z"/></svg>' })
        );
      }
      const slot = el("div", { className: "slot" + (a.id === state.active ? " active" : "") }, [btn]);
      if (state.settings.show_labels) slot.append(el("span", { className: "name", textContent: a.label }));
      slot.dataset.id = a.id;
      return slot;
    })
  );
}

function renderServices() {
  $("#service-grid").replaceChildren(
    ...services.map((s) =>
      el(
        "button",
        {
          className: "svc",
          "aria-pressed": String(picked === s.id),
          onclick: () => {
            picked = s.id;
            $("#confirm-add").disabled = false;
            renderServices();
          },
        },
        [logo(s.id, s.color, "dot"), s.name]
      )
    )
  );
}

function renderSwatches() {
  const acct = state.accounts.find((a) => a.id === editing);
  if (!acct) return;
  const brand = serviceOf(acct.service).color;
  $("#swatches").replaceChildren(
    el("button", { className: "swatch brand", style: `--c:${brand}`, title: "Brand color", "aria-pressed": String(!editColor), onclick: () => ((editColor = ""), renderSwatches()) }, [glyph(acct.service)]),
    ...COLORS.map((c) => el("button", { className: "swatch", style: `--c:${c}`, title: c, "aria-pressed": String(editColor === c), onclick: () => ((editColor = c), renderSwatches()) }))
  );
}

function renderSettings() {
  const s = state.settings;
  $("#manage-list").replaceChildren(
    ...state.accounts.map((a) => {
      const svc = serviceOf(a.service);
      return el("li", {}, [
        logo(svc.id, colorOf(a), "dot"),
        el("span", { className: "name" }, [a.label, el("small", { textContent: svc.name })]),
        el("button", { className: "ghost", textContent: "Reload", onclick: () => invoke("reload_account", { id: a.id }) }),
        el("button", { className: "ghost", textContent: "Edit", onclick: () => window.masterChat.edit(a.id) }),
        el("button", { className: "danger", textContent: "Remove", onclick: () => window.masterChat.remove(a.id) }),
      ]);
    })
  );
  for (const b of $$("#theme button")) b.setAttribute("aria-pressed", String(b.dataset.theme === s.theme));
  for (const box of $$("[data-setting]")) box.checked = Boolean(s[box.dataset.setting]);
  $("[data-setting=start_minimized]").disabled = !s.start_with_windows;
  $("#dnd-status").textContent = !s.dnd_until
    ? "Off"
    : s.dnd_until > 4e15
      ? "On until you turn it off"
      : `On until ${new Date(s.dnd_until * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  $("#lock-status").textContent = s.lock_enabled ? "On: a PIN is needed to open the app" : "Off";
  $("#lock-buttons").replaceChildren(
    ...(s.lock_enabled
      ? [
          el("button", { className: "ghost", textContent: "Lock now", onclick: () => invoke("lock_now").then(setState) }),
          el("button", { className: "ghost", textContent: "Change PIN", onclick: () => showPinForm(true) }),
          el("button", { className: "danger", textContent: "Turn off", onclick: turnOffLock }),
        ]
      : [el("button", { className: "ghost", textContent: "Set a PIN…", onclick: () => showPinForm(false) })])
  );
  $("#idle-row").hidden = !s.lock_enabled;
  $("#idle-minutes").value = String(s.lock_idle_minutes);
  $("#version").textContent = state.version;
}

function setState(next) {
  state = next;
  render();
}

function showOnly(id) {
  for (const p of $$(".panel")) p.hidden = p.id !== id;
}

// Panels sit under the service webviews, so the active one is hidden while a panel is open.
async function openPanel(id) {
  if (state.locked) return;
  showOnly(id);
  if (id === "add-panel") {
    picked = null;
    $("#label").value = "";
    $("#confirm-add").disabled = true;
    renderServices();
  }
  if (id === "manage-panel") {
    $("#pin-form").hidden = true;
    $("#update-status").textContent = "";
    renderSettings();
  }
  await invoke("set_overlay", { open: true });
}

async function closePanel() {
  if (state.locked) return;
  showOnly(state.accounts.length ? null : "welcome");
  if (state.accounts.length) await invoke("set_overlay", { open: false });
}

async function select(id) {
  state.active = id;
  renderSidebar();
  showOnly(null);
  await invoke("select_account", { id });
}

const update = (changes) => invoke("update_settings", { update: changes }).then(setState).catch((e) => alert(e));

$("#add").addEventListener("click", () => openPanel("add-panel"));
$("#manage").addEventListener("click", () => openPanel("manage-panel"));
$("#dnd").addEventListener("click", () => invoke("show_dnd_menu"));
$("#dnd-settings").addEventListener("click", () => invoke("show_dnd_menu"));
document.addEventListener("click", (e) => {
  if (e.target.closest("[data-close]")) closePanel();
  const open = e.target.closest("[data-open]");
  if (open) openPanel(open.dataset.open);
  const theme = e.target.closest("#theme button");
  if (theme) update({ theme: theme.dataset.theme });
});
for (const box of $$("[data-setting]")) box.addEventListener("change", () => update({ [box.dataset.setting]: box.checked }));
$("#idle-minutes").addEventListener("change", (e) => update({ lock_idle_minutes: Number(e.target.value) }));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closePanel();
  if (e.key === "Enter" && !$("#add-panel").hidden && picked) $("#confirm-add").click();
  if (e.key === "Enter" && !$("#edit-panel").hidden) $("#confirm-edit").click();
});

$("#confirm-add").addEventListener("click", async () => {
  if (!picked) return;
  setState(await invoke("add_account", { service: picked, label: $("#label").value }));
  showOnly(null);
});

$("#confirm-edit").addEventListener("click", async () => {
  const label = $("#edit-label").value.trim();
  if (!editing || !label) return;
  setState(await invoke("update_account", { id: editing, label, color: editColor }));
  editing = null;
  closePanel();
});

// The lock screen.
const unlock = async () => {
  const pin = $("#unlock-pin").value;
  if (!pin) return;
  try {
    const next = await invoke("unlock", { pin });
    $("#unlock-pin").value = "";
    $("#unlock-error").hidden = true;
    showOnly(next.accounts.length ? null : "welcome");
    setState(next);
  } catch (e) {
    $("#unlock-error").textContent = String(e);
    $("#unlock-error").hidden = false;
    $("#unlock-pin").select();
  }
};
$("#confirm-unlock").addEventListener("click", unlock);
$("#unlock-pin").addEventListener("keydown", (e) => e.key === "Enter" && unlock());

// Setting or changing the PIN.
function showPinForm(change) {
  $("#pin-form").hidden = false;
  $("#pin-current").hidden = !change;
  $("#pin-error").hidden = true;
  for (const id of ["#pin-current", "#pin-new", "#pin-repeat"]) $(id).value = "";
  $(change ? "#pin-current" : "#pin-new").focus();
}
$("#pin-cancel").addEventListener("click", () => ($("#pin-form").hidden = true));
$("#pin-save").addEventListener("click", async () => {
  const pin = $("#pin-new").value;
  const fail = (message) => {
    $("#pin-error").textContent = message;
    $("#pin-error").hidden = false;
  };
  if (pin !== $("#pin-repeat").value) return fail("The two new PINs don't match.");
  try {
    const current = $("#pin-current").hidden ? null : $("#pin-current").value;
    setState(await invoke("set_pin", { current, pin }));
    $("#pin-form").hidden = true;
  } catch (e) {
    fail(String(e));
  }
});
async function turnOffLock() {
  const pin = prompt("Enter your PIN to turn the app lock off:");
  if (pin == null) return;
  try {
    setState(await invoke("disable_lock", { pin }));
  } catch (e) {
    alert(e);
  }
}

// Updates.
$("#check-update").addEventListener("click", async () => {
  const status = $("#update-status");
  status.textContent = "Checking…";
  try {
    const found = await invoke("check_update");
    if (!found) {
      status.textContent = "You have the latest version.";
      return;
    }
    status.textContent = `Version ${found.version} is available.`;
    if (confirm(`Install Master Chat ${found.version} now? The app will restart.`)) {
      status.textContent = "Downloading…";
      await invoke("install_update");
    }
  } catch (e) {
    status.textContent = "Couldn't check for updates.";
    console.warn(e);
  }
});

// Drag an account up or down the sidebar to reorder it.
const accountList = $("#accounts");
accountList.addEventListener("pointerdown", (e) => {
  const slot = e.target.closest(".slot");
  if (!slot || e.button !== 0) return;
  drag = { slot, startY: e.clientY, moved: false };
  slot.setPointerCapture(e.pointerId);
});
accountList.addEventListener("pointermove", (e) => {
  if (!drag || (!drag.moved && Math.abs(e.clientY - drag.startY) < 6)) return;
  drag.moved = true;
  drag.slot.classList.add("dragging");
  const others = [...accountList.children].filter((s) => s !== drag.slot);
  const before = others.find((s) => {
    const r = s.getBoundingClientRect();
    return e.clientY < r.top + r.height / 2;
  });
  if (drag.slot.nextElementSibling !== (before ?? null)) accountList.insertBefore(drag.slot, before ?? null);
});
const endDrag = async () => {
  if (!drag) return;
  const { slot, moved } = drag;
  drag = null;
  slot.classList.remove("dragging");
  if (!moved) return;
  // The click that ends a drag doesn't open the account.
  justDragged = true;
  setTimeout(() => (justDragged = false));
  const ids = [...accountList.children].map((s) => s.dataset.id);
  setState(await invoke("reorder_accounts", { ids }).catch(() => invoke("list_accounts")));
};
accountList.addEventListener("pointerup", endDrag);
accountList.addEventListener("pointercancel", endDrag);

// The shell is an app, not a page: no browser menu on right-click.
document.addEventListener("contextmenu", (e) => {
  if (!e.target.closest("input")) e.preventDefault();
});

// Called from Rust: title changes, the menus, notifications, the lock.
window.masterChat = {
  unread(id, count) {
    const acct = state.accounts.find((a) => a.id === id);
    if (acct) {
      acct.unread = count;
      renderSidebar();
    }
  },
  async refresh() {
    setState(await invoke("list_accounts"));
  },
  async edit(id) {
    const acct = state.accounts.find((a) => a.id === id);
    if (!acct) return;
    editing = id;
    editColor = acct.color || "";
    await openPanel("edit-panel");
    renderSwatches();
    $("#edit-label").value = acct.label;
    $("#edit-label").select();
  },
  async remove(id) {
    const acct = state.accounts.find((a) => a.id === id);
    if (!acct) return;
    const svc = serviceOf(acct.service);
    if (!confirm(`Remove "${acct.label}" (${svc.name}) and sign it out of this computer?`)) return;
    setState(await invoke("remove_account", { id }));
    if (state.accounts.length === 0) closePanel();
  },
};

(async () => {
  services = await invoke("list_services");
  state = await invoke("list_accounts");
  render();
  if (!state.locked && state.accounts.length === 0) showOnly("welcome");
})();
