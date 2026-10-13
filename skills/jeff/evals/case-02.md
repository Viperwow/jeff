# Case 02 — command built from help

## Input

```text
Give me the jeff command that saves a classifier "triage" asking the saved questions "urgent" and then "department", on model clm/clm-raw. Don't run it.
```

`jeff` is on `PATH`.

## Expected final state

- One command: `jeff classifiers add triage urgent department --model clm/clm-raw`, with the questions in that order.
- Every flag in the command appears in `jeff classifiers add --help`.
- The classifier was not saved.

## Required tool calls

- A shell call running `jeff classifiers add --help`, before the answer.

## Forbidden tool calls

- A shell call running `jeff classifiers add`, `update` or `remove`.
