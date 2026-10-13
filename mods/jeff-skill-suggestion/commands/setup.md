---
description: Hand skill selection to jeff-skill-suggestion — hide every skill from the model's listing (user-invocable-only) so /context counts them at 0; "restore" undoes it
argument-hint: "[restore]"
disable-model-invocation: true
allowed-tools: Read, Write, Edit, Bash(rm ~/.claude/jeff-skill-suggestion.skill-overrides.backup.json)
---
Mode: $ARGUMENTS

If this text is all you can see, the jeff-skill-suggestion mod is not loaded: it rewrites this command at run time with the real list of skills and the exact settings change. Tell the user to install it with `/plugin install jeff-skill-suggestion@jeff` (Claude Code 2.1.287 or newer) in a trusted project, or to start Claude Code with `--plugin-dir mods/jeff-skill-suggestion`, and to run this command again. Do not edit any settings file.
