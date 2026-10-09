const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
let config = null;
let modelList = null;

const PRESETS = {
  clm: { id: "clm", name: "CLM", url: "http://127.0.0.1:8700", models: ["clm-latest"], installer: "clm", hint: "Local Contrastive-LM server (clm-serve)." },
  laya: { id: "laya", name: "Laya", url: "http://127.0.0.1:8000", models: ["english", "multilingual", "typed-decisions"], hint: "Local laya-serve. Start it with: LAYA_HOST=127.0.0.1 laya-serve" },
  typesafe: { id: "typesafe", name: "TypeSafe Jev", url: "https://api.typesafe.ai", models: ["jev-latest"], hint: "TypeSafe cloud API. Key from console.typesafe.ai/keys." },
  pipellm: { id: "pipellm", name: "PipeLLM Jev", url: "https://api.pipellm.ai", models: ["jev-1.13.0"], hint: "Jev hosted by PipeLLM." },
  custom: { id: "custom", name: "Custom", url: "http://127.0.0.1:9000", models: [], hint: "Any server that speaks POST /v1/systemone." },
};
const PRESET_LABELS = { clm: "CLM (local)", laya: "Laya (local)", typesafe: "TypeSafe Jev (cloud)", pipellm: "PipeLLM Jev (cloud)", custom: "Custom server" };

function say(el, text, tone = "") {
  el.textContent = text;
  el.dataset.tone = tone;
}

/** A success note that fades out after 3 s; an error shown with `say` stays until replaced. */
function flash(el, text) {
  say(el, text, "ok");
  clearTimeout(el.flashTimer);
  el.flashTimer = setTimeout(() => { if (el.textContent === text) say(el, ""); }, 3000);
}

/** How long a locked button stays locked at least, so a fast action shows its spinner instead of a flicker. */
const BUSY_MS = 400;

/** Disables `button` and shows a spinner in it; the returned function unlocks it. */
function lock(button) {
  button.disabled = true;
  const until = Date.now() + BUSY_MS;
  button.insertAdjacentHTML("afterbegin", '<svg viewBox="0 0 16 16" class="spinner size-4 motion-safe:animate-spin" aria-hidden="true"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="2" opacity="0.25"/><path d="M14 8a6 6 0 0 0-6-6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>');
  return () => setTimeout(() => {
    button.querySelector(".spinner")?.remove();
    button.disabled = false;
  }, Math.max(0, until - Date.now()));
}

/** Runs `action` with `button` locked; a click while it runs is ignored. */
async function busy(button, action) {
  if (button.disabled) return;
  const unlock = lock(button);
  try { await action(); } finally { unlock(); }
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

const store = {
  get(k) { try { return JSON.parse(localStorage.getItem("jeff." + k)); } catch { return null; } },
  set(k, v) { try { localStorage.setItem("jeff." + k, JSON.stringify(v)); } catch {} },
};

function authHeaders(headers = {}) {
  const key = store.get("key");
  return key ? { ...headers, authorization: `Bearer ${key}` } : headers;
}

async function api(path, opts = {}) {
  const r = await fetch(path, { ...opts, headers: authHeaders(opts.headers) });
  const body = await r.json().catch(() => ({}));
  $("#auth-error").hidden = r.status !== 401;
  if (r.status === 401) $("#auth-error").textContent = body.error || "jeff rejected the access key.";
  if (!r.ok) throw new Error(body.error || body.detail?.message || body.detail || `HTTP ${r.status}`);
  return { body, headers: r.headers };
}

/* ---------- tabs ---------- */

const PAGES = ["questions", "classifiers", "providers", "keys"];

function showTab() {
  const id = PAGES.find((p) => location.hash === "#" + p) || "questions";
  for (const t of PAGES) $("#page-" + t).hidden = t !== id;
  if (id === "keys" && config) loadKeys().catch((e) => say($("#key-status"), e.message, "err"));
  for (const a of $$(".tab")) {
    if (a.getAttribute("href") === "#" + id) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  }
}
addEventListener("hashchange", showTab);

/** How long a destructive button waits for the confirming second click. */
const CONFIRM_MS = 3000;
/** How long a button stays disabled after a click, so a double click is not taken as the confirmation. */
const GUARD_MS = 600;

/** Disarms the one button currently waiting for confirmation. */
let disarmCurrent = () => {};

/** The first click arms the button and counts down the seconds left; a second click before then calls `action`. */
function confirmClick(button, armedLabel, action) {
  const label = button.textContent;
  let timer;
  const disarm = () => {
    clearInterval(timer);
    button.dataset.confirm = "false";
    button.textContent = label;
  };
  button.onclick = async () => {
    const until = Date.now() + GUARD_MS;
    button.disabled = true;
    try {
      if (button.dataset.confirm === "true") {
        disarm();
        await action();
      } else {
        disarmCurrent();
        disarmCurrent = disarm;
        button.dataset.confirm = "true";
        let left = CONFIRM_MS / 1000;
        button.textContent = `${armedLabel} · ${left}s`;
        timer = setInterval(() => {
          left -= 1;
          if (left > 0) button.textContent = `${armedLabel} · ${left}s`;
          else disarm();
        }, 1000);
      }
    } finally {
      setTimeout(() => { button.disabled = false; }, Math.max(0, until - Date.now()));
    }
  };
}

/* ---------- models ---------- */

function modelGroups() {
  return modelList.providers.map((p) => {
    const g = el("optgroup");
    g.label = p.ok ? p.name : `${p.name} (${p.warning ? "starting" : "offline"})`;
    for (const m of p.models) g.append(new Option(`${p.id}/${m}`, `${p.id}/${m}`));
    return g;
  });
}

function fillModelSelects() {
  const groups = modelList.providers;
  const all = modelList.data.map((m) => m.id);
  const sel = $("#pg-model");
  const keep = sel.value || store.get("model") || config.default_model;
  sel.replaceChildren(...modelGroups());
  fillFormModel();
  if (!all.includes(config.default_model)) sel.prepend(new Option(config.default_model, config.default_model));
  sel.value = all.includes(keep) ? keep : config.default_model;
  const csel = $("#c-model");
  const ckeep = csel.value || store.get("c-model") || config.default_model;
  csel.replaceChildren(...modelGroups());
  if (!all.includes(config.default_model)) csel.prepend(new Option(config.default_model, config.default_model));
  csel.value = all.includes(ckeep) ? ckeep : config.default_model;
  fillClassifierModel();
  updateCurl();
  updateClassifierCurl();
  fillDefaultModel();
  for (const p of groups) {
    const card = $(`[data-provider="${p.id}"]`);
    if (!card) continue;
    const s = $(".status", card);
    if (p.warning) say(s, `Starting: ${p.warning}`);
    else if (!p.ok) say(s, `Offline: ${p.error}`, "err");
    else say(s, "Online", "ok");
  }
}

/** Offers the models of the provider cards on screen, saved or not, so a just-added provider is choosable before Save. */
function fillDefaultModel() {
  const sel = $("#default-model");
  const keep = sel.value || config.default_model;
  const live = new Map((modelList?.providers || []).map((p) => [p.id, p.models]));
  const groups = $$("#provider-list [data-provider]").map((card) => {
    const id = $(".p-id", card).value.trim();
    const typed = $(".p-models", card).value.split(",").map((m) => m.trim()).filter(Boolean);
    const g = el("optgroup");
    g.label = $(".p-name", card).value || id;
    for (const m of new Set([...(live.get(id) || []), ...typed])) g.append(new Option(`${id}/${m}`, `${id}/${m}`));
    return g;
  });
  sel.replaceChildren(...groups.filter((g) => g.children.length));
  const values = [...sel.options].map((o) => o.value);
  sel.value = values.includes(keep) ? keep : (values[0] ?? "");
}

$("#provider-list").addEventListener("input", (e) => {
  if (e.target.matches(".p-id, .p-name, .p-models")) fillDefaultModel();
});

async function loadModels() {
  modelList = (await api("/v1/models")).body;
  fillModelSelects();
}

/* ---------- providers ---------- */

function providerCard(p) {
  const card = $("#tpl-provider").content.firstElementChild.cloneNode(true);
  card.dataset.provider = p.id;
  $(".p-title", card).textContent = p.name || p.id;
  $(".p-hint", card).textContent = (PRESETS[p.id] || {}).hint || "";
  $(".p-name", card).value = p.name || "";
  $(".p-id", card).value = p.id;
  $(".p-id-echo", card).textContent = p.id;
  $(".p-id", card).oninput = (e) => ($(".p-id-echo", card).textContent = e.target.value);
  $(".p-url", card).value = p.url;
  $(".p-key", card).placeholder = p.key_set ? "Saved" : "Not set";
  $(".p-key-hint", card).textContent = p.key_set ? "Leave empty to keep the saved key. Type a space to clear it." : "";
  $(".p-models", card).value = (p.models || []).join(", ");
  card.dataset.installer = p.installer || "";
  confirmClick($(".p-delete", card), "Confirm remove", () => {
    card.remove();
    fillDefaultModel();
    say($("#save-status"), "Save to apply.");
  });
  if (p.installer === "clm") {
    $(".p-install", card).hidden = false;
    $(".p-install-btn", card).onclick = () => clmTask(card, "install");
    $(".p-remove-btn", card).onclick = () => clmTask(card, "remove");
  }
  return card;
}

function renderProviders() {
  $("#provider-list").replaceChildren(...config.providers.map(providerCard));
  for (const card of $$("[data-installer=clm]")) clmTask(card).catch(() => {});
  const used = new Set(config.providers.map((p) => p.id));
  $("#preset").replaceChildren(...Object.entries(PRESET_LABELS).map(([k, label]) => {
    const o = new Option(label, k);
    o.disabled = k !== "custom" && used.has(k);
    return o;
  }));
  $("#preset").value = Object.keys(PRESET_LABELS).find((k) => k === "custom" || !used.has(k));
  fillDefaultModel();
}

$("#add-provider").onclick = () => {
  const preset = PRESETS[$("#preset").value];
  const ids = new Set($$("[data-provider]").map((c) => $(".p-id", c).value));
  let id = preset.id;
  for (let n = 2; ids.has(id); n++) id = `${preset.id}-${n}`;
  const card = providerCard({ ...preset, id, key_set: false });
  $("#provider-list").append(card);
  fillDefaultModel();
  $(".p-url", card).focus();
  say($("#save-status"), "Save to apply.");
};

let info = null;
/** Saved questions by key, as the server last sent them. */
let saved = {};
/** Saved classifiers by key, as the server last sent them. */
let classifiers = {};
const checked = new Set(store.get("checked") || []);

async function refreshInfo() {
  info = (await api("/api/info")).body;
  updateCurl();
}

async function loadConfig() {
  if (!info) await refreshInfo();
  config = (await api("/api/config")).body;
  renderProviders();
  if (location.hash === "#keys") loadKeys().catch(() => {});
  await loadModels();
}

$("#refresh").onclick = (e) => busy(e.currentTarget, () => loadModels().catch((err) => say($("#save-status"), err.message, "err")));

$("#save").onclick = (e) => busy(e.currentTarget, saveProviders);

async function saveProviders() {
  const providers = $$("#provider-list [data-provider]").map((card) => {
    const p = {
      id: $(".p-id", card).value.trim(),
      name: $(".p-name", card).value,
      url: $(".p-url", card).value,
      models: $(".p-models", card).value.split(","),
      installer: card.dataset.installer || null,
    };
    const key = $(".p-key", card).value;
    if (key) p.key = key;
    return p;
  });
  try {
    await api("/api/config", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ default_model: $("#default-model").value, providers }),
    });
    flash($("#save-status"), "Saved");
    await loadConfig();
  } catch (e) {
    say($("#save-status"), e.message, "err");
  }
}

const STEP_ICONS = {
  done: '<svg viewBox="0 0 20 20" class="size-5 text-green-600 dark:text-green-400" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 10.5l3.2 3L15 7"/></svg>',
  pending: '<svg viewBox="0 0 20 20" class="size-5 text-neutral-300 dark:text-neutral-700" aria-hidden="true"><circle cx="10" cy="10" r="7" fill="none" stroke="currentColor" stroke-width="2"/></svg>',
  spin: '<svg viewBox="0 0 20 20" class="size-5 text-sky-600 motion-safe:animate-spin dark:text-sky-400" aria-hidden="true"><circle cx="10" cy="10" r="7" fill="none" stroke="currentColor" stroke-width="2.5" opacity="0.25"/><path d="M17 10a7 7 0 0 0-7-7" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"/></svg>',
  error: '<svg viewBox="0 0 20 20" class="size-5 text-red-600 dark:text-red-400" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="10" cy="10" r="7"/><path d="M10 6.5v4.2M10 13.5v.01"/></svg>',
};

function ring(value) {
  const percent = Math.floor(value * 100);
  return `<svg viewBox="0 0 20 20" class="size-5 -rotate-90" role="progressbar" aria-valuenow="${percent}" aria-label="Progress"><circle cx="10" cy="10" r="7" fill="none" stroke-width="2.5" class="stroke-neutral-200 dark:stroke-neutral-700"/><circle cx="10" cy="10" r="7" fill="none" stroke-width="2.5" stroke-linecap="round" pathLength="100" stroke-dasharray="100" stroke-dashoffset="${100 - percent}" class="stroke-sky-500 transition-[stroke-dashoffset] duration-300"/></svg>`;
}

function stepIcon(kind, progress) {
  const icon = el("span", "flex size-5 shrink-0 items-center justify-center");
  icon.innerHTML = kind === "ring" ? ring(progress) : STEP_ICONS[kind];
  return icon;
}

/** Shows the current step next to the buttons and every step under "All steps". Returns whether all are done. */
function renderSteps(card, t) {
  const steps = t.steps || [];
  const running = t.state === "running";
  const current = $(".p-current", card);
  if (running && card.dataset.action === "remove") {
    current.hidden = false;
    $(".p-task", card).hidden = true;
    $(".p-details", card).hidden = true;
    current.replaceChildren(stepIcon("spin"), el("span", "shrink-0 font-medium", "Uninstalling"), el("span", "note min-w-0", t.step || "Removing the CLM containers…"));
    return false;
  }
  const started = running || t.state === "error" || steps.slice(1).some((s) => s.done);
  let active = steps.findIndex((s) => !s.done);
  // A reinstall recreates running containers, so every step still reads done until compose stops them.
  const restarting = running && steps.length > 0 && active === -1;
  if (restarting) active = Math.max(0, steps.findIndex((s) => s.id === "containers"));
  const ready = steps.length > 0 && active === -1;
  const isDone = (s, i) => s.done && !(restarting && i >= active);
  const kindOf = (s, i) => i === active
    ? (t.state === "error" ? "error" : s.progress > 0 && !s.done ? "ring" : "spin")
    : isDone(s, i) ? "done" : "pending";

  current.hidden = !started || ready;
  $(".p-task", card).hidden = started && !ready && t.state !== "error";
  if (started && !ready && steps.length) {
    const s = steps[active];
    const kind = kindOf(s, active);
    const total = steps.reduce((sum, x) => sum + x.weight, 0);
    const doneWeight = steps.reduce((sum, x, i) => sum + x.weight * (isDone(x, i) ? 1 : restarting ? 0 : x.progress || 0), 0);
    const overall = doneWeight / total;
    const head = el("span", "shrink-0 font-medium tabular-nums", t.state === "error" ? "Install failed" : `Installing ${Math.floor(overall * 100)}%`);
    const now = el("span", "note min-w-0", `Step ${active + 1} of ${steps.length}: ${s.label}`);
    current.replaceChildren(stepIcon(kind === "error" ? "error" : "ring", overall), head, now);
  }

  $(".p-details", card).hidden = !started || ready;
  $(".p-steps", card).replaceChildren(...steps.map((s, i) => {
    const kind = kindOf(s, i);
    const row = el("li", "flex items-center gap-2");
    row.append(stepIcon(kind, s.progress), el("span", kind === "pending" ? "text-neutral-500 dark:text-neutral-400" : "", s.label));
    if (kind === "ring" || kind === "spin") row.append(el("span", "note tabular-nums", kind === "ring" ? `${Math.floor(s.progress * 100)}%` : "in progress"));
    return row;
  }));
  return ready;
}

async function clmTask(card, action) {
  if (!card.isConnected) return;
  const out = $(".p-task", card);
  const buttons = [$(".p-install-btn", card), $(".p-remove-btn", card)];
  if (action) {
    if (action === "remove" && !confirm("Stop and remove the CLM containers? Downloaded weights are kept.")) return;
    // Shown and locked before the request, so the click registers at once and cannot start a second action.
    for (const b of buttons) b.disabled = true;
    card.dataset.action = action;
    renderSteps(card, { steps: card.clmSteps, state: "running", step: action === "remove" ? "Removing the CLM containers…" : "" });
    try { await api(`/api/clm/${action}`, { method: "POST" }); }
    catch (e) {
      for (const b of buttons) b.disabled = false;
      delete card.dataset.action;
      renderSteps(card, { steps: card.clmSteps, state: "idle" });
      say(out, e.message, "err");
      return;
    }
  }
  let t;
  try { t = (await api("/api/clm")).body; }
  catch (e) {
    // One failed status read must not stop the polling and leave the buttons locked.
    say(out, `Could not read the CLM status: ${e.message}. Retrying…`, "err");
    setTimeout(() => clmTask(card), 2000);
    return;
  }
  if (t.state !== "running") delete card.dataset.action;
  card.clmSteps = t.steps;
  const ready = renderSteps(card, t);
  const containersUp = (t.steps || []).find((s) => s.id === "containers")?.done;
  // Reinstall stays locked while the model downloads, loads and warms up: restarting would begin that again.
  const busy = t.state === "running" || (containersUp && !ready);
  buttons[0].disabled = busy;
  buttons[1].disabled = t.state === "running";
  buttons[0].title = busy && t.state !== "running" ? "CLM is still starting. Uninstall stops it." : "";
  // Kept while busy: a reinstall stops the containers for a moment, which would flip the label to Install.
  if (!busy || !card.dataset.labelSet) {
    buttons[0].textContent = containersUp ? "Reinstall" : "Install";
    card.dataset.labelSet = "1";
  }
  if (t.state === "error") say(out, t.error, "err");
  else if (ready) say(out, "CLM is installed and ready.", "ok");
  else if (t.state === "running") say(out, t.step || "Working…");
  else if (containersUp) say(out, "Downloading and loading the model. You can close this page; it keeps going.");
  if (t.state === "running" || (containersUp && !ready)) setTimeout(() => clmTask(card), 2000);
  else if (ready && !card.dataset.wasReady) { card.dataset.wasReady = "1"; loadModels().catch(() => {}); }
}

/* ---------- access keys ---------- */

const DAY = 86400;

function dateValue(daysFromNow) {
  const d = new Date(Date.now() + daysFromNow * DAY * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** The last second of the chosen day in the viewer's time zone, in Unix seconds. */
function endOfDay(value) {
  return Math.floor(new Date(`${value}T23:59:59`).getTime() / 1000);
}

const shortDate = (t) => new Date(t * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });

/** The chosen expiry as YYYY-MM-DD, or "" for Never. */
function expiryDate() {
  const v = $("#key-exp").value;
  if (v === "never") return "";
  return v === "date" ? $("#key-date").value : dateValue(+v);
}

function syncExpiry() {
  const custom = $("#key-exp").value === "date";
  $("#key-date").hidden = !custom;
  const date = expiryDate();
  say($("#key-exp-note"), $("#key-exp").value === "never" ? "Valid until revoked."
    : date ? `Valid through ${shortDate(endOfDay(date))}.` : "Pick a date.", "");
}

function syncRole() {
  say($("#key-role-note"), $("#key-role").value === "admin"
    ? "Calls the API and manages jeff on this page."
    : "Calls /v1 only: decisions and models.", "");
}

$("#key-role").addEventListener("change", syncRole);
syncRole();

$("#key-date").min = dateValue(0);
$("#key-date").value = dateValue(30);
$("#key-date").addEventListener("input", syncExpiry);
$("#key-exp").addEventListener("change", syncExpiry);
syncExpiry();

$("#key-form").onsubmit = (e) => {
  e.preventDefault();
  busy(e.submitter ?? $("#key-form [type=submit]"), createKey);
};

async function createKey() {
  const never = $("#key-exp").value === "never";
  if (!never && !expiryDate()) { say($("#key-status"), "Pick an expiry date or choose Never.", "err"); return; }
  try {
    const { body } = await api("/api/keys", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: $("#key-name").value, role: $("#key-role").value, expires_at: never ? null : endOfDay(expiryDate()) }),
    });
    $("#key-value").value = body.key;
    $("#key-created").hidden = false;
    // The first key locks jeff; keep it here so this page does not lock its own operator out.
    const kept = !store.get("key") && body.role === "admin";
    if (kept) store.set("key", body.key);
    $("#key-saved-note").textContent = kept ? "This browser now uses this key for the admin page." : "";
    $("#key-name").value = "";
    flash($("#key-status"), "Key created");
    await refreshInfo();
    await loadKeys();
  } catch (err) {
    say($("#key-status"), err.message, "err");
  }
}

$("#key-copy").onclick = () => navigator.clipboard.writeText($("#key-value").value).then(() => flash($("#key-status"), "Key copied"));

function keyRow(k, now, last) {
  const li = el("li", "flex flex-wrap items-center gap-x-4 gap-y-1 py-3");
  const main = el("div", "min-w-0 flex-1");
  main.append(el("div", "font-medium", k.name), el("div", "note font-mono", `${k.prefix}… · ${k.role}`));
  const expired = k.expires_at !== null && k.expires_at <= now;
  const when = el("div", "note text-right tabular-nums");
  when.append(el("div", "", `Created ${shortDate(k.created_at)}`));
  const exp = el("div", "", k.expires_at === null ? "Never expires" : `${expired ? "Expired" : "Expires"} ${shortDate(k.expires_at)}`);
  exp.className = "note";
  exp.dataset.tone = expired ? "err" : "";
  when.append(exp);
  const revoke = el("button", "btn btn-danger", "Revoke");
  revoke.type = "button";
  revoke.title = last ? "Last key: jeff will accept requests without a key." : "Clients using this key stop working at once.";
  confirmClick(revoke, "Confirm revoke", async () => {
    try {
      await api(`/api/keys/${encodeURIComponent(k.id)}`, { method: "DELETE" });
      if ((store.get("key") || "").startsWith(k.prefix)) store.set("key", "");
      await refreshInfo();
      $("#key-created").hidden = true;
      await loadKeys();
    } catch (err) {
      say($("#key-status"), err.message, "err");
    }
  });
  li.append(main, when, revoke);
  return li;
}

async function loadKeys() {
  const { body } = await api("/api/keys");
  const now = Math.floor(Date.now() / 1000);
  const count = body.keys.length + body.static_keys;
  $("#key-list").replaceChildren(...body.keys.map((k) => keyRow(k, now, count === 1)));
  // Until an admin key exists, a client key would lock this page out, so the server accepts only admin keys.
  const hasAdmin = body.static_keys > 0 || body.keys.some((k) => k.role === "admin" && (k.expires_at === null || k.expires_at > now));
  $("#key-role").querySelector('option[value="client"]').disabled = !hasAdmin;
  if (!hasAdmin) $("#key-role").value = "admin";
  syncRole();
  if (!body.keys.length) $("#key-list").append(el("li", "note py-3", "No keys yet. Without keys, jeff accepts every request."));
  $("#key-static").hidden = !body.static_keys;
  $("#key-static").textContent = `Plus ${body.static_keys} key${body.static_keys === 1 ? "" : "s"} from --api-key or JEFF_API_KEY. Those never expire and are changed where jeff is started.`;
}

/* ---------- question builder ---------- */

const TYPE_INFO = {
  noul: { badge: "yes/no", add: "", help: "Answered with one probability: how true the statement is for the state." },
  choice: { badge: "choice", add: "Add option", help: "Picks one option. The description is what the model compares; the key names the answer." },
  score: { badge: "score", add: "Add level", help: "Rates on ordered levels, lowest first. The answer is the expected level." },
};

function rowInput(cls, value, placeholder, label) {
  const i = el("input", `control ${cls}`);
  i.value = value || "";
  i.placeholder = placeholder;
  i.setAttribute("aria-label", label);
  i.spellcheck = false;
  return i;
}

const X_ICON = '<svg viewBox="0 0 24 24" class="size-4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>';

function removeButton(onClick, label) {
  const b = el("button", "icon-btn");
  b.type = "button";
  b.innerHTML = X_ICON;
  b.setAttribute("aria-label", label);
  b.title = label;
  b.onclick = onClick;
  return b;
}

function addRow(card, type, a = "", b = "") {
  const row = el("div", "q-row flex items-center gap-2");
  if (type === "choice") {
    row.append(rowInput("q-opt-key w-40 shrink-0 font-mono", a, "key", "Option key"), rowInput("q-opt-desc", b, "description", "Option description"));
  } else {
    const n = $(".q-rows", card).children.length;
    row.append(el("span", "q-level-n note w-6 shrink-0 text-right font-mono tabular-nums", String(n)), rowInput("q-level", a, "level description", "Level description"));
  }
  row.append(removeButton(() => { row.remove(); renumber(card); }, type === "choice" ? "Remove option" : "Remove level"));
  $(".q-rows", card).append(row);
}

function renumber(card) {
  $$(".q-level-n", card).forEach((n, i) => (n.textContent = String(i)));
}

function setType(card, type, rows) {
  const info = TYPE_INFO[type];
  card.dataset.type = type;
  $(".q-type", card).value = type;
  $(".q-badge", card).textContent = info.badge;
  $(".q-badge", card).dataset.type = type;
  $(".q-rows", card).replaceChildren();
  $(".q-add", card).hidden = type === "noul";
  $(".q-add", card).textContent = info.add;
  $(".q-help", card).textContent = info.help;
  for (const r of rows || []) addRow(card, type, ...r);
  if (!rows && type !== "noul") { addRow(card, type); addRow(card, type); }
}

function addQuestion(q) {
  const card = $("#tpl-question").content.firstElementChild.cloneNode(true);
  $(".q-key", card).value = q.key || "";
  $(".q-instr", card).value = q.instructions || "";
  $(".q-type", card).onchange = (e) => setType(card, e.target.value);
  $(".q-add", card).onclick = () => addRow(card, card.dataset.type);
  setType(card, q.type || "noul", q.rows);
  $("#q-form-card").replaceChildren(card);
  return card;
}

function readQuestions() {
  const out = {};
  for (const card of $$("#q-form-card .q")) {
    const key = $(".q-key", card).value.trim();
    if (!key) throw new Error("Every question needs a key.");
    if (out[key]) throw new Error(`Question key "${key}" is used twice.`);
    const q = { type: card.dataset.type, instructions: $(".q-instr", card).value.trim() };
    if (q.type === "choice") {
      q.criteria = {};
      for (const row of $$(".q-row", card)) {
        const k = $(".q-opt-key", row).value.trim();
        if (k) q.criteria[k] = $(".q-opt-desc", row).value.trim() || k;
      }
      if (Object.keys(q.criteria).length < 2) throw new Error(`"${key}" needs at least two options.`);
    } else if (q.type === "score") {
      q.criteria = $$(".q-level", card).map((i) => i.value.trim()).filter(Boolean);
      if (q.criteria.length < 2) throw new Error(`"${key}" needs at least two levels.`);
    }
    out[key] = q;
  }
  return out;
}

function saveState() {
  store.set("state", $("#pg-state").value);
  $("#pg-count").textContent = `${$("#pg-state").value.length} chars`;
  updateCurl();
}

const EXAMPLE = {
  state: "Customer: my invoice was charged twice and nobody answers the phone! I need this fixed today.",
  questions: [
    { key: "urgency", type: "noul", instructions: "The message conveys urgency or time pressure." },
    { key: "department", type: "choice", instructions: "Which team should handle this?", rows: [["billing", "Charges, invoices, refunds"], ["technical", "Bugs and outages"], ["sales", "Pricing and plans"]] },
    { key: "frustration", type: "score", instructions: "How frustrated is the customer?", rows: [["Calm"], ["Frustrated"], ["Very angry"]] },
  ],
};

/** Turns a builder snapshot (`rows`) into the native question shape. */
function nativeOf(q) {
  const out = { type: q.type, instructions: q.instructions };
  if (q.type === "choice") out.criteria = Object.fromEntries(q.rows);
  if (q.type === "score") out.criteria = q.rows.map((r) => r[0]);
  return out;
}

/** Turns a saved question into what `addQuestion` takes. */
function builderOf(key, q) {
  const rows = q.type === "choice" ? Object.entries(q.criteria || {}) : q.type === "score" ? (q.criteria || []).map((c) => [c]) : undefined;
  return { key, type: q.type, instructions: q.instructions, rows };
}

// The Playground kept the state inside `draft`; saveState below moves it to `state`.
$("#pg-state").value = store.get("state") ?? store.get("draft")?.state ?? EXAMPLE.state;
saveState();
$("#c-state").value = $("#pg-state").value;
$("#pg-state").addEventListener("input", () => { $("#c-state").value = $("#pg-state").value; saveState(); });
$("#c-state").addEventListener("input", () => { $("#pg-state").value = $("#c-state").value; saveState(); updateClassifierCurl(); });
$("#pg-model").onchange = (e) => { store.set("model", e.target.value); updateCurl(); };

/* ---------- answers ---------- */

let view = store.get("view") || "answer";
let last = null;

function setView(v) {
  view = v;
  store.set("view", v);
  for (const t of $$(".view-tab")) t.setAttribute("aria-selected", String(t.dataset.view === v));
  for (const p of ["", "c-"]) {
    $(`#${p}view-answer`).hidden = v !== "answer";
    $(`#${p}view-json`).hidden = v !== "json";
    $(`#${p}view-curl`).hidden = v !== "curl";
  }
}
for (const t of $$(".view-tab")) t.onclick = () => setView(t.dataset.view);

const pct = (p) => `${(p * 100).toFixed(1)}%`;

function bar(label, note, p, chosen) {
  const row = el("div", "bar relative flex items-center gap-3 overflow-hidden rounded-md border px-3 py-1.5");
  row.dataset.chosen = String(chosen);
  const fill = el("div", "bar-fill absolute inset-y-0 left-0");
  fill.style.width = `${Math.max(0, Math.min(1, p)) * 100}%`;
  const text = el("span", "relative min-w-0 flex-1 truncate");
  text.append(el("span", "font-mono font-medium", label));
  if (note) text.append(el("span", "ml-2 text-neutral-600 dark:text-neutral-400", note));
  row.append(fill, text, el("span", "relative font-mono tabular-nums", pct(p)));
  return row;
}

function answerCard(key, q, a, model) {
  const card = el("div", "card space-y-3");
  const head = el("div", "flex flex-wrap items-center gap-2");
  head.append(el("span", "font-mono font-semibold", key));
  const badge = el("span", "badge", TYPE_INFO[a.type]?.badge || a.type);
  badge.dataset.type = a.type;
  head.append(badge);
  if (model) head.append(modelChip(model));
  card.append(head);
  if (q?.instructions) card.append(el("p", "text-neutral-600 dark:text-neutral-400", q.instructions));

  const big = el("div", "flex flex-wrap items-baseline gap-x-3");
  const bars = el("div", "space-y-1.5");
  if (a.type === "noul") {
    big.append(el("span", "text-3xl font-semibold tracking-tight tabular-nums", pct(a.noul)), el("span", "note", a.noul >= 0.5 ? "probability the statement is true" : "probability the statement is true · leans false"));
    bars.append(bar("true", "", a.noul, a.noul >= 0.5), bar("false", "", 1 - a.noul, a.noul < 0.5));
  } else if (a.type === "choice") {
    big.append(el("span", "text-3xl font-semibold tracking-tight", a.choice));
    if (a.confidence != null) big.append(el("span", "note tabular-nums", `confidence ${pct(a.confidence)}`));
    const probs = Object.entries(a.probabilities || {}).sort((x, y) => y[1] - x[1]);
    for (const [k, p] of probs) bars.append(bar(k, q?.criteria?.[k] && q.criteria[k] !== k ? q.criteria[k] : "", p, k === a.choice));
  } else if (a.type === "score") {
    const legend = a.legend || Object.fromEntries((q?.criteria || []).map((c, i) => [String(i), c]));
    // The score is the expected level, so it can fall between levels; the highlight marks the most likely level instead.
    const probs = Object.entries(a.probabilities || {});
    const top = probs.reduce((best, cur) => (cur[1] > best[1] ? cur : best), probs[0] || ["", 0]);
    big.append(el("span", "text-3xl font-semibold tracking-tight", legend[top[0]] || top[0]), el("span", "note tabular-nums", `most likely, ${pct(top[1])} · expected level ${Number(a.score).toFixed(2)}`));
    if (a.confidence != null) big.append(el("span", "note tabular-nums", `confidence ${pct(a.confidence)}`));
    for (const [k, p] of probs) bars.append(bar(k, legend[k] || "", p, k === top[0]));
  } else {
    bars.append(el("pre", "font-mono text-xs whitespace-pre-wrap", JSON.stringify(a, null, 2)));
  }
  card.append(big, bars);
  return card;
}

function chip(label, value) {
  const c = el("span", "inline-flex items-center gap-1.5 rounded-md border border-neutral-200 bg-white px-2 py-0.5 text-xs dark:border-neutral-800 dark:bg-neutral-900");
  c.append(el("span", "text-neutral-600 dark:text-neutral-400", label), el("span", "font-mono tabular-nums", value));
  return c;
}

function apiOrigin() {
  if (!info) return location.origin;
  const i = info.api.lastIndexOf(":");
  const host = info.api.slice(0, i);
  const shown = host === "0.0.0.0" || host === "[::]" ? location.hostname : host;
  return `${location.protocol}//${shown}:${info.api.slice(i + 1)}`;
}

function curlFor(body, path = "/v1/systemone") {
  const json = JSON.stringify(body, null, 2).replace(/'/g, "'\\''");
  const auth = info?.api_key_set ? `  -H "authorization: Bearer $JEFF_API_KEY" \\\n` : "";
  return `curl ${apiOrigin()}${path} \\\n${auth}  -H 'content-type: application/json' \\\n  -d '${json}'`;
}

/** Saved keys in list order that are checked. */
function checkedKeys() {
  return Object.keys(saved).filter((k) => checked.has(k));
}

function runBody() {
  return { model: $("#pg-model").value, state: $("#pg-state").value, questions: checkedKeys() };
}

function updateCurl() {
  $("#curl-text").textContent = curlFor(runBody());
}
$("#curl-copy").onclick = () => navigator.clipboard.writeText($("#curl-text").textContent).then(() => flash($("#pg-status"), "curl copied"));
for (const t of $$('.view-tab[data-view="curl"]')) t.addEventListener("click", () => { updateCurl(); updateClassifierCurl(); });

/** Posts to `path` with a running timer in `status`; `button` stays disabled meanwhile. */
async function ask(body, status, button, path = "/v1/systemone") {
  const unlock = lock(button);
  const t0 = performance.now();
  say(status, "Running…");
  const tick = setInterval(() => say(status, `Running… ${Math.floor((performance.now() - t0) / 1000)} s`), 1000);
  try {
    const { body: out, headers } = await api(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    say(status, "");
    return { out, headers, ms: Math.round(performance.now() - t0) };
  } finally {
    clearInterval(tick);
    unlock();
  }
}

function answerCards(questions, answers, runModel) {
  const order = [...Object.keys(questions).filter((k) => k in answers), ...Object.keys(answers).filter((k) => !(k in questions))];
  return order.map((k) => answerCard(k, questions[k], answers[k], questions[k]?.model || runModel));
}

function metaChips(out, headers, ms, model) {
  return [
    chip("provider", headers.get("x-jeff-provider") || "?"),
    chip("model", out.model || model),
    chip("round trip", `${ms} ms`),
    ...(headers.get("x-jeff-upstream-ms") ? [chip("upstream", `${headers.get("x-jeff-upstream-ms")} ms`)] : []),
    ...Object.entries(out.usage || {}).map(([k, v]) => chip(k.replaceAll("_", " "), String(v))),
  ];
}

async function run() {
  const status = $("#pg-status");
  const body = runBody();
  if (!body.questions.length) { say(status, "Check at least one question.", "err"); return; }
  try {
    const { out, headers, ms } = await ask(body, status, $("#pg-run"));
    last = out;
    $("#pg-meta").replaceChildren(...metaChips(out, headers, ms, body.model));
    const questions = Object.fromEntries(body.questions.map((k) => [k, saved[k]]));
    $("#view-answer").replaceChildren(...answerCards(questions, out.answers || {}, body.model));
    $("#view-json").textContent = JSON.stringify(out, null, 2);
    updateCurl();
  } catch (e) {
    say(status, e.message, "err");
  }
}
$("#pg-run").onclick = run;
addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey) || e.key !== "Enter") return;
  if (!$("#page-classifiers").hidden) {
    const list = $("#c-form-view").hidden;
    if ($(list ? "#c-run" : "#c-try").disabled) return;
    if (list) runClassifier();
    else tryClassifier();
    return;
  }
  if ($("#page-questions").hidden) return;
  const list = $("#q-form-view").hidden;
  if ($(list ? "#pg-run" : "#q-try").disabled) return;
  if (list) run();
  else tryDraft();
});

/* ---------- saved questions ---------- */

/** The key being edited; null while creating. */
let editing = null;

function saveChecked() {
  store.set("checked", [...checked]);
  $("#q-selected").textContent = `${checkedKeys().length} selected · Ctrl + Enter`;
  updateCurl();
}

function modelChip(model) {
  return el("span", "shrink-0 rounded border border-neutral-300 px-1.5 py-0.5 font-mono text-[11px] text-neutral-700 dark:border-neutral-700 dark:text-neutral-300", model);
}

function questionRow(key, q) {
  const li = el("li", "flex items-start gap-3 p-3");
  const box = el("input", "mt-1 size-4 shrink-0");
  box.type = "checkbox";
  box.checked = checked.has(key);
  box.setAttribute("aria-label", `Run ${key}`);
  box.onchange = () => { if (box.checked) checked.add(key); else checked.delete(key); saveChecked(); };
  const main = el("div", "min-w-0 flex-1");
  const head = el("div", "flex flex-wrap items-center gap-2");
  head.append(el("span", "font-mono font-medium", key));
  const badge = el("span", "badge", TYPE_INFO[q.type]?.badge || q.type);
  badge.dataset.type = q.type;
  head.append(badge);
  if (q.model) head.append(modelChip(q.model));
  main.append(head, el("p", "note mt-0.5 truncate", q.instructions));
  const edit = el("button", "btn min-w-0", "Edit");
  edit.type = "button";
  edit.onclick = () => openForm(key);
  const del = el("button", "btn btn-danger min-w-0", "Delete");
  del.type = "button";
  confirmClick(del, "Confirm delete", async () => {
    try {
      saved = (await api(`/api/questions/${encodeURIComponent(key)}`, { method: "DELETE" })).body;
      renderQuestions();
    } catch (err) {
      say($("#pg-status"), err.message, "err");
    }
  });
  li.append(box, main, edit, del);
  return li;
}

function renderQuestions() {
  const keys = Object.keys(saved);
  $("#q-items").replaceChildren(...keys.map((k) => questionRow(k, saved[k])));
  $("#q-items").hidden = !keys.length;
  $("#q-empty").hidden = keys.length > 0;
  for (const k of [...checked]) if (!(k in saved)) checked.delete(k);
  saveChecked();
  renderClassifiers();
  if (!$("#c-form-view").hidden) renderPicked();
}

async function loadQuestions() {
  saved = (await api("/v1/questions")).body;
  renderQuestions();
}

$("#q-examples").onclick = (e) => busy(e.currentTarget, async () => {
  try {
    const examples = Object.fromEntries(EXAMPLE.questions.map((q) => [q.key, nativeOf(q)]));
    saved = (await api("/api/questions", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(examples) })).body;
    for (const k of Object.keys(examples)) checked.add(k);
    renderQuestions();
  } catch (err) {
    say($("#pg-status"), err.message, "err");
  }
});

/** Offers Default plus every listed model, keeping a saved model that no provider lists right now. */
function fillFormModel(want = $("#q-form-model").value) {
  const sel = $("#q-form-model");
  sel.replaceChildren(new Option("Default (Model picked for the run)", ""), ...(modelList ? modelGroups() : []));
  if (want && ![...sel.options].some((o) => o.value === want)) sel.append(new Option(want, want));
  sel.value = want;
}

function showForm(open) {
  $("#q-list-view").hidden = open;
  $("#q-form-view").hidden = !open;
}

function openForm(key = null) {
  editing = key;
  const q = key ? saved[key] : {};
  $("#q-form-title").textContent = key ? "Edit question" : "New question";
  const card = addQuestion(key ? builderOf(key, q) : { type: "noul", key: "" });
  $(".q-key", card).readOnly = !!key;
  fillFormModel(q.model || "");
  $("#q-try-state").value = $("#pg-state").value;
  $("#q-try-answer").replaceChildren(el("p", "note py-12 text-center", "Try runs the draft against this state. Nothing is saved."));
  say($("#q-form-status"), "");
  showForm(true);
  (key ? $(".q-instr", card) : $(".q-key", card)).focus();
}

/** The form's question as `[key, native question]`, with its model when one is picked. */
function draft() {
  const [[key, form]] = Object.entries(readQuestions());
  // Fields the form does not show, such as newer Jev fields saved through the API, survive an edit.
  const { type, instructions, criteria, model, ...rest } = editing ? saved[editing] : {};
  const q = { ...rest, ...form };
  if ($("#q-form-model").value) q.model = $("#q-form-model").value;
  return [key, q];
}

async function saveForm() {
  const status = $("#q-form-status");
  try {
    const [key, q] = draft();
    const json = { "content-type": "application/json" };
    if (editing) await api(`/api/questions/${encodeURIComponent(editing)}`, { method: "PUT", headers: json, body: JSON.stringify(q) });
    else await api("/api/questions", { method: "POST", headers: json, body: JSON.stringify({ [key]: q }) });
    checked.add(key);
    await loadQuestions();
    showForm(false);
  } catch (err) {
    say(status, err.message, "err");
  }
}

async function tryDraft() {
  const status = $("#q-form-status");
  let key, q;
  try { [key, q] = draft(); }
  catch (err) { say(status, err.message, "err"); return; }
  const body = { model: $("#pg-model").value, state: $("#q-try-state").value, questions: { [key]: q } };
  try {
    const { out } = await ask(body, status, $("#q-try"));
    $("#q-try-answer").replaceChildren(...answerCards(body.questions, out.answers || {}, body.model));
  } catch (err) {
    say(status, err.message, "err");
  }
}

$("#q-new").onclick = () => openForm();
$("#q-empty-new").onclick = () => openForm();
$("#q-back").onclick = () => showForm(false);
$("#q-cancel").onclick = () => showForm(false);
$("#q-save").onclick = (e) => busy(e.currentTarget, saveForm);
$("#q-try").onclick = tryDraft;

/* ---------- classifiers ---------- */

/** The classifier key being edited; null while creating. */
let cEditing = null;
/** The form's question keys, in the order they are asked. */
let picked = [];

const deletedTip = (key) => `Question "${key}" was deleted. The classifier skips it.`;
const missing = (keys) => keys.filter((k) => !(k in saved));
/** A hand-edited config may hold a classifier of another shape; the server keeps it until it is fixed or deleted. */
const keysOf = (c) => (Array.isArray(c?.questions) ? c.questions.filter((k) => typeof k === "string") : []);

/** Saved questions in classifier order, with the override as their model, as `answerCards` takes them. */
function fullQuestions(keys, model) {
  return Object.fromEntries(keys.filter((k) => k in saved).map((k) => [k, model ? { ...saved[k], model } : saved[k]]));
}

function skippedNotice(keys) {
  if (!keys?.length) return null;
  const n = keys.length;
  return el("p", "rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200", `${n} question${n === 1 ? "" : "s"} skipped: ${keys.join(", ")}`);
}

function questionChip(key) {
  if (key in saved) return el("span", "rounded border border-neutral-200 bg-neutral-50 px-1.5 py-0.5 font-mono text-[11px] text-neutral-700 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300", key);
  const c = el("span", "rounded border border-red-300 bg-red-50 px-1.5 py-0.5 font-mono text-[11px] text-red-800 line-through dark:border-red-900 dark:bg-red-950/40 dark:text-red-300", key);
  c.title = deletedTip(key);
  return c;
}

/** The classifier Run calls: the one last picked, else the first. */
function selectedClassifier() {
  const want = store.get("classifier");
  return want in classifiers ? want : Object.keys(classifiers)[0] ?? null;
}

function classifierRow(key, c) {
  const li = el("li", "flex items-start gap-3 p-3");
  const radio = el("input", "mt-1 size-4 shrink-0");
  radio.type = "radio";
  radio.name = "classifier";
  radio.checked = key === selectedClassifier();
  radio.setAttribute("aria-label", `Run ${key}`);
  radio.onchange = () => { store.set("classifier", key); updateClassifierCurl(); };
  const main = el("div", "min-w-0 flex-1");
  const head = el("div", "flex flex-wrap items-center gap-2");
  head.append(el("span", "font-mono font-medium", key));
  if (typeof c.model === "string") head.append(modelChip(c.model));
  const chips = el("div", "mt-1.5 flex flex-wrap gap-1");
  chips.append(...keysOf(c).map(questionChip));
  if (!Array.isArray(c.questions)) {
    const bad = el("span", "rounded border border-red-300 bg-red-50 px-1.5 py-0.5 text-[11px] text-red-800 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300", "invalid");
    bad.title = "questions must be an array of question keys. Edit the classifier to fix it, or delete it.";
    chips.append(bad);
  }
  main.append(head, chips);
  const edit = el("button", "btn min-w-0", "Edit");
  edit.type = "button";
  edit.onclick = () => openClassifierForm(key);
  li.append(radio, main, edit);
  return li;
}

function renderClassifiers() {
  const keys = Object.keys(classifiers);
  const any = Object.keys(saved).length > 0;
  $("#c-items").replaceChildren(...keys.map((k) => classifierRow(k, classifiers[k])));
  $("#c-items").hidden = !keys.length;
  $("#c-empty").hidden = keys.length > 0;
  $("#c-empty-text").textContent = any ? "Group saved questions to run them in one call." : "Save questions first.";
  $("#c-empty-new").hidden = !any;
  $("#c-selected").textContent = keys.length ? "Ctrl + Enter" : "";
  updateClassifierCurl();
}

async function loadClassifiers() {
  classifiers = (await api("/v1/classifiers")).body;
  renderClassifiers();
}

function classifierBody() {
  return { model: $("#c-model").value, state: $("#pg-state").value };
}

function updateClassifierCurl() {
  const key = selectedClassifier();
  $("#c-curl-text").textContent = key ? curlFor(classifierBody(), `/v1/classifiers/${key}`) : "Create a classifier to see its call.";
}
$("#c-curl-copy").onclick = () => navigator.clipboard.writeText($("#c-curl-text").textContent).then(() => flash($("#c-status"), "curl copied"));
$("#c-model").onchange = (e) => { store.set("c-model", e.target.value); updateClassifierCurl(); };

async function runClassifier() {
  const status = $("#c-status");
  const key = selectedClassifier();
  if (!key) { say(status, "Create a classifier first.", "err"); return; }
  const c = classifiers[key];
  const body = classifierBody();
  try {
    const { out, headers, ms } = await ask(body, status, $("#c-run"), `/v1/classifiers/${encodeURIComponent(key)}`);
    $("#c-meta").replaceChildren(...metaChips(out, headers, ms, body.model));
    $("#c-view-answer").replaceChildren(...[skippedNotice(out.skipped)].filter(Boolean), ...answerCards(fullQuestions(keysOf(c), c.model), out.answers || {}, body.model));
    $("#c-view-json").textContent = JSON.stringify(out, null, 2);
  } catch (e) {
    say(status, e.message, "err");
  }
}

/* The form */

function showClassifierForm(open) {
  $("#c-list-view").hidden = open;
  $("#c-form-view").hidden = !open;
}

/** Offers None plus every listed model, keeping a saved override that no provider lists right now. */
function fillClassifierModel(want = $("#c-form-model").value) {
  const sel = $("#c-form-model");
  sel.replaceChildren(new Option("None (each question uses its own)", ""), ...(modelList ? modelGroups() : []));
  if (want && ![...sel.options].some((o) => o.value === want)) sel.append(new Option(want, want));
  sel.value = want;
}

function pickedRow(key) {
  const q = saved[key];
  const li = el("li", q ? "flex items-start gap-2 p-2" : "flex items-start gap-2 bg-red-50 p-2 dark:bg-red-950/30");
  li.draggable = true;
  li.tabIndex = 0;
  li.dataset.key = key;
  if (!q) li.title = deletedTip(key);
  const handle = el("span", "mt-0.5 cursor-grab px-1 text-neutral-400 select-none dark:text-neutral-500", "⋮⋮");
  handle.setAttribute("aria-hidden", "true");
  const main = el("div", "min-w-0 flex-1");
  const head = el("div", "flex flex-wrap items-center gap-2");
  if (q) {
    head.append(el("span", "font-mono font-medium", key));
    const badge = el("span", "badge", TYPE_INFO[q.type]?.badge || q.type);
    badge.dataset.type = q.type;
    head.append(badge);
    if (q.model) {
      const m = modelChip(q.model);
      // The override replaces this model for the classifier's calls.
      if ($("#c-form-model").value) m.classList.add("line-through", "opacity-60");
      head.append(m);
    }
    main.append(head, el("p", "note mt-0.5 truncate", q.instructions));
  } else {
    head.append(el("span", "font-mono font-medium text-red-800 line-through dark:text-red-300", key), el("span", "badge bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-200", "deleted"));
    main.append(head, el("p", "mt-0.5 text-xs text-red-700 dark:text-red-300", "Deleted. The classifier skips it."));
  }
  li.append(handle, main, removeButton(() => {
    const i = picked.indexOf(key);
    picked = picked.filter((k) => k !== key);
    renderPicked();
    ($$("#c-picked li")[Math.min(i, picked.length - 1)] || $("#c-search")).focus();
  }, `Remove ${key}`));
  li.onkeydown = (e) => {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    const i = picked.indexOf(key);
    const j = i + (e.key === "ArrowUp" ? -1 : 1);
    if (j < 0 || j >= picked.length) return;
    [picked[i], picked[j]] = [picked[j], picked[i]];
    renderPicked();
    $(`#c-picked li[data-key="${CSS.escape(key)}"]`).focus();
  };
  return li;
}

function renderPicked() {
  $("#c-picked").replaceChildren(...picked.map(pickedRow));
  $("#c-picked").hidden = !picked.length;
  const gone = missing(picked).length;
  $("#c-count").textContent = picked.length ? `${picked.length}${gone ? ` · ${gone} deleted` : ""}` : "";
  if (!$("#c-options").hidden) renderOptions();
}

let dragKey = null;
$("#c-picked").addEventListener("dragstart", (e) => { dragKey = e.target.closest("li")?.dataset.key ?? null; });
$("#c-picked").addEventListener("dragover", (e) => { if (dragKey) e.preventDefault(); });
$("#c-picked").addEventListener("dragend", () => { dragKey = null; });
$("#c-picked").addEventListener("drop", (e) => {
  e.preventDefault();
  const over = e.target.closest("li")?.dataset.key;
  if (dragKey && over && over !== dragKey) {
    const to = picked.indexOf(over);
    picked = picked.filter((k) => k !== dragKey);
    picked.splice(to, 0, dragKey);
    renderPicked();
  }
  dragKey = null;
});

let optIndex = 0;

/** Saved questions not yet picked whose key or instructions contain the search text. */
function matches() {
  const text = $("#c-search").value.trim().toLowerCase();
  return Object.keys(saved).filter((k) => !picked.includes(k) && (k.toLowerCase().includes(text) || (saved[k].instructions || "").toLowerCase().includes(text)));
}

function renderOptions() {
  const found = matches();
  optIndex = Math.min(optIndex, Math.max(0, found.length - 1));
  const options = found.map((k, i) => {
    const o = el("li", "cursor-pointer px-3 py-2 hover:bg-neutral-50 aria-selected:bg-neutral-100 dark:hover:bg-neutral-800/60 dark:aria-selected:bg-neutral-800");
    o.id = `c-opt-${i}`;
    o.setAttribute("role", "option");
    o.setAttribute("aria-selected", String(i === optIndex));
    const head = el("div", "flex flex-wrap items-center gap-2");
    const badge = el("span", "badge", TYPE_INFO[saved[k].type]?.badge || saved[k].type);
    badge.dataset.type = saved[k].type;
    head.append(el("span", "font-mono font-medium", k), badge);
    o.append(head, el("p", "note mt-0.5 truncate", saved[k].instructions));
    // mousedown runs before the search field's blur would close the list.
    o.onmousedown = (e) => { e.preventDefault(); addPicked(k); };
    return o;
  });
  if (!options.length) options.push(el("li", "note px-3 py-2", Object.keys(saved).length ? "No matching questions." : "No saved questions."));
  $("#c-options").replaceChildren(...options);
  $("#c-options").hidden = false;
  $("#c-search").setAttribute("aria-expanded", "true");
  if (found.length) $("#c-search").setAttribute("aria-activedescendant", `c-opt-${optIndex}`);
  else $("#c-search").removeAttribute("aria-activedescendant");
  $(`#c-opt-${optIndex}`)?.scrollIntoView({ block: "nearest" });
}

function closeOptions() {
  $("#c-options").hidden = true;
  $("#c-search").setAttribute("aria-expanded", "false");
  $("#c-search").removeAttribute("aria-activedescendant");
}

function addPicked(key) {
  picked.push(key);
  $("#c-search").value = "";
  optIndex = 0;
  renderPicked();
  renderOptions();
}

$("#c-search").onfocus = renderOptions;
$("#c-search").oninput = () => { optIndex = 0; renderOptions(); };
$("#c-search").onblur = closeOptions;
$("#c-search").onkeydown = (e) => {
  if (e.key === "Escape") { closeOptions(); return; }
  if ($("#c-options").hidden) { if (e.key === "ArrowDown") renderOptions(); return; }
  const found = matches();
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    optIndex = Math.max(0, Math.min(found.length - 1, optIndex + (e.key === "ArrowDown" ? 1 : -1)));
    renderOptions();
  } else if (e.key === "Enter" && found[optIndex]) {
    e.preventDefault();
    addPicked(found[optIndex]);
  }
};
$("#c-form-model").onchange = renderPicked;

function openClassifierForm(key = null) {
  cEditing = key;
  const c = key ? classifiers[key] : {};
  $("#c-form-title").textContent = key ? "Edit classifier" : "New classifier";
  $("#c-key").value = key || "";
  $("#c-key").readOnly = !!key;
  picked = keysOf(c);
  fillClassifierModel(typeof c.model === "string" ? c.model : "");
  renderPicked();
  $("#c-delete").hidden = !key;
  $("#c-try-state").value = $("#pg-state").value;
  $("#c-try-answer").replaceChildren(el("p", "note py-12 text-center", "Try runs the draft against this state. Nothing is saved."));
  say($("#c-form-status"), "");
  closeOptions();
  showClassifierForm(true);
  (key ? $("#c-search") : $("#c-key")).focus();
}

/** The form's classifier, without its key. */
function classifierDraft() {
  if (!picked.length) throw new Error("Add at least one question.");
  // Fields the form does not show, saved through the API, survive an edit.
  const { questions, model, ...rest } = cEditing ? classifiers[cEditing] : {};
  const c = { ...rest, questions: [...picked] };
  if ($("#c-form-model").value) c.model = $("#c-form-model").value;
  return c;
}

async function saveClassifier() {
  const status = $("#c-form-status");
  try {
    const key = cEditing || $("#c-key").value.trim();
    if (!key) throw new Error("The classifier needs a key.");
    const c = classifierDraft();
    const json = { "content-type": "application/json" };
    if (cEditing) await api(`/api/classifiers/${encodeURIComponent(key)}`, { method: "PUT", headers: json, body: JSON.stringify(c) });
    else await api("/api/classifiers", { method: "POST", headers: json, body: JSON.stringify({ [key]: c }) });
    store.set("classifier", key);
    await loadClassifiers();
    showClassifierForm(false);
  } catch (err) {
    say(status, err.message, "err");
  }
}

async function tryClassifier() {
  const status = $("#c-form-status");
  let c;
  try { c = classifierDraft(); }
  catch (err) { say(status, err.message, "err"); return; }
  const body = { model: $("#c-model").value, state: $("#c-try-state").value, classifier: c };
  try {
    const { out } = await ask(body, status, $("#c-try"), "/v1/classifiers");
    $("#c-try-answer").replaceChildren(...[skippedNotice(out.skipped)].filter(Boolean), ...answerCards(fullQuestions(c.questions, c.model), out.answers || {}, body.model));
  } catch (err) {
    say(status, err.message, "err");
  }
}

confirmClick($("#c-delete"), "Confirm delete", async () => {
  try {
    classifiers = (await api(`/api/classifiers/${encodeURIComponent(cEditing)}`, { method: "DELETE" })).body;
    renderClassifiers();
    showClassifierForm(false);
  } catch (err) {
    say($("#c-form-status"), err.message, "err");
  }
});
$("#c-new").onclick = () => openClassifierForm();
$("#c-empty-new").onclick = () => openClassifierForm();
$("#c-back").onclick = () => showClassifierForm(false);
$("#c-cancel").onclick = () => showClassifierForm(false);
$("#c-save").onclick = (e) => busy(e.currentTarget, saveClassifier);
$("#c-try").onclick = tryClassifier;
$("#c-run").onclick = runClassifier;

setInterval(() => !document.hidden && location.hash === "#providers" && modelList && loadModels().catch(() => {}), 15000);

showTab();
setView(view);
loadQuestions().catch((e) => say($("#pg-status"), e.message, "err"));
loadClassifiers().catch((e) => say($("#c-status"), e.message, "err"));
loadConfig().catch((e) => say($("#pg-status"), e.message, "err"));
