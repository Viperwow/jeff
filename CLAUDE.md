# jeff

## Logging

- jeff.log holds one readable line per event: what happened, which provider, and the cause.
- A payload too large for one line, such as a provider answer, goes to `logs/<id>.<ext>` next to jeff.log, byte for byte as received.
- `<id>` is the SHA-256 of the payload, so the same payload is saved once and every repeat points to the same file.
- The jeff.log line names that file by its path relative to jeff.log, so a reader goes from the line to the full text in one step.
- A test of a log line splits it into its fixed text and its variable parts. It checks fixed text exactly and each variable part by its property: present, non-empty, the right shape. An OS error is checked as non-empty, never by its wording.
- A log test writes its entry among many others, single-line and multi-line, and finds it by its fixed text, never by its position or by the log holding one line.
