# Classifiers

## Goal

Group saved questions into a named classifier and run the group by key. A classifier may override the model of
every question it runs, without touching the questions themselves. Providers see nothing new: a classifier call
becomes the same grouped `/v1/systemone` calls that saved questions already make.

## Scope

In scope:

- Storage, management API and the call endpoint.
- `jeff classifiers` CLI and `jeff ask --classifier`.
- The Classifiers page.

Out of scope:

- Cascading question renames into classifiers.
- Per-question overrides inside a classifier beyond the model.

## Data

`jeff.json` gets a `classifiers` field next to `questions`:

```json
"classifiers": {
  "support-triage": { "questions": ["urgent", "team", "tone"], "model": "clm/clm-latest" }
}
```

- The classifier key follows the question key rules.
- `questions` is a non-empty array of question keys, without duplicates. Its order is kept.
- `model` is optional. When present, it is a non-empty string.
- Saving does not check that the questions exist. A classifier stays saveable after one of its questions is deleted.
- The field reads with `#[serde(default)]`. Existing configs load unchanged; no migration is needed.

A classifier references questions by key. Editing a saved question changes what every classifier using it asks.
Deleting or renaming a question leaves its key in the classifier; the call skips it.

## Resolving a call

`resolve(classifier, saved_questions) -> (questions_input, skipped)`:

1. Each key that is missing from saved questions, or whose saved question fails `validate`, goes to `skipped`.
2. With no working key left, the call fails with 400 `classifier '<key>' has no working questions`.
3. Without `model`, the working keys become the array `["urgent", "team"]`.
4. With `model`, they become the override map `{"urgent": {"model": "clm/clm-latest"}, ...}`. The saved questions
   keep their own `model`; the override applies to this call only.

The result goes through the existing `expand`, `group`, provider calls and `merge`.

## API

Access matches questions: `/v1` needs a client key, `/api` an admin key.

| Method | Path | Body | Result |
|---|---|---|---|
| GET | `/v1/classifiers` | | Every classifier, as stored |
| GET | `/v1/classifiers/{key}` | | One classifier, as stored; 404 if unknown |
| POST | `/v1/classifiers/{key}` | `{"state": "...", "model"?: "..."}` | The merged answer |
| POST | `/api/classifiers` | Map of key to classifier | All classifiers; an existing key fails and saves nothing |
| PUT | `/api/classifiers/{key}` | One classifier | All classifiers; 404 if unknown |
| DELETE | `/api/classifiers/{key}` | | All classifiers; 404 if unknown |

The call body's `model` has the `/v1/systemone` meaning: the model for questions without their own, used when the
classifier has no override.

The call response is the `/v1/systemone` response plus `"skipped": ["spam"]`. The field is present only when the
list is not empty.

## CLI

The commands edit the config file, like `jeff questions`. A running server picks changes up on the next request.

```
jeff classifiers list [-o file]
jeff classifiers get <key> [-o file]
jeff classifiers add <key> <question>... [--model M]
jeff classifiers update <key> <question>... [--model M]
jeff classifiers remove <key>
```

- `list` prints key, model and questions. A question that is not saved shows as `spam (deleted)`.
- `add` fails on an existing key.
- `update` replaces the classifier. Without `--model`, the override is removed.
- `add` and `update` save a key that is not a saved question and print
  `question 'spam' is not saved; the classifier skips it` to stderr.

`jeff ask --classifier <key> --state ... [--model M]` sends `POST /v1/classifiers/{key}` to a running jeff and prints
the answer JSON. `--classifier` conflicts with positional keys, `--questions` and `--request`. A non-empty `skipped`
also prints `skipped: spam` to stderr.

## UI

A Classifiers tab sits between Questions and Providers. It reuses the Questions page markup and classes.

### List view

Left column:

- Rows follow `questionRow`: a radio picks the classifier to run, then the key and a chip for the override model.
  Below the key, a chip per question. A deleted question's chip is red; its tooltip reads
  `Question "spam" was deleted. The classifier skips it.` An Edit link closes the row.
- Empty state: "No classifiers" with New classifier. With no saved questions, it says "Save questions first".
- Model and State controls as on Questions. State shares its value with the Questions page.
- Run sends `POST /v1/classifiers/{key}`.

Right column: the Questions Answer / JSON / curl panel, rendered by the same `answerCards()`. A non-empty `skipped`
adds a yellow notice above the cards: `1 question skipped: spam`. The curl tab shows the classifier call.

### Form view

- Key: editable on create, read-only on edit, as in the question form.
- Questions: the selected questions in order. Each row shows the key, the type badge and the instructions on a
  second line, as in `questionRow`. A deleted question shows as a red row with a `deleted` badge and the tooltip.
  - Native HTML5 drag and drop reorders rows. Alt+Up and Alt+Down move the focused row.
  - The row's remove button drops it.
- Add question: a search field with a dropdown. It filters by substring of key and instructions and hides questions
  already selected. Each option shows key, type badge and instructions. Arrow keys move, Enter adds, Escape closes.
- Model override: a select with the listed models. The first option is "None (each question uses its own)".
- Footer: Save, Cancel, Delete with the page's two-click confirmation (`confirmClick`), Try.
- Try resolves the draft in the browser, drops deleted questions, and sends the result to `/v1/systemone`. The
  answer renders with `answerCards()` and the same skipped notice. Nothing is saved.

## Errors

| Case | Result |
|---|---|
| Unknown classifier on call, GET, PUT or DELETE | 404 `unknown classifier '<key>'` |
| Invalid classifier on save | 400 with the validation message |
| Every question missing or invalid | 400 `classifier '<key>' has no working questions` |
| Provider failure | As in `/v1/systemone` |

## Testing

- `src/classifiers.rs` unit tests, in the style of `questions.rs`: validate, create, update, remove, and resolve
  with a deleted, an invalid and an overridden question, plus the no-working-questions error.
- Manual UI check with Playwright: create, Run, Try, and a classifier with a deleted question.
