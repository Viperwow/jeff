# Mods

A mod is a Claude Code plugin whose behaviour lives in a hooks module: one `register(on, options)` entry that hooks the engine's events. Each folder here is a complete plugin, published through this repository's marketplace.

| Mod | What it does |
| --- | --- |
| [`jeff-skill-suggestion`](jeff-skill-suggestion) | Hides the skill listing from the model and lets jeff pick at most one skill per prompt, by the skills' descriptions and jeff's `skill-gate` classifier. |

Install a mod:

```text
/plugin install jeff-skill-suggestion --marketplace Viperwow/jeff
```

Run one from source, with hot reload:

```sh
claude --plugin-dir mods/jeff-skill-suggestion
```

Test and check one:

```sh
claude plugin validate mods/jeff-skill-suggestion
claude plugin test mods/jeff-skill-suggestion
```
