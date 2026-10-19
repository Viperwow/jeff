# Shared Answers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A library of shared answers that questions reuse by content, with an API, a CLI and an admin page, while saved questions stay plain Jev.

**Architecture:** A new core module `src/answers.rs` owns validation, matching (`uses`) and the update that rewrites matching questions, in the shape of `src/classifiers.rs`. `src/main.rs` stores `answers` in `Config`, serves `/api/answers` through `change_config`, and adds `jeff answers`. The UI gets an Answers page and answer chips in the question editor.

**Tech Stack:** Rust (axum, serde_json, clap), plain JS + Tailwind in `ui/`.

**Spec:** `docs/superpowers/specs/2026-10-19-shared-answers-design.md`

## Global Constraints

- Questions and classifiers API, `/v1` and provider calls do not change. No new field in a question.
- `jeff.json` field `answers`, read with `#[serde(default)]`; jeff never adds answers by itself.
- Answer fields: `text` (non-empty, stored trimmed), `types` (non-empty subset of `choice`, `score`, no duplicates), `default` (optional bool). A `questions` field on input is ignored; any other field is refused: `Answer '<key>': Unknown field '<f>'. Keep only text, types and default.`
- Key rules: `questions::check_key`.
- Errors: two sentences, what happened then what to do, e.g. `No answer 'x'. Check the key or create the answer first.`
- `/api/answers` needs an admin key, like `/api/questions`.
- Every new flag or field documented in README in the same PR.
- Commit messages and PR title conventional (`feat:`), body lines ≤ 100 chars.

## Review Focus

- Library text typed with surrounding spaces: the UI trims option text on save, so an untrimmed library text would never match. Stored trimmed (Task 1 test `text_is_stored_trimmed`).
- A hand-edited config with a non-object or invalid question: `uses` and `update` skip it, never panic (Task 1 test `update_skips_broken_questions`).
- A `score` question holding the old text twice: every equal level is rewritten (Task 1 test `update_rewrites_every_equal_level`).
- `jeff answers get k -o f.json` then `jeff answers update k f.json` round-trips despite the computed `questions` field (Task 3 test `update_ignores_questions_field` in Task 1, CLI uses the same path).
- A chip whose key is taken in the open question by a hand-typed option with another text: disabled with the spec's title, never overwriting the typed text (Task 5 browser check).

---

### Task 1: Core module `answers.rs`

**Files:**
- Create: `src/answers.rs` (unit tests inline under `#[cfg(test)]`, as in `classifiers.rs`)
- Modify: `src/main.rs:1-3` (add `mod answers;`)

**Interfaces:**
- Consumes: `questions::{check_key, Change, Questions}`.
- Produces:
  - `pub type Answers = serde_json::Map<String, Value>;`
  - `pub fn validate(key: &str, a: &Value) -> Result<(), String>` — one answer; ignores `questions`.
  - `pub fn validate_all(all: &Answers) -> Result<(), String>` — each answer plus score-text uniqueness.
  - `pub fn uses(key: &str, a: &Value, qs: &Questions) -> Vec<String>` — question keys using the answer, in question order.
  - `pub fn with_uses(all: &Answers, qs: &Questions) -> Answers` — copy with a computed `questions` array on each answer.
  - `pub fn create(saved: &mut Answers, input: Answers) -> Result<(), Change>`
  - `pub fn update(saved: &mut Answers, qs: &mut Questions, key: &str, a: Value) -> Result<Vec<String>, Change>` — returns rewritten question keys.
  - `pub fn remove(saved: &mut Answers, key: &str) -> Result<(), Change>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn rejects_broken_answers() {
    // non-object, empty/whitespace text, empty types, unknown type "noul", duplicate type, bad key "a b"
    // each Err starts with "Answer '<key>': "
}
#[test] fn refuses_unknown_fields_but_ignores_questions() {
    assert_eq!(validate("a", &json!({"text":"x","types":["choice"],"txt":1})).unwrap_err(),
        "Answer 'a': Unknown field 'txt'. Keep only text, types and default.");
    assert!(validate("a", &json!({"text":"x","types":["choice"],"questions":["q"]})).is_ok());
}
#[test] fn score_texts_are_unique() {
    let all = json!({"a":{"text":"Same","types":["score"]},"b":{"text":"Same","types":["score","choice"]}});
    assert_eq!(validate_all(all.as_object().unwrap()).unwrap_err(),
        "Answer 'b' has the same text as 'a'. Change one of the texts.");
    // same text with types ["choice"] on both is fine
}
#[test] fn uses_matches_choice_by_key_and_text_and_score_by_text() {
    // choice q1 {"uncertain":"Not sure."} uses; q2 {"uncertain":"Other"} does not;
    // score q3 ["Low","Not sure.","High"] uses only when types holds "score"; noul never
}
#[test] fn text_is_stored_trimmed() { /* create {"text":"  Not sure.  "} -> saved text "Not sure." */ }
#[test] fn create_strips_questions_and_refuses_existing() {
    // stored answer has no "questions"; existing key -> Change::Exists
    // "Answer 'a' already exists. Pick another key or update the saved one."
}
#[test] fn update_rewrites_only_matching_questions() {
    // choice q1 matches -> criteria["uncertain"] = new text; q2 hand-edited text untouched;
    // returns ["q1", ...] in question order
}
#[test] fn update_rewrites_every_equal_level() { /* score ["Not sure.","Low","Not sure."] -> both rewritten */ }
#[test] fn update_narrowing_types_leaves_questions() { /* types ["choice"] -> ["score"]: choice q keeps old text, not returned */ }
#[test] fn update_refuses_a_level_clash() {
    // score q has "Not sure." (answer) and "Maybe"; update text to "Maybe"
    // -> Change::Invalid("Question 'q' already has the level 'Maybe'. Change the text or edit q."), nothing written
}
#[test] fn update_skips_broken_questions() { /* qs with "x": 5 and {"type":"choice"} without criteria -> no panic, not returned */ }
#[test] fn update_ignores_questions_field() { /* body with "questions":["q"] saves without it */ }
#[test] fn remove_leaves_questions() { /* remove -> qs unchanged; missing key -> NotFound
    "No answer 'x'. Check the key or create the answer first." */ }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test answers::`
Expected: compile errors (module items missing).

- [ ] **Step 3: Implement `src/answers.rs`**

Mirror `classifiers.rs`: `validate` = `check` + unknown-field scan; `create`/`update` validate, trim `text`, drop `questions`, then run `validate_all` on the would-be library. `update` checks every level clash first and writes nothing on error; it finds users with the old answer, then rewrites them. Use `Change::{Invalid, Exists, NotFound}` with the messages from the tests.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test answers::`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/answers.rs src/main.rs
git commit -m "feat: add the shared answers core"
```

### Task 2: Config field and `/api/answers`

**Files:**
- Modify: `src/main.rs` — `Config` (~line 68), `Config::default` (~145), `load_config` (~154), `build_config` (~1650), handlers next to classifier handlers (~1511), routes (~2772), tests module.

**Interfaces:**
- Consumes: Task 1 `Answers`, `validate_all`, `with_uses`, `create`, `update`, `remove`, `uses`.
- Produces: `Config.answers: answers::Answers` with `#[serde(default)]`; routes `GET/POST /api/answers`, `GET/PUT/DELETE /api/answers/{key}`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn config_without_answers_loads_empty_and_keeps_questions() {
    // write a 0.11.4-shaped jeff.json with questions; load_config -> answers empty;
    // serde round-trip keeps every question value equal
}
#[test] fn load_config_rejects_invalid_answers() { /* {"answers":{"a":{"text":""}}} -> Err mentions "Answer 'a'" */ }
#[test] fn build_config_keeps_answers() { /* as build_config_keeps_classifiers */ }
#[tokio::test] async fn answers_api_round_trip() {
    // with test_app + router: POST {"uncertain":{...}} -> 201 map;
    // GET /api/answers -> uncertain.questions == ["q1"] when q1 matches;
    // PUT /api/answers/uncertain {"text":"New.","types":["choice"]} -> 200 {"answer":{...},"questions":["q1"]}
    //   and GET /v1/questions/q1 shows "New.";
    // DELETE -> 200 map without it, q1 unchanged; DELETE again -> 404
}
```

Follow the shape of the existing classifier API tests for building requests.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test answers`
Expected: FAIL (field and routes missing).

- [ ] **Step 3: Implement**

- `Config.answers` with `#[serde(default)]`; `Config::default` sets empty; `load_config` adds `answers::validate_all` problems to the list; `build_config` copies `current.answers`.
- Handlers `list_answers`, `get_answer`, `create_answers`, `update_answer`, `delete_answer`. `update_answer` captures the rewritten keys from inside the `change_config` closure and returns `{"answer": c.answers[key], "questions": rewritten}`. GET responses go through `with_uses`.
- Routes on the admin router beside `/api/classifiers`.

- [ ] **Step 4: Run tests**

Run: `cargo test` then `cargo clippy --all-targets --locked -- -D warnings` and `cargo fmt --all --check`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: store shared answers and serve /api/answers"
```

### Task 3: `jeff answers` CLI and README

**Files:**
- Modify: `src/main.rs` — `Command` enum (beside `Classifiers` ~1810), new `AnswersAction`, `run_answers`, `answers_table`, tests.
- Modify: `README.md` — new section "Shared answers" after "Saved questions"; command reference block.

**Interfaces:**
- Consumes: Task 2 endpoints; existing `admin_call`, `read_json`, `print_json`, `segment`, `table`.
- Produces: `jeff answers list [-o FILE] | get KEY [-o FILE] | add FILE | update KEY FILE | remove KEY`, with `--url` / `JEFF_ADMIN_URL` exactly as `jeff questions`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn lists_answers_one_line_each() {
    let a = json!({"uncertain":{"text":"Not\n sure.","types":["choice"],"default":true,"questions":["q1","q2"]},
                   "low":{"text":"Low","types":["score"],"questions":[]}});
    assert_eq!(answers_table(a.as_object().unwrap(), false),
        "uncertain\tchoice\t2\tNot sure.\nlow\tscore\t0\tLow\n");
}
#[test] fn update_message_names_rewritten_questions() {
    assert_eq!(updated_message(&["a".into(),"b".into()]), "Updated 2 questions: a, b.");
    assert_eq!(updated_message(&[]), "Updated no questions.");
    assert_eq!(updated_message(&["a".into()]), "Updated 1 question: a.");
}
#[test] fn answers_cli_parses() { /* list, get k, add f.json, update k f.json, remove k; "update k" alone fails */ }
```

Table columns: `key`, `types` (comma-joined), `questions` (count), `text`; header row and boxing follow `questions_table`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test answers`
Expected: FAIL.

- [ ] **Step 3: Implement** `answers_table(a: &answers::Answers, boxed: bool) -> String`, `updated_message(keys: &[String]) -> String`, `AnswersAction`, `run_answers(action, url)`. `update` prints `updated_message` to stdout instead of JSON.

- [ ] **Step 4: Write README section**

Cover: what a shared answer is; the example library from the spec (`insufficient_context`, `uncertain`, `types: ["choice"]`, `default: true`); matching rule; update rewrites matching questions and prints `Updated N questions`; remove keeps options; the `score` average warning; the endpoint table; the CLI commands. Add `jeff answers` lines to the command reference.

- [ ] **Step 5: Run checks**

Run: `cargo test` and clippy/fmt as in Task 2.
Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs README.md
git commit -m "feat: add jeff answers"
```

### Task 4: Answers page

**Files:**
- Modify: `ui/index.html` — nav tab `#answers` after Classifiers; `#page-answers` with list view and form view, modelled on `#page-classifiers`.
- Modify: `ui/app.js` — `PAGES` (line 153) gets `"answers"`; `let library = {}`; `loadAnswers()`, `renderAnswers()`, `openAnswerForm(key)`, `saveAnswerForm()`, `deleteAnswer(key)`; drafts like the classifier form (`answer:<key|new>`).
- Rebuild `ui/dist/app.css` with `pnpm build` in `ui/` if new classes appear.

**Interfaces:**
- Consumes: `GET/POST/PUT/DELETE /api/answers` (Task 2).
- Produces: global `library` (map with computed `questions`) and `loadAnswers()` for Task 5.

- [ ] **Step 1: Build the list**: rows show key (mono), type chips, text, `Used in N questions`, a `Default` chip when set; empty state "No shared answers yet." with a New button.
- [ ] **Step 2: Build the form**: key (read-only on edit), text, checkboxes `choice` and `score`, checkbox `Add to new questions` (`default`). Save: POST for new, PUT for edit; on PUT show `updated_message` wording (`Updated N questions.`) in the form status, then `loadQuestions()` and `loadAnswers()`.
- [ ] **Step 3: Delete** with `confirm()`: `Delete 'k'? N questions keep this option as plain text.`
- [ ] **Step 4: Verify in a browser** on a temporary jeff (`JEFF_HOME` in the scratchpad, ports 18080/18081, admin key via `jeff keys create --role admin --admin-url`): create, edit with a matching question (count and rewritten text visible on Questions), delete, error on duplicate score text shown in form status.
- [ ] **Step 5: Commit**

```bash
git add ui/
git commit -m "feat(ui): add the Answers page"
```

### Task 5: Answer chips in the question editor

**Files:**
- Modify: `ui/index.html` — in `#tpl-question`, a `.q-shared` chip row after `.q-rows`.
- Modify: `ui/app.js` — `setType`, `addRow`, `readQuestions`, `questionSnapshot`, `openForm`.

**Interfaces:**
- Consumes: `library` from Task 4.
- Produces: nothing new for other tasks; saved questions stay plain Jev.

- [ ] **Step 1: Chips**: `renderShared(card)` lists every answer whose `types` hold the card type; hidden for `noul`. A chip is on when a row matches (choice: key and text; score: text). Click on: `addRow(card, type, key, text)` for choice, `addRow(card, type, text)` for score, appended at the end. Click off: remove the matching row(s).
- [ ] **Step 2: Shared rows**: a matching row gets grey, read-only inputs and a `shared` tag; its ✕ works like the chip. Re-render chips and row state on every input in the card.
- [ ] **Step 3: Key clash**: a chip whose key exists in the card with another text is disabled with the title `Option 'uncertain' is already used. Rename it to add the shared answer.`
- [ ] **Step 4: Defaults**: a new question, and a type change, append every `default` answer of the type after the blank rows. A restored draft is not touched.
- [ ] **Step 5: Verify in a browser** (same temporary jeff): choice and score chips on/off; grey rows; saved JSON via `jeff questions get` is plain criteria; a new choice question starts with defaults; key clash chip disabled; noul shows no chips.
- [ ] **Step 6: Commit**

```bash
git add ui/
git commit -m "feat(ui): add shared answer chips to the question editor"
```
