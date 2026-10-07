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
- [ ] **Container image.** A jeff container image for servers.

## Product

- [ ] **Documentation reorganization.** Rewrite the README for a new user: a short explanation of the product, key features, supported platforms, the main installation paths, `jeff --help` / `jeff --version`, and one complete quick-start use case from provider setup to a classification request and its expected result. Keep common commands, model discovery/selection and essential access-key guidance easy to find, aiming to cover most initial and routine user needs. Put links to documentation, downloads and contributing in the header. Create `CONTRIBUTING.md` for development setup, builds, checks, commit/PR conventions and releases. Move detailed provider/model guides, local CLM setup, API and CLI reference, configuration, deployment/security and troubleshooting into a navigable `docs/` directory with an index and links back to related guides. Preserve useful existing content without duplicating full reference material in the README, and verify examples and relative links. Use the local whispio README as a layout reference, and adapt useful patterns from [uv](https://github.com/astral-sh/uv/blob/main/README.md), [Ollama](https://github.com/ollama/ollama/blob/main/README.md) and [FastAPI](https://github.com/fastapi/fastapi/blob/master/README.md) to this project's user journey.
- [ ] **Built-in provider and model identity.** Give predefined providers and Jev-compatible models immutable canonical ids and names, with model identity scoped to its provider. Reserve built-in ids so Custom entries cannot impersonate them. Enforce these rules in the configuration API as well as the UI. Define which connection fields remain editable: cloud presets must not be repointed to a different service under the same identity; local presets may need configurable addresses. Keep credentials editable and use Custom for independently configured Jev-compatible servers.
- [ ] **OpenJev / Codiv preset.** Add a separate entry to the provider selector for [OpenJev](https://github.com/razorback16/openjev) hosted by Codiv (`https://api.codiv.ai`, model `openjev-latest`; [dashboard](https://codiv.ai/dashboard) for account setup). Keep its provider and model identity separate from TypeSafe Jev, even though it supports the same wire API and accepts compatibility aliases such as `jev-latest`. Allow self-hosted OpenJev through Custom.
- [ ] **OpenJev / DiffusionGemma 26B-A4B.** Support classification with `openjev-0.1` and its `openjev-latest` alias through OpenJev, including text and image inputs. Document self-hosted vLLM (NVIDIA) and MLX (Apple silicon) setup. Treat `diffusiongemma-26b` as the same weights exposed for text generation, not another classification model.
- [ ] **OpenJev / Laya.** Support `laya-1.0` (`convaiinnovations/laya-typed-decisions`, ModernBERT-large, 421M), including self-hosted PyTorch CPU/GPU setup. Document text-only input, the 1,024-token read limit and truncation, and the shared option-token budget.
- [ ] **OpenJev / Verdict.** Support `verdict-1.4` (`heman10x/rlcd-modernbert-151m`, ModernBERT-base + GLiClass, 151M), including self-hosted PyTorch CPU/GPU setup. Document text-only input, the 512-token read limit and truncation, the 24-choice limit, and OpenJev's probability renormalization after removing Verdict's built-in insufficient-evidence option.
- [ ] **OpenJev / CLM.** Support `clm-v0.1` (`Contrastive-LM/CLM-v0.1-8B` heads over Qwen3-8B) through OpenJev, alongside Jeff's existing CLM integration. Document self-hosted vLLM setup, FP8 and bf16 weights, text-only input, the 2,048-token read limit and truncation, and the known score-question limitation.
- [ ] **OpenJev / JevK5.** Support `jevk5-0.2` (`alibiserikbay/JevK5`, distilled Qwen3.5-4B), including self-hosted vLLM setup. Document text-only input, the 16,384-token read limit with rejection of oversized inputs, and multiple inference passes for more than 16 options.

For each model, enable discovery, selection and classification requests through Jeff, using the connected server's `/v1/models` rather than assuming every model is hosted by Codiv. Keep OpenJev identities separate from TypeSafe Jev and document model-specific availability and request limits. Model ids and backends were checked against OpenJev [configuration](https://github.com/razorback16/openjev/blob/75f22b6dad8c360fdba0e0ebd3dc0a1187628f60/openjev/config.py), [deployment](https://github.com/razorback16/openjev/blob/75f22b6dad8c360fdba0e0ebd3dc0a1187628f60/docker-compose.yml) and [model documentation](https://github.com/razorback16/openjev/blob/75f22b6dad8c360fdba0e0ebd3dc0a1187628f60/README.md#models).
- [ ] **Admin page sign-in.** A way to enter a key in a browser that did not create it.
- [ ] **Uninstall confirmation.** The two-click confirmation used by Revoke, for CLM Uninstall.
- [ ] **CLM quality.** Measure FP8 against bf16, for example on the "app crashes" ticket that CLM routes to sales.
