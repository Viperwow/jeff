---
name: jeff
description: Drive the jeff CLI on the user's machine — check that jeff is installed, discover what the installed version can do, run its commands, and hand it typed decisions. Use whenever the user mentions jeff, a jeff classifier or saved question, typed yes/no, choice or score questions, Jev or System One models, CLM, or wants to ask, list, add, update or remove questions, classifiers, keys or providers through jeff, even when they do not name the CLI. Also use when a task needs typed decisions over text — fixed labels, yes/no, a rating on a scale — that should be cheap or come in bulk, and jeff is installed.
---

# jeff

jeff is a CLI and server that asks Jev-compatible models typed questions and returns a probability for every answer. Learn the commands from the installed binary, not from memory: versions differ, and `help` is always current.

## 1. Check jeff once per session

At the start of a session, and again after it resumes, run both once:

```sh
jeff --version
jeff --help
```

Do not run either again in this session. Their output stays valid.

- `--version` prints `jeff X.Y.Z` → installed. Keep the `--help` command list for the session.
- Command not found → stop and tell the user. Offer an install, and run it only after they agree:
  - Prebuilt binary: <https://github.com/Viperwow/jeff/releases/latest>, then put `jeff` on `PATH`.
  - With Rust: `cargo install --git https://github.com/Viperwow/jeff --locked`.

## 2. Read a command's help only when this skill does not show it

```sh
jeff <command> --help
```

Run it once, before the first call of a command this skill does not show. Build that call from its output only: a flag missing from help does not exist in this version.

## 3. Run the command

- Read input from a file, or from stdin with `-`, when a flag accepts it. Inline JSON in a shell argument breaks on quoting.
- Use `-o FILE` when the answer is needed later. Otherwise read stdout.
- Exit code `0` means success. Any other code means failure: show stderr to the user verbatim and do not retry blindly.
- `ask`, `questions` and `classifiers` talk to a running jeff. On a connection error, tell the user to start it with `jeff` or `jeff serve`, or to point `--url` at theirs.
- The key comes from `JEFF_API_KEY` in the environment. Never print, log or pass a key on the command line.

## 4. Hand typed decisions to jeff in one batch

A Jev-compatible model answers a typed question for a fraction of what reasoning it out costs, and its probabilities say how sure it is. Hand a decision to jeff when all of these hold:

- The answer set is fixed: yes/no (`noul`), one of several options (`choice`), or a level on a scale (`score`).
- The answer is read from the text: intent, routing, tool choice, a fact the text states, a policy check, a severity level.
- Reasoning it out would cost more than the decision is worth, or many decisions of the same kind are due.

Keep the decision yourself when it needs date math, arithmetic, probabilities, weighing trade-offs, or chaining facts across a long text. Decision models are weakest there.

Every round trip to jeff costs you a turn. Collect all the decisions the task needs first, send them in one go, then read all the answers at once. Never ask one question per turn.

1. Take saved classifiers and questions first (step 5). Write every remaining question into one JSON map:

   ```json
   {
     "intent": { "type": "choice", "instructions": "Which intent does the message express?",
                 "criteria": { "refund": "Wants money back", "track": "Asks where an order is" } },
     "urgent": { "type": "noul", "instructions": "Does the message ask for action today?",
                 "criteria": { "true": "The sender needs action today", "false": "The request can wait" } },
     "anger":  { "type": "score", "instructions": "How angry is the customer?",
                 "criteria": ["Calm", "Annoyed", "Very angry"] }
   }
   ```

   - `noul`: ask a yes/no question, and say in `criteria` what `true` and `false` mean. The criteria raise accuracy.
   - `choice`: keep every option the task allows, with the task's own wording, and describe each option.
   - `score`: list the levels from lowest to highest.
2. Send the batch. Add `--model` only when the user named one.
   - One text: one call with every question.

     ```sh
     jeff ask --questions-file q.json --state-file state.txt
     ```

   - Many texts: one shell command that asks about each text and writes every answer to its own file.

     ```sh
     for f in states/*.txt; do jeff ask --questions-file q.json --state-file "$f" -o "answers/$(basename "$f" .txt).json"; done
     ```

     When a few calls fail, rerun only those.
3. Read the answers: `choice` with `confidence`, `noul` as the probability of yes, `score` with `confidence`. Return each label exactly as the task spelled it.
4. Below 0.5 confidence, or when jeff fails, judge that case yourself and say that jeff was unsure or unavailable.

## 5. Bake repeated checks into questions and classifiers

A check that comes back is a standard check: save it once, then call it by key. A key keeps the question text out of your context, and one classifier key replaces a whole question map. A saved question also keeps its text the same to the character, and models that cache answers, such as CLM, answer a repeat in milliseconds without input tokens. Saving needs an admin key in `JEFF_API_KEY`.

Before writing any question, look up what is saved. Run both lists once per session:

```sh
jeff classifiers list
jeff questions list
```

Then call the narrowest thing that covers the check, in this order:

1. A classifier that asks exactly the questions you need:

   ```sh
   jeff ask --classifier triage --state-file state.txt
   ```

2. Saved questions by key, all in one call:

   ```sh
   jeff ask urgent intent --state-file state.txt
   ```

3. A new question map from step 4, only for what nothing saved covers.

Save as you go:

- Save a new question when the same check can come again: in a hook, a script, another text of this batch, or later in this session. Prefer saving over asking it inline. Use the JSON map from step 4:

  ```sh
  jeff questions add q.json
  ```

- Join saved questions that are asked together into a classifier, and call the classifier from then on:

  ```sh
  jeff classifiers add triage urgent intent --description "Routes support mail"
  ```

- Name keys after the check, not the task: `urgent`, `refund_intent`, `pii_present`.

`add` never touches an existing key: it fails and saves nothing. Tell the user which keys you saved.

## 6. Confirm before changing state

Ask the user before any command that removes, revokes, replaces or installs something: `remove`, `revoke`, `update`, `install`, `keys create`. Read-only commands — `--version`, `--help`, `list`, `get`, `ask` — need no confirmation. Neither does `questions add` or `classifiers add`: they only create new keys.
