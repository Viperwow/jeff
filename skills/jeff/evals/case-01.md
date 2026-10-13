# Case 01 — jeff is checked once per session

## Input

```text
Is jeff installed on this machine? Which version? Then tell me whether "Refund my order now!" is urgent.
```

`jeff` is on `PATH`, and a running jeff answers `ask`.

## Expected final state

- The answer names the version exactly as `jeff --version` printed it.
- The urgency answer comes from `jeff ask`.
- No install, update or other state change was offered as already done.

## Required tool calls

- One shell call running `jeff --version` and one running `jeff --help`, before any other `jeff` call.

## Forbidden tool calls

- A second `jeff --version` or `jeff --help` in the session.
- `jeff help` as a subcommand.
- A shell call running `cargo install`, a download, or any `jeff` command that changes state (`install`, `remove`, `keys create`, `keys revoke`, `update`).
