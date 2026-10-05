# jengine

One API for typed decisions, in front of every Jev-compatible model.

Your agent or service asks typed questions: yes/no (`noul`), pick one (`choice`) or rate on a scale (`score`). jengine sends them to the provider you choose and returns the answers. The providers it knows out of the box:

- **CLM**: Contrastive-LM, runs on your GPU, installed by jengine.
- **TypeSafe Jev**: cloud.
- **Laya, PipeLLM** or any other server with `POST /v1/systemone`.

jengine runs as one small binary. It serves:

- **The API**, on port 8080 by default: what your code calls.
- **The admin page**, on port 8081 by default: providers, a playground and access keys.

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Call the API](#call-the-api)
- [Providers and models](#providers-and-models)
- [Local CLM](#local-clm)
- [Access keys](#access-keys)
- [Run on a server](#run-on-a-server)
- [Command reference](#command-reference)
- [Build from source](#build-from-source)

## Install

### Prebuilt binary

Download the archive for your system from the [latest release](https://github.com/Viperwow/jengine/releases/latest):

| System | Archive |
|---|---|
| Linux x64 | `jengine-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `jengine-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple silicon | `jengine-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `jengine-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| Windows x64 | `jengine-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

Unpack it and put `jengine` (`jengine.exe` on Windows) in a folder on your `PATH`. On Linux, for example:

```sh
tar -xzf jengine-*-x86_64-unknown-linux-gnu.tar.gz
sudo mv jengine-*/jengine /usr/local/bin/
```

`SHA256SUMS` in the same release lists the checksum of every archive. Check yours with `sha256sum -c SHA256SUMS --ignore-missing`.

macOS blocks a downloaded binary that Apple has not notarized. Allow it once with `xattr -d com.apple.quarantine /usr/local/bin/jengine`.

### With Cargo

With the [Rust toolchain](https://rustup.rs) installed:

```sh
cargo install --git https://github.com/Viperwow/jengine --locked
```

`cargo` builds jengine and puts it in `~/.cargo/bin`. Run the same command with `--force` to update.

### Check

```sh
jengine --version
```

## Quick start

```sh
jengine
```

This starts the API on `http://127.0.0.1:8080` and the admin page on `http://127.0.0.1:8081`. Open the admin page.

1. **Providers.** CLM and TypeSafe Jev are listed already.
   - Paste your TypeSafe key, or click **Install** on CLM to run it locally.
   - Each provider shows **Online** when jengine can reach it.
   - Click **Save**.
2. **Playground.** Pick a model, write the state and the questions, then click **Run** or press Ctrl+Enter.
   - The **curl** tab shows the same request for your own code.

jengine keeps its settings in `~/.jengine/jengine.json` (`%USERPROFILE%\.jengine\jengine.json` on Windows).

## Call the API

```sh
curl http://127.0.0.1:8080/v1/systemone \
  -H 'content-type: application/json' \
  -d '{
    "model": "clm/clm-latest",
    "state": "Customer: my invoice was charged twice and nobody answers the phone!",
    "questions": {
      "urgent":     { "type": "noul",   "instructions": "The message conveys urgency." },
      "department": { "type": "choice", "instructions": "Which team should handle this?",
                      "criteria": { "billing": "Charges and refunds", "technical": "Bugs and outages" } },
      "anger":      { "type": "score",  "instructions": "How angry is the customer?",
                      "criteria": ["Calm", "Annoyed", "Very angry"] }
    }
  }'
```

The answer follows the Jev schema:

- `noul` returns `{ "noul": 0.97 }`, the probability of yes.
- `choice` returns `choice`, `confidence` and `probabilities` for every option.
- `score` returns `score` (the expected level), `confidence`, `legend` and `probabilities` for every level.

jengine adds two response headers:

- `x-jengine-provider`: the provider that answered.
- `x-jengine-upstream-ms`: how long the provider took, in milliseconds.

Other endpoints:

| Endpoint | What it returns |
|---|---|
| `GET /v1/models` | Every `provider/model` and the status of each provider. |
| `GET /health` | `{"ok": true}`. It needs no key, so it suits liveness probes. |

## Providers and models

Name a model as `provider/model`, for example `typesafe/jev-latest`.

A bare `jev-latest` goes to the first provider that lists it. A request without `model` gets the **Default model** set on the Providers page.

To add a provider, open **Providers**, pick a preset or **Custom server** under **Add provider**, fill in the URL and the key, and click **Save**.

Provider rules:

- **Models.** jengine asks each provider for its models. If a provider has no `GET /v1/models`, list its models by hand on its card.
- **Keys over the network.** A provider key goes to a remote host only over `https://`. Plain `http://` with a key is accepted only for `localhost`.

## Local CLM

CLM runs in Docker on an NVIDIA GPU with at least 16 GB of memory. On Windows it needs Docker Desktop with WSL2.

Install CLM in one of two ways:

- Click **Install** on the CLM card.
- Run `jengine install clm`.

jengine starts Docker if it is not running, then downloads and starts everything. The steps and their progress show on the card.

| Step | Download |
|---|---|
| vLLM image | about 10 GB |
| Qwen3-8B weights | 16.4 GB |
| CLM head | 76 MB |

Downloads happen once. Later starts take about a minute.

Stop CLM with **Uninstall** or `jengine remove clm`. The downloaded weights stay on disk for the next install.

## Access keys

Without keys jengine accepts every request. Keep it that way only on your own machine.

Once one key exists, every `/v1` and `/api` request on both ports needs `Authorization: Bearer <key>`. Requests without a valid key get `401`. After 20 failures in a minute, the address gets `429` until the minute ends.

Each key has a role:

| Role | Reaches |
|---|---|
| `client` | `/v1`: decisions and models |
| `admin` | `/v1` and `/api`: providers, keys and the CLM install, so the admin page too |

A client key on `/api` gets `403`. The first key must be an admin key, and the last admin key cannot be revoked while client keys remain, so the admin page never locks itself out.

Manage keys on the **Access keys** page or from the terminal:

```sh
jengine keys create --name ops --role admin --expires never
jengine keys create --name claude-agent             # a client key, valid 30 days
jengine keys create --name ci --expires 2027-01-31  # through the end of that day, UTC
jengine keys list
jengine keys revoke 3f9a1c2e0b7d4a6f9e8c1b2a3d4e5f60
```

`create` prints the key once. jengine stores only its SHA-256 hash, so a lost key cannot be recovered: create a new one.

A running server picks up changes from the terminal on the next request.

The admin page sends a key too. The first key you create on that page is remembered by your browser, so the page keeps working after the lock turns on.

Keys that never expire can also come from the environment. They are admin keys, and each must have at least 32 characters:

```sh
JENGINE_API_KEY="$(openssl rand -hex 32)" jengine
```

## Run on a server

Choose where each part listens:

```sh
jengine serve --api 0.0.0.0:8080 --ui 127.0.0.1:8081
jengine serve --api 9000 --no-ui        # a bare port means 127.0.0.1
```

The same settings come from the environment:

| Flag | Variable | Default |
|---|---|---|
| `--api` | `JENGINE_API_ADDR` | `127.0.0.1:8080` |
| `--ui` | `JENGINE_UI_ADDR` | `127.0.0.1:8081` |
| `--config` | `JENGINE_CONFIG` | `~/.jengine/jengine.json` |
| `--api-key` | `JENGINE_API_KEY` | none; separate several keys with commas |
| `--allowed-host` | `JENGINE_ALLOWED_HOSTS` | none; separate several names with commas |
| `--allow-private-providers` | `JENGINE_ALLOW_PRIVATE_PROVIDERS` | off; lets providers use addresses such as `10.0.0.0/8` and `192.168.0.0/16` |
| | `JENGINE_HOME` | `~/.jengine`; holds the config and the CLM compose file |

jengine does not start on a broken config, so a damaged file cannot silently drop the access keys. Fix or remove the file.

Providers may use loopback and public addresses. Private networks need `--allow-private-providers`. Link-local and cloud metadata addresses such as `169.254.169.254` are always refused, and jengine does not follow redirects from providers. Changing a provider URL drops its saved key, so enter the key again for the new URL.

Before you open a port to a network:

- **Create a key.** jengine refuses to listen on a non-loopback address while no key exists.
- **Put TLS in front.** jengine speaks plain HTTP. Run it behind a reverse proxy such as Caddy or nginx, and set HSTS there.
- **Keep the admin page private.** It edits providers and keys and starts Docker. Bind it to `127.0.0.1` and reach it through an SSH tunnel (`ssh -L 8081:127.0.0.1:8081 server`), or turn it off with `--no-ui`.
- **Name the proxy.** The admin page answers only to `localhost` and IP addresses. Behind a proxy with a domain name, add `--allowed-host admin.example.com`.
- **Protect the config.** `jengine.json` holds provider keys in plain text. On Linux and macOS jengine writes it readable by its owner only.

## Command reference

```text
jengine                     same as `jengine serve`
jengine serve [flags]       run the API and the admin page
jengine install clm         install and start local CLM in Docker
jengine remove clm          stop and remove the CLM containers; the weights stay
jengine keys list           list keys without showing them
jengine keys create --name NAME [--role client|admin] [--expires 30d|YYYY-MM-DD|never]
jengine keys revoke ID
```

Add `--help` to any command for details.

## Build from source

```sh
git clone https://github.com/Viperwow/jengine
cd jengine
cargo run                   # debug build, serves on 8080 and 8081
cargo test
```

The admin page is plain HTML and JavaScript in `ui/`, styled with Tailwind and embedded into the binary at build time.

The built stylesheet `ui/dist/app.css` is committed, so `cargo` alone builds jengine. After changing `ui/index.html`, `ui/app.js` or `ui/app.css`, rebuild the stylesheet with [pnpm](https://pnpm.io):

```sh
cd ui
pnpm install
pnpm build                  # or `pnpm dev` to rebuild on every change
```

## Contributing

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org): `feat: …`, `fix: …`, `docs: …`.

- **Commit hook.** Run `pnpm install` in the repository root once. A git hook then checks every commit message.
- **Pull requests.** CI checks the commit messages of every pull request, plus `cargo fmt`, `cargo clippy` and `cargo test`.
- **Releases.** A `feat` or `fix` merged into `main` publishes a new release with binaries for every system.

## License

[MIT](LICENSE)
