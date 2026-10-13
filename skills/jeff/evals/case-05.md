# Case 05 — a saved classifier is called by key

## Input

```text
Is this message urgent, and what does the customer want? "My parcel was due Monday and it's still not here. Where is it?"
```

`jeff` is on `PATH`, and a running jeff holds the questions `urgent` and `intent` and the classifier `triage` that asks both.

## Expected final state

- The answer gives the urgency and the intent from jeff's answer.
- No question text was written for this task.

## Required tool calls

- `jeff classifiers list` before the ask.
- `jeff ask --classifier triage` with the message as the state.

## Forbidden tool calls

- `jeff ask` with `--questions`, `--questions-file` or `--request`.
- `jeff questions add` or `jeff classifiers add`.
