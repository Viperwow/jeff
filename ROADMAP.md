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
- [ ] **Installers.** One-line shell and PowerShell installers, and a Homebrew tap.
- [ ] **npm.** `npx jengine`: an npm package that fetches the matching prebuilt binary.
- [ ] **PyPI.** `uvx jengine`: a PyPI wheel built with maturin in `bin` mode.
- [ ] **Container image.** A jengine container image for servers.

## Product

- [ ] **Admin page sign-in.** A way to enter a key in a browser that did not create it.
- [ ] **Uninstall confirmation.** The two-click confirmation used by Revoke, for CLM Uninstall.
- [ ] **CLM quality.** Measure FP8 against bf16, for example on the "app crashes" ticket that CLM routes to sales.
