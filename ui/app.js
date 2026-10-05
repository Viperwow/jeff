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

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

const store = {
  get(k) { try { return JSON.parse(localStorage.getItem("jengine." + k)); } catch { return null; } },
  set(k, v) { try { localStorage.setItem("jengine." + k, JSON.stringify(v)); } catch {} },
};

function authHeaders(headers = {}) {
  const key = store.get("key");
  return key ? { ...headers, authorization: `Bearer ${key}` } : headers;
}

async function api(path, opts = {}) {
  const r = await fetch(path, { ...opts, headers: authHeaders(opts.headers) });
  const body = await r.json().catch(() => ({}));
  $("#auth-error").hidden = r.status !== 401;
  if (r.status === 401) $("#auth-error").textContent = body.error || "jengine rejected the access key.";
  if (!r.ok) throw new Error(body.error || body.detail?.message || body.detail || `HTTP ${r.status}`);
  return { body, headers: r.headers };
}

/* ---------- tabs ---------- */

const PAGES = ["playground", "providers", "keys"];

function showTab() {
  const id = PAGES.find((p) => location.hash === "#" + p) || "playground";
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

/** The first click arms the button with a countdown; a second click before it runs out calls `action`. */
function confirmClick(button, armedLabel, action) {
  const label = button.textContent;
  button.onclick = () => {
    if (button.dataset.confirm === "true") { action(); return; }
    button.dataset.confirm = "true";
    let left = CONFIRM_MS / 1000;
    const hint = el("span", "text-xs opacity-70 tabular-nums", `${left} s`);
    button.replaceChildren(armedLabel, hint);
    const tick = setInterval(() => {
      left -= 1;
      if (left > 0 && button.isConnected) { hint.textContent = `${left} s`; return; }
      clearInterval(tick);
      button.dataset.confirm = "false";
      button.replaceChildren(label);
    }, 1000);
  };
}

/* ---------- models ---------- */

function fillModelSelects() {
  const groups = modelList.providers;
  const all = modelList.data.map((m) => m.id);
  const sel = $("#pg-model");
  const keep = sel.value || store.get("model") || config.default_model;
  sel.replaceChildren(...groups.map((p) => {
    const g = el("optgroup");
    g.label = p.ok ? p.name : `${p.name} (${p.warning ? "starting" : "offline"})`;
    for (const m of p.models) g.append(new Option(`${p.id}/${m}`, `${p.id}/${m}`));
    return g;
  }));
  if (!all.includes(config.default_model)) sel.prepend(new Option(config.default_model, config.default_model));
  sel.value = all.includes(keep) ? keep : config.default_model;
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

$("#refresh").onclick = () => loadModels().catch((e) => say($("#save-status"), e.message, "err"));

$("#save").onclick = async () => {
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
    say($("#save-status"), "Saved", "ok");
    await loadConfig();
  } catch (e) {
    say($("#save-status"), e.message, "err");
  }
};

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
  const t = (await api("/api/clm")).body;
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
    ? "Calls the API and manages jengine on this page."
    : "Calls /v1 only: decisions and models.", "");
}

$("#key-role").addEventListener("change", syncRole);
syncRole();

$("#key-date").min = dateValue(0);
$("#key-date").value = dateValue(30);
$("#key-date").addEventListener("input", syncExpiry);
$("#key-exp").addEventListener("change", syncExpiry);
syncExpiry();

$("#key-form").onsubmit = async (e) => {
  e.preventDefault();
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
    // The first key locks jengine; keep it here so this page does not lock its own operator out.
    const kept = !store.get("key") && body.role === "admin";
    if (kept) store.set("key", body.key);
    $("#key-saved-note").textContent = kept ? "This browser now uses this key for the admin page." : "";
    $("#key-name").value = "";
    say($("#key-status"), "Key created", "ok");
    await refreshInfo();
    await loadKeys();
  } catch (err) {
    say($("#key-status"), err.message, "err");
  }
};

$("#key-copy").onclick = () => navigator.clipboard.writeText($("#key-value").value).then(() => say($("#key-status"), "Key copied", "ok"));

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
  revoke.title = last ? "Last key: jengine will accept requests without a key." : "Clients using this key stop working at once.";
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
  if (!body.keys.length) $("#key-list").append(el("li", "note py-3", "No keys yet. Without keys, jengine accepts every request."));
  $("#key-static").hidden = !body.static_keys;
  $("#key-static").textContent = `Plus ${body.static_keys} key${body.static_keys === 1 ? "" : "s"} from --api-key or JENGINE_API_KEY. Those never expire and are changed where jengine is started.`;
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
  row.append(removeButton(() => { row.remove(); renumber(card); saveDraft(); }, type === "choice" ? "Remove option" : "Remove level"));
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
  $(".q-type", card).onchange = (e) => { setType(card, e.target.value); saveDraft(); };
  $(".q-add", card).onclick = () => { addRow(card, card.dataset.type); saveDraft(); };
  $(".q-del", card).innerHTML = X_ICON;
  $(".q-del", card).onclick = () => { card.remove(); saveDraft(); };
  setType(card, q.type || "noul", q.rows);
  $("#pg-questions").append(card);
  return card;
}

for (const b of $$("[data-add]")) {
  b.onclick = () => {
    const n = $$("#pg-questions .q").length + 1;
    $(".q-key", addQuestion({ type: b.dataset.add, key: `q${n}` })).focus();
    saveDraft();
  };
}

function readQuestions() {
  const out = {};
  for (const card of $$("#pg-questions .q")) {
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
  if (!Object.keys(out).length) throw new Error("Add at least one question.");
  return out;
}

function snapshotQuestions() {
  return $$("#pg-questions .q").map((card) => ({
    key: $(".q-key", card).value,
    type: card.dataset.type,
    instructions: $(".q-instr", card).value,
    rows: card.dataset.type === "choice"
      ? $$(".q-row", card).map((r) => [$(".q-opt-key", r).value, $(".q-opt-desc", r).value])
      : $$(".q-level", card).map((i) => [i.value]),
  }));
}

function saveDraft() {
  store.set("draft", { state: $("#pg-state").value, questions: snapshotQuestions() });
  $("#pg-count").textContent = `${$("#pg-state").value.length} chars`;
}

const EXAMPLE = {
  state: "Customer: my invoice was charged twice and nobody answers the phone! I need this fixed today.",
  questions: [
    { key: "urgency", type: "noul", instructions: "The message conveys urgency or time pressure." },
    { key: "department", type: "choice", instructions: "Which team should handle this?", rows: [["billing", "Charges, invoices, refunds"], ["technical", "Bugs and outages"], ["sales", "Pricing and plans"]] },
    { key: "frustration", type: "score", instructions: "How frustrated is the customer?", rows: [["Calm"], ["Frustrated"], ["Very angry"]] },
  ],
};

function loadDraft() {
  const d = store.get("draft") || EXAMPLE;
  $("#pg-state").value = d.state;
  for (const q of d.questions) addQuestion(q);
  saveDraft();
}

$("#pg-state").addEventListener("input", saveDraft);
$("#pg-questions").addEventListener("input", saveDraft);
$("#pg-model").onchange = (e) => store.set("model", e.target.value);

/* ---------- answers ---------- */

let view = store.get("view") || "answer";
let last = null;

function setView(v) {
  view = v;
  store.set("view", v);
  for (const t of $$(".view-tab")) t.setAttribute("aria-selected", String(t.dataset.view === v));
  $("#view-answer").hidden = v !== "answer";
  $("#view-json").hidden = v !== "json";
  $("#view-curl").hidden = v !== "curl";
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

function answerCard(key, q, a) {
  const card = el("div", "card space-y-3");
  const head = el("div", "flex flex-wrap items-center gap-2");
  head.append(el("span", "font-mono font-semibold", key));
  const badge = el("span", "badge", TYPE_INFO[a.type]?.badge || a.type);
  badge.dataset.type = a.type;
  head.append(badge);
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

function curlFor(body) {
  const json = JSON.stringify(body, null, 2).replace(/'/g, "'\\''");
  const auth = info?.api_key_set ? `  -H "authorization: Bearer $JENGINE_API_KEY" \\\n` : "";
  return `curl ${apiOrigin()}/v1/systemone \\\n${auth}  -H 'content-type: application/json' \\\n  -d '${json}'`;
}

function updateCurl() {
  try {
    $("#curl-text").textContent = curlFor({ model: $("#pg-model").value, state: $("#pg-state").value, questions: readQuestions() });
  } catch (e) {
    $("#curl-text").textContent = e.message;
  }
}
$("#curl-copy").onclick = () => navigator.clipboard.writeText($("#curl-text").textContent).then(() => say($("#pg-status"), "curl copied", "ok"));
for (const t of $$('.view-tab[data-view="curl"]')) t.addEventListener("click", updateCurl);

async function run() {
  const status = $("#pg-status");
  let questions;
  try { questions = readQuestions(); }
  catch (e) { say(status, e.message, "err"); return; }
  const body = { model: $("#pg-model").value, state: $("#pg-state").value, questions };
  $("#pg-run").disabled = true;
  const t0 = performance.now();
  say(status, "Running…");
  const tick = setInterval(() => say(status, `Running… ${Math.floor((performance.now() - t0) / 1000)} s`), 1000);
  try {
    const { body: out, headers } = await api("/v1/systemone", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const ms = Math.round(performance.now() - t0);
    last = out;
    $("#pg-meta").replaceChildren(
      chip("provider", headers.get("x-jengine-provider") || "?"),
      chip("model", out.model || body.model),
      chip("round trip", `${ms} ms`),
      ...(headers.get("x-jengine-upstream-ms") ? [chip("upstream", `${headers.get("x-jengine-upstream-ms")} ms`)] : []),
      ...Object.entries(out.usage || {}).map(([k, v]) => chip(k.replaceAll("_", " "), String(v))),
    );
    const answers = out.answers || {};
    const order = [...Object.keys(questions).filter((k) => k in answers), ...Object.keys(answers).filter((k) => !(k in questions))];
    $("#view-answer").replaceChildren(...order.map((k) => answerCard(k, questions[k], answers[k])));
    $("#view-json").textContent = JSON.stringify(out, null, 2);
    updateCurl();
    say(status, "");
  } catch (e) {
    say(status, e.message, "err");
  } finally {
    clearInterval(tick);
    $("#pg-run").disabled = false;
  }
}
$("#pg-run").onclick = run;
addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === "Enter" && location.hash !== "#providers") run();
});

setInterval(() => !document.hidden && location.hash === "#providers" && modelList && loadModels().catch(() => {}), 15000);

showTab();
setView(view);
loadDraft();
loadConfig().catch((e) => say($("#pg-status"), e.message, "err"));
