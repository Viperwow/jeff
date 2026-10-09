# jeff

A classifier built on Jev-compatible models, local or in the cloud.

Your agent or service describes what it needs to know as typed questions: yes/no (`noul`), pick one (`choice`) or rate on a scale (`score`). jeff asks a Jev-compatible model and returns a probability for every answer, so the caller gets a label it can act on, never free text to parse. One API covers every model, and the providers it knows out of the box are:

- **CLM**: Contrastive-LM, runs on your GPU, installed by jeff.
- **TypeSafe Jev**: cloud.
- **Laya, PipeLLM** or any other server with `POST /v1/systemone`.

jeff runs as one small binary. It serves:

- **The API**, on port 8080 by default: what your code calls.
- **The admin page**, on port 8081 by default: saved questions, providers and access keys.

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Call the API](#call-the-api)
- [Saved questions](#saved-questions)
- [Providers and models](#providers-and-models)
- [Local CLM](#local-clm)
- [Access keys](#access-keys)
- [Run on a server](#run-on-a-server)
- [Command reference](#command-reference)
- [Build from source](#build-from-source)

## Install

### Prebuilt binary

Download the archive for your system from the [latest release](https://github.com/Viperwow/jeff/releases/latest):

| System | Archive |
|---|---|
| Linux x64 | `jeff-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `jeff-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple silicon | `jeff-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `jeff-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| Windows x64 | `jeff-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

Unpack it and put `jeff` (`jeff.exe` on Windows) in a folder on your `PATH`. On Linux, for example:

```sh
tar -xzf jeff-*-x86_64-unknown-linux-gnu.tar.gz
sudo mv jeff-*/jeff /usr/local/bin/
```

`SHA256SUMS` in the same release lists the checksum of every archive. Check yours with `sha256sum -c SHA256SUMS --ignore-missing`.

macOS blocks a downloaded binary that Apple has not notarized. Allow it once with `xattr -d com.apple.quarantine /usr/local/bin/jeff`.

### With Cargo

With the [Rust toolchain](https://rustup.rs) installed:

```sh
cargo install --git https://github.com/Viperwow/jeff --locked
```

`cargo` builds jeff and puts it in `~/.cargo/bin`. Run the same command with `--force` to update.

### Check

```sh
jeff --version
```

## Quick start

```sh
jeff
```

This starts the API on `http://127.0.0.1:8080` and the admin page on `http://127.0.0.1:8081`. Open the admin page.

1. **Providers.** CLM and TypeSafe Jev are listed already.
   - Paste your TypeSafe key, or click **Install** on CLM to run it locally.
   - Each provider shows **Online** when jeff can reach it.
   - Click **Save**.
2. **Questions.** Click **New question** or **Add examples**, check the questions to ask, write the state, then click **Run** or press Ctrl+Enter. **Try** in the question form runs a draft before you save it.
   - The **curl** tab shows the same request for your own code.

jeff keeps its settings in `~/.jeff/jeff.json` (`%USERPROFILE%\.jeff\jeff.json` on Windows).

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

jeff adds two response headers:

- `x-jeff-provider`: the provider that answered.
- `x-jeff-upstream-ms`: how long the provider took, in milliseconds.

Other endpoints:

| Endpoint | What it returns |
|---|---|
| `GET /v1/models` | Every `provider/model` and the status of each provider. |
| `GET /v1/questions` | Every saved question, by key. |
| `GET /v1/questions/{key}` | One saved question. |
| `GET /health` | `{"ok": true}`. It needs no key, so it suits liveness probes. |

## Saved questions

Save a question once and ask it by key. Manage saved questions on the admin page under **Questions**, or through the admin port with an admin key:

| Endpoint | What it does |
|---|---|
| `POST /api/questions` | Saves new questions from a map of key to question. An existing key fails with `409`. |
| `PUT /api/questions/{key}` | Replaces one saved question. |
| `DELETE /api/questions/{key}` | Deletes one saved question. |

A key uses `A-Z`, `a-z`, `0-9`, `_` and `-`, up to 64 characters.

To ask saved questions, pass their keys as an array:

```sh
curl http://127.0.0.1:8080/v1/systemone   -H 'content-type: application/json'   -d '{ "state": "Customer: my invoice was charged twice!", "questions": ["urgent", "department"] }'
```

To change a saved question for one request, send it in the map without `type`. Only the fields you send change, and nothing is saved:

```json
"questions": { "urgent": {}, "department": { "model": "typesafe/jev-latest" } }
```

From the command line, `jeff ask` does the same through a running jeff:

```sh
jeff ask urgent department --state "My invoice was charged twice!"
jeff ask urgent --questions-file refund.json --state-file ticket.txt
jeff ask urgent --questions '{"urgent": {"model": "clm/clm-raw"}}' --state "..."
cat call.json | jeff ask --request - -o answer.json
```

- `--state` takes the text. `--state-file` reads it from a file, or from stdin for `-`.
- `--questions` takes a JSON map of custom questions. `--questions-file` reads the map from a file, or from stdin for `-`.
- `--request` takes a whole `/v1/systemone` body with `state` and `questions`, from a file or `-`.
- `-o FILE` writes the answer JSON to the file and prints its path.

Windows PowerShell 5.1 strips the inner quotes from JSON passed to a program, so put the JSON in a file there and pass it with `--questions-file`.

`jeff ask` calls `http://127.0.0.1:8080` unless `--url` or `JEFF_URL` says otherwise, and sends `JEFF_API_KEY` as its key. It waits 5 seconds to connect (`--connect-timeout`) and 60 seconds for the answer (`--max-time`). A custom question is saved only by `jeff questions add`, the admin page or `POST /api/questions`.

A question may name its own `model`. It goes to that model, and the others go to the request's `model` or the default model. jeff sends one call per model, at most 8 per request, and merges the answers into one response. `x-jeff-provider` then lists every provider that answered.

## Providers and models

Name a model as `provider/model`, for example `typesafe/jev-latest`.

A bare `jev-latest` goes to the first provider that lists it. A request without `model` gets the **Default model** set on the Providers page.

To add a provider, open **Providers**, pick a preset or **Custom server** under **Add provider**, fill in the URL and the key, and click **Save**.

Provider rules:

- **Models.** jeff asks each provider for its models. If a provider has no `GET /v1/models`, list its models by hand on its card.
- **Keys over the network.** A provider key goes to a remote host only over `https://`. Plain `http://` with a key is accepted only for `localhost`.
- **Timeouts.** jeff waits 10 seconds to connect to a provider and 60 seconds for its answer. Set `connect_timeout` and `max_time` in seconds on the provider in `jeff.json` to change them. A provider that does not answer in time gets `504`, one jeff cannot reach gets `502`.

## Local CLM

CLM runs in Docker on an NVIDIA GPU with at least 16 GB of memory. On Windows it needs Docker Desktop with WSL2.

Install CLM in one of two ways:

- Click **Install** on the CLM card.
- Run `jeff install clm`.

jeff starts Docker if it is not running, then downloads and starts everything. The steps and their progress show on the card.

| Step | Download |
|---|---|
| vLLM image | about 10 GB |
| Qwen3-8B weights | 16.4 GB |
| CLM head | 76 MB |

Downloads happen once. Later starts take about a minute.

Stop CLM with **Uninstall** or `jeff remove clm`. The downloaded weights stay on disk for the next install.

## Access keys

Without keys jeff accepts every request. Keep it that way only on your own machine.

Once one key exists, every `/v1` and `/api` request on both ports needs `Authorization: Bearer <key>`. Requests without a valid key get `401`. After 20 failures in a minute, the address gets `429` until the minute ends.

Each key has a role:

| Role | Reaches |
|---|---|
| `client` | `/v1`: decisions and models |
| `admin` | `/v1` and `/api`: providers, keys and the CLM install, so the admin page too |

A client key on `/api` gets `403`. The first key must be an admin key, and the last admin key cannot be revoked while client keys remain, so the admin page never locks itself out.

Manage keys on the **Access keys** page or from the terminal:

```sh
jeff keys create --name ops --role admin --expires never
jeff keys create --name claude-agent             # a client key, valid 30 days
jeff keys create --name ci --expires 2027-01-31  # through the end of that day, UTC
jeff keys list
jeff keys revoke 3f9a1c2e0b7d4a6f9e8c1b2a3d4e5f60
```

`create` prints the key once. jeff stores only its SHA-256 hash, so a lost key cannot be recovered: create a new one.

A running server picks up changes from the terminal on the next request.

The admin page sends a key too. The first key you create on that page is remembered by your browser, so the page keeps working after the lock turns on.

Keys that never expire can also come from the environment. They are admin keys, and each must have at least 32 characters:

```sh
JEFF_API_KEY="$(openssl rand -hex 32)" jeff
```

## Run on a server

Choose where each part listens:

```sh
jeff serve --api 0.0.0.0:8080 --ui 127.0.0.1:8081
jeff serve --api 9000 --no-ui        # a bare port means 127.0.0.1
```

The same settings come from the environment:

| Flag | Variable | Default |
|---|---|---|
| `--api` | `JEFF_API_ADDR` | `127.0.0.1:8080` |
| `--ui` | `JEFF_UI_ADDR` | `127.0.0.1:8081` |
| `--config` | `JEFF_CONFIG` | `~/.jeff/jeff.json` |
| `--api-key` | `JEFF_API_KEY` | none; separate several keys with commas |
| `--allowed-host` | `JEFF_ALLOWED_HOSTS` | none; separate several names with commas |
| `--allow-private-providers` | `JEFF_ALLOW_PRIVATE_PROVIDERS` | off; lets providers use addresses such as `10.0.0.0/8` and `192.168.0.0/16` |
| | `JEFF_HOME` | `~/.jeff`; holds the config and the CLM compose file |

jeff does not start on a broken config, so a damaged file cannot silently drop the access keys. Fix or remove the file.

Providers may use loopback and public addresses. Private networks need `--allow-private-providers`. Link-local and cloud metadata addresses such as `169.254.169.254` are always refused, and jeff does not follow redirects from providers. Changing a provider URL drops its saved key, so enter the key again for the new URL.

Before you open a port to a network:

- **Create a key.** jeff refuses to listen on a non-loopback address while no key exists.
- **Put TLS in front.** jeff speaks plain HTTP. Run it behind a reverse proxy such as Caddy or nginx, and set HSTS there.
- **Keep the admin page private.** It edits providers and keys and starts Docker. Bind it to `127.0.0.1` and reach it through an SSH tunnel (`ssh -L 8081:127.0.0.1:8081 server`), or turn it off with `--no-ui`.
- **Name the proxy.** The admin page answers only to `localhost` and IP addresses. Behind a proxy with a domain name, add `--allowed-host admin.example.com`.
- **Protect the config.** `jeff.json` holds provider keys in plain text. On Linux and macOS jeff writes it readable by its owner only.

## Command reference

```text
jeff                     same as `jeff serve`
jeff serve [flags]       run the API and the admin page
jeff install clm         install and start local CLM in Docker
jeff remove clm          stop and remove the CLM containers; the weights stay
jeff keys list           list keys without showing them
jeff keys create --name NAME [--role client|admin] [--expires 30d|YYYY-MM-DD|never]
jeff keys revoke ID
jeff ask [KEY...] (--state TEXT | --state-file FILE|-) [--questions JSON | --questions-file FILE|-]
         [--model MODEL] [--url URL]
         [--connect-timeout SECONDS] [--max-time SECONDS] [-o FILE]
jeff ask --request FILE|- [--model MODEL] [-o FILE]
                         ask through a running jeff and print the answer JSON
jeff questions list [-o FILE]
                         list saved questions: a table in a terminal, tab-separated lines in a pipe
jeff questions get KEY [-o FILE]
                         print one saved question as JSON
jeff questions add FILE  save new questions from a JSON map of key to question; `-` reads stdin
jeff questions update KEY FILE
jeff questions remove KEY
jeff classifiers list [-o FILE]
jeff classifiers get KEY [-o FILE]
jeff classifiers add KEY QUESTION... [--model MODEL] [--description TEXT]
jeff classifiers add --file FILE
                         save new classifiers from a JSON map of key to classifier; `-` reads stdin
jeff classifiers update KEY (QUESTION... [--model MODEL] [--description TEXT] | --file FILE)
jeff classifiers remove KEY
```

`jeff questions` and `jeff classifiers` call a running jeff on `http://127.0.0.1:8081` unless `--url` or `JEFF_ADMIN_URL` says otherwise. They send `JEFF_API_KEY`, which must be an admin key once any key exists.

An error exits with code 1 and prints the API error, for example:

```
error[config_invalid]: jeff.json has 2 problems; fix it before changing questions or classifiers
log: jeff.log
```

jeff refuses to start on a broken `jeff.json`. If the file breaks while jeff runs, jeff keeps the last working config, refuses changes and writes the problems to `jeff.log` next to the config.

Add `--help` to any command for details.

## Build from source

```sh
git clone https://github.com/Viperwow/jeff
cd jeff
cargo run                   # debug build, serves on 8080 and 8081
cargo test
```

The admin page is plain HTML and JavaScript in `ui/`, styled with Tailwind and embedded into the binary at build time.

The built stylesheet `ui/dist/app.css` is committed, so `cargo` alone builds jeff. After changing `ui/index.html`, `ui/app.js` or `ui/app.css`, rebuild the stylesheet with [pnpm](https://pnpm.io):

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
