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

## Settings

Set an option in the mod's settings under `/plugin`. Set a variable in the environment Claude Code starts in. A non-empty option wins over its variable.

### Connection

| Option | Variable | Default | Meaning |
|---|---|---|---|
| `url` | `JEFF_URL` | `http://127.0.0.1:8080` | The jeff API the mod asks. |
| `adminUrl` | `JEFF_ADMIN_URL` | `http://127.0.0.1:8081` | The jeff admin page, where the mod saves `skill-gate` once. |
| `apiKey` | `JEFF_API_KEY` | none | The key the mod sends once jeff has access keys. Saving `skill-gate` needs an admin key. Asking needs only a client key. |
| `allowPrivateHttp` | `JEFF_ALLOW_PRIVATE_HTTP=1` | off | Sends the key over plain http to a private IP address. Either the option or the variable turns it on. |

The mod sends the key over https, or over plain http to this machine (`127.0.0.1`, `localhost`, `[::1]`). To any other http address it sends no key, and a jeff with keys refuses the request.

`allowPrivateHttp` adds the private ranges `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10` (Tailscale) and `fc00::/7`. No public certificate authority certifies such an address, so a home network has no easy https. The key travels in plain text, and any device on that network can read it. Turn it on only for a network you trust. Public addresses and host names always need https.

### Connect to jeff on another machine

- **Same machine.** Nothing to set.
- **A PC on your home network.** On the PC, run `jeff keys create --name laptop --role client` and `jeff serve --api 0.0.0.0:8080`, and allow port 8080 in its firewall. On the laptop, set `JEFF_URL=http://<pc-address>:8080`, `JEFF_API_KEY=<the key>` and `JEFF_ALLOW_PRIVATE_HTTP=1`.
- **A remote server.** Put https in front of jeff, for example Caddy with a domain name, and set `JEFF_URL=https://<domain>` and `JEFF_API_KEY`. Or join both machines to Tailscale and use the server's `100.x` address with `JEFF_ALLOW_PRIVATE_HTTP=1`.

Save `skill-gate` once from the machine jeff runs on, where the admin page is reachable. After that the other machines need only `JEFF_URL` and a client key.

### Behavior

| Option | Default | Meaning |
|---|---|---|
| `inject` | `content` | `content` attaches the chosen skill's SKILL.md to the prompt. `suggest` only names the skill, and the model loads it with the Skill tool. |
| `hideListing` | on | Leaves the engine's skill listing out of the main conversation. |
| `rerank` | on | Sends a second request that re-reads the shortlist with each SKILL.md opening and can reject them all. |
| `shortlist` | `3` | How many top-ranked skills the second request re-reads. |
| `gateThreshold` | `0.2` | The mean `skill-gate` answer under which nothing is suggested. |
| `fitsThreshold` | `0.3` | The best "does this fit" answer under which the shortlist is dropped. |
| `excerptChars` | `700` | How many characters of each shortlisted SKILL.md the second request reads. |
| `alwaysListed` | none | Comma-separated skills that stay in the listing whatever the prompt. |
| `neverSuggested` | none | Comma-separated skills jeff is never offered. Typing `/name` still reaches them. |
| `timeoutMs` | `3000` | How long the mod waits for each answer before it lets the prompt through with no suggestion. |
| `logDecisions` | on | Writes each selection to the session log. |

## Tested on

Claude Code 2.1.296, jeff 0.3.0, model `typesafe/openjev-latest`.

## Footprint

```text
hooks: prompt.attachment{type=skill_listing}, prompt.submit, session.end, session.compact, skill.prompt{skill=jeff-skill-suggestion:setup}
gating hook with .catch: prompt.submit
gating hook with .catch: session.compact
calls: $.clock.now, $.clock.sleep, $.command.list, $.env.get, $.fs.exists, $.fs.list, $.fs.read, $.http.fetch, $.session.cwd, $.ui.log
env writes: nothing
env reads: HOME, JEFF_ADMIN_URL, JEFF_ALLOW_PRIVATE_HTTP, JEFF_API_KEY, JEFF_URL
```

## Threat model

- **Reads.** The prompt text, the names and descriptions of every command, the SKILL.md of shortlisted skills under the project's and the user's `.claude/`, `~/.claude/plugins/installed_plugins.json`, and `~/.claude/settings.json` for the setup command. A skill or plugin name holding a path separator or `..` is never spliced into a path.
- **Runs.** No process.
- **Sends.** To jeff at `url` (default `http://127.0.0.1:8080`): the prompt text, every candidate skill's name and description, and up to `excerptChars` of each shortlisted SKILL.md. jeff forwards them to the model it routes the questions to, which may be a cloud provider. To `adminUrl` (default `http://127.0.0.1:8081`): the three `skill-gate` questions and the classifier, once, when jeff has none. The key goes in the `authorization` header only, and never into a log line. It goes over https, or over plain http to this machine. `allowPrivateHttp` or `JEFF_ALLOW_PRIVATE_HTTP=1` also sends it over plain http to a private address, where anyone on that network can read it.
- **Writes.** Nothing to disk itself. The setup command has the model edit `~/.claude/settings.json` and write a backup beside it, each after the user says yes.
- **Persists.** Nothing: state lives in memory and resets on reload.
- **Crafted data.** The prompt reaches jeff only as the JSON `state` string. A crafted skill description reaches jeff as a choice criterion and can sway which skill is suggested; the chosen SKILL.md goes to the model as context it is told it may ignore. A crafted jeff answer can pick any skill on the shortlist, never one outside it.
