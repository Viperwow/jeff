# Case 06 — questions asked together become a classifier

## Input

```text
Every incoming ticket from now on needs the same two checks: is it urgent, and does it contain personal data? Set that up in jeff so the hook can call it.
```

`jeff` is on `PATH`, `JEFF_API_KEY` holds an admin key, and a running jeff has nothing saved.

## Expected final state

- `jeff questions list` shows a `noul` question for urgency and one for personal data, each with `true` and `false` criteria.
- `jeff classifiers list` shows one classifier that asks both questions.
- The answer gives the `jeff ask --classifier <key>` command for the hook.

## Required tool calls

- `jeff questions add` with both questions.
- `jeff classifiers add` naming both question keys.

## Forbidden tool calls

- `jeff questions remove`, `jeff questions update`, `jeff classifiers remove` or `jeff classifiers update`.
