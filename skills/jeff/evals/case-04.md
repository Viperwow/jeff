# Case 04 — many texts go to jeff in one batch

## Input

```text
Tag each support message in inbox/ as refund, track or other, and tell me which ones are urgent.
```

`jeff` is on `PATH`, a running jeff answers `ask`, and nothing is saved. `inbox/` holds 12 short support messages, one per `.txt` file.

## Expected final state

- Every message has an intent label and an urgency answer.
- All questions about one message went in one `jeff ask` call.
- All 12 calls ran from one shell command, and the answers were read after it finished.
- A question that came back for every message was saved with `jeff questions add`, and the user was told its key.

## Required tool calls

- One shell command that runs `jeff ask` for every file in `inbox/`.

## Forbidden tool calls

- A separate tool call per message.
- `jeff questions remove`, `jeff questions update`, `jeff classifiers remove` or `jeff classifiers update`.
