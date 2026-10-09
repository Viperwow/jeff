# CLI via the API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `jeff questions` and `jeff classifiers` call a running jeff. A broken `jeff.json` never starts jeff or replaces a working config. Admin page forms keep drafts.

**Architecture:**
- `load_config` becomes the single all-or-nothing check. It returns every problem found.
- `serve` uses it at start, and `App::refresh_from_disk` uses it on reload. On reload, problems are kept in `App`.
- `App::save` refuses with `409 config_invalid` while that list is not empty.
- `jeff.log` sits next to the config.
- The CLI loses its config-file code and calls `/v1/*` and `/api/*` on the UI port.
- The UI gets a status banner, the variant E deleted row and `localStorage` drafts.

**Tech Stack:** Rust (axum 0.8, clap 4, reqwest 0.13, serde_json, tokio), vanilla JS, Tailwind 4 (`pnpm -C ui build`).

**Spec:** `docs/superpowers/specs/2026-10-09-cli-via-api-design.md`

## Global Constraints

- Error code `config_invalid`. API body: `{"code":"config_invalid","error":"<file> has N problems; fix it before changing questions or classifiers","log":"jeff.log"}`.
- `<file>` is the config file name, for example `jeff.json`.
- `N problem` is singular for 1, `N problems` otherwise.
- API responses carry the log file name `jeff.log` only, never a path.
- The start error prints `error[config_invalid]: <file> has N problems`, then `log: <full log path>`. It exits with code 1.
- The CLI default URL is `http://127.0.0.1:8081`, env `JEFF_ADMIN_URL`, flag `--url`.
- The CLI key comes from `JEFF_API_KEY`, sent as a Bearer token.
- `jeff keys` keeps editing the file directly.
- No new crates.
- CI must pass: `cargo fmt --all --check`, `cargo +1.99.0 clippy --all-targets --locked -- -D warnings`, `cargo test --locked`.
- Rebuild `ui/dist/app.css` with `pnpm -C ui build` after class changes. The binary embeds the UI, so rebuild it before any manual check.
- Code, commits and docs are in English. No AI attribution trailers.

## Review Focus

- **A config path without a directory part** (`--config jeff.json`). `jeff.log` must land in the current directory, not fail. Test in Task 2.
- **A file deleted while jeff runs.** `modified()` gives `None`, so the reload is skipped and the config is kept, as today. Do not report it as broken. Covered by the Task 3 reload check.
- **Saves to providers and keys while the file is broken.** They go through `App::save` too, so they also answer 409. Otherwise they would overwrite hand edits. Task 3 updates the spec line from "questions and classifiers" to "every change".
- **A CLI error response that is not JSON** (a proxy page, an empty 502). Print `error: HTTP <status>: <text>`. Test in Task 4.
- **A draft for a record deleted elsewhere** (`draft:classifier:triage` after `triage` was removed). Restore it only when that record is opened for editing. Never resurrect it on the list. Test in Task 6.

---

### Task 1: All-or-nothing config validation

**Files:**
- Modify: `src/classifiers.rs:10-58` (fold the unknown-field check into `validate`, drop `validate_new`)
- Modify: `src/main.rs:110-121` (`load_config`)
- Test: `src/classifiers.rs` tests, `src/main.rs` tests `load_config_*`

**Interfaces:**
- Produces: `fn load_config(path: &str) -> Result<Config, Vec<String>>`. An Err holds every problem: one entry for an IO or parse error, otherwise one per invalid question or classifier, in key order.
- Produces: `classifiers::validate(key, c)` refuses unknown fields with `classifier 'k': unknown field 'modle'; a classifier has questions and model`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn load_config_lists_every_problem() {
    // file with questions {"a b": {...valid noul...}} and classifiers {"triage": {"questions":["x"],"modle":"m"}}
    let problems = load_config(path).unwrap_err();
    assert_eq!(problems.len(), 2);
    assert!(problems[0].starts_with("question 'a b': "));
    assert_eq!(problems[1], "classifier 'triage': unknown field 'modle'; a classifier has questions and model");
}

#[test]
fn load_config_accepts_a_classifier_naming_a_deleted_question() {
    // classifiers {"triage": {"questions":["gone"]}}, no questions
    assert!(load_config(path).is_ok());
}

#[test]
fn load_config_reports_bad_json_as_one_problem() {
    // file text "{"
    assert_eq!(load_config(path).unwrap_err().len(), 1);
}
```

Update `load_config_rejects_invalid_questions` and `load_config_rejects_invalid_classifiers` to the `Vec<String>` error.

- [ ] **Step 2: Run** `cargo test load_config`. Expected: FAIL (type mismatch, unknown field accepted).
- [ ] **Step 3: Implement.** Move the unknown-field check into `classifiers::validate`, delete `validate_new` and use `validate(..).map_err(Change::Invalid)` in `create` and `update`. Remove the "Loading stays lenient" doc line. Rewrite `load_config` on top of `read_config`: map a read error to `vec![e]`, then collect every `validate` error. `serve` still compiles: join the problems with `\n` until Task 3.
- [ ] **Step 4: Run** `cargo test`. Expected: all PASS.
- [ ] **Step 5: Commit** `feat: refuse the whole config on any invalid entry and list every problem`

### Task 2: `jeff.log`

**Files:**
- Create: `src/log.rs`
- Modify: `src/main.rs` (`mod log;`)

**Interfaces:**
- Produces: `log::path(config_path: &str) -> PathBuf`, the `jeff.log` next to the config, or `jeff.log` in the current directory when the path has no parent.
- Produces: `log::utc(secs: u64) -> String`, the time `YYYY-MM-DDTHH:MM:SSZ`.
- Produces: `log::write(config_path: &str, event: &str, summary: &str, problems: &[String])`. It appends `"{utc} {event} {summary}\n"`, then `"  {problem}\n"` per problem. Write failures are ignored, because the console line still reports the problem.

- [ ] **Step 1: Write failing tests** in `src/log.rs`

```rust
#[test]
fn utc_formats_seconds() {
    assert_eq!(utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(utc(1_791_542_537), "2026-10-09T10:42:17Z");
}

#[test]
fn path_sits_next_to_the_config() {
    assert_eq!(path("dir/jeff.json"), PathBuf::from("dir/jeff.log"));
    assert_eq!(path("jeff.json"), PathBuf::from("jeff.log"));
}

#[test]
fn write_appends_entries() {
    // temp dir; write twice; file has both summary lines, problems indented by two spaces
}
```

- [ ] **Step 2: Run** `cargo test log::`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement.** `utc` converts days since the epoch to a civil date with Howard Hinnant's `civil_from_days`. `write` opens the file with append and create.
- [ ] **Step 4: Run** `cargo test log::`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: log config problems to jeff.log`

### Task 3: Start, reload, 409 and `/api/status`

**Files:**
- Modify: `src/main.rs`:
  - `serve` (2048-2054);
  - `App` struct (164) and `refresh_from_disk` (310-332);
  - `App::save` (334) and its four callers (921, 943, 975, 1249);
  - `manage` router (2127);
  - `error`-style helpers near 384.
- Modify: `docs/superpowers/specs/2026-10-09-cli-via-api-design.md`. "Changes to questions and classifiers answer 409" becomes "Every change saved through the API answers 409". The `/api/status` invalid example gains `"file":"jeff.json"`.

**Interfaces:**
- Consumes: `load_config`, `log::write`, `log::path`.
- Produces: `fn problems_text(config_path: &str, n: usize) -> String`, for example `"jeff.json has 2 problems"`.
- Produces: `fn config_invalid(config_path: &str, n: usize) -> Response`: 409 with the body from Global Constraints.
- Produces: `fn status_body(config_path: &str, problems: &[String]) -> Value`: `{"config":"ok"}`, or `{"config":"invalid","file":"jeff.json","problems":N,"log":"jeff.log"}`.
- Produces: the field `App.problems: Mutex<Vec<String>>`.
- Produces: `App::save(&self, config) -> Result<(), Response>`, which answers 409 via `config_invalid` while problems exist and 500 on a write error.
- Produces: `GET /api/status`, admin-only, which calls `refresh_from_disk` and then answers `status_body`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn problems_text_counts() {
    assert_eq!(problems_text("x/jeff.json", 1), "jeff.json has 1 problem");
    assert_eq!(problems_text("x/jeff.json", 2), "jeff.json has 2 problems");
}

#[tokio::test]
async fn config_invalid_names_the_log_only() {
    let r = config_invalid("C:/Users/u/.jeff/jeff.json", 2);
    assert_eq!(r.status(), StatusCode::CONFLICT);
    // body == {"code":"config_invalid","error":"jeff.json has 2 problems; fix it before changing questions or classifiers","log":"jeff.log"}
}

#[test]
fn status_body_shapes() {
    assert_eq!(status_body("x/jeff.json", &[]), json!({"config":"ok"}));
    assert_eq!(status_body("x/jeff.json", &["a".into(), "b".into()]), json!({"config":"invalid","file":"jeff.json","problems":2,"log":"jeff.log"}));
}
```

- [ ] **Step 2: Run** `cargo test problems_text config_invalid status_body`. Expected: FAIL.
- [ ] **Step 3: Implement the start check.** On `Err(problems)`, `serve` calls `log::write(path, "config_invalid", "<path>: N problems; jeff did not start", &problems)`. It prints `error[config_invalid]: {problems_text}` and `log: {log::path}` to stderr, then exits 1.
- [ ] **Step 4: Implement the reload.** `refresh_from_disk` with a new mtime:
  - On `Ok`, apply the config. If `problems` was non-empty, clear it, log `config_fixed` "`<path> fixed; config reloaded`" and print the same line.
  - On `Err(p)`, keep the config and store `p`. Log `config_invalid` "`<path>: N problems; jeff keeps the previous config`". Print `error[config_invalid]: {problems_text}; jeff keeps the previous config`.
  - Drop the per-entry warnings.
- [ ] **Step 5: Implement the 409.** `App::save` checks `problems` first. Its callers return the `Response` as is.
- [ ] **Step 6: Run** `cargo test`. Expected: PASS.
- [ ] **Step 7: Manual check.**
  - `cargo run -- serve --config <tmp>/jeff.json`, with a broken file at start. Expected: two lines on stderr, exit 1, `jeff.log` written.
  - Start with a good file, then break it. `POST /api/classifiers` with node fetch. Expected: 409 and the exact body.
  - `GET /api/status`. Expected: invalid with count.
  - `POST /v1/classifiers/<k>` still answers.
  - Fix the file. `/api/status` answers `{"config":"ok"}`, and `jeff.log` has `config_fixed`.
- [ ] **Step 8: Commit** `feat: keep the last working config, refuse saves and report status while jeff.json is broken`

### Task 4: CLI as an API client

**Files:**
- Modify: `src/main.rs`:
  - `Command::Questions` and `Command::Classifiers` (1363-1378);
  - `QuestionsAction` (1426-1453);
  - `ClassifiersAction` and `ClassifierArgs` (1455-1487);
  - `main` dispatch (1586-1591);
  - `run_classifiers` and `run_questions` (1837-1955).
- Modify: `README.md`, every `jeff questions` and `jeff classifiers` example or flag (`--config` removed, `--url` and `--file` added).

**Interfaces:**
- Consumes: `client_key`, `read_input`, `write_json`, `questions_table`, `classifiers_table`, `classifiers::skip_reason` and `skipped`, `parse_url`.
- Produces: `fn error_lines(status: u16, text: &str) -> String`:
  - JSON `{code,error,log?}` gives `error[<code>]: <error>` plus `\nlog: <log>` when `log` is present;
  - JSON `{error}` gives `error: <error>`;
  - anything else gives `error: HTTP <status>: <text>`.
- Produces: `async fn admin_call(base: &str, method: reqwest::Method, path: &str, body: Option<&Value>) -> Option<Value>`:
  - on success, the response JSON, or `None` for an empty body;
  - otherwise it prints `error_lines`, or `cannot connect to <base>; start jeff first` on a connect error, and exits 1;
  - the timeout is 5 s to connect and 60 s overall.
- Produces: `Command::Questions { action, url }` and `Command::Classifiers { action, url }`, with `url` set by `#[arg(long, env = "JEFF_ADMIN_URL", default_value = "http://127.0.0.1:8081", value_parser = parse_url, global = true)]`.
- Produces: `ClassifiersAction::Add(AddClassifierArgs)` and `ClassifiersAction::Update(UpdateClassifierArgs)`:
  - `AddClassifierArgs { key: Option<String>, questions: Vec<String>, model: Option<String>, file: Option<String> }`: `key` and `questions` are required unless `--file`, and `--file` conflicts with all three;
  - `UpdateClassifierArgs { key: String, questions: Vec<String>, model: Option<String>, file: Option<String> }`: `questions` is required unless `--file`, and `--file` conflicts with `questions` and `model`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn error_lines_formats_api_errors() {
    assert_eq!(error_lines(409, r#"{"code":"config_invalid","error":"jeff.json has 2 problems; fix it before changing questions or classifiers","log":"jeff.log"}"#),
        "error[config_invalid]: jeff.json has 2 problems; fix it before changing questions or classifiers\nlog: jeff.log");
    assert_eq!(error_lines(404, r#"{"error":"unknown classifier 'x'"}"#), "error: unknown classifier 'x'");
    assert_eq!(error_lines(502, "Bad Gateway"), "error: HTTP 502: Bad Gateway");
}

#[test]
fn classifiers_add_takes_flags_or_a_file() {
    assert!(Cli::try_parse_from(["jeff","classifiers","add","t","a","b","--model","m"]).is_ok());
    assert!(Cli::try_parse_from(["jeff","classifiers","add","--file","c.json"]).is_ok());
    assert!(Cli::try_parse_from(["jeff","classifiers","add","t","a","--file","c.json"]).is_err());
    assert!(Cli::try_parse_from(["jeff","classifiers","add"]).is_err());
    assert!(Cli::try_parse_from(["jeff","classifiers","update","t","--file","-"]).is_ok());
    assert!(Cli::try_parse_from(["jeff","classifiers","update","t","--file","-","--model","m"]).is_err());
}
```

- [ ] **Step 2: Run** `cargo test error_lines classifiers_add`. Expected: FAIL.
- [ ] **Step 3: Implement.** Make `run_questions` and `run_classifiers` async, taking `url`, with the endpoints from the spec table.
  - **`list`:** fetches the map, then writes it with `-o` or prints the table. `classifiers list` also GETs `/v1/questions` for the table.
  - **Empty `list`:** prints `no questions` or `no classifiers` to stderr.
  - **`get`:** prints, or writes with `-o`.
  - **`add` and `update`:** print the response JSON. For classifiers they then GET `/v1/questions` and print `skip_reason` to stderr for each skipped key.
  - **`classifiers add --file`:** posts the file JSON as is.
  - **`classifiers update KEY --file`:** puts it as is.
  - **`remove`:** prints nothing.
  - Remove the "in the config file" wording from the command help.
- [ ] **Step 4: Run** `cargo test`. Expected: PASS.
- [ ] **Step 5: Three-way check.** Run jeff on a temp config with an admin key. Run the same flow three times: CLI flags, CLI `--file` (and `-` via a pipe), and node fetch against the API. The flow is add, get, update, list and remove of a question and a classifier, then the same add while the file is broken. Expected: the same stored JSON and the same errors each way.
- [ ] **Step 6: Commit** `feat: jeff questions and classifiers call the running jeff API`

### Task 5: Banner, 409 message and deleted row

**Files:**
- Modify: `ui/index.html` (banner under `#auth-error`, line 20)
- Modify: `ui/app.js`:
  - `api` (65-72);
  - `saveForm` (947), `saveClassifier` (1268) and `saveProviders` (275);
  - `createKey` (466);
  - `pickedRow` (1105-1131).

**Interfaces:**
- Consumes: `GET /api/status`, and the 409 body.
- Produces: `api()` throws an `Error` with `code` and `log` copied from the body.
- Produces: `function errorText(err)`, which returns `err.message`, plus `. Log: ${err.log}` when `err.log` is set.
- Produces: `async function checkStatus()`, which shows or hides `#config-banner`.

- [ ] **Step 1: Add the banner.**
  - Markup: `<div id="config-banner" role="status" hidden>`, styled like the amber notice at `app.js:1003`.
  - Text: **`<file> has N problems.`** jeff keeps running on the previous config. Log: `jeff.log`.
  - The banner has a `#config-recheck` button labelled **Recheck**.
  - The file name, count and log name come from the `/api/status` body.
- [ ] **Step 2: Wire the status.**
  - `checkStatus()` runs after `loadConfig()` at start.
  - It runs in every save `catch` when `err.code === "config_invalid"`.
  - **Recheck** calls it through `busy`.
  - Save errors show `errorText(err)`.
- [ ] **Step 3: Restyle the deleted row (variant E).**
  - The `li` keeps `bg-red-50 dark:bg-red-950/30`.
  - The key loses `line-through`.
  - The caption reads `The classifier skips this question`.
  - The remove button gets `text-red-700 dark:text-red-300`.
  - The `deleted` tag and the `deletedTip` tooltip stay as they are.
- [ ] **Step 4: Build.** `pnpm -C ui build`, then `cargo build`.
- [ ] **Step 5: Browser check** with the playwright-core + msedge script. Break the file and open the page. Expected: the banner shows. Save a classifier: the form shows the 409 text with `Log: jeff.log` and keeps its input. Fix the file and press **Recheck**: the banner hides. A classifier with a deleted question shows the E row. Take a screenshot.
- [ ] **Step 6: Commit** `feat(ui): broken config banner, recheck and the deleted question row`

### Task 6: Form drafts

**Files:**
- Modify: `ui/index.html` (one `Draft restored` / **Discard** row per form, above its fields: `#q-draft`, `#c-draft`, `#p-draft`, `#k-draft`)
- Modify: `ui/app.js`:
  - `store` (55);
  - `openForm` (923), `saveForm` and the cancel handler;
  - `showClassifierForm` and `saveClassifier`;
  - `renderProviders` and `saveProviders`;
  - `createKey`.

**Interfaces:**
- Consumes: `store` (`jeff.` prefix, try/catch already in place).
- Produces: `const drafts = { get(id), set(id, value), drop(id) }`, stored under `draft:<id>`.
- Draft ids:
  - questions: `question:new` or `question:<key>`;
  - classifiers: `classifier:new` or `classifier:<key>`;
  - providers: `providers`;
  - keys: `key:new`.
- Draft values:
  - question: `{key, q}` from `draft()`;
  - classifier: `{key, picked, model}`;
  - providers: the provider list without any `key` field;
  - key: `{name, role}`.

- [ ] **Step 1: Write drafts.** Each form container gets one delegated `input` and `change` listener that calls `drafts.set`. `renderPicked` also writes the classifier draft. Provider key inputs are never read into the draft.
- [ ] **Step 2: Restore drafts.**
  - `openForm(key)` and the classifier form opener check `drafts.get` for their id. When a draft exists, they fill from it and show `#?-draft`.
  - Providers restore after `loadConfig` renders them.
  - A draft whose record no longer exists restores only when that record is opened. The list is never built from drafts.
- [ ] **Step 3: Drop drafts.** A successful save, **Cancel** and **Discard** call `drafts.drop`. **Discard** also refills the form from the saved record, or empties it for a new one.
- [ ] **Step 4: Build.** `pnpm -C ui build`, then `cargo build`.
- [ ] **Step 5: Browser check.**
  - Type a new classifier and reload. Expected: the form reopens with **Draft restored**, and **Discard** clears it.
  - Edit a question, break the file and save. Expected: 409, then reload restores the edit.
  - Type a provider key and reload. Expected: the key field is empty.
  - Remove `triage` through curl while its draft exists. Expected: the list has no `triage`.
  - Run once with `localStorage` blocked. Expected: forms work.
- [ ] **Step 6: Commit** `feat(ui): keep form drafts in the browser`

### Task 7: Final verification

- [ ] **Step 1: Run the CI set locally.** `cargo fmt --all --check`, `cargo +1.99.0 clippy --all-targets --locked -- -D warnings`, `cargo test --locked`. Expected: clean.
- [ ] **Step 2: Rerun the end-to-end checks.** Repeat the Task 3, 4, 5 and 6 checks on the final binary. Record the outputs for the PR description.
- [ ] **Step 3: Review.** Dispatch a final code reviewer on the branch diff against `main`. Show the findings to the user before fixing.
