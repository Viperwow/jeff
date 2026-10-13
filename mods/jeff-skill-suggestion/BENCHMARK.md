# jeff-skill-suggestion benchmark

The same prompts went through Claude Code with the mod and without it, and each run's skill choice, cost, context and time were compared.

## Setup

- Claude Code 2.1.296, model Opus 5.5, jeff 0.3.0.
- `skill-gate` on `typesafe/openjev-latest`; mod defaults `gateThreshold` 0.2, `fitsThreshold` 0.3, `timeoutMs` 2000.
- 122 skills installed; `/jeff-skill-suggestion:setup` not run.
- 10 prompts, each run 3 times with the mod and 3 times without it: 60 runs.
- Each run is one `claude -p PROMPT --output-format stream-json --verbose`, with `--plugin-dir mods/jeff-skill-suggestion` for the mod's runs, in an empty directory.
- A run chose right when the expected skill was loaded: by the Skill tool without the mod, by the mod's injection or the Skill tool with it. A prompt that needs no skill chose right when nothing was loaded.

## Results

With the mod, Claude chose the right skill in 90% of runs instead of 50%, at 40% lower cost. On the 7 prompts that needed a skill, it chose right in 18 of 21 runs (86%) instead of 6 of 21 (29%).

| Metric | Without the mod | With the mod | Change |
|---|---|---|---|
| Right skill chosen | 15 of 30 | 27 of 30 | +12 |
| Cost, 30 runs | $5.13 | $3.07 | −40% |
| Context of a one-turn run | 39,589 tokens | 31,287 tokens | −21% |
| Tokens written to the cache, per run | 18,441 | 10,308 | −44% |
| Model turns, 30 runs | 73 | 62 | −11 |
| Time, 30 runs | 258 s | 306 s | +19% |

## By prompt

Each cell is right choices of 3, then the mean cost of a run.

| Prompt | Expected skill | Without the mod | With the mod | Need score with the mod |
|---|---|---|---|---|
| Draw a mermaid sequence diagram of an OAuth login | `mermaid-diagrams` | 0/3, $0.137 | 3/3, $0.055 | 0.32–0.34 |
| Write a Conventional Commits message for this change: added retries … | `caveman:caveman-commit` | 0/3, $0.065 | 3/3, $0.042 | 0.32 |
| Review this Rust function for idiomatic issues: … | `rust-best-practices` | 0/3, $0.135 | 2/3, $0.086 | 0.28–0.31 |
| Which saved classifiers does my jeff have? Use the jeff CLI. | `jeff` | 3/3, $0.223 | 3/3, $0.131 | 0.66–0.67 |
| Is there an installable agent skill that can help me automate Slack? … | `find-skills` | 3/3, $0.314 | 2/3, $0.270 | 0.59–0.60 |
| Rewrite this landing page line so it does not sound AI-generated: … | `kill-ai-slop` or `humanizer` | 0/3, $0.135 | 3/3, $0.131 | 0.26–0.33 |
| Make a 3-slide Slidev deck about Rust ownership … | `slidev` | 0/3, $0.195 | 2/3, $0.126 | 0.35–0.41 |
| What is a monad? Answer in two sentences. | none | 3/3, $0.130 | 3/3, $0.009 | 0.03 |
| Explain the difference between TCP and UDP in three bullets. | none | 3/3, $0.189 | 3/3, $0.073 | 0.00 |
| Translate 'good morning, see you tomorrow' into French and Spanish. | none | 3/3, $0.187 | 3/3, $0.100 | 0.02–0.03 |

## Findings

- Without the mod, Claude loaded a skill for 2 of the 7 prompts that needed one.
- With the mod, the right skill was chosen in every run where jeff answered: 23 of 23.
- The 3 misses with the mod were runs where jeff did not answer within 2000 ms, so the prompt went through with no suggestion. jeff timed out in 7 of 30 runs; the mod now waits 3000 ms by default.
- Prompts that needed a skill scored 0.26–0.67 on Need; prompts that did not scored 0.00–0.03. The default `gateThreshold` of 0.2 sits between them.
- The mod adds the wait for jeff to every prompt: about 1–3 s.

## Limits

- Each `claude -p` run starts a new session, so the skill listing is written to the cache at full price. In a long interactive session it is read from the cache, and the saving per turn is smaller.
- Runs followed each other within minutes, so some shared a warm cache; single-run costs, such as $0.009, are noisy.
- 10 prompts and 3 runs each are a guide, not a statistic.
