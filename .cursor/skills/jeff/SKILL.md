---
name: jeff
description: >-
  Use jeff (Jev classifier gateway) from an agent shell: verify the binary and
  server, discover commands via --help, run classifiers and questions through
  jeff ask or /v1/*, and manage rules with jeff questions/classifiers. Use when
  delegating fast structured decisions (routing, policy, triage) to a cheap
  classifier model, wiring Claude/Cursor/Codex hooks, or working with saved
  questions and classifiers.
---

# jeff for coding agents

jeff is a small binary plus HTTP API. It asks **typed** questions (`noul`,
`choice`, `score`) against Jev-compatible models and returns **JSON
probabilities**, not prose to parse. The main model should treat jeff as a
**decision engine**: pass state text (or later multimodal state), read labels and
scores, then spend reasoning budget only where the classifier is uncertain or
the task needs depth.

Ground truth for flags and subcommands is always **`jeff … --help`** on the
machine you run on. This skill describes workflow; it does not replace help
output after an upgrade.

## Install this skill where the agent loads skills

| Host | Path (copy or symlink this folder) |
| --- | --- |
| Cursor | `.cursor/skills/jeff/` in the project, or a user skills directory |
| Claude Code | `.claude/skills/jeff/` |
| Codex / other agents | Same `SKILL.md` in the product’s skills or rules directory |

Keep **`JEFF_API_KEY`** (and URLs if non-default) in the agent environment, not
in the skill file.

## Preflight (run at the start of every jeff-related task)

Run these in order. Stop on the first hard failure and tell the user what is
missing.

```bash
# 1) Binary on PATH
command -v jeff
jeff --version

# 2) API up (no key required)
curl -sf "${JEFF_URL:-http://127.0.0.1:8080}/health"

# 3) Auth when jeff is locked down (skip if /v1 works without a key)
if [ -n "${JEFF_API_KEY:-}" ]; then
  curl -sf -H "Authorization: Bearer ${JEFF_API_KEY}" \
    "${JEFF_URL:-http://127.0.0.1:8080}/v1/models" -o /dev/null
fi
```

| Outcome | Action |
| --- | --- |
| `jeff` not found | Install from [README](../../../README.md) (release binary or `cargo install`). On dev trees, `cargo build --release` → `./target/release/jeff`. |
| Health fails | Start `jeff` or `jeff serve` (API default `127.0.0.1:8080`). Check `JEFF_URL` / `JEFF_API_ADDR`. |
| `/v1/models` → 401 | Set `JEFF_API_KEY` to a **client** key (`jeff keys create --name … --role client`). |
| Admin CLI fails | Management uses **UI port** default `http://127.0.0.1:8081` (`JEFF_ADMIN_URL`) and an **admin** key. |

Optional repo helper (same checks, JSON summary):

```bash
./scripts/jeff-preflight.sh
```

## Discover capabilities through help (do not guess flags)

```bash
jeff --help
jeff serve --help
jeff ask --help
jeff questions --help
jeff classifiers --help
jeff keys --help
jeff install --help
```

For any subcommand: `jeff help <subcommand>` or `jeff <subcommand> --help`.

After preflight, read [references/api-map.md](references/api-map.md) only if you
need the HTTP ↔ CLI mapping; still confirm odd flags with `--help`.

## Ports, env vars, keys

| Variable | Default | Used by |
| --- | --- | --- |
| `JEFF_URL` | `http://127.0.0.1:8080` | `jeff ask`, client `/v1/*` |
| `JEFF_ADMIN_URL` | `http://127.0.0.1:8081` | `jeff questions`, `jeff classifiers` |
| `JEFF_API_KEY` | none | `Authorization: Bearer` for CLI and curl |
| `JEFF_CONFIG` | `~/.jeff/jeff.json` | `jeff keys`, server `--config` |

| Role | Can call |
| --- | --- |
| **client** | `/v1/*` — classify, list models/questions/classifiers, **run** classifiers |
| **admin** | `/v1/*` and `/api/*` — providers, keys, CLM install, CRUD questions/classifiers |

`jeff keys` edits the config file directly so you can bootstrap the first admin
key before any server accepts `/api`. A running server reloads keys on the next
request.

## Runtime: classify (prefer this during agent turns)

**Saved classifier** (named rule pack):

```bash
jeff ask --classifier support-triage --state "Customer: invoice charged twice"
```

**Saved questions by key**:

```bash
jeff ask urgent department --state "…"
```

**One request body** (inline map, overrides, or draft question):

```bash
jeff ask --request request.json
# or
jeff ask --questions '{"spam":{"type":"noul","instructions":"…"}}' --state "…"
```

**Pipe large state** (avoid shell quoting limits):

```bash
jeff ask --classifier triage --state-file context.txt
cat context.txt | jeff ask --classifier triage --state-file -
```

**Stable output path** for hooks parsing stdout:

```bash
jeff ask --classifier triage --state "…" -o /tmp/jeff-answer.json
```

Classifier calls hit `POST /v1/classifiers/{key}` under the hood. The JSON
response matches `/v1/systemone` plus optional `"skipped": ["question-key"]`
when a classifier references deleted or invalid questions.

### Reading answers (act on structure, not narrative)

| `type` | Decide using |
| --- | --- |
| `noul` | `"noul"` probability (threshold e.g. 0.7 yes / 0.3 no — tune per question) |
| `choice` | `"choice"` label; use `"confidence"` and `"probabilities"` when borderline |
| `score` | `"score"` (expected level); use `"probabilities"` when borderline |

Response headers (curl `-i`): `x-jeff-provider`, `x-jeff-upstream-ms` — use
upstream ms to spot slow providers during hook budgets.

### HTTP equivalents (when shell has no `jeff`)

```bash
API="${JEFF_URL:-http://127.0.0.1:8080}"
AUTH=()
[ -n "${JEFF_API_KEY:-}" ] && AUTH=(-H "Authorization: Bearer ${JEFF_API_KEY}")

curl -sf "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"state":"…"}' "$API/v1/classifiers/support-triage"
```

Try an unsaved classifier: `POST /v1/classifiers` with
`{"classifier":{…},"state":"…"}`. See [references/api-map.md](references/api-map.md).

## Setup: questions and classifiers (admin)

Do not edit `jeff.json` by hand unless recovering from corruption. Use CLI or
`/api/*`.

```bash
jeff questions list
jeff questions get urgent
jeff questions add new-questions.json    # map key → question; `-` = stdin
jeff questions update urgent one.json
jeff questions remove urgent

jeff classifiers list
jeff classifiers get support-triage
jeff classifiers add support-triage urgent team tone --model clm/clm-latest \
  --description "Routes incoming tickets"
jeff classifiers add --file classifiers.json
jeff classifiers update support-triage urgent team --description "…"
jeff classifiers remove support-triage
```

Question shape (for `add` / inline `--questions`): `type` +
`instructions`; `choice` needs `criteria` object (≥2 keys); `score` needs
`criteria` array (≥2 levels). Optional per-question `model`.

Classifier shape: non-empty `questions` array (ordered keys), optional `model`
override for all questions in that run, optional `description` (for humans/agents
browsing the list).

## Agent integration patterns

### Delegate vs reason

1. **Before** a costly tool loop: run one classifier on the user message +
   recent context (`--state-file`).
2. If probabilities are decisive, follow the label (route, allow/deny, pick
   template).
3. If borderline, ask the main model or a human once, then optionally **save** a
   new question via admin CLI.

### Cursor

Load this skill when the task mentions jeff, classifiers, routing, triage, or
Jev. Use the terminal for `jeff ask`; keep `-o` for machine-readable artifacts.

### Claude Code (hooks / subagents)

Typical hook flow: build state string → `jeff ask --classifier … -o …` → parse
JSON → inject a short system line (“triage: billing, confidence 0.91”) or gate
tool use. Keep hook timeouts above `jeff ask --max-time` (default 60s) plus
provider latency; use a fast local model (`clm/clm-latest`) for sub-second hooks
when CLM is warm.

Use a **client** key in the hook environment. Reserve **admin** keys for setup
sessions only.

### Codex and other shell agents

Same commands; export `JEFF_URL` and `JEFF_API_KEY`. Prefer `--request` or
`-o` over parsing table output from `list` subcommands.

## Errors

CLI prints `error[code]: message` and exits 1. Common cases:

| Signal | Meaning |
| --- | --- |
| `cannot connect to http://127.0.0.1:8081` | Start jeff; admin CLI needs UI/API server. |
| `error[config_invalid]: …` | Fix `jeff.json`; server may keep previous config while invalid. |
| `log: jeff.log` | Details on disk next to config — read for validation errors. |
| HTTP 401 / 403 | Wrong or missing key; client key cannot call `/api/*`. |
| HTTP 409 | Config invalid or changed on disk; fix and retry. |
| `skipped` in classifier response | Update classifier question list or restore questions. |

## Server lifecycle (when you manage jeff, not only call it)

```bash
jeff serve                  # API :8080, UI :8081
jeff serve --no-ui          # API only
jeff install clm            # local GPU provider (Docker)
jeff remove clm
jeff keys list
jeff keys create --name cursor-agent --role client
```

See `jeff serve --help` for `JEFF_API_ADDR`, `JEFF_UI_ADDR`, exposure rules
(non-loopback bind requires keys).

## Tests and docs in this repo

- User-facing overview: [README](../../../README.md)
- Classifiers design: [docs/superpowers/specs/2026-10-09-classifiers-design.md](../../../docs/superpowers/specs/2026-10-09-classifiers-design.md)
- CLI-via-API behavior: [docs/superpowers/specs/2026-10-09-cli-via-api-design.md](../../../docs/superpowers/specs/2026-10-09-cli-via-api-design.md)
