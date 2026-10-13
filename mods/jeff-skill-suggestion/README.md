# jeff-skill-suggestion

## What it does

Hides the skill listing from Claude Code and, for each prompt, asks jeff which one skill (if any) fits, attaching that skill's instructions to the prompt.

Install it in Claude Code 2.1.287 or later, with jeff running:

```text
/plugin install jeff-skill-suggestion --marketplace Viperwow/jeff
```

On the first prompt the mod saves the `skill-gate` classifier in jeff, with three yes/no questions: `skill-gate-acts-on-system`, `skill-gate-follows-procedure` and `skill-gate-beyond-prose`. A question already saved under one of these keys is kept as it is. Edit the questions, add more to the classifier, or give it a `model` on jeff's admin page: the mod asks whatever `skill-gate` holds, and every yes counts towards a skill.

The yes/no answers decide whether a skill is suggested, so the model behind `skill-gate` matters. Measured on four prompts with one right skill each, `typesafe/openjev-latest` gave the right skill a `fits` near 1.00 and the others near 0.00. `clm/clm-latest` rated wrong skills higher than the right one. The mod asks both of its requests on jeff's default model, set on the Providers page. A `model` on the `skill-gate` classifier overrides it for this mod alone.

A jeff with no access keys lets the mod save the classifier with no key. A jeff with keys needs an admin key in `JEFF_API_KEY`, or in the mod's `apiKey` option, to save it once; a client key is enough after that.

`/jeff-skill-suggestion:setup` hides every user skill from the model's listing, so `/context` counts them at 0. `/jeff-skill-suggestion:setup restore` undoes it.

## Tested on

Claude Code 2.1.296, jeff 0.3.0, model `typesafe/openjev-latest`.

## Footprint

```text
hooks: prompt.attachment{type=skill_listing}, prompt.submit, session.end, session.compact, skill.prompt{skill=jeff-skill-suggestion:setup}
gating hook with .catch: prompt.submit
gating hook with .catch: session.compact
calls: $.clock.now, $.clock.sleep, $.command.list, $.env.get, $.fs.exists, $.fs.list, $.fs.read, $.http.fetch, $.session.cwd, $.ui.log
env writes: nothing
env reads: HOME, JEFF_ADMIN_URL, JEFF_API_KEY, JEFF_URL
```

## Threat model

- **Reads.** The prompt text, the names and descriptions of every command, the SKILL.md of shortlisted skills under the project's and the user's `.claude/`, `~/.claude/plugins/installed_plugins.json`, and `~/.claude/settings.json` for the setup command. A skill or plugin name holding a path separator or `..` is never spliced into a path.
- **Runs.** No process.
- **Sends.** To jeff at `url` (default `http://127.0.0.1:8080`): the prompt text, every candidate skill's name and description, and up to `excerptChars` of each shortlisted SKILL.md. jeff forwards them to the model it routes the questions to, which may be a cloud provider. To `adminUrl` (default `http://127.0.0.1:8081`): the three `skill-gate` questions and the classifier, once, when jeff has none. The key goes in the `authorization` header only, and never into a log line.
- **Writes.** Nothing to disk itself. The setup command has the model edit `~/.claude/settings.json` and write a backup beside it, each after the user says yes.
- **Persists.** Nothing: state lives in memory and resets on reload.
- **Crafted data.** The prompt reaches jeff only as the JSON `state` string. A crafted skill description reaches jeff as a choice criterion and can sway which skill is suggested; the chosen SKILL.md goes to the model as context it is told it may ignore. A crafted jeff answer can pick any skill on the shortlist, never one outside it.
