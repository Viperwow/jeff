# jeff

A classifier built on Jev-compatible models, local or in the cloud.

Your agent or service describes what it needs to know as typed questions: yes/no (`noul`), pick one (`choice`) or rate on a scale (`score`). jeff asks a Jev-compatible model and returns a probability for every answer, so the caller gets a label it can act on, never free text to parse. One API covers every model, and the providers it knows out of the box are:

- **CLM**: Contrastive-LM, runs on your GPU, installed by jeff.
- **H2O-Lightning**: H2O-Lightning-4B, runs on your GPU, installed by jeff.
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
- [Shared answers](#shared-answers)
- [Providers and models](#providers-and-models)
- [Local CLM](#local-clm)
- [Local H2O-Lightning](#local-h2o-lightning)
- [Access keys](#access-keys)
- [Run on a server](#run-on-a-server)
- [Agent skill](#agent-skill)
- [Claude Code mod](#claude-code-mod)
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

jeff adds a `verdict` to every answer of a request that has `questions`. The provider's own fields stay as they are. An answer jeff cannot read, with neither a `noul` number nor `probabilities`, gets no `verdict`, and an error response from the provider passes through unchanged. jeff writes the keys of every JSON object in alphabetical order, so read a level by its key: `"10"` comes before `"2"`.

```json
"department": {
  "type": "choice", "choice": "billing", "probabilities": { "billing": 0.91, "technical": 0.09 }, "confidence": 0.56,
  "verdict": { "result": "warn", "confidence": 0.56, "source": "jeff", "confidence_soft_cap": 0.8, "confidence_hard_cap": 0.3 }
}
```

- `result` is `fail` when the confidence is below `confidence_hard_cap`, `warn` when it is below `confidence_soft_cap`, and `pass` otherwise. A confidence equal to a cap passes it. A question without caps always gets `pass`.
- `confidence` is how sure the model is of its answer, from 0 to 1. It is the number the caps were compared against.
- `source` says who computed it: `jeff` or `provider`.
- `confidence_soft_cap` and `confidence_hard_cap` repeat the caps that applied. A cap that is not set is left out.

A question sets these optional fields:

| Field | Values | What it does |
|---|---|---|
| `confidence_soft_cap` | 0 to 1 | Below it the verdict is `warn`: ask a person. |
| `confidence_hard_cap` | 0 to 1, below the soft cap | Below it the verdict is `fail`: don't trust the answer. |
| `confidence_source` | `jeff` (default) or `provider` | Which confidence the caps are compared against. |

How the confidence is computed:

| Type | `jeff` | `provider` |
|---|---|---|
| `noul` | `\|2P − 1\|`, where P is the probability of yes. A sure yes and a sure no both score high. | Providers send none, so jeff computes it and reports `source: "jeff"`. |
| `choice`, `score` | `1 − H / ln k`: 1 minus the entropy of `probabilities`, divided by its maximum for k options. 0 for an even spread, 1 when one option holds everything. | The provider's `confidence`. Providers compute it differently, so the same cap can mean different things for different providers. |

jeff never sends these fields to the provider, so changing a cap needs no new model call: a cached answer gets the new verdict.

Set the fields in the question JSON that `jeff questions add` and `jeff questions update` take, in the question editor under **Confidence** and **Caps**, or through `POST /api/questions` and `PUT /api/questions/{key}`.

jeff adds two response headers:

- `x-jeff-provider`: the provider that answered.
- `x-jeff-upstream-ms`: how long the provider took, in milliseconds.
- `x-jeff-cache`: `hit` when the answer came from jeff's answer cache, `miss` when a provider answered. A cached answer also has `usage.cached: true` and every `usage` token count set to 0, since the provider spent nothing on it.

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

`null` removes a saved confidence field for that request, so `{ "urgent": { "confidence_hard_cap": null } }` asks without the hard cap.

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

`jeff ask` calls `http://127.0.0.1:8080` unless `--url` or `JEFF_URL` says otherwise, and sends `JEFF_API_KEY` as its key. It waits 5 seconds to connect (`--connect-timeout`) and 60 seconds for the answer (`--max-time`). `--no-cache` asks the providers again instead of reusing a cached answer. A custom question is saved only by `jeff questions add`, the admin page or `POST /api/questions`.

A question may name its own `model`. It goes to that model, and the others go to the request's `model` or the default model. jeff sends one call per model, at most 8 per request, and merges the answers into one response. `x-jeff-provider` then lists every provider that answered.

## Shared answers

A shared answer is an option many questions reuse, such as "Not sure about the answer." Save it once and add it to a question with one click in the question editor. The library starts empty. Manage it on the admin page under **Answers**, with `jeff answers`, or through the admin port with an admin key.

```json
{
  "insufficient_context": { "text": "Not enough context to answer.", "types": ["choice"], "default": true },
  "uncertain": { "text": "Not sure about the answer.", "types": ["choice"], "default": true }
}
```

| Field | Meaning |
|---|---|
| `text` | The option text. It must not be empty. |
| `types` | `choice`, `score` or both. |
| `default` | Optional. A new question starts with every default answer whose `types` hold its type. |

Keep an "I don't know" answer to `choice`. On a `score` scale an answer is a level, and the expected level is an average over the scale, so such a level shifts the average. No two answers that allow `score` may share a text.

A saved question stays plain Jev with every option written out. It uses an answer when it holds it word for word: a `choice` question has the answer's key with exactly its text, and a `score` question has a level with exactly its text. Spaces around a text do not count. Nothing else links them.

Updating an answer rewrites the text in every question that used it, in the same save. A question the new `types` no longer cover keeps its option as written. If a `score` question already has another level with the new text, the update fails and saves nothing. Removing an answer leaves the options in the questions as plain text.

| Endpoint | What it does |
|---|---|
| `GET /api/answers` | Lists the answers, each with the `questions` that use it. |
| `GET /api/answers/{key}` | Returns one answer with `questions`. |
| `POST /api/answers` | Saves new answers from a map of key to answer. An existing key fails with `409`. |
| `PUT /api/answers/{key}` | Replaces one answer. Returns `answer` and the rewritten `questions`. |
| `DELETE /api/answers/{key}` | Deletes one answer. Returns the library. |

```sh
jeff answers add answers.json
jeff answers list
jeff answers update uncertain uncertain.json
```

`update` prints `Updated 3 questions: a, b, c.` or `Updated no questions.` The keys follow the question key rules.

## Providers and models

Name a model as `provider/model`, for example `typesafe/jev-latest`.

A bare `jev-latest` goes to the first provider that lists it. A request without `model` gets the **Default model** set on the Providers page.

To add a provider, open **Providers**, choose **Local** or **Cloud** under **Add provider**, then a provider or **Custom server**. Fill in the URL and the key, and click **Save**. Each card is tagged Local or Cloud by its URL.

Provider rules:

- **Models.** jeff asks each provider for its models. If a provider has no `GET /v1/models`, list its models by hand on its card.
- **Keys over the network.** A provider key goes to a remote host only over `https://`. Plain `http://` with a key is accepted only for `localhost`.
- **Answer cache.** jeff keeps successful answers and returns one again, without calling the provider, when the same provider gets the same request body, exact to the character. It keeps 1,000 answers for an hour by default. Set `"cache": {"entries": 1000, "ttl_secs": 3600}` in `jeff.json` to change that, and `"entries": 0` turns the cache off. Send `Cache-Control: no-cache` to get a fresh answer; jeff caches that one too.
- **Timeouts.** jeff waits 10 seconds to connect to a provider and 60 seconds for its answer. Set `connect_timeout` and `max_time` in seconds on the provider in `jeff.json` to change them. A provider that does not answer in time gets `504`, one jeff cannot reach gets `502`.
- **Description.** A provider card shows a short line under its name. Set it in the Description field, or as `description` in `jeff.json`. A preset provider shows the preset's own line, which the page does not let you change.

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

## Local H2O-Lightning

[H2O-Lightning-4B](https://huggingface.co/h2oai/h2o-lightning-4b) runs in Docker on an NVIDIA GPU with at least 16 GB of memory: vLLM 0.30.0 serves the model, and the model's own shim answers `/v1/systemone` on port 8741. On Windows it needs Docker Desktop with WSL2.

Install H2O-Lightning in one of two ways:

- Click **Install** on the H2O-Lightning card. A config saved before jeff knew this model has no such card: add it under **Add provider** with **Local**, then **H2O-Lightning**, first.
- Run `jeff install h2o-lightning`.

| Step | Download |
|---|---|
| vLLM image | about 9 GB |
| H2O-Lightning-4B weights | 9.1 GB |

Its model is `h2o-lightning/h2oai/h2o-lightning-4b`. Stop it with **Uninstall** or `jeff remove h2o-lightning`. The downloaded weights stay on disk for the next install.

## Access keys

Without keys jeff accepts every request. Keep it that way only on your own machine.

Once one key exists, every `/v1` request on the API port needs `Authorization: Bearer <key>`. Every `/v1` and `/api` request on the admin port needs that header or an admin page session. Requests without a valid key get `401`. After 20 failures in a minute, the address gets `429` until the minute ends.

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

The admin page signs in with an admin key. Once jeff has keys, the page opens on **Sign in to jeff**. It takes a key in two ways:

- **The link `jeff keys create --role admin` prints**, such as `sign in to the admin page while jeff serve runs: http://127.0.0.1:8081/#key=jeff_…`. Open it while jeff runs. The link points at `JEFF_ADMIN_URL`, or at `--admin-url`, when the admin page is elsewhere.
- **The Sign in form.** Paste an admin key and press **Sign in**. A client key is refused.

A first key created on the page, while jeff has no keys yet, stays on screen to copy; sign in with it next.

Signing in starts a session in two halves. One half sits in an HttpOnly cookie, `jeff_session`, that no script on the page can read. The other half sits in the page's local storage and goes in the `X-Jeff-Session` header. A request needs both halves. Browsers send a cookie to every port of the host, so another server on the same host can see the cookie half, but never the header half. Neither half holds the key. The cookie is `SameSite=Strict`, and `Secure` when the page is on https. Only the admin port reads sessions; the API port takes the `Authorization` header alone. A session lasts 30 days. A session cannot start another one: signing in takes the key itself.

jeff keeps sessions in `sessions.json`, next to the config, so a restart keeps every browser signed in. The file holds only the SHA-256 of both halves and of the key. Deleting an entry from the file signs that browser out without a restart. If the file breaks, jeff keeps the sessions it read before, refuses new sign-ins, and logs `sessions_invalid` to `jeff.log` until the file is fixed or deleted.

Revoking the key signs the browser out at once. **Sign out**, at the bottom of the menu, ends the session. On **Access keys**, the key this browser signs in with carries a **This browser** chip, and revoking it asks for **Revoke and sign out**.

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

## Agent skill

[![skills.sh](https://skills.sh/b/Viperwow/jeff)](https://skills.sh/Viperwow/jeff)

[skills/jeff/SKILL.md](skills/jeff/SKILL.md) teaches a coding agent to run jeff, hand it typed decisions in batches, and save repeated checks as questions and classifiers. Install it with:

```sh
npx skills add Viperwow/jeff
```

## Claude Code mod

[mods/jeff-skill-suggestion](mods/jeff-skill-suggestion) hides the skill listing from Claude Code and lets jeff pick at most one skill per prompt. It saves the `skill-gate` classifier in jeff on first use, so the questions that decide whether a prompt needs a skill can be edited on the admin page. Install it in Claude Code 2.1.287 or later, with jeff running:

```text
/plugin install jeff-skill-suggestion --marketplace Viperwow/jeff
```

With the mod, Claude chose the right skill in 90% of runs instead of 50%, at 40% lower cost. On prompts that needed a skill, it chose right in 86% of runs instead of 29%.

Measured on 10 prompts, each run 3 times with the mod and 3 times without it, with `skill-gate` on `typesafe/openjev-latest`:

| Metric | Without the mod | With the mod |
|---|---|---|
| Right skill chosen | 15 of 30 | 27 of 30 |
| Cost | $5.13 | $3.07, −40% |
| Context of a one-turn prompt | 39.6k tokens | 31.3k tokens, −21% |

The mod waits for jeff before each prompt, about 1–3 s. [BENCHMARK.md](mods/jeff-skill-suggestion/BENCHMARK.md) has the setup, every prompt and the limits of the measurement.

[Settings](mods/jeff-skill-suggestion/README.md#settings) lists every option and variable of the mod, and how to reach jeff on another machine.

## Command reference

```text
jeff                     same as `jeff serve`
jeff serve [flags]       run the API and the admin page
jeff install clm         install and start local CLM in Docker
jeff remove clm          stop and remove the CLM containers; the weights stay
jeff install h2o-lightning
                         install and start local H2O-Lightning-4B in Docker
jeff remove h2o-lightning
                         stop and remove the H2O-Lightning containers; the weights stay
jeff keys list           list keys without showing them
jeff keys create --name NAME [--role client|admin] [--expires 30d|YYYY-MM-DD|never] [--admin-url URL]
jeff keys revoke ID
jeff ask [KEY...] (--state TEXT | --state-file FILE|-) [--questions JSON | --questions-file FILE|-]
         [--model MODEL] [--url URL]
         [--connect-timeout SECONDS] [--max-time SECONDS] [--no-cache] [-o FILE]
jeff ask --request FILE|- [--model MODEL] [--no-cache] [-o FILE]
                         ask through a running jeff and print the answer JSON
jeff questions list [-o FILE]
                         list saved questions: a table in a terminal, tab-separated lines in a pipe
jeff questions get KEY [-o FILE]
                         print one saved question as JSON
jeff questions add FILE  save new questions from a JSON map of key to question; `-` reads stdin
jeff questions update KEY FILE
jeff questions remove KEY
jeff answers list [-o FILE]
                         list shared answers: key, types, questions that use it, text
jeff answers get KEY [-o FILE]
                         print one shared answer as JSON, with the questions that use it
jeff answers add FILE    save new answers from a JSON map of key to answer; `-` reads stdin
jeff answers update KEY FILE
                         replace one answer and print the questions it rewrote
jeff answers remove KEY
jeff classifiers list [-o FILE]
jeff classifiers get KEY [-o FILE]
jeff classifiers add KEY QUESTION... [--model MODEL] [--description TEXT]
jeff classifiers add --file FILE
                         save new classifiers from a JSON map of key to classifier; `-` reads stdin
jeff classifiers update KEY (QUESTION... [--model MODEL] [--description TEXT] | --file FILE)
jeff classifiers remove KEY
```

`jeff questions`, `jeff answers` and `jeff classifiers` call a running jeff on `http://127.0.0.1:8081` unless `--url` or `JEFF_ADMIN_URL` says otherwise. They send `JEFF_API_KEY`, which must be an admin key once any key exists.

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
- **Error messages.** Each error says what happened, then what to do next: `Couldn't save jeff.json. Check jeff.log for the cause.` One situation gets one wording in the API, the CLI, the admin page and the mod. An error that has its cause on the server names `jeff.log`. An error a client receives never holds a server path.

## License

[MIT](LICENSE)
