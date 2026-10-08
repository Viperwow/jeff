# Saved questions

## Goal

Store Jev questions on the jeff server and reuse them by key. A client runs saved questions without sending their
definitions again. Each question may name its own model, so cheap questions go to a local model and important ones
to a paid model.

The entity takes the name of the native Jev field: `questions`. A saved question has exactly the native shape that
`/v1/systemone` accepts, plus an optional `model`.

## Scope

In scope:

- Storage, management API and reuse in `/v1/systemone`.
- Per-question model routing.
- The Questions page, which replaces the Playground page.

Out of scope, each in its own later PR:

- A Classifier entity: a named set of question keys with its own model.
- Links to skills.
- Benchmarks, question revisions, calibration.

## Data

`jeff.json` gets a `questions` field. It maps a key to a question:

```json
"questions": {
  "urgency": { "type": "noul", "instructions": "The message conveys urgency or time pressure." },
  "department": {
    "type": "choice",
    "instructions": "Which team should handle this?",
    "criteria": { "billing": "Charges, invoices, refunds", "technical": "Bugs and outages" },
    "model": "typesafe/jev-latest"
  }
}
```

A missing field loads as an empty map. `build_config` keeps `questions` from the current config, the same way it
keeps `keys`, so saving providers never drops questions.

A question is valid when:

- The key matches `^[A-Za-z0-9_-]{1,64}$`. The key appears in URL paths.
- `type` is `noul`, `choice` or `score`.
- `instructions` is a non-empty string.
- `choice` has `criteria` as an object with at least two entries.
- `score` has `criteria` as an array with at least two entries.
- `model`, when present, is a non-empty string. It is not checked against the provider list: a provider may be
  offline when the question is saved.

Other fields pass through unchanged, so newer Jev fields need no jeff change.

## API

| Method and path | Role | Effect |
|---|---|---|
| `GET /v1/questions` | client | Returns the saved map. |
| `GET /v1/questions/{key}` | client | Returns one question, or `404`. |
| `PUT /api/questions` | admin | Upserts every question in the body map. Other saved keys stay. Returns the full saved map. |
| `DELETE /api/questions/{key}` | admin | Deletes one question, or `404`. |

`PUT` validates the whole body before writing. One invalid question rejects the request with `400`, names the key
and saves nothing.

Clients read questions because an agent needs the catalogue to pick from. Only admins change it.

## Running questions

`/v1/systemone` accepts `questions` in two forms:

- A map, as today. Inline questions may carry `model`.
- An array of saved keys, for example `["urgency", "department"]`. jeff replaces it with the saved map before
  routing. An unknown key returns `400` and names the key.

Model routing per question, first match wins:

1. The question's `model`.
2. The request's `model`.
3. `default_model`.

jeff removes `model` from each question before forwarding, so providers receive plain Jev questions.

When all questions resolve to one model, the request follows the current path unchanged.

When they resolve to several models, jeff groups the questions by model and sends one request per group in
parallel, each with the same `state` and other request fields. It then merges the results:

- `answers` is the union of the group answers, keyed by question.
- `usage` sums each numeric field across groups.
- `model` lists the group models, comma-separated.
- Other top-level fields come from the first group's response.
- `x-jeff-provider` lists the providers, comma-separated.
- `x-jeff-upstream-ms` is the slowest group's time.

If any group fails, jeff returns that group's error status and body, and names its provider. It returns no partial
answers.

## UI

The Questions page replaces Playground in the navigation.

List view:

- The left panel lists saved questions. Each row shows the key, the type badge, the question's model when set, and
  the instructions. Each row has a checkbox, Edit and Delete.
- Delete asks for a second click, like Revoke on Access keys.
- Below the list: Model (for questions without their own), State and Run.
- Run sends `{"state": …, "model": …, "questions": [checked keys]}`.
- The right panel shows Answer, JSON and curl, as Playground does today. Each answer card shows the model that
  answered it.
- An empty list shows "No saved questions" with New question and Add examples. Add examples saves the three
  current Playground examples.

Form view, opened by New question or Edit:

- The left panel holds today's question builder for one question, plus a Model select with a Default entry.
- On Edit the key is read-only. Renaming is a new question plus Delete.
- The footer holds Save and Cancel on the left and Try on the right.
- The right panel holds State and the Try answer. Try sends the draft as an inline map; nothing is saved.
- Save sends `PUT /api/questions` with the one question and returns to the list.

Every `select.control` drops the native arrow (`appearance: none`) and draws one chevron with even right padding,
so selects look the same on every page and platform.

`localStorage` keeps State and the run Model only. The Playground question draft goes away.

## Errors

- Validation errors return `400` with the key and the broken rule, in the existing `error` JSON shape.
- The UI shows API errors in the status line next to the action that failed.

## Testing

Rust unit tests in `src/main.rs`:

- Question validation: each rule above, valid and invalid.
- Key expansion: saved keys become the map; an unknown key fails.
- Grouping: questions split by question model, request model and default model; `model` is stripped.
- Merging: answers from two groups merge; one failing group fails the whole response.
- `build_config` keeps `questions`.

Manual check in the UI against CLM: save, edit, delete, run with one model, run with two models, Try a draft.
