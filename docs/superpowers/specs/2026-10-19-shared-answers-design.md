# Shared answers

## Goal

Keep a library of answers that many questions reuse, such as `insufficient_context` and `uncertain`, and add them
to a question with one click instead of typing them. Editing an answer in the library updates every question that
uses it.

Shared answers are an authoring aid of jeff's admin tools. The questions and classifiers API does not change:
a saved question stays a plain Jev question with every option written out in `criteria`. Providers, `/v1`
callers and the answer cache see nothing new.

## Scope

In scope:

- Storage and the management API `/api/answers`.
- `jeff answers` CLI.
- The Answers page and the answer chips in the question editor.

Not in jeff: stock answers. The library starts empty on every install, and jeff never adds answers by itself.

Out of scope:

- Any new field in a question or a classifier. A question never stores a reference to an answer.
- Shared answers for `noul` questions. Their two answers are fixed.
- Renaming an answer. Add a new one and remove the old one, as with questions.
- Reordering the levels of a `score` question.
- Converting options of saved questions to shared answers. A question uses an answer as soon as it matches it
  (see Matching).

PR #30, the `insufficient_context` / `uncertain` checkbox, is closed in favour of this design.

## Data

`jeff.json` gets an `answers` field next to `questions`. An example library:

```json
"answers": {
  "insufficient_context": { "text": "Not enough context to answer.", "types": ["choice"], "default": true },
  "uncertain": { "text": "Not sure about the answer.", "types": ["choice"], "default": true }
}
```

- The answer key follows the question key rules: `A-Z`, `a-z`, `0-9`, `_` and `-`, up to 64 characters.
- `text` is a non-empty string after trimming.
- `types` is a non-empty array of `choice` and `score`, without duplicates.
- `default` is optional and false when absent. A new question in the editor starts with every default answer
  whose `types` hold the question's type.
- No two answers that allow `score` share a text, since `score` matching goes by text alone. A save that would
  break this is refused: `Answer 'b' has the same text as 'a'. Change one of the texts.`
- A save refuses any other field, in the classifiers' form: `Unknown field 'txt'. Keep only text, types and
  default.` A `questions` field is the exception: it is ignored, so the output of `jeff answers get` can be
  edited and sent back as is.
- A bad entry in a hand-edited `jeff.json` stops `jeff serve`, as a bad saved question does.
- `PUT /api/config` keeps `answers`, as it keeps questions and classifiers.

### Loading

The field reads with `#[serde(default)]`: a config without `answers` loads an empty library, and no saved
question changes. jeff adds no answers on its own, so nothing the owner removed comes back.

A downgrade to a jeff without `answers` drops the field on its next config write, as it drops `classifiers`
today. Questions keep their options, since they hold plain text. This is accepted.

An "I don't know" answer belongs to `choice` only. On a `score` scale an answer is a level, and the expected level
is an average over the scale; such a level would shift that average. The README says so where it shows the
example library.

## Matching

A question uses an answer when it holds the answer word for word, ignoring spaces around the text:

- `choice`: `types` holds `choice`, and `criteria` has the answer's key with exactly the answer's text.
- `score`: `types` holds `score`, and `criteria` has a level with exactly the answer's text.

There is no stored link. A question whose matching option was edited by hand stops using the answer. Adding an answer to
the library makes every question that already holds it word for word use it at once.

`uses(answer, questions) -> Vec<question key>` lives in core next to the questions code, so the API, the CLI and
the UI count usage the same way.

## Changing an answer

Update replaces `text`, `types` and `default` in one config write, and in the same write rewrites every question
that used the answer before the change:

- `choice`: `criteria[key]` gets the new text.
- `score`: the level equal to the old text gets the new text, in place.
- A question no longer covered by the new `types` keeps its option as written and stops using the answer.
- If a `score` question that uses the answer already has another level equal to the new text, the whole update is
  refused and nothing is written: `Question 'q' already has the level 'Not sure.'. Change the text or edit q.`

The response names the rewritten questions. The answer cache needs no work: its key is the body sent to the
provider, so a changed text misses the old entries.

Remove deletes the answer from the library only. Questions keep the option as plain text.

## API

Admin key, admin port, like `/api/questions`.

| Method | Path | Body | Result |
|---|---|---|---|
| GET | `/api/answers` | | `200` map of key to answer, each with a computed `questions` array of the keys that use it |
| GET | `/api/answers/{key}` | | `200` one answer with `questions`; `404` when missing |
| POST | `/api/answers` | map of key to answer | `201` the whole library; `409` when a key exists; one bad entry saves nothing |
| PUT | `/api/answers/{key}` | one answer | `200` `{ "answer": {...}, "questions": ["k1", "k2"] }`, the rewritten questions; `404` when missing |
| DELETE | `/api/answers/{key}` | | `200` the whole library, as classifiers do; `404` when missing |

Errors follow the house rule, what happened then what to do: `Answer 'uncertain' does not exist. List the answers
with jeff answers list.`

## CLI

`jeff answers` calls the API above, with the same `--url`, `JEFF_ADMIN_URL` and admin `JEFF_API_KEY` as
`jeff questions`:

- `list`: one line per answer: key, types, how many questions use it, text.
- `get KEY`: the answer as JSON, with `questions`.
- `add FILE`: a JSON map of key to answer, or `-` for stdin. An existing key fails and saves nothing.
- `update KEY FILE`: replaces one answer and prints `Updated 3 questions: a, b, c.` or `Updated no questions.`
- `remove KEY`.

## UI

### Answers page

A new nav entry, Answers, next to Questions and Classifiers, in the same list-and-form layout as Classifiers:

- The list shows the key, type chips, the text, `Used in N questions` and a Default mark.
- The form edits text, the `choice` and `score` checkboxes and Default. The key is read-only when editing.
- Saving an edit reports `Updated N questions.` in the form status and reloads the question list. A question open
  in the editor is not touched; saving it afterwards writes what the editor shows, as with any other concurrent
  edit today.
- Delete asks for confirmation and says that N questions keep the option as plain text.

### Question editor

Below the options of a `choice` or `score` question, a row of chips lists every answer whose `types` hold the
question's type. `noul` shows no row.

- A chip is on when the question uses that answer.
- Turning a chip on appends the answer: a `choice` row with the answer's key and text, or a `score` level with
  its text at the end of the scale.
- Turning it off removes that row.
- A row that matches an answer is grey, read-only and tagged `shared`. Its remove button works like the chip.
- A new question starts with the default answers of its type on. Changing the type keeps today's blank rows and
  adds the defaults of the new type after them.
- A chip whose key clashes with a hand-typed option of different text is disabled, with the title
  `Option 'uncertain' is already used. Rename it to add the shared answer.`

The saved question is plain Jev, exactly as if the rows were typed.

## Docs

README gets a Shared answers section: the library, matching, update rewriting questions, the example library
and the CLI commands. The `answers` config field is listed with the other config fields.

## Testing

- Core unit tests: matching for `choice` and `score`, update rewriting only matching questions, `types` narrowing,
  remove leaving questions untouched, validation messages.
- Loading: a config written by the previous version loads with an empty library and keeps every question byte
  for byte.
- API tests for each endpoint and status.
- CLI tests in the style of the existing `questions` CLI tests.
- UI checked in a browser on a temporary jeff: chips on and off for `choice` and `score`, grey rows, a new
  question with defaults, an edit in the library updating an open question list, delete keeping options.

## Left to the builder

- Module layout: a new `answers.rs` beside `questions.rs` and `classifiers.rs` fits the existing pattern.
- Exact Tailwind classes, following the Classifiers page.
- Ordering of the list: insertion order, as for questions.
