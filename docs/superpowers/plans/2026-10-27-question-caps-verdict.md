# Question Confidence Caps and Verdict Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every question may carry `confidence_soft_cap`, `confidence_hard_cap` and `confidence_source`, and every `/v1/systemone` answer gains a `verdict` object.

**Architecture:** A new pure module `src/verdict.rs` computes confidence and the verdict from one answer and its question. `src/questions.rs` validates the three fields and strips them before providers see them. `answer()` in `src/main.rs` keeps the expanded questions and applies the verdict to the joined response. The question editor in `ui/` gets the caps slider and the Try badge.

**Tech Stack:** Rust (axum, serde_json), vanilla JS + Tailwind 4 in `ui/`.

**Spec:** `docs/superpowers/specs/2026-10-27-question-caps-verdict-design.md`

## Global Constraints

- Field names: `confidence_soft_cap`, `confidence_hard_cap`, `confidence_source`. Never `soft_cap`/`hard_cap` (reserved for ROADMAP "Thresholds").
- Caps are numbers 0–1; when both are set, `confidence_hard_cap < confidence_soft_cap` (equal refused).
- `confidence_source` is `"jeff"` or `"provider"`; absent means `"jeff"`.
- Confidence, `jeff`: `noul` → `|2P − 1|`; `choice`/`score` → `1 − H(p)/ln k`, `H = −Σ pᵢ ln pᵢ`, 0·ln 0 = 0, k = entries in `probabilities`.
- Confidence, `provider`: the answer's numeric `confidence`; when missing, use `jeff` and report `source: "jeff"`.
- Verdict: `fail` if hard set and c < hard; else `warn` if soft set and c < soft; else `pass`. Strict `<`.
- `verdict` = `{result, confidence, source}` plus each cap only when set. Provider fields stay untouched.
- Errors: two sentences, "what happened. What to do.", capital letter, full stop, no "please", prefixed `Question '<key>': ` by `validate`.
- Every icon and icon-only button in the UI has a tooltip (`title`) equal to its `aria-label`.
- After every `ui/app.js` edit: `node --check ui/app.js`.
- Never run `jeff serve` against the user's config; verify on a throwaway instance (project skill `jeff-throwaway-instance-verify`).

## Review Focus

1. A request override with `"confidence_hard_cap": null` must ask without the hard cap, not fail validation — test in Task 2.
2. A provider answer with no `type` field (README shows bare `{"noul": 0.97}`) must still get a `noul` verdict — test in Task 1.
3. A non-JSON or error provider body must pass through unchanged, never 500 — test in Task 3 (`apply` on a non-object leaves it) and the `status.is_success()` guard.
4. Choice `probabilities` with a zero entry must not yield NaN — test in Task 1.
5. Editing a question saved through the API with caps, then saving from the UI with a cap removed, must drop that cap, not resurrect it from `...rest` — check in Task 4.

---

### Task 1: Confidence and verdict

**Files:**
- Create: `src/verdict.rs`
- Modify: `src/main.rs` (add `mod verdict;` next to `mod questions;`)

**Interfaces:**
- Produces:
  - `pub fn confidence(answer: &Value, source: &str) -> Option<(f64, &'static str)>` — the number and the source actually used (`"jeff"` or `"provider"`); `None` when the answer has neither a numeric `noul` nor a non-empty `probabilities` object.
  - `pub fn verdict(answer: &Value, question: &Value) -> Option<Value>` — the `verdict` object, reading the three fields from `question`.
  - `pub fn apply(response: &mut Value, questions: &Questions)` — for each `response["answers"][key]` whose key is in `questions`, inserts `"verdict"` when `verdict()` returns `Some`. Non-object input is left alone.

- [ ] **Step 1: Write the failing tests** in `src/verdict.rs` `mod tests`:

```rust
#[test]
fn jeff_confidence_matches_openjev() {
    let choice = json!({"type": "choice", "probabilities": {
        "120 words": 0.0007616466220741751, "15 words": 0.0061794831192532465,
        "30 words": 0.855626768666195, "300 words": 0.0004921765548989722,
        "5 words": 0.0002882554682521148, "50 words": 0.07728563919190698,
        "90 words": 0.05890993994416244, "Unlimited": 0.00003213464363223489,
        "Yes/No only": 0.00003547371546969253, "insufficient_context": 0.00016975864041433232,
        "uncertain": 0.00021872343374085176}, "confidence": 0.7726711895383636});
    let (c, s) = confidence(&choice, "jeff").unwrap();
    assert!((c - 0.7726711895383636).abs() < 1e-12 && s == "jeff");
    let score = json!({"type": "score", "probabilities": {"0": 0.0002131869375836218,
        "1": 0.0032810850259026047, "2": 0.9965057280365138}});
    assert!((confidence(&score, "jeff").unwrap().0 - 0.9781026173054966).abs() < 1e-12);
}

#[test]
fn noul_counts_both_ways_even_without_type() {
    for (p, want) in [(0.5, 0.0), (0.03, 0.94), (1.0, 1.0), (0.0, 1.0)] {
        let (c, _) = confidence(&json!({"noul": p}), "jeff").unwrap();
        assert!((c - want).abs() < 1e-12, "{p}");
    }
}

#[test]
fn provider_source_falls_back_to_jeff() {
    let a = json!({"type": "choice", "probabilities": {"a": 0.9, "b": 0.1}, "confidence": 0.42});
    assert_eq!(confidence(&a, "provider"), Some((0.42, "provider")));
    assert_eq!(confidence(&json!({"noul": 0.9}), "provider").unwrap().1, "jeff");
}

#[test]
fn a_zero_probability_is_not_nan() {
    let (c, _) = confidence(&json!({"probabilities": {"a": 1.0, "b": 0.0}}), "jeff").unwrap();
    assert_eq!(c, 1.0);
}

#[test]
fn verdict_rules_are_strict() {
    let a = json!({"noul": 0.9}); // confidence 0.8
    let q = |s: Value, h: Value| json!({"confidence_soft_cap": s, "confidence_hard_cap": h});
    assert_eq!(verdict(&a, &q(json!(0.8), json!(0.5))).unwrap()["result"], "pass");
    assert_eq!(verdict(&a, &q(json!(0.81), json!(0.5))).unwrap()["result"], "warn");
    assert_eq!(verdict(&a, &q(json!(0.9), json!(0.81))).unwrap()["result"], "fail");
    let none = verdict(&a, &json!({})).unwrap();
    assert_eq!(none, json!({"result": "pass", "confidence": 0.8, "source": "jeff"}));
}

#[test]
fn apply_skips_unreadable_and_unknown_answers() {
    let qs: Questions = json!({"a": {}, "b": {}}).as_object().unwrap().clone();
    let mut r = json!({"answers": {"a": {"noul": 0.9}, "b": {"oops": 1}, "c": {"noul": 0.1}}, "usage": {}});
    apply(&mut r, &qs);
    assert!(r["answers"]["a"]["verdict"].is_object());
    assert!(r["answers"]["b"].get("verdict").is_none());
    assert!(r["answers"]["c"].get("verdict").is_none());
    let mut s = json!("not an object");
    apply(&mut s, &qs);
    assert_eq!(s, json!("not an object"));
}
```

Add one property test without new dependencies: a seeded LCG over 1,000 random distributions (k from 2 to 12, normalized) asserting `0.0 <= c && c <= 1.0` and that raising the hard cap from 0 to 1 in 0.05 steps never turns `fail` back into `pass`/`warn`.

- [ ] **Step 2: Run** `cargo test verdict` — Expected: FAIL, module missing.
- [ ] **Step 3: Implement** the three functions. Clamp the entropy result to 0–1 against rounding. Read caps with `as_f64()`; a non-number is treated as unset (validation already refused it).
- [ ] **Step 4: Run** `cargo test verdict` — Expected: PASS. Then `cargo clippy --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** `git add src/verdict.rs src/main.rs && git commit -m "feat: compute answer confidence and verdict"`

### Task 2: Validate, override and strip the fields

**Files:**
- Modify: `src/questions.rs` (`check`, `expand`, `group`, tests)

**Interfaces:**
- Produces: `pub fn check_caps(q: &serde_json::Map<String, Value>) -> Result<(), &'static str>`, called from `check` and from `expand` for inline questions that carry `type`.
- `group()` removes `confidence_soft_cap`, `confidence_hard_cap`, `confidence_source` from every question it returns (alongside `model`).

- [ ] **Step 1: Write the failing tests** in `src/questions.rs`:

```rust
#[test]
fn checks_confidence_fields() {
    let ok = json!({"type": "noul", "instructions": "x", "confidence_soft_cap": 0.8,
        "confidence_hard_cap": 0.5, "confidence_source": "provider"});
    assert!(validate("k", &ok).is_ok());
    for (field, v, msg) in [
        ("confidence_soft_cap", json!(1.5), "Question 'k': The confidence_soft_cap is not a number from 0 to 1. Set a number from 0 to 1 or remove the field."),
        ("confidence_hard_cap", json!("0.5"), "Question 'k': The confidence_hard_cap is not a number from 0 to 1. Set a number from 0 to 1 or remove the field."),
        ("confidence_source", json!("model"), "Question 'k': The confidence_source is not jeff or provider. Pick one of them or remove the field."),
    ] {
        let mut q = json!({"type": "noul", "instructions": "x"});
        q[field] = v;
        assert_eq!(validate("k", &q).unwrap_err(), msg);
    }
    let equal = json!({"type": "noul", "instructions": "x", "confidence_soft_cap": 0.5, "confidence_hard_cap": 0.5});
    assert_eq!(validate("k", &equal).unwrap_err(),
        "Question 'k': The confidence_hard_cap is not below the confidence_soft_cap. Lower the hard cap or raise the soft cap.");
}

#[test]
fn a_null_override_drops_a_saved_cap() {
    let saved = map(json!({"u": {"type": "noul", "instructions": "x", "confidence_soft_cap": 0.9, "confidence_hard_cap": 0.5}}));
    let out = expand(&json!({"u": {"confidence_hard_cap": null}}), &saved).unwrap();
    assert!(out["u"].get("confidence_hard_cap").is_none());
    assert_eq!(out["u"]["confidence_soft_cap"], 0.9);
    let bad = expand(&json!({"u": {"confidence_soft_cap": 0.4}}), &saved).unwrap_err();
    assert!(bad.contains("is not below"), "{bad}");
}

#[test]
fn inline_questions_get_the_cap_checks() {
    let e = expand(&json!({"i": {"type": "noul", "instructions": "y", "confidence_soft_cap": 2}}), &Questions::new()).unwrap_err();
    assert!(e.starts_with("Question 'i': "), "{e}");
}

#[test]
fn group_strips_confidence_fields() {
    let qs = map(json!({"a": {"type": "noul", "instructions": "x", "confidence_soft_cap": 0.8,
        "confidence_hard_cap": 0.5, "confidence_source": "jeff"}}));
    let groups = group(qs, "p/m").unwrap();
    assert_eq!(groups[0].1["a"], json!({"type": "noul", "instructions": "x"}));
}
```

- [ ] **Step 2: Run** `cargo test questions` — Expected: the four new tests FAIL.
- [ ] **Step 3: Implement.** In `expand`'s override branch, after `base.extend(...)`, remove any of the three fields whose value is `null`. In the `_ =>` branch, run `check_caps` on a typed inline object and map the error with the `Question '<key>': ` prefix. Keep a `const CONFIDENCE_FIELDS: [&str; 3]` for both `expand` and `group`.
- [ ] **Step 4: Run** `cargo test` — Expected: PASS, existing tests included.
- [ ] **Step 5: Commit** `git commit -am "feat: validate and strip question confidence caps"`

### Task 3: Verdict in every answer

**Files:**
- Modify: `src/main.rs` `answer()` (lines ~799–874)

**Interfaces:**
- Consumes: `verdict::apply(&mut Value, &Questions)` (Task 1); `questions::expand` output before `group` (Task 2).

- [ ] **Step 1: Implement.** Bind the result of `questions::expand` to `rules: Option<Questions>` before passing a clone to `group`. Build the final `Upstream` for both the single-part and merged paths, then, when `rules` is `Some` and `status.is_success()`, parse `bytes`; on parse success run `verdict::apply` and re-serialize, on failure keep the bytes. Keep `provider`, `ms` and `cached` as they are.
- [ ] **Step 2: Run** `cargo test && cargo clippy --all-targets -- -D warnings` — Expected: PASS.
- [ ] **Step 3: Verify end to end** on a throwaway jeff (skill `jeff-throwaway-instance-verify`, temp config, free port, the H2O provider at `http://127.0.0.1:8741`):
  - Save `{"t": {"type": "noul", "instructions": "Is it urgent?", "confidence_soft_cap": 0.99, "confidence_hard_cap": 0.2}}` via `POST /api/questions`; `GET /v1/questions/t` returns the three fields.
  - `jeff ask t --state "Fix it now" --url <throwaway>` — every answer has `verdict` with `source: "jeff"` and both caps; the provider's own fields are unchanged.
  - Ask again with `--questions '{"t": {"confidence_soft_cap": null}}'` — header `x-jeff-cache: hit`, `verdict` has no `confidence_soft_cap`.
  - `POST /api/questions` with `confidence_hard_cap` 0.9 above soft 0.5 — `400` with the Task 2 message.
- [ ] **Step 4: Commit** `git commit -am "feat: add a confidence verdict to every answer"`

### Task 4: Caps in the question editor and Try

**Files:**
- Modify: `ui/index.html` (question form, below the **Model** label, line ~97)
- Modify: `ui/app.js` (`questionSnapshot`, `openForm`, `draft`, `answerCard`)
- Reference: mockup `.superpowers/brainstorm/872-1793091596/content/caps-slider-v4.html` (copy its markup and behaviour, using the app's `.control`, `.btn`, `.icon-btn`, `X_ICON`, `removeButton`)

**Interfaces:**
- Produces: `function capsValue() -> {confidence_soft_cap?, confidence_hard_cap?, confidence_source?}` (only set fields; `confidence_source` only when `"provider"`), `function fillCaps(q)` (from a saved question or draft snapshot).

- [ ] **Step 1: Implement the editor block.** **Confidence** select with the two option texts from the spec; for `noul` a `.note` "Confidence counts both ways: a sure yes and a sure no both score high."; the bar, triangles, labels, tooltips and number inputs exactly as v4; step 0.01, hard ≥ 0.01 below soft; arrow keys move a focused triangle by 0.01.
- [ ] **Step 2: Wire save and drafts.** `questionSnapshot()` includes `capsValue()`; `openForm` and draft restore call `fillCaps`; `draft()` removes the three fields from `rest` before spreading, then adds `capsValue()`.
- [ ] **Step 3: Badge in Try.** In `answerCard`, when `a.verdict` exists, append to `head` a badge (`fail` red, `warn` amber, `pass` green, light and dark like `.badge`) and a `.note`: `confidence 0.77 (jeff) · soft 0.80 · hard 0.50`. The classifier Try uses the same function, so it shows the badge too.
- [ ] **Step 4: Run** `node --check ui/app.js` — Expected: no output.
- [ ] **Step 5: Verify in the browser** on the throwaway jeff: create a `choice` question, drag both triangles, type exact values, remove and re-add a cap, save, reopen (values persist), Try shows the badge; edit a question saved with caps via the API, remove one cap, save, `GET /v1/questions/<key>` no longer has it; hover a segment, a triangle and both `×` buttons — each shows a tooltip.
- [ ] **Step 6: Commit** `git add ui/ && git commit -m "feat(ui): edit confidence caps and show the verdict"`

### Task 5: Docs

**Files:**
- Modify: `README.md` (after "The answer follows the Jev schema", line ~110; the saved-questions section)
- Modify: `ROADMAP.md` (already edited in the worktree; commit as is)

- [ ] **Step 1: Write** in README: the three fields with their ranges and default, the confidence table, the verdict table, one request/response example with `verdict`, and that a request override with `null` drops a cap for that call.
- [ ] **Step 2: Check** every example in the README section against the throwaway instance from Task 3 (same output shape).
- [ ] **Step 3: Commit** `git add README.md ROADMAP.md && git commit -m "docs: document confidence caps and the verdict"`

The PR title must be conventional (`feat: add confidence caps and a verdict to answers`) so a release is cut.
