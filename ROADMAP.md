# Roadmap

## Security, before a public release

Found in a read-only audit against the OWASP API Security Top 10 (2023).

- [x] **Key roles.** A `client` key reaches `/v1/*` only. An `admin` key also reaches `/api/*`: providers, keys and the CLM install.
- [x] **Provider key on URL change.** Changing a provider URL drops its saved key, so the key cannot be redirected to another host.
- [x] **SSRF guard.** Do not follow redirects from providers.
- [x] **SSRF guard.** Refuse link-local and cloud-metadata addresses (`169.254.0.0/16`, `fd00:ec2::254`).
- [x] **SSRF guard.** Allow private networks only with an explicit flag.
- [x] **SSRF guard.** Parse provider URLs with a real URL parser.
- [x] **Safe binding.** Refuse to listen on a non-loopback address while no access key exists, for the API and the admin page alike.
- [x] **Config writes.** Write the config atomically: a temporary file, then a rename.
- [x] **Config errors.** Refuse to start on a broken config instead of falling back to the defaults.
- [x] **Key ids.** Make key ids 128-bit, so a valid key holder cannot guess and revoke other keys.
- [x] **Request size.** Set an explicit request body limit.
- [ ] **Public info.** Limit the unauthenticated `/api/info` to what the admin page needs.

## Security, later

- [ ] **Provider keys at rest.** Encrypt them, or read them from a secret store.
- [ ] **Request rates.** Limit the rate and concurrency of successful requests per key. Today only failed key checks are throttled.
- [ ] **CLM head format.** Load the CLM head from safetensors or a hash-pinned file. Today `clm-serve` downloads a pickle (`.pt`) without a hash check.
- [ ] **CLM image.** Bake CLM into its own image pinned by digest, instead of running `pip install` on every container start.

## Distribution

- [x] **Prebuilt binaries.** Every release carries binaries for Linux, macOS and Windows.
- [ ] **1.0.** Remove the `"breaking": true` → `minor` rule from `package.json`: it holds only while versions are 0.x.
- [ ] **Installers.** One-line shell and PowerShell installers, and a Homebrew tap.
- [ ] **npm / npx.** Publish an npm package that dispatches the matching prebuilt binary, allowing one-off execution without a persistent global install (for example, `npx --yes jeff@<version> --help`). Support non-interactive CI usage, version pinning and reuse of the package-manager cache; if the wrapper downloads a binary separately, cache and verify that binary too. Document platform support and CI cache setup.
- [ ] **PyPI / uvx.** Publish platform wheels containing the binary, for example with maturin in `bin` mode, allowing one-off execution without a persistent global install (for example, `uvx --from jeff==<version> jeff --help`). Support non-interactive CI usage, version pinning and reuse of the uv cache. Document platform support and CI cache setup.
- [ ] **Container image publish.** Publish an OCI image of jeff on a registry.
- [ ] **Container sidecar deployment guide.** Document optional local sidecar use.
- [ ] **Container team server deployment guide.** Document shared automation server use.
- [ ] **Container CI deployment guide.** Document CI use of the image.
- [ ] **Shell-first agent clients.** Document `jeff ask` as the primary integration path for coding agents.
- [ ] **HTTP integrations without a terminal.** Document calling the same API when the host has no shell.
- [ ] **MCP adapter.** Expose classify operations over MCP without changing jeff core.
- [ ] **MCP catalog adapter.** Expose classifiers and questions catalog operations over MCP without changing jeff core.

## Agent integration

jeff as an optional accelerator for coding agents (like a linter for decisions): routing, moderation, tool choice, priority, human-in-the-loop hints, soft bias — not a replacement for the main model's reasoning. If jeff is down, slow, or low-confidence, the agent decides next steps (including escalation to a human) without jeff blocking the session.

- [ ] **Agent skill for Cursor.** Ship `.cursor/skills/jeff/` in this repository.
- [ ] **Agent skill for Claude Code.** Ship the Claude Code skills path in this repository.
- [ ] **Agent skill for Codex.** Ship Codex-oriented skill instructions in this repository.
- [ ] **Agent skill preflight.** Document optional preflight: binary or Docker endpoint, `GET /health`.
- [ ] **Agent skill session classifiers catalog.** Document loading the classifiers catalog once per session.
- [ ] **Agent skill session questions catalog.** Document loading the questions catalog once per session.
- [ ] **Agent skill CLI discovery.** Document using `jeff --help` for flags instead of memorizing them.
- [ ] **Agent skill saved classifiers.** Document `jeff ask --classifier` for saved rules.
- [ ] **Agent skill fixed classifier keys.** Document hard-coded classifier keys where the project requires them.
- [ ] **Agent skill ad-hoc questions.** Document `jeff ask --questions` to trial rules before saving them.
- [ ] **Agent skill static state prefix.** Document a stable static block at the start of `state`.
- [ ] **Agent skill dynamic state tail.** Document the changing tail of `state` after the static prefix.
- [ ] **Agent skill stable repeat payloads.** Document keeping repeat requests byte-identical when possible for provider-side cache.
- [ ] **Agent skill key handling.** Document that the agent calls `jeff`; keys live in jeff's environment only.
- [ ] **Agent skill no admin default.** Document that coding agents must not use admin keys by default.
- [ ] **Session-start prompts for jeff.** Document recommended first-turn requests for agents outside the jeff binary.
- [ ] **Claude hooks adapter.** Ship hook recipes that consume JSON from `jeff ask -o`.
- [ ] **Cursor host patterns.** Ship turn-level patterns that consume the same JSON.
- [ ] **Load project questions.** Load saved questions from the repository or launch directory.
- [ ] **Load project classifiers.** Load saved classifiers from the repository or launch directory.
- [ ] **Load project policies.** Load policy files from the repository or launch directory.
- [ ] **Fallback global providers.** Use `~/.jeff/jeff.json` providers when the project does not define them.
- [ ] **Fallback global keys.** Use global access keys when the project does not define them.
- [ ] **Fallback global defaults.** Use global defaults such as default model when the project does not define them.
- [ ] **Project vs global merge rules.** Specify how project keys overlap with global keys.
- [ ] **Version classifiers in git.** Store classifier definitions in the project repository.
- [ ] **Version questions in git.** Store question definitions in the project repository.
- [ ] **Sync repo artifacts to shared jeff.** Publish project rules to the team jeff server.
- [ ] **CI project classifiers.** Run project classifiers in CI against the same artifacts as local dev.
- [ ] **Policy threshold value.** Store a numeric or probability threshold on a policy rule.
- [ ] **Policy threshold event.** Store whether a rule emits `warning` or `error`.
- [ ] **Policy cap description.** Store agent-facing text explaining why a threshold exists.
- [ ] **Policy soft-cap.** Emit a warning when a soft threshold fails; the agent still decides.
- [ ] **Policy hard-cap.** Emit an error when a hard threshold fails.
- [ ] **Policy hard-cap CLI exit code.** Non-zero exit from `jeff ask` on hard-cap in programmatic mode.
- [ ] **Policy hard-cap HTTP status.** Non-success HTTP status on hard-cap in programmatic mode.
- [ ] **Policy response envelope.** Define JSON shape for jeff answers plus policy results (TBD).
- [ ] **Classifier-bound policy fields.** Attach policy thresholds to a specific classifier answer field.
- [ ] **Decision logs.** Record classification requests and outcomes for review.
- [ ] **Quality metrics.** Measure classifier behavior against agreed criteria.
- [ ] **Policy quality metrics.** Measure policy cap behavior against agreed criteria.
- [ ] **Review workflows.** Support human review of logged decisions.
- [ ] **A/B experiment documentation.** Document A/B rollouts of classifiers and policies.
- [ ] **Flag experiment documentation.** Document flag-style rollouts of classifiers and policies.
- [ ] **Fast classifier rollout.** Enable quick rollout of classifier changes in shared deployments.
- [ ] **Fast classifier rollback.** Enable quick rollback of classifier changes in shared deployments.
- [ ] **Fast policy rollout.** Enable quick rollout of policy changes in shared deployments.
- [ ] **Fast policy rollback.** Enable quick rollback of policy changes in shared deployments.
- [ ] **Per-question model choice.** Pick the best model per question for each call.
- [ ] **Single-provider batching.** Batch questions that share a provider into one upstream call where possible.
- [ ] **Model benchmarks endpoint.** Surface fixed answer sets on large samples with human confirmation.
- [ ] **Model usage statistics endpoint.** Surface usage stats to guide model choice.
- [ ] **Learned routing from classifier stats.** Optional routing layer trained on classifier usage (later).
- [ ] **Learned routing from question stats.** Optional routing layer trained on per-question usage (later).
- [ ] **Agent integration for Deepseek harness.** Extend the skill pattern to Deepseek harness (later).
- [ ] **Agent integration for Hermes.** Extend the skill pattern to Hermes (later).
- [ ] **Agent integration for Pi.** Extend the skill pattern to Pi (later).
- [ ] **Agent integration for OpenCode.** Extend the skill pattern to OpenCode (later).
- [ ] **Agent integration for Gemini CLI.** Extend the skill pattern to Gemini CLI (later).

## Product

- [ ] **Documentation reorganization.** Rewrite the README for a new user: a short explanation of the product, key features, supported platforms, the main installation paths, `jeff --help` / `jeff --version`, and one complete quick-start use case from provider setup to a classification request and its expected result. Keep common commands, model discovery/selection and essential access-key guidance easy to find, aiming to cover most initial and routine user needs. Put links to documentation, downloads and contributing in the header. Create `CONTRIBUTING.md` for development setup, builds, checks, commit/PR conventions and releases. Move detailed provider/model guides, local CLM setup, API and CLI reference, configuration, deployment/security and troubleshooting into a navigable `docs/` directory with an index and links back to related guides. Preserve useful existing content without duplicating full reference material in the README, and verify examples and relative links. Use the local whispio README as a layout reference, and adapt useful patterns from [uv](https://github.com/astral-sh/uv/blob/main/README.md), [Ollama](https://github.com/ollama/ollama/blob/main/README.md) and [FastAPI](https://github.com/fastapi/fastapi/blob/master/README.md) to this project's user journey.
- [ ] **Built-in provider and model identity.** Give predefined providers and Jev-compatible models immutable canonical ids and names, with model identity scoped to its provider. Reserve built-in ids so Custom entries cannot impersonate them. Enforce these rules in the configuration API as well as the UI. Define which connection fields remain editable: cloud presets must not be repointed to a different service under the same identity; local presets may need configurable addresses. Keep credentials editable and use Custom for independently configured Jev-compatible servers.
- [ ] **OpenJev / Codiv preset.** Add a separate entry to the provider selector for [OpenJev](https://github.com/razorback16/openjev) hosted by Codiv (`https://api.codiv.ai`, model `openjev-latest`; [dashboard](https://codiv.ai/dashboard) for account setup). Keep its provider and model identity separate from TypeSafe Jev, even though it supports the same wire API and accepts compatibility aliases such as `jev-latest`. Allow self-hosted OpenJev through Custom.
- [ ] **OpenJev / DiffusionGemma 26B-A4B.** Add predefined model configuration and setup guidance for classification with `openjev-0.1` and its `openjev-latest` alias through OpenJev, including text and image inputs. Document self-hosted vLLM (NVIDIA) and MLX (Apple silicon) setup. Treat `diffusiongemma-26b` as the same weights exposed for text generation, not another classification model.
- [ ] **OpenJev / Verdict.** Add predefined model configuration and setup guidance for `verdict-1.4` (`heman10x/rlcd-modernbert-151m`, ModernBERT-base + GLiClass, 151M), including self-hosted PyTorch CPU/GPU setup. Document text-only input, the 512-token read limit and truncation, the 24-choice limit, and OpenJev's probability renormalization after removing Verdict's built-in insufficient-evidence option.
- [ ] **OpenJev / JevK5.** Add predefined model configuration and setup guidance for `jevk5-0.2` (`alibiserikbay/JevK5`, distilled Qwen3.5-4B), including self-hosted vLLM setup. Document text-only input, the 16,384-token read limit with rejection of oversized inputs, and multiple inference passes for more than 16 options.

Build on Jeff's existing Jev-compatible discovery and request forwarding, using the connected server's `/v1/models` rather than assuming every model is hosted by Codiv. Keep OpenJev identities separate from TypeSafe Jev and document model-specific availability and request limits. Model ids and backends were checked against OpenJev [configuration](https://github.com/razorback16/openjev/blob/75f22b6dad8c360fdba0e0ebd3dc0a1187628f60/openjev/config.py), [deployment](https://github.com/razorback16/openjev/blob/75f22b6dad8c360fdba0e0ebd3dc0a1187628f60/docker-compose.yml) and [model documentation](https://github.com/razorback16/openjev/blob/75f22b6dad8c360fdba0e0ebd3dc0a1187628f60/README.md#models).

- [x] **Classifiers.** Named groups of saved questions, run in one call, with an optional model override, through the API, the CLI and the Classifiers page.
- [ ] **Multimodal state shape.** Design request bodies for static plus dynamic state parts before adding modalities beyond text.
- [ ] **Multimodal cache-friendly payloads.** Design request bodies so provider-side caching can reuse stable prefixes.
- [ ] **Multimodal state agent skill docs.** Document multimodal state conventions in the agent skill.
- [ ] **Modality capability matrix.** Record which models support which modalities.
- [ ] **Refuse unsupported modality.** Return a clear error when state uses a modality the model does not support.
- [ ] **Per-model modality limits.** Show formats, size, and count limits per model in the admin UI.
- [ ] **State completeness over speed.** Prefer full context over latency when they conflict, within the 16 MB request body limit.
- [ ] **Image state on questions.** Accept an image as state for a single question.
- [ ] **Image state on classifiers.** Accept an image as state for a classifier call.
- [ ] **Image state HTTP API.** Accept image state through the HTTP API.
- [ ] **Image state CLI file.** Accept image state through a file argument in `jeff ask`.
- [ ] **Image state admin upload.** Accept image state through upload on the Classifiers page.
- [ ] **Image state admin upload on Questions.** Accept image state through upload on the Questions page.
- [ ] **Audio input.** The same for audio, as its own feature: record which models hear audio, refuse it for the others, and document formats, length and size limits per model. Accept it through the API, a file in the CLI and an upload in the UI.
- [ ] **Video input.** After images and audio, and only where a model supports it: the heaviest input, with the strictest limits on length and size.
- [ ] **Admin page sign-in.** A way to enter a key in a browser that did not create it.
- [ ] **Recent unsaved questions.** List questions asked through Try, `jeff ask` or inline `/v1/systemone` maps but not saved, newest first, so a good question can be saved or used as a template in one click instead of typed again.
- [ ] **Streaming mode.** Deliver each model's answers as they arrive, through the API and `jeff ask`, instead of waiting for the slowest model.
- [ ] **Uninstall confirmation.** The two-click confirmation used by Revoke, for CLM Uninstall.
- [ ] **CLM quality.** Measure FP8 against bf16, for example on the "app crashes" ticket that CLM routes to sales.
