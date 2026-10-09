# CLI via the API, broken config handling and form drafts

## Goal

The HTTP API is the reference for jeff's behavior. `jeff questions` and `jeff classifiers` become clients of a running
jeff, like `jeff ask`, instead of editing `jeff.json` directly. A broken `jeff.json` never starts jeff and never
replaces a working config at runtime. Unsaved form input in the admin page survives a reload.

## Out of scope

- `jeff keys` keeps editing `jeff.json` directly: it creates the first admin key before any server can accept one.
- Log rotation for `jeff.log`.
- `/api/info` stays as it is.

## Broken `jeff.json`

### What counts as broken

The rule is all or nothing. The file is broken when either:

- it is not valid JSON, or does not parse into the config shape;
- an entry has an invalid structure: a bad key (`"a b"`), a wrong field type, an unknown field (`modle`), an empty
  `questions` list, or duplicate questions.

A classifier that names a deleted question is not broken. That is normal state, however the question was deleted:
by hand in the file, in the UI, through the API or through the CLI. Deletion does not cascade. The classifier skips
the question and reports it in `skipped`. A classifier with no working questions answers its calls with
"classifier 'x' has no working questions".

Validation collects every problem in the file, not only the first one.

### Start

A broken file stops jeff before it listens. The console shows one short error and the full log path, and the
process exits with code 1:

```
error[config_invalid]: jeff.json has 2 problems
log: C:\Users\viper\.jeff\jeff.log
```

The full path appears only on the server console, which belongs to the server owner.

### Reload while running

jeff rereads `jeff.json` when its modification time changes. A broken file is refused as a whole, and jeff keeps
the last working config: providers, keys, questions and classifiers alike. A key revoked in a broken file therefore
stays valid until the file is fixed.

- The console shows one line: `error[config_invalid]: jeff.json has 2 problems; jeff keeps the previous config`.
- Classification through `/v1/*`, the UI, `jeff ask` and curl keeps working on the last working config.
- Every change saved through the API answers `409`: questions, classifiers, providers and keys:

  ```json
  {"code":"config_invalid","error":"jeff.json has 2 problems; fix it before changing questions or classifiers","log":"jeff.log"}
  ```

  jeff saves the whole file, so saving now would overwrite the hand edits.

- Once the file is fixed, the next reread applies it and changes work again.

API responses name the log file only, never a server path.

### `jeff.log`

- `jeff.log` lives next to the config file: `~/.jeff/jeff.log` by default, or the `--config` directory.
- Each entry is appended with a UTC timestamp.
- Three events are logged: problems at start, problems on reload, and the fix (`jeff.json fixed; config reloaded`).

```
2026-10-09T10:42:17Z config_invalid C:\Users\viper\.jeff\jeff.json: 2 problems; jeff keeps the previous config
  questions."a b": key must be 1-64 characters of A-Z, a-z, 0-9, '_' and '-'
  classifiers.triage: unknown field 'modle'; a classifier has questions, model and description
2026-10-09T10:45:03Z config_fixed C:\Users\viper\.jeff\jeff.json fixed; config reloaded
```

### `GET /api/status`

- An admin-only route on the UI port.
- Before it answers, jeff rereads the file if its modification time changed.

```json
{"config":"ok"}
{"config":"invalid","file":"jeff.json","problems":2,"log":"jeff.log"}
```

## Classifier description

- A classifier may carry `description`, a non-empty string of up to 1000 characters. It does not change how the
  classifier runs.

  ```json
  {"support-triage": {"description": "Routes incoming tickets", "questions": ["refund", "team"]}}
  ```

- A classifier has `questions`, `model` and `description`. Any other field is an unknown field.
- The classifier form has an optional **Description** under **Key**. The list shows it under the key.
- `jeff classifiers add|update` take `--description TEXT`. `classifiers list` gains a `description` column.

## Admin page

### Broken config banner

The page asks `/api/status` when it opens, and after every `409 config_invalid`. While the config is invalid, a
banner shows on every page:

> **jeff.json has 2 problems.** jeff keeps running on the previous config. Log: `jeff.log` — **Recheck**

**Recheck** asks `/api/status` again. The banner disappears once the file is fixed.

A failed **Save** shows the API error under the form:
"jeff.json has 2 problems; fix it before changing questions or classifiers. Log: `jeff.log`". The form keeps its
input.

### Deleted question in a classifier

The row of a deleted question:

- has a red background (`red-50`);
- shows its key in red, without strikethrough;
- shows a red `deleted` tag;
- shows the red caption "The classifier skips this question";
- has the tooltip `Question "spam" was deleted. The classifier skips it.`;
- has a red ✕ that removes it from the classifier.

The questions count shows `· 1 deleted`.

### Form drafts

- Every form with **Save** keeps a draft in `localStorage`: the question and classifier forms, and the Providers and
  Keys forms.
- A draft is keyed by its form and record, for example `draft:classifier:new` or `draft:classifier:triage`.
- A draft is written on every field change.
- It is restored after a page reload, a tab switch or a failed save. The form then shows **Draft restored** and a
  **Discard** button.
- A successful **Save**, **Cancel** or **Discard** removes the draft.
- Provider keys and created access keys are never written to the browser.
- When `localStorage` is unavailable, forms work as today, without drafts.

## CLI as an API client

### Connection

- `jeff questions` and `jeff classifiers` call the management port.
- They take `--url`, env `JEFF_ADMIN_URL`, default `http://127.0.0.1:8081`.
- Changes (`/api/*`) exist on that port only, so reads use it too.
- `JEFF_API_KEY` is sent as `Authorization: Bearer`, as `jeff ask` sends it.
- `--config` is removed from both commands.

### Commands

| CLI | API |
|---|---|
| `questions list` | `GET /v1/questions` |
| `questions get KEY` | `GET /v1/questions/KEY` |
| `questions add FILE` | `POST /api/questions` |
| `questions update KEY FILE` | `PUT /api/questions/KEY` |
| `questions remove KEY` | `DELETE /api/questions/KEY` |
| `classifiers list` | `GET /v1/classifiers` |
| `classifiers get KEY` | `GET /v1/classifiers/KEY` |
| `classifiers add KEY Q... [--model M] [--description D]` | `POST /api/classifiers` |
| `classifiers add --file FILE` | `POST /api/classifiers`, the body as is |
| `classifiers update KEY Q... [--model M] [--description D]` | `PUT /api/classifiers/KEY` |
| `classifiers update KEY --file FILE` | `PUT /api/classifiers/KEY`, the body as is |
| `classifiers remove KEY` | `DELETE /api/classifiers/KEY` |

- `FILE` is a path, or `-` for stdin.
- `classifiers add --file` takes a JSON map of key to classifier, like `questions add`.
- `classifiers update --file` takes one classifier.
- `--file` conflicts with the positional questions, `--model` and `--description`.

### Output

- `list` prints the table it prints today.
- `get`, `add` and `update` print the response JSON.
- `-o FILE` writes that JSON to a file.
- `remove` prints nothing on success.

### Errors

Any non-2xx response prints the error and exits with code 1:

```
error[config_invalid]: jeff.json has 2 problems; fix it before changing questions or classifiers
log: jeff.log
```

- Without `code`, the line is `error: <error>`.
- `log:` appears only when the response has it.
- When jeff is not running, the CLI prints `cannot connect to http://127.0.0.1:8081; start jeff first` and exits
  with code 1.

## Testing

Every scenario runs three ways, and the results must match:

- a JSON file through the CLI;
- CLI flags;
- curl against the API.

Scenarios:

- start with a broken file;
- breaking the file while jeff runs;
- a change while the file is broken: UI, curl and CLI;
- classification while the file is broken;
- fixing the file and **Recheck**;
- a deleted question in a classifier;
- a draft surviving a reload.

Unit tests cover:

- collecting every problem;
- refusing a broken file on reload;
- `409 config_invalid`;
- the `/api/status` answers;
- appending to `jeff.log`.
