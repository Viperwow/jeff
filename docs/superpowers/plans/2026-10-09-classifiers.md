# Classifiers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Save named groups of question keys as classifiers, manage them through API, CLI and a UI tab, and run them by key with an optional model override.

**Architecture:** A new `src/classifiers.rs` mirrors `src/questions.rs`: validation, create/update/remove and `resolve`, which turns a classifier into the `questions` input that `/v1/systemone` already understands plus a `skipped` list. `main.rs` stores `classifiers` in `Config`, extracts the body of `systemone` into a reusable `answer`, and adds the routes, the CLI and `ask --classifier`. The UI adds a Classifiers page built from the Questions page markup and reuses `answerCards()`.

**Tech Stack:** Rust (axum, serde_json, clap, tokio), vanilla JS, Tailwind 4 (`pnpm -C ui build`).

**Spec:** `docs/superpowers/specs/2026-10-09-classifiers-design.md`

## Global Constraints

- Existing configs load unchanged: `classifiers` reads with `#[serde(default)]`; no migration.
- A classifier key follows the question key rules: 1-64 characters of A-Z, a-z, 0-9, `_` and `-`.
- `questions` is a non-empty array of strings without duplicates; order is kept.
- `model`, when present, is a non-empty string. It overrides each question's model for the call only; saved questions never change.
- Saving never checks that question keys exist.
- Missing or invalid saved questions go to `skipped`; with none left the call answers 400 `classifier '<key>' has no working questions`.
- `skipped` appears in a call response only when non-empty.
- Unknown classifier: 404 `unknown classifier '<key>'`.
- Access: `/v1/classifiers*` needs a client key, `/api/classifiers*` an admin key.
- UI answers render through the existing `answerCards()`; no new answer markup.
- No new dependencies, Rust or JS.

## Review Focus

1. A provider save from the Providers page (`PUT /api/config`) must keep classifiers, as it keeps questions. Test: `build_config_keeps_classifiers` (Task 2).
2. A call body that carries its own `questions` must not reach the provider: the classifier's questions replace it. Test: `call_body_replaces_client_questions` (Task 2).
3. `skipped` must not be written into an error response or a body that is not a JSON object. Test: `with_skipped_leaves_errors_and_non_objects` (Task 1).
4. A hand-edited config with an invalid classifier: `serve` refuses it at start like an invalid question, a live reload only warns. Test: `load_config_rejects_invalid_classifiers` (Task 2).
5. A classifier listing the same key twice is refused on save. Test: inside `rejects_broken_classifiers` (Task 1).

---

### Task 1: `classifiers` module

**Files:**
- Create: `src/classifiers.rs`
- Modify: `src/questions.rs` (expose the key check), `src/main.rs:3` (`mod classifiers;`)

**Interfaces:**
- Consumes: `questions::Questions`, `questions::Change`, `questions::validate`.
- Produces:
  - `questions::check_key(key: &str) -> Result<(), &'static str>`: the key rule moved out of `questions::check`, same message.
  - `pub type Classifiers = serde_json::Map<String, Value>;`
  - `pub fn validate(key: &str, c: &Value) -> Result<(), String>`: messages prefixed `classifier '<key>': `.
  - `pub fn create(saved: &mut Classifiers, input: Classifiers) -> Result<(), Change>`: empty input is `Invalid("no classifiers given")`; existing key is `Exists("classifier '<key>' already exists")`.
  - `pub fn update(saved: &mut Classifiers, key: &str, c: Value) -> Result<(), Change>`
  - `pub fn remove(saved: &mut Classifiers, key: &str) -> Result<(), Change>`
  - `pub fn skipped(c: &Value, saved: &Questions) -> Vec<String>`: keys missing from `saved` or failing `questions::validate`, in classifier order.
  - `pub fn resolve(key: &str, c: &Value, saved: &Questions) -> Result<(Value, Vec<String>), String>`: `(questions_input, skipped)`.
  - `pub fn with_skipped(bytes: Vec<u8>, skipped: &[String]) -> Vec<u8>`

- [ ] **Step 1: Write the failing tests** in `src/classifiers.rs` `mod tests`, using a `map(json!(...))` helper like `questions.rs`:

```rust
#[test]
fn rejects_broken_classifiers() {
    for (key, c) in [
        ("a b", json!({"questions": ["u"]})),
        ("k", json!({"questions": []})),
        ("k", json!({"questions": "u"})),
        ("k", json!({"questions": ["u", 1]})),
        ("k", json!({"questions": ["u", "u"]})),
        ("k", json!({"questions": ["u"], "model": ""})),
        ("k", json!({"questions": ["u"], "model": 3})),
        ("k", json!(["u"])),
    ] {
        assert!(validate(key, &c).is_err(), "{key} {c}");
    }
    assert!(validate("k", &json!({"questions": ["missing"], "model": "p/m"})).is_ok());
}

#[test]
fn resolve_without_model_names_saved_keys() {
    let saved = qs(json!({"a": NOUL, "b": NOUL}));
    let (input, skipped) = resolve("c", &json!({"questions": ["b", "a"]}), &saved).unwrap();
    assert_eq!(input, json!(["b", "a"]));
    assert!(skipped.is_empty());
}

#[test]
fn resolve_with_model_overrides_each_question() {
    let saved = qs(json!({"a": {"type": "noul", "instructions": "x", "model": "p/own"}}));
    let (input, _) = resolve("c", &json!({"questions": ["a"], "model": "p/m"}), &saved).unwrap();
    assert_eq!(input, json!({"a": {"model": "p/m"}}));
    assert_eq!(saved["a"]["model"], "p/own");
}

#[test]
fn resolve_skips_missing_and_invalid() {
    let saved = qs(json!({"a": NOUL, "bad": {"type": "maybe", "instructions": "x"}}));
    let (input, skipped) = resolve("c", &json!({"questions": ["gone", "a", "bad"]}), &saved).unwrap();
    assert_eq!(input, json!(["a"]));
    assert_eq!(skipped, ["gone", "bad"]);
}

#[test]
fn resolve_needs_one_working_question() {
    let e = resolve("c", &json!({"questions": ["gone"]}), &Questions::new()).unwrap_err();
    assert_eq!(e, "classifier 'c' has no working questions");
}

#[test]
fn with_skipped_adds_the_list() {
    let out = with_skipped(br#"{"answers":{}}"#.to_vec(), &["s".into()]);
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), json!({"answers": {}, "skipped": ["s"]}));
}

#[test]
fn with_skipped_leaves_errors_and_non_objects() {
    for body in [&b"[1]"[..], b"not json"] {
        assert_eq!(with_skipped(body.to_vec(), &["s".into()]), body);
    }
    assert_eq!(with_skipped(b"{}".to_vec(), &[]), b"{}");
}

#[test]
fn create_update_remove() {
    let mut saved = Classifiers::new();
    let c = json!({"questions": ["u"]});
    create(&mut saved, cs(json!({"k": c}))).unwrap();
    assert!(matches!(create(&mut saved, cs(json!({"k": c}))), Err(Change::Exists(_))));
    assert!(matches!(create(&mut saved, Classifiers::new()), Err(Change::Invalid(_))));
    assert!(matches!(update(&mut saved, "x", c.clone()), Err(Change::NotFound(_))));
    assert!(matches!(update(&mut saved, "k", json!({"questions": []})), Err(Change::Invalid(_))));
    update(&mut saved, "k", json!({"questions": ["v"], "model": "p/m"})).unwrap();
    assert_eq!(saved["k"]["model"], "p/m");
    remove(&mut saved, "k").unwrap();
    assert_eq!(remove(&mut saved, "k"), Err(Change::NotFound("unknown classifier 'k'".into())));
}
```

`NOUL` is `json!({"type": "noul", "instructions": "x"})` inlined or a `fn noul() -> Value`; `qs` / `cs` turn a `json!` object into the map type.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test classifiers::`
Expected: FAIL to compile (functions not defined).

- [ ] **Step 3: Implement** the Interfaces above. `with_skipped` returns `bytes` unchanged when `skipped` is empty or the body is not a JSON object. `resolve` validates the classifier first and returns that error as is.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS, including the existing `questions::` tests after the `check_key` move.

- [ ] **Step 5: Commit**

```bash
git add src/classifiers.rs src/questions.rs src/main.rs
git commit -m "feat: add classifiers module"
```

### Task 2: Storage and HTTP API

**Files:**
- Modify: `src/main.rs` (`Config`, `load_config`, `App::refresh_from_disk`, `build_config`, `systemone`, `change_questions`, router at `serve`)

**Interfaces:**
- Consumes: everything Task 1 produces.
- Produces:
  - `Config.classifiers: classifiers::Classifiers` with `#[serde(default)]`; `Config::default()` sets it empty.
  - `async fn answer(app: &App, config: &Config, body: Value) -> Result<Upstream, Response>`: today's `systemone` body after the config clone. `systemone` becomes `answer(...)` then `Upstream::reply`, or the error `Response`.
  - `fn call_body(body: Value, questions: Value) -> Result<Value, String>`: body must be a JSON object (`"body must be a JSON object"`); sets `questions`, overwriting any client value.
  - `async fn change_config(app: &App, change: impl FnOnce(&mut Config) -> Result<(), questions::Change>) -> Result<Config, Box<Response>>`: today's `change_questions` over the whole config. Question handlers call it with `|c| questions::create(&mut c.questions, input)` and so on.
  - Handlers: `list_classifiers`, `get_classifier`, `call_classifier`, `create_classifiers`, `update_classifier`, `delete_classifier`, with the same status codes as their question counterparts.

- [ ] **Step 1: Write the failing tests** in `main.rs` `mod tests`:

```rust
#[test]
fn build_config_keeps_classifiers() {
    // as build_config_keeps_questions, with
    // current.classifiers.insert("triage".into(), json!({"questions": ["u"]}));
    // assert_eq!(next.classifiers, current.classifiers);
}

#[test]
fn call_body_replaces_client_questions() {
    let body = json!({"state": "s", "model": "p/m", "questions": {"x": {"type": "noul", "instructions": "y"}}});
    assert_eq!(
        call_body(body, json!(["a"])).unwrap(),
        json!({"state": "s", "model": "p/m", "questions": ["a"]})
    );
    assert!(call_body(json!([1]), json!(["a"])).is_err());
}

#[test]
fn load_config_rejects_invalid_classifiers() {
    // as load_config_rejects_invalid_questions, with
    // config["classifiers"] = json!({"k": {"questions": []}});
    // load_config fails and names the file; read_config still parses it.
}

#[test]
fn config_without_classifiers_still_loads() {
    // A config JSON with no "classifiers" field reads and gives an empty map.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile (`classifiers` field, `call_body` not defined).

- [ ] **Step 3: Add `Config.classifiers`.** `load_config` validates every classifier after the questions and reports `"{path}: {e}"`. `refresh_from_disk` prints `"{path}: {e}; it cannot be called until fixed"` and still loads. `build_config` copies `current.classifiers`. The doc line over `put_config` becomes "Keys, questions and classifiers have their own endpoints; ...".

- [ ] **Step 4: Extract `answer` and `call_body`; generalize `change_questions` into `change_config`.**

- [ ] **Step 5: Add the handlers and routes.**
  - `GET /v1/classifiers`, `GET /v1/classifiers/{key}` and `POST /v1/classifiers/{key}` go into the `api` router, under `require_client`.
  - `POST /api/classifiers` and `PUT`/`DELETE /api/classifiers/{key}` go into `manage`, under `require_admin`.
  - `call_classifier`: look up the key (404), `classifiers::resolve` (400), `call_body` (422 as in `systemone`), `answer`. On success, reply with `Upstream { bytes: classifiers::with_skipped(bytes, &skipped), .. }`.
  - `create_classifiers` answers 201 with all classifiers. `update_classifier` answers with the one classifier. `delete_classifier` answers with all classifiers. This matches the question handlers.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 7: Smoke-check the API by hand.** Start `cargo run -- serve` with a temp `JEFF_CONFIG`, then with curl:
  - create a classifier over one saved question plus `gone`;
  - `GET /v1/classifiers` lists it;
  - `POST /v1/classifiers/<key>` returns `"skipped":["gone"]`, or a provider error with no `skipped` when no provider runs;
  - `DELETE` returns `{}`.

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "feat: serve classifiers over the API"
```

### Task 3: CLI

**Files:**
- Modify: `src/main.rs` (`Command`, new `ClassifiersAction`, `AskArgs`, `ask_request`, `run_ask`, `main`, `questions_table`)

**Interfaces:**
- Consumes: Task 1 functions, `read_config`, `save_config`, `write_json`, `config_file`.
- Produces:
  - `Command::Classifiers { action: ClassifiersAction, config: Option<String> }`, doc: "Manage classifiers in the config file; a running server picks changes up on the next request".
  - `ClassifiersAction::{List { output }, Get { key, output }, Add { key, questions: Vec<String>, model: Option<String> }, Update { same }, Remove { key }}`. `questions` is `#[arg(required = true)]`.
  - `fn table<const N: usize>(header: [&str; N], rows: Vec<[String; N]>, boxed: bool) -> String`: the rendering half of `questions_table`, cutting the last column to 60 characters. `questions_table` calls it.
  - `fn classifiers_table(cs: &Classifiers, saved: &Questions, boxed: bool) -> String`: columns `key`, `model`, `questions`. Questions are joined by `, `, and a skipped one shows as `spam (deleted)`.
  - `AskArgs.classifier: Option<String>`, conflicting with `keys`, `questions`, `questions_file` and `request`.
  - `fn ask_target(args: &AskArgs) -> String`: `/v1/classifiers/{key}` when `--classifier` is set, else `/v1/systemone`. The key goes in raw; the server rejects keys outside the key rules.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn lists_classifiers_one_line_each() {
    let cs = json!({"triage": {"questions": ["a", "gone"], "model": "p/m"}, "plain": {"questions": ["a"]}});
    let saved = json!({"a": {"type": "noul", "instructions": "x"}});
    assert_eq!(
        classifiers_table(cs.as_object().unwrap(), saved.as_object().unwrap(), false),
        "triage\tp/m\ta, gone (deleted)\nplain\t-\ta\n"
    );
}

#[test]
fn ask_calls_a_classifier() {
    let args = ask_args(&["--classifier", "triage", "--state", "s"]).unwrap();
    assert_eq!(ask_target(&args), "/v1/classifiers/triage");
    assert_eq!(ask_request(&args, stdin_is("")).unwrap(), json!({"state": "s"}));
    assert!(ask_args(&["u", "--classifier", "triage", "--state", "s"]).is_err());
    assert!(ask_args(&["--classifier", "t", "--questions", "{}", "--state", "s"]).is_err());
    assert_eq!(ask_target(&ask_args(&["u", "--state", "s"]).unwrap()), "/v1/systemone");
}
```

`lists_questions_one_line_each` must keep passing unchanged. Tests keep the existing order of `serde_json::Map`, which is sorted by key, so `plain` comes before `triage`: write the expected string in that order.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile.

- [ ] **Step 3: Implement.**
  - `ask_body` sends `{"state": ...}` plus `model` when `--classifier` is set, and does not require keys in that case.
  - `run_ask` posts to `ask_target`. When the answer has a non-empty `skipped`, it prints `skipped: a, b` to stderr before the JSON.
  - `run_classifiers` mirrors `run_questions`:
    - `add` and `update` build `{"questions": [...], "model"?: ...}`.
    - Each key `classifiers::skipped` reports prints `question '<k>' is not saved; the classifier skips it` to stderr.
    - Then they save and print `added <key>` or `updated <key>`.
    - `list` with no classifiers prints `no classifiers in <path>` to stderr.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: add jeff classifiers and ask --classifier"
```

### Task 4: Classifiers page

**Files:**
- Modify: `ui/index.html` (nav link, `#page-classifiers` with list and form views), `ui/app.js`, `ui/app.css` if a new component class is needed, `ui/dist/app.css` (rebuilt)

**Interfaces:**
- Consumes: `GET /v1/classifiers`, `POST /v1/classifiers/{key}`, `POST /api/classifiers`, `PUT`/`DELETE /api/classifiers/{key}`, `/v1/systemone`; existing JS `api`, `ask`, `answerCards`, `answerCard`, `modelChip`, `modelGroups`, `TYPE_INFO`, `confirmClick`, `busy`, `say`, `el`, `$`, `saved` (questions).
- Produces (JS):
  - `PAGES` gains `"classifiers"` between `"questions"` and `"providers"`.
  - `ask(body, status, button, path = "/v1/systemone")`: one new optional parameter.
  - `classifiers`: the loaded map. `loadClassifiers()`, `renderClassifiers()`.
  - `classifierRow(key, c)`, `questionChip(key)`, `pickedRow(key)`.
  - `openClassifierForm(key = null)`, `classifierDraft() -> [key, c]`, `saveClassifier()`, `tryClassifier()`.
  - `fullQuestions(keys, model) -> {key: question}`: saved questions in classifier order with `model` overridden when set. Feeds `answerCards()`.
  - `skippedNotice(list) -> Element | null`.

- [ ] **Step 1: Markup.** Copy the `#page-questions` structure into `#page-classifiers`, keeping the same classes, with these differences:
  - List view: `#c-items`, `#c-empty` (two texts: "No classifiers" and "Save questions first"), `#c-model`, `#c-state` and `#c-run`. The right panel has its own Answer/JSON/curl tabs.
  - Form view: `#c-key`, `#c-picked` (an `ul`), `#c-search` with `#c-options` (`role="listbox"`), `#c-form-model`, and Save, Cancel, Delete and Try buttons. The right column has a Try state field and the answer area.
  - `#c-state` and `#pg-state` keep the same value: each `input` handler copies its value to the other.

- [ ] **Step 2: List behavior.**
  - `classifierRow` shows a radio (name `classifier`) and the key in mono, plus `modelChip(c.model)` when set. Below it goes a chip per question; a skipped one gets red styling and `title='Question "<k>" was deleted. The classifier skips it.'`. The row ends with Edit.
  - Run sends `POST /v1/classifiers/<key>` with `{state, model}` via `ask(..., path)`.
  - The answer area gets `skippedNotice(out.skipped)` followed by `answerCards(fullQuestions(c.questions, c.model), out.answers || {}, body.model)`, so answers keep classifier order.
  - JSON shows the raw response. curl shows a `curl` of the classifier call in the format the Questions curl tab uses.

- [ ] **Step 3: Form behavior.**
  - Key is read-only when editing.
  - `pickedRow` shows a drag handle, key, type badge, instructions (`note truncate`) and a remove button. A key missing from `saved` renders as a red row with a `deleted` badge and the tooltip.
  - Reordering uses native `draggable` with `dragstart`/`dragover`/`drop` on `#c-picked`, plus Alt+ArrowUp and Alt+ArrowDown on the focused row. No library.
  - `#c-search` filters `saved` by a lower-case substring of key or instructions and hides keys already picked. Each option shows key, badge and instructions. ArrowUp/ArrowDown move `aria-selected`, Enter adds and clears the field, Escape closes.
  - `#c-form-model` is filled like `fillFormModel`, with the first option `None (each question uses its own)` and value `""`.
  - Save sends `POST /api/classifiers` with `{[key]: c}`, or `PUT` with `c` when editing. The `model` key is left out when the select is `""`.
  - Delete uses `confirmClick(button, "Delete?", ...)` and then `DELETE`. This is the page's two-click pattern.
  - Try sends `{state, model: $("#pg-model").value, questions}` to `/v1/systemone`. `questions` is `{k: {model}}` for each non-deleted key when the override is set, else the array of keys. Deleted keys go to `skippedNotice`. When nothing is left, the status line says `No working questions`.

- [ ] **Step 4: Build CSS**

Run: `pnpm -C ui build`
Expected: `ui/dist/app.css` rewritten, exit 0.

- [ ] **Step 5: Manual check with Playwright** against `cargo run -- serve` using a temp config with two saved questions and a provider stub or a running CLM:
  1. Open `#classifiers`. The empty state is shown.
  2. Create `triage` from both questions with a model override. Reorder by drag and by Alt+ArrowUp, then Save. The row shows the chips and the override chip.
  3. Run. The cards appear in classifier order, rendered exactly like the Questions page.
  4. Delete one question on the Questions tab and go back. Its chip is red with the tooltip. Run shows the yellow notice `1 question skipped: <key>`.
  5. Edit, remove the red row, then Try. The answer appears and nothing is saved.
  6. Delete the classifier with two clicks.

- [ ] **Step 6: Commit**

```bash
git add ui/index.html ui/app.js ui/app.css ui/dist/app.css
git commit -m "feat: add the Classifiers page"
```
