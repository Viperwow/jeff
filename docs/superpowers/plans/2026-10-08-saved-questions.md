# Saved Questions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Save Jev questions in `jeff.json`, reuse them by key in `/v1/systemone` with per-question model routing, and manage them on a Questions page that replaces Playground.

**Architecture:** A new `src/questions.rs` holds pure logic: validation, key expansion, grouping by model, answer merging. `src/main.rs` adds the `questions` config field, four thin handlers and a fan-out path in `systemone`. The UI keeps its single `ui/index.html` + `ui/app.js`, swapping the Playground page for a Questions page with a list view and a form view.

**Tech Stack:** Rust (axum, serde_json, tokio, futures_util `join_all`), vanilla JS, Tailwind CSS v4.

**Spec:** `docs/superpowers/specs/2026-10-08-saved-questions-design.md`

## Global Constraints

- Question key: `^[A-Za-z0-9_-]{1,64}$`.
- `type`: `noul`, `choice` or `score`. `instructions`: non-empty string. `choice.criteria`: object, ≥ 2 entries. `score.criteria`: array, ≥ 2 entries. `model`: non-empty string when present.
- Unknown question fields pass through unchanged.
- Model precedence: question `model`, then request `model`, then `default_model`. `model` is stripped before forwarding.
- Errors use the existing `error(status, msg)` JSON shape `{"error": "..."}`.
- Reads (`GET /v1/questions*`) need a client key. Writes (`POST /api/questions`, `PUT /api/questions/{key}`, `DELETE /api/questions/{key}`) need an admin key.
- CI gates: `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`.
- UI CSS is rebuilt with `pnpm --dir ui build` and `ui/dist/app.css` is committed.
- Commits follow Conventional Commits. No AI attribution trailers.

## Review Focus

1. `questions` is an empty array `[]` or empty map `{}` in `/v1/systemone`: expect `400 "questions must not be empty"`, no provider call. Test in Task 3.
2. A saved key repeated in the array, `["urgency", "urgency"]`: expect one question, no error. Test in Task 3.
3. A saved question names a model whose provider was later removed: expect `400` naming that model, as for an unknown request model. Covered by the resolve step in Task 4; manual check in Task 4.
4. `POST /api/questions` with a key containing `/` or a space: expect `400` naming the key, nothing saved. Test in Task 1.
5. Saving providers on the Providers page after questions exist: questions stay. Test in Task 1.

---

### Task 1: Question model and validation

**Files:**
- Create: `src/questions.rs`
- Modify: `src/main.rs` (`mod questions;`, `Config` at line 45, `Config::default` at line 53, `build_config` at line 753)
- Test: `src/questions.rs` (`#[cfg(test)] mod tests`), `src/main.rs` existing `mod tests`

**Interfaces:**
- Produces:
  - `pub type Questions = serde_json::Map<String, Value>;`
  - `pub fn validate(key: &str, q: &Value) -> Result<(), String>` — error text starts with `question '{key}': `.
  - `pub fn validate_all(input: &Questions) -> Result<(), String>` — fails on empty input with `"no questions given"`, else the first `validate` error.
  - `Config.questions: questions::Questions` with `#[serde(default)]`.

- [ ] **Step 1: Write the failing tests in `src/questions.rs`**

```rust
#[test]
fn accepts_native_questions() {
    assert!(validate("urgency", &json!({"type": "noul", "instructions": "Urgent?"})).is_ok());
    assert!(validate("dept", &json!({"type": "choice", "instructions": "Team?",
        "criteria": {"a": "A", "b": "B"}, "model": "clm/clm-latest", "extra": 1})).is_ok());
    assert!(validate("anger", &json!({"type": "score", "instructions": "How?",
        "criteria": ["Calm", "Angry"]})).is_ok());
}

#[test]
fn rejects_broken_questions() {
    let bad = [
        ("a/b", json!({"type": "noul", "instructions": "x"})),
        ("a b", json!({"type": "noul", "instructions": "x"})),
        (&"k".repeat(65), json!({"type": "noul", "instructions": "x"})),
        ("k", json!({"type": "maybe", "instructions": "x"})),
        ("k", json!({"type": "noul", "instructions": " "})),
        ("k", json!({"type": "choice", "instructions": "x", "criteria": {"a": "A"}})),
        ("k", json!({"type": "choice", "instructions": "x", "criteria": ["a", "b"]})),
        ("k", json!({"type": "score", "instructions": "x", "criteria": ["one"]})),
        ("k", json!({"type": "noul", "instructions": "x", "model": ""})),
        ("k", json!("not an object")),
    ];
    for (key, q) in bad {
        let e = validate(key, &q).unwrap_err();
        assert!(e.starts_with(&format!("question '{key}': ")), "{e}");
    }
}

#[test]
fn validate_all_rejects_empty_input() {
    assert_eq!(validate_all(&Questions::new()).unwrap_err(), "no questions given");
}
```

In `src/main.rs` `mod tests`, add `build_config_keeps_questions`: build a `Config::default()` with one question in `questions`, call `build_config` with a `ConfigInput` holding the default providers and `default_model`, assert the result's `questions` equals the input config's.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked questions`
Expected: compile error, `validate` / `questions` not found.

- [ ] **Step 3: Implement `validate` and `validate_all` in `src/questions.rs`; add `questions` to `Config`, `Config::default` (empty map) and `build_config` (`questions: current.questions.clone()`)**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked`
Expected: PASS, all tests.

- [ ] **Step 5: Commit**

```bash
git add src/questions.rs src/main.rs
git commit -m "feat: validate saved questions in the config"
```

### Task 2: Questions API

**Files:**
- Modify: `src/main.rs` (handlers near `list_keys` at line 668; routes at lines 1229-1241)
- Modify: `README.md` (endpoint table at line 117)

**Interfaces:**
- Consumes: `questions::validate_all`, `Config.questions`.
- Produces:
  - `GET /v1/questions` → `200` with the saved map.
  - `GET /v1/questions/{key}` → `200` with one question, or `404 "unknown question '{key}'"`.
  - `POST /api/questions` (body: map) → `201` with the full saved map; `400` from `validate_all`; `409 "question '{key}' already exists"`, nothing saved.
  - `PUT /api/questions/{key}` (body: one question) → `200` with the question; `400` from `validate`; `404 "unknown question '{key}'"`.
  - `DELETE /api/questions/{key}` → `200` with the full saved map, or `404 "unknown question '{key}'"`.

- [ ] **Step 1: Implement `list_questions`, `get_question`, `create_questions`, `update_question`, `delete_question` in `src/main.rs`**

Writes follow `revoke_key`: take `app.config.write()`, clone, change `questions`, `app.save(&next)`, then replace. A save failure returns `500` and keeps memory unchanged.

- [ ] **Step 2: Register routes**

`/v1/questions` and `/v1/questions/{key}` on the `api` router (before `route_layer(require_client)`). `POST /api/questions`, `PUT /api/questions/{key}` and `DELETE /api/questions/{key}` on the `manage` router.

- [ ] **Step 3: Add the five endpoints to the README endpoint table**

One row each, same style as the existing rows.

- [ ] **Step 4: Verify by hand**

Run `cargo run -- serve` with a scratch config (`--config` pointing into the scratchpad), then:

```bash
curl -s -X PUT localhost:<ui-port>/api/questions -H 'content-type: application/json' \
  -d '{"urgency":{"type":"noul","instructions":"Urgent?"}}'
curl -s localhost:<api-port>/v1/questions/urgency
curl -s -X POST localhost:<ui-port>/api/questions -H 'content-type: application/json' -d '{"a b":{"type":"noul","instructions":"x"}}'
curl -s -X DELETE localhost:<ui-port>/api/questions/urgency
curl -s -o /dev/null -w '%{http_code}\n' localhost:<api-port>/v1/questions/urgency
```

Expected: `201` map with `urgency`; `409 question 'urgency' already exists`; the updated question; the updated question; `{"error":"question 'a b': ..."}`; `{}`; `404`. The config file holds the same map after the first call.

- [ ] **Step 5: Run CI gates and commit**

Run: `cargo fmt --all --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`
Expected: clean.

```bash
git add src/main.rs README.md
git commit -m "feat: add the saved questions API"
```

### Task 3: Expansion, grouping and merging

**Files:**
- Modify: `src/questions.rs`
- Test: `src/questions.rs` `mod tests`

**Interfaces:**
- Produces:
  - `pub fn expand(input: &Value, saved: &Questions) -> Result<Questions, String>` — map passes through; array of strings becomes the saved entries; unknown key → `"unknown question '{key}'"`; empty map or array → `"questions must not be empty"`; any other shape → `"questions must be a map or an array of saved keys"`.
  - `pub fn group(questions: Questions, request_model: &str) -> Vec<(String, Questions)>` — one entry per model, in order of first appearance; `model` removed from every question.
  - `pub fn merge(parts: Vec<Value>) -> Value` — rules from the spec's "Running questions" section: union of `answers`, `usage` numeric fields summed, `model` comma-joined, other fields from `parts[0]`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn expands_saved_keys() {
    let saved = json!({"u": {"type": "noul", "instructions": "x"}}).as_object().unwrap().clone();
    assert_eq!(expand(&json!(["u", "u"]), &saved).unwrap(), saved);
    assert_eq!(expand(&json!(["nope"]), &saved).unwrap_err(), "unknown question 'nope'");
    assert_eq!(expand(&json!([]), &saved).unwrap_err(), "questions must not be empty");
    assert_eq!(expand(&json!({}), &saved).unwrap_err(), "questions must not be empty");
    assert!(expand(&json!("u"), &saved).is_err());
    let inline = json!({"i": {"type": "noul", "instructions": "y"}});
    assert_eq!(Value::Object(expand(&inline, &saved).unwrap()), inline);
}

#[test]
fn groups_by_model_and_strips_it() {
    let qs = json!({
        "a": {"type": "noul", "instructions": "x"},
        "b": {"type": "noul", "instructions": "x", "model": "typesafe/jev-latest"},
        "c": {"type": "noul", "instructions": "x", "model": "clm/clm-latest"},
    }).as_object().unwrap().clone();
    let groups = group(qs, "clm/clm-latest");
    let names: Vec<_> = groups.iter().map(|(m, q)| (m.as_str(), q.keys().cloned().collect::<Vec<_>>())).collect();
    assert_eq!(names, [("clm/clm-latest", vec!["a".to_owned(), "c".to_owned()]),
                       ("typesafe/jev-latest", vec!["b".to_owned()])]);
    assert!(groups.iter().all(|(_, q)| q.values().all(|v| v.get("model").is_none())));
}

#[test]
fn merges_group_answers() {
    let merged = merge(vec![
        json!({"model": "clm-latest", "answers": {"a": {"noul": 0.9}}, "usage": {"input_tokens": 10}, "id": "x"}),
        json!({"model": "jev-latest", "answers": {"b": {"noul": 0.1}}, "usage": {"input_tokens": 5}}),
    ]);
    assert_eq!(merged, json!({"model": "clm-latest, jev-latest", "id": "x",
        "answers": {"a": {"noul": 0.9}, "b": {"noul": 0.1}}, "usage": {"input_tokens": 15}}));
}
```

Group order: `serde_json::Map` without `preserve_order` iterates keys sorted, so "first appearance" means sorted key order. The test above relies on that.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked questions`
Expected: compile error, `expand` / `group` / `merge` not found.

- [ ] **Step 3: Implement `expand`, `group`, `merge`**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/questions.rs
git commit -m "feat: expand, group and merge saved questions"
```

### Task 4: Route questions per model in `/v1/systemone`

**Files:**
- Modify: `src/main.rs` (`systemone` at line 335)

**Interfaces:**
- Consumes: `questions::{expand, group, merge}`, `resolve`, `refresh_models`, `request`, `read_capped`.
- Produces: `async fn forward(app: &App, config: &Config, model: &str, body: Value) -> Result<(String, u64, StatusCode, Vec<u8>), Response>` — returns provider id, upstream ms, status and body bytes; `Err` carries the ready error response (unknown model `400`, unreachable `502`, unreadable `502`), using today's messages.

- [ ] **Step 1: Extract `forward` from today's `systemone` body without changing behaviour**

Run: `cargo test --locked` — PASS.

- [ ] **Step 2: Rewrite `systemone`**

1. Request model = `body.model` or `config.default_model`.
2. If `body.questions` is present: `expand` against `config.questions` (`400` on error), then `group`.
3. One group: set `questions` and `model` in the body, `forward`, respond as today.
4. Several groups: `join_all` over `forward` with a cloned body per group. The first `Err` or non-2xx status is returned as is, with its `x-jeff-provider`. Otherwise parse each body as JSON, `merge`, respond `200` with `x-jeff-provider` = provider ids joined by `, ` and `x-jeff-upstream-ms` = the max.
5. A body without `questions` keeps today's path (provider validates it).

- [ ] **Step 3: Verify by hand against CLM (or any two providers)**

Save `urgency` without a model and `department` with `"model": "typesafe/jev-latest"`, then post `{"state": "...", "questions": ["urgency", "department"]}`.
Expected: `200`, both answers, `x-jeff-provider: clm, typesafe`. With TypeSafe unreachable: `502` naming `typesafe`. With a saved model `gone/x`: `400 unknown model 'gone/x'`.

- [ ] **Step 4: Run CI gates and commit**

Run: `cargo fmt --all --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`

```bash
git add src/main.rs
git commit -m "feat: route saved questions to their own models"
```

### Task 5: Questions page, list view

**Files:**
- Modify: `ui/index.html` (nav at line 13, `#page-playground` at lines 21-75), `ui/app.js` (`PAGES` at line 47, question builder 464-601, `run` 698-737), `ui/app.css`, `ui/dist/app.css`

**Interfaces:**
- Consumes: `GET /v1/questions`, `POST /api/questions`, `PUT /api/questions/{key}`, `DELETE /api/questions/{key}`, `/v1/systemone` with an array.
- Produces (for Task 6): `async function loadQuestions()` refreshes the list from the server; `saved` holds the last map; `#page-questions` contains `#q-list` (list view) and `#q-form` (form view, hidden).

- [ ] **Step 1: Style selects in `ui/app.css`**

`select.control`: `appearance-none bg-no-repeat pr-8`, a neutral-500 chevron SVG as `background-image` at `right 0.5rem center`, size `1rem`. Same rule as the v3 mockup.

- [ ] **Step 2: Replace Playground with Questions in `ui/index.html`**

Nav link `#questions` labelled "Questions"; `PAGES = ["questions", "providers", "keys"]`, default `questions`. Left panel: header "Questions" + "New question", `<ul id="q-items">`, empty state ("No saved questions", New question, Add examples), Model (note "For questions without their own model."), State, sticky Run bar with "N selected". Right panel: Answer / JSON / curl, unchanged.

- [ ] **Step 3: Render the list and run in `ui/app.js`**

Each row: checkbox, key (mono), type badge, model chip when set, instructions (truncated), Edit, Delete. Delete uses the two-click confirm already used by Revoke. Checked keys persist in `localStorage` under `checked`; the question draft (`draft`) goes away, State and Model stay. Run posts `{model, state, questions: [checked keys]}` and shows answers ordered by the checked list; `answerCard` gets the question from `saved`. curl shows the same array body. Add examples POSTs the three `EXAMPLE` questions converted to native shape.

- [ ] **Step 4: Rebuild CSS and check by hand**

Run: `pnpm --dir ui build`, then `cargo run -- serve` and open the UI.
Expected: empty state → Add examples → three rows; Run with two checked → two answer cards; Delete twice → row gone; selects show one chevron in light and dark mode.

- [ ] **Step 5: Commit**

```bash
git add ui/index.html ui/app.js ui/app.css ui/dist/app.css
git commit -m "feat(ui): replace Playground with a saved questions list"
```

### Task 6: Questions page, form view with Try

**Files:**
- Modify: `ui/index.html`, `ui/app.js`, `ui/dist/app.css`

**Interfaces:**
- Consumes: `loadQuestions()`, `saved`, today's `addQuestion` / `readQuestions` builder, `answerCard`.

- [ ] **Step 1: Add the form view markup**

Left: "← Questions" link, title "New question" / "Edit question", one builder card (`#tpl-question`, without the remove button), Model select whose first option is `Default` (empty value) followed by the `/v1/models` list, footer with Save and Cancel left and Try right (`ml-auto`). Right: State textarea and the Try answer.

- [ ] **Step 2: Wire it in `ui/app.js`**

New question opens an empty form. Edit opens the form filled from `saved[key]`, key input `readOnly`. Save: `readQuestions()` on the one card, add `model` when not Default, new: `POST /api/questions` with `{[key]: question}`, edit: `PUT /api/questions/{key}` with the question, then `loadQuestions()` and back to the list; on Edit, the new question stays checked. Cancel returns without saving. Try posts the draft as an inline map with the form State; result shows in the right panel; nothing saved. API errors go to the footer status line.

- [ ] **Step 3: Rebuild CSS and check by hand**

Run: `pnpm --dir ui build`, `cargo run -- serve`.
Expected: create `department` with model `typesafe/jev-latest` → row shows the model chip; Try before Save shows an answer and the list is unchanged; Edit keeps the key read-only; Save with one option shows "needs at least two options"; Run with `urgency` + `department` shows both providers in the provider chip.

- [ ] **Step 4: Commit**

```bash
git add ui/index.html ui/app.js ui/dist/app.css
git commit -m "feat(ui): create and edit saved questions"
```
