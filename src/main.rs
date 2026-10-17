mod classifiers;
mod install;
mod keys;
mod log;
mod questions;
mod sessions;

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::delete,
    routing::{get, post},
};
use clap::{Args, Parser, Subcommand};
use futures_util::future::{join_all, try_join_all};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env, fs,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::RwLock;

#[derive(Clone, Serialize, Deserialize)]
struct Provider {
    id: String,
    name: String,
    url: String,
    #[serde(default)]
    key: Option<String>,
    /// Shown when the provider has no `GET /v1/models`.
    #[serde(default)]
    models: Vec<String>,
    /// Names a local stack jeff can install, such as "clm".
    #[serde(default)]
    installer: Option<String>,
    /// Seconds jeff waits to connect [default: 10].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connect_timeout: Option<u64>,
    /// Seconds jeff waits for an answer to `/v1/systemone` [default: 60].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_time: Option<u64>,
    /// A short line the admin page shows under the provider name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl Provider {
    fn connect_secs(&self) -> u64 {
        self.connect_timeout.filter(|s| *s > 0).unwrap_or(10)
    }

    fn max_secs(&self) -> u64 {
        self.max_time.filter(|s| *s > 0).unwrap_or(60)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    default_model: String,
    providers: Vec<Provider>,
    /// jeff access keys, hashed.
    #[serde(default)]
    keys: Vec<keys::StoredKey>,
    #[serde(default)]
    questions: questions::Questions,
    #[serde(default)]
    classifiers: classifiers::Classifiers,
    #[serde(default)]
    cache: CacheConfig,
}

/// How many successful provider answers jeff keeps, and for how long, so a repeated request skips the provider.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
struct CacheConfig {
    /// 0 turns the cache off.
    entries: usize,
    ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            entries: 1000,
            ttl_secs: 3600,
        }
    }
}

/// Answers by a hash of the provider and the exact body sent to it.
type AnswerCache = HashMap<[u8; 32], (Instant, StatusCode, Vec<u8>)>;

impl Default for Config {
    fn default() -> Self {
        let env_or = |var: &str, default: &str| env::var(var).unwrap_or_else(|_| default.into());
        let key = |var: &str| env::var(var).ok().filter(|k| !k.is_empty());
        Config {
            default_model: env_or("JEFF_DEFAULT_MODEL", "clm/clm-latest"),
            providers: vec![
                Provider {
                    id: "clm".into(),
                    name: "CLM".into(),
                    url: env_or("CLM_URL", "http://127.0.0.1:8700"),
                    key: key("CLM_API_KEY"),
                    models: vec!["clm-latest".into()],
                    installer: Some("clm".into()),
                    connect_timeout: None,
                    max_time: None,
                    description: None,
                },
                Provider {
                    id: "typesafe".into(),
                    name: "TypeSafe Jev".into(),
                    url: env_or("TYPESAFE_URL", "https://api.typesafe.ai"),
                    key: key("TYPESAFE_API_KEY"),
                    models: vec!["jev-latest".into()],
                    installer: None,
                    connect_timeout: None,
                    max_time: None,
                    description: None,
                },
                Provider {
                    id: "h2o-lightning".into(),
                    name: "H2O-Lightning".into(),
                    url: env_or("H2O_LIGHTNING_URL", "http://127.0.0.1:8741"),
                    key: None,
                    models: vec!["h2oai/h2o-lightning-4b".into()],
                    installer: Some("h2o-lightning".into()),
                    connect_timeout: None,
                    max_time: None,
                    description: None,
                },
            ],
            keys: Vec::new(),
            questions: questions::Questions::new(),
            classifiers: classifiers::Classifiers::new(),
            cache: CacheConfig::default(),
        }
    }
}

/// A missing file gives the defaults. A broken one is refused whole, with every problem found: falling back would
/// drop every access key and open jeff to anyone, and a partial config would run on what the owner did not write.
fn load_config(path: &str) -> Result<Config, Vec<String>> {
    let config = read_config(path).map_err(|e| vec![e])?;
    let problems: Vec<String> = config
        .questions
        .iter()
        .filter_map(|(k, q)| questions::validate(k, q).err())
        .chain(
            config
                .classifiers
                .iter()
                .filter_map(|(k, c)| classifiers::validate(k, c).err()),
        )
        .collect();
    if problems.is_empty() {
        Ok(config)
    } else {
        Err(problems)
    }
}

/// Parses the config without checking saved questions and classifiers, so the CLI can still revoke a key or remove
/// the broken entry from a config that `serve` refuses.
fn read_config(path: &str) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{path}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

/// Writes the config readable by its owner only: it holds provider keys and access-key hashes. The text goes to a
/// temporary file first, so a crash mid-write cannot leave a truncated config behind.
fn save_config(path: &str, config: &Config) -> Result<(), String> {
    write_private(path, &serde_json::to_string_pretty(config).unwrap())
}

fn write_private(path: &str, text: &str) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Some(dir) = std::path::Path::new(path)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
    {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = format!("{path}.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = options.open(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{path}: {e}")
    })
}

fn modified(path: &str) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

struct App {
    /// HTTP clients by connect timeout in seconds; reqwest sets that timeout per client, not per request.
    clients: Mutex<HashMap<u64, reqwest::Client>>,
    config: RwLock<Config>,
    config_path: String,
    /// Model lists last fetched from each provider, keyed by provider id.
    listed: RwLock<HashMap<String, Vec<String>>>,
    /// The running or last install action of each local stack, indexed by `install::Stack`.
    tasks: Arc<install::Tasks>,
    /// 0 cold, 1 warming, 2 warm: whether each stack's server has answered its first, slow request.
    warm: [AtomicU8; 2],
    /// Keys from `--api-key` / `JEFF_API_KEY`; they never expire and are managed outside the UI.
    static_keys: Vec<String>,
    sessions: Mutex<sessions::Sessions>,
    /// Problems of a config file that failed to reload; the config read before stays in force while it is not empty.
    problems: Mutex<Vec<String>>,
    /// When the config file was last read, so keys created or revoked by `jeff keys` apply without a restart.
    config_mtime: Mutex<Option<SystemTime>>,
    /// Host names the UI answers to besides localhost and IP literals.
    allowed_hosts: Vec<String>,
    /// Failed key checks per client address in the current window: (count, window start).
    failures: Mutex<HashMap<IpAddr, (u32, u64)>>,
    /// A listener is on a non-loopback address: keys stay required even after the last one is revoked.
    exposed: bool,
    /// Providers may sit in private networks such as 10.0.0.0/8.
    allow_private: bool,
    answers: Mutex<AnswerCache>,
}

/// Failed key checks one address may make per window before it gets 429 until the window ends.
const MAX_FAILURES: u32 = 20;
const FAILURE_WINDOW_SECS: u64 = 60;
/// The largest provider response jeff relays; typed answers are a few kilobytes.
const MAX_UPSTREAM_BYTES: usize = 16 * 1024 * 1024;
/// Shortest `--api-key` accepted; generated keys carry 256 bits.
const MIN_STATIC_KEY_LEN: usize = 32;
/// The largest request body jeff accepts.
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

enum ReadError {
    TooLarge,
    Failed(reqwest::Error),
}

/// Reads a response while `total`, shared by every call of one request, stays within `MAX_UPSTREAM_BYTES`.
async fn read_capped(
    mut resp: reqwest::Response,
    total: &AtomicUsize,
) -> Result<Vec<u8>, ReadError> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(ReadError::Failed)? {
        if total.fetch_add(chunk.len(), Ordering::Relaxed) + chunk.len() > MAX_UPSTREAM_BYTES {
            return Err(ReadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn is_loopback_url(url: &reqwest::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Where a provider may point. Loopback stays open for local models. Link-local addresses, which hold cloud
/// metadata endpoints, are always refused; private networks need `--allow-private-providers`.
fn check_ip(ip: IpAddr, allow_private: bool) -> Result<(), String> {
    let ip = ip.to_canonical();
    let (forbidden, private) = match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            (
                // 100.100.100.200 is the Alibaba Cloud metadata endpoint, inside the private 100.64.0.0/10.
                v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || v4.is_multicast()
                    || v4 == std::net::Ipv4Addr::new(100, 100, 100, 200),
                v4.is_private() || (a == 100 && (b & 0xc0) == 64),
            )
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            (
                (first & 0xffc0) == 0xfe80
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || v6 == std::net::Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254),
                (first & 0xfe00) == 0xfc00,
            )
        }
    };
    if forbidden {
        Err(format!(
            "{ip} is a link-local or metadata address. Use the provider's public address."
        ))
    } else if private && !allow_private {
        Err(format!(
            "{ip} is in a private network. Start jeff with --allow-private-providers to allow it."
        ))
    } else {
        Ok(())
    }
}

fn parse_provider_url(url: &str, allow_private: bool) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("URL '{url}': {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(
            "The URL does not start with http:// or https://. Start it with https://.".into(),
        );
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("The URL holds a provider key. Put the key in the key field instead.".into());
    }
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => check_ip(ip.into(), allow_private)?,
        Some(url::Host::Ipv6(ip)) => check_ip(ip.into(), allow_private)?,
        Some(url::Host::Domain(_)) => {}
        None => return Err("The URL has no host. Add the provider's host name.".into()),
    }
    Ok(parsed)
}

/// Resolves provider host names and refuses addresses `check_ip` rejects, so a name cannot point jeff at
/// metadata endpoints, even if its DNS record changes after the provider was saved.
struct GuardedResolver {
    allow_private: bool,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow_private = self.allow_private;
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if let Some(e) = addrs
                .iter()
                .find_map(|a| check_ip(a.ip(), allow_private).err())
            {
                return Err(format!("{host}: {e}").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl App {
    /// A broken file is refused whole and the config read before stays in force, keys included, until it is fixed.
    async fn refresh_from_disk(&self) {
        let now = modified(&self.config_path);
        let seen = *self.config_mtime.lock().unwrap();
        if now.is_none() {
            // A deleted file leaves no hand edits to protect; the next save writes the config back.
            if !std::mem::take(&mut *self.problems.lock().unwrap()).is_empty() {
                let line = format!(
                    "{} was removed. The next save writes it again.",
                    self.config_path
                );
                eprintln!("{line}");
                log::write(&self.config_path, "config_removed", &line, &[]);
            }
            *self.config_mtime.lock().unwrap() = None;
            return;
        }
        if now != seen {
            let path = &self.config_path;
            match load_config(path) {
                Ok(config) => {
                    *self.config.write().await = config;
                    let was = std::mem::take(&mut *self.problems.lock().unwrap());
                    if !was.is_empty() {
                        let line = format!("{path} is fixed. jeff reloaded the config.");
                        eprintln!("{line}");
                        log::write(path, "config_fixed", &line, &[]);
                    }
                }
                Err(problems) => {
                    let n = problems.len();
                    let summary = format!(
                        "{path}: {}. jeff keeps the previous config until you fix the file.",
                        count_problems(n)
                    );
                    log::write(path, "config_invalid", &summary, &problems);
                    eprintln!(
                        "error[config_invalid]: {}. jeff keeps the previous config until you fix the file.",
                        problems_text(path, n)
                    );
                    *self.problems.lock().unwrap() = problems;
                }
            }
            *self.config_mtime.lock().unwrap() = now;
        }
    }

    /// Refuses to write over a file that is broken, or that changed since jeff last read it: either way the write
    /// would lose a hand edit.
    async fn save(&self, config: &Config) -> Result<(), Box<Response>> {
        let path = &self.config_path;
        let n = self.problems.lock().unwrap().len();
        if n > 0 {
            return Err(Box::new(config_invalid(path, n)));
        }
        if modified(path) != *self.config_mtime.lock().unwrap() {
            return Err(Box::new(
                (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "code": "config_changed",
                        "error": format!("{} changed on disk since jeff read it. Reload and try again.", file_name(path)),
                    })),
                )
                    .into_response(),
            ));
        }
        save_config(path, config).map_err(|e| {
            eprintln!("error[config_not_saved]: {e}");
            let logged = log::write(path, "config_not_saved", &e, &[]);
            Box::new(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "Couldn't save {}. Check {} for the cause.",
                    file_name(path),
                    log::cause_in(logged)
                ),
            ))
        })?;
        *self.config_mtime.lock().unwrap() = modified(&self.config_path);
        Ok(())
    }

    async fn auth_required(&self) -> bool {
        self.exposed || !self.static_keys.is_empty() || !self.config.read().await.keys.is_empty()
    }

    /// Whether some key can still reach `/api`, so the admin page does not lock itself out.
    fn has_admin(&self, keys: &[keys::StoredKey]) -> bool {
        let now = keys::now();
        !self.static_keys.is_empty() || keys.iter().any(|k| k.active_admin(now))
    }
}

/// `provider/model` picks the provider explicitly; a bare model goes to the first provider that lists it.
fn resolve(
    config: &Config,
    listed: &HashMap<String, Vec<String>>,
    model: &str,
) -> Option<(Provider, String)> {
    if let Some((id, rest)) = model.split_once('/')
        && let Some(p) = config.providers.iter().find(|p| p.id == id)
    {
        return Some((p.clone(), rest.to_owned()));
    }
    config
        .providers
        .iter()
        .find(|p| {
            p.models
                .iter()
                .chain(listed.get(&p.id).into_iter().flatten())
                .any(|m| m == model)
        })
        .map(|p| (p.clone(), model.to_owned()))
}

fn http_client(connect_secs: u64, allow_private: bool) -> reqwest::Client {
    // A provider that redirects could send the provider key on to a host jeff never checked.
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(connect_secs))
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(GuardedResolver { allow_private })
        .build()
        .expect("HTTP client")
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// `2 problems`, `1 problem`.
fn count_problems(n: usize) -> String {
    format!("{n} problem{}", if n == 1 { "" } else { "s" })
}

/// The config's file name only: API callers must not learn where the server keeps it.
fn file_name(config_path: &str) -> String {
    std::path::Path::new(config_path)
        .file_name()
        .map_or_else(|| config_path.into(), |n| n.to_string_lossy().into_owned())
}

fn problems_text(config_path: &str, n: usize) -> String {
    format!("{} has {}", file_name(config_path), count_problems(n))
}

/// jeff saves the whole file, so a save while it is broken would overwrite the hand edits.
fn config_invalid(config_path: &str, n: usize) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "code": "config_invalid",
            "error": format!("{}. Fix it before changing questions or classifiers.", problems_text(config_path, n)),
            "log": "jeff.log",
        })),
    )
        .into_response()
}

fn status_body(config_path: &str, problems: &[String]) -> Value {
    if problems.is_empty() {
        return json!({ "config": "ok" });
    }
    json!({ "config": "invalid", "file": file_name(config_path), "problems": problems.len(), "log": "jeff.log" })
}

async fn status(State(app): State<Arc<App>>) -> Json<Value> {
    Json(status_body(&app.config_path, &app.problems.lock().unwrap()))
}

fn request(
    app: &App,
    p: &Provider,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let client = app
        .clients
        .lock()
        .unwrap()
        .entry(p.connect_secs())
        .or_insert_with(|| http_client(p.connect_secs(), app.allow_private))
        .clone();
    let req = client.request(method, format!("{}{path}", p.url.trim_end_matches('/')));
    match &p.key {
        Some(key) => req.bearer_auth(key),
        None => req,
    }
}

/// One provider's answer to `/v1/systemone`, before it becomes a response.
struct Upstream {
    provider: String,
    ms: u64,
    status: StatusCode,
    bytes: Vec<u8>,
    /// Came from jeff's answer cache, not the provider.
    cached: bool,
}

impl Upstream {
    fn reply(self) -> Response {
        let mut res = (
            self.status,
            [(header::CONTENT_TYPE, "application/json")],
            self.bytes,
        )
            .into_response();
        let headers = res.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&self.provider) {
            headers.insert("x-jeff-provider", v);
        }
        headers.insert("x-jeff-upstream-ms", HeaderValue::from(self.ms));
        let cache = if self.cached { "hit" } else { "miss" };
        headers.insert("x-jeff-cache", HeaderValue::from_static(cache));
        res
    }
}

/// Finds the provider and its model name for `model`. A bare name refreshes the model lists unless `refreshed`
/// says this request already did.
async fn find_model(
    app: &App,
    config: &Config,
    model: &str,
    refreshed: &mut bool,
) -> Result<(Provider, String), (StatusCode, String)> {
    let mut found = resolve(config, &*app.listed.read().await, model);
    if found.is_none() && !model.contains('/') && !*refreshed {
        refresh_models(app, config).await;
        *refreshed = true;
        found = resolve(config, &*app.listed.read().await, model);
    }
    found.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!(
                "Unknown model '{model}'. Use provider/model, for example {}.",
                config.default_model
            ),
        )
    })
}

/// Sends `body` to provider `p` as `upstream_model`; the error is the status and message for the client. `fresh`
/// skips the cached answer but still caches the new one.
async fn forward(
    app: &App,
    p: Provider,
    upstream_model: String,
    mut body: Value,
    total: &AtomicUsize,
    cache: CacheConfig,
    fresh: bool,
) -> Result<Upstream, (StatusCode, String)> {
    body["model"] = Value::String(upstream_model);
    let sent = serde_json::to_vec(&body).unwrap();
    let ttl = Duration::from_secs(cache.ttl_secs);
    let key: [u8; 32] = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for part in [p.id.as_bytes(), p.url.as_bytes(), &sent] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part);
        }
        h.finalize().into()
    };
    let hit = app
        .answers
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(at, _, _)| cache.entries > 0 && !fresh && at.elapsed() < ttl)
        .map(|(_, status, bytes)| (*status, bytes.clone()));
    if let Some((status, bytes)) = hit {
        return Ok(Upstream {
            provider: p.id,
            ms: 0,
            status,
            bytes: cached_usage(bytes),
            cached: true,
        });
    }

    let t0 = Instant::now();
    let max = p.max_secs();
    let upstream = request(app, &p, reqwest::Method::POST, "/v1/systemone")
        .header(header::CONTENT_TYPE, "application/json")
        .body(sent)
        .timeout(Duration::from_secs(max));
    let late = || {
        (
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "Provider '{}' did not answer within {max}s. Try again, or raise its max_time.",
                p.id
            ),
        )
    };
    // Clients get the provider id only; the provider URL and the transport error stay in the server log.
    let resp = upstream.send().await.map_err(|e| {
        // A connect timeout is a timeout too, but it means the provider is unreachable.
        let timed_out = e.is_timeout() && !e.is_connect();
        eprintln!("provider {}: {}", p.id, e.without_url());
        if timed_out {
            late()
        } else {
            (
                StatusCode::BAD_GATEWAY,
                format!("provider '{}' is unreachable", p.id),
            )
        }
    })?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let bytes = read_capped(resp, total).await.map_err(|e| match e {
        ReadError::TooLarge => (
            StatusCode::BAD_GATEWAY,
            format!(
                "The answer is larger than {} MB. Ask fewer questions per request.",
                MAX_UPSTREAM_BYTES >> 20
            ),
        ),
        ReadError::Failed(e) if e.is_timeout() => late(),
        ReadError::Failed(e) => {
            eprintln!("provider {}: {}", p.id, e.without_url());
            unreadable(&p.id)
        }
    })?;
    if cache.entries > 0 && status.is_success() {
        let mut answers = app.answers.lock().unwrap();
        if answers.len() >= cache.entries {
            answers.retain(|_, (at, _, _)| at.elapsed() < ttl);
        }
        // ponytail: finds the oldest with a linear scan; an ordered index if entries grow past ~100k
        if answers.len() >= cache.entries {
            let oldest = answers
                .iter()
                .min_by_key(|(_, (at, _, _))| *at)
                .map(|(k, _)| *k);
            answers.remove(&oldest.unwrap());
        }
        answers.insert(key, (Instant::now(), status, bytes.clone()));
    }
    Ok(Upstream {
        provider: p.id,
        ms: t0.elapsed().as_millis() as u64,
        status,
        bytes,
        cached: false,
    })
}

/// A cached answer cost the provider nothing: its `usage` token counts become 0 and `usage.cached` true, so summing
/// `usage` gives what was really spent.
fn cached_usage(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(mut v) = serde_json::from_slice::<Value>(&bytes) else {
        return bytes;
    };
    let Some(usage) = v.get_mut("usage").and_then(Value::as_object_mut) else {
        return bytes;
    };
    for (k, n) in usage.iter_mut() {
        if k.ends_with("_tokens") && n.is_number() {
            *n = json!(0);
        }
    }
    usage.insert("cached".into(), json!(true));
    serde_json::to_vec(&v).unwrap()
}

type Batch = (Provider, String, Option<questions::Questions>);

/// Groups whose models resolve to the same provider and model become one call.
fn batch(resolved: Vec<Batch>) -> Vec<Batch> {
    let mut out: Vec<Batch> = Vec::new();
    for (p, model, qs) in resolved {
        match out.iter_mut().find(|(q, m, _)| q.id == p.id && *m == model) {
            Some((_, _, all)) => {
                if let (Some(all), Some(qs)) = (all, qs) {
                    all.extend(qs);
                }
            }
            None => out.push((p, model, qs)),
        }
    }
    out
}

fn unreadable(provider: &str) -> (StatusCode, String) {
    (
        StatusCode::BAD_GATEWAY,
        format!("Provider '{provider}' sent an unreadable response. Check jeff.log for the cause."),
    )
}

/// `Cache-Control: no-cache` asks for an answer from the provider, not from jeff's cache.
fn no_cache(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|d| {
            matches!(
                d.trim().to_ascii_lowercase().as_str(),
                "no-cache" | "no-store"
            )
        })
}

async fn systemone(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let config = app.config.read().await.clone();
    answer(&app, &config, body, no_cache(&headers))
        .await
        .map_or_else(|res| *res, Upstream::reply)
}

/// Asks the providers for `body` and joins their answers; `Err` is the response for the client.
async fn answer(
    app: &App,
    config: &Config,
    mut body: Value,
    fresh: bool,
) -> Result<Upstream, Box<Response>> {
    let Some(obj) = body.as_object_mut() else {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The request body is not a JSON object. Send a JSON object.",
        )
        .into());
    };
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(&config.default_model)
        .to_owned();
    let groups = match obj.get("questions") {
        // The provider reports a missing `questions` itself.
        None => vec![(model, None)],
        Some(input) => match questions::expand(input, &config.questions)
            .and_then(|qs| questions::group(qs, &model))
        {
            Ok(groups) => groups.into_iter().map(|(m, qs)| (m, Some(qs))).collect(),
            Err(e) => return Err(error(StatusCode::BAD_REQUEST, e).into()),
        },
    };
    // Every model resolves before any call, so a request bound to fail costs no provider anything.
    let mut refreshed = false;
    let mut resolved = Vec::new();
    for (model, qs) in groups {
        match find_model(app, config, &model, &mut refreshed).await {
            Ok((p, upstream_model)) => resolved.push((p, upstream_model, qs)),
            Err((status, msg)) => return Err(error(status, msg).into()),
        }
    }
    let total = AtomicUsize::new(0);
    let calls = batch(resolved).into_iter().map(|(p, upstream_model, qs)| {
        let mut body = body.clone();
        if let Some(qs) = qs {
            body["questions"] = Value::Object(qs);
        }
        let total = &total;
        async move {
            match forward(app, p, upstream_model, body, total, config.cache, fresh).await {
                Ok(u) if u.status.is_success() => Ok(u),
                Ok(u) => Err(u.reply()),
                Err((status, msg)) => Err(error(status, msg)),
            }
        }
    });
    // The first failure answers the request and drops the calls still running.
    let mut parts = try_join_all(calls).await.map_err(Box::new)?;
    if parts.len() == 1 {
        return Ok(parts.pop().unwrap());
    }
    let mut answers = Vec::new();
    for u in &parts {
        match serde_json::from_slice(&u.bytes) {
            Ok(v) => answers.push(v),
            Err(_) => {
                let (status, msg) = unreadable(&u.provider);
                return Err(error(status, msg).into());
            }
        }
    }
    let providers: Vec<_> = parts.iter().map(|u| u.provider.as_str()).collect();
    Ok(Upstream {
        provider: providers.join(", "),
        ms: parts.iter().map(|u| u.ms).max().unwrap_or(0),
        status: StatusCode::OK,
        bytes: serde_json::to_vec(&questions::merge(answers)).unwrap(),
        cached: parts.iter().all(|u| u.cached),
    })
}

/// The classifier's questions replace any `questions` the client sent.
fn call_body(mut body: Value, questions: Value) -> Result<Value, String> {
    let obj = body
        .as_object_mut()
        .ok_or("The request body is not a JSON object. Send a JSON object.")?;
    obj.insert("questions".into(), questions);
    Ok(body)
}

/// Lists one provider's models. A provider without `GET /v1/models` falls back to its configured list.
async fn provider_models(app: &App, p: &Provider) -> Value {
    let resp = request(app, p, reqwest::Method::GET, "/v1/models")
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let fallback = || p.models.iter().map(|m| json!(m)).collect::<Vec<_>>();
    let (ok, error, listed) = match resp {
        Ok(r) if r.status().is_success() => (true, None, r.json::<Value>().await.ok()),
        Ok(r) if r.status() == StatusCode::NOT_FOUND => (true, None, None),
        Ok(r) if matches!(r.status().as_u16(), 401 | 403) && p.key.is_none() => {
            (false, Some("API key not set".to_owned()), None)
        }
        Ok(r) => (false, Some(format!("HTTP {}", r.status())), None),
        Err(e) if e.is_connect() || e.is_timeout() => {
            (false, Some("not reachable".to_owned()), None)
        }
        Err(e) => (false, Some(e.without_url().to_string()), None),
    };
    let stack = p.installer.as_deref().and_then(install::Stack::parse);
    // clm-serve opens its port only after downloading its head, which takes minutes on first start.
    if !ok
        && let Some(stack) = stack
        && tokio::task::spawn_blocking(move || stack.running())
            .await
            .unwrap_or(false)
    {
        return json!({ "id": p.id, "name": p.name, "ok": false, "warning": "Downloading the model. Wait until it finishes.", "models": fallback() });
    }
    // clm-serve lists {"models": [{"name"}]}; OpenAI-style servers list {"data": [{"id"}]}.
    let names: Vec<Value> = listed
        .and_then(|v| {
            let items = v["models"].as_array().or(v["data"].as_array())?.clone();
            Some(
                items
                    .iter()
                    .filter_map(|m| m["name"].as_str().or(m["id"].as_str()).map(|s| json!(s)))
                    .collect(),
            )
        })
        .filter(|ms: &Vec<Value>| !ms.is_empty())
        .unwrap_or_else(fallback);

    let mut out = json!({ "id": p.id, "name": p.name, "ok": ok, "models": names });
    if let Some(e) = error {
        out["error"] = json!(e);
    }
    // Both servers answer before their model has loaded; requests fail until then.
    if ok
        && let Some(stack) = stack
        && !model_loaded(app, p, stack).await
    {
        out["warning"] = json!("The model is loading. Wait a moment.");
    }
    out
}

/// clm-serve reports `embedder: true` once its encoder is up; the H2O-Lightning shim answers `/health` with 200
/// only once vLLM serves the model.
async fn model_loaded(app: &App, p: &Provider, stack: install::Stack) -> bool {
    let Ok(r) = request(app, p, reqwest::Method::GET, "/health")
        .timeout(Duration::from_secs(10))
        .send()
        .await
    else {
        return false;
    };
    match stack {
        install::Stack::Clm => r.json::<Value>().await.is_ok_and(|h| h["embedder"] == true),
        install::Stack::H2oLightning => r.status().is_success(),
    }
}

async fn refresh_models(app: &App, config: &Config) -> Vec<Value> {
    let all = join_all(config.providers.iter().map(|p| provider_models(app, p))).await;
    let mut listed = app.listed.write().await;
    listed.clear();
    for p in &all {
        let names = p["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str().map(String::from));
        listed.insert(
            p["id"].as_str().unwrap_or_default().to_owned(),
            names.collect(),
        );
    }
    all
}

async fn models(State(app): State<Arc<App>>) -> Json<Value> {
    let config = app.config.read().await.clone();
    let providers = refresh_models(&app, &config).await;
    let data: Vec<Value> = providers
        .iter()
        .flat_map(|p| {
            let id = p["id"].as_str().unwrap_or_default().to_owned();
            p["models"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |m| {
                    m.as_str()
                        .map(|m| json!({ "id": format!("{id}/{m}"), "provider": id, "model": m }))
                })
        })
        .collect();
    Json(json!({ "data": data, "providers": providers }))
}

fn public(config: &Config) -> Value {
    let providers: Vec<Value> = config
        .providers
        .iter()
        .map(|p| {
            json!({
                "id": p.id, "name": p.name, "url": p.url, "key_set": p.key.is_some(),
                "models": p.models, "installer": p.installer, "description": p.description,
            })
        })
        .collect();
    json!({ "default_model": config.default_model, "providers": providers })
}

async fn require_client(
    state: State<Arc<App>>,
    peer: ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    require_key(state, peer, req, next, keys::Role::Client).await
}

async fn require_admin(
    state: State<Arc<App>>,
    peer: ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    require_key(state, peer, req, next, keys::Role::Admin).await
}

/// With any jeff key set, even an expired one, every API and management request must carry a valid key, and
/// management requests an admin key.
async fn require_key(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
    needed: keys::Role,
) -> Response {
    app.refresh_from_disk().await;
    if app.auth_required().await {
        let now = keys::now();
        let ip = peer.ip();
        let blocked = app
            .failures
            .lock()
            .unwrap()
            .get(&ip)
            .is_some_and(|&(n, start)| n >= MAX_FAILURES && now < start + FAILURE_WINDOW_SECS);
        if blocked {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many failed key checks. Try again in a minute.",
            );
        }
        let auth = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let role = {
            let config = app.config.read().await;
            let stored = &config.keys;
            match req.extensions().get::<SessionKey>() {
                Some(SessionKey(digest)) => {
                    keys::verify_digest(digest, &app.static_keys, stored, now)
                }
                None => keys::verify(auth, &app.static_keys, stored, now),
            }
        };
        if let Some(role) = role
            && !role.allows(needed)
        {
            return error(
                StatusCode::FORBIDDEN,
                "This is a client key. Use an admin key to manage jeff.",
            );
        }
        if role.is_none() {
            let mut failures = app.failures.lock().unwrap();
            // Old windows are dropped once the map grows, so a flood of addresses cannot exhaust memory.
            if failures.len() > 10_000 {
                failures.retain(|_, &mut (_, start)| now < start + FAILURE_WINDOW_SECS);
            }
            let entry = failures.entry(ip).or_insert((0, now));
            if now >= entry.1 + FAILURE_WINDOW_SECS {
                *entry = (0, now);
            }
            entry.0 += 1;
            eprintln!(
                "auth failure from {ip} on {} {} ({} in this window)",
                req.method(),
                req.uri().path(),
                entry.0
            );
            return error(
                StatusCode::UNAUTHORIZED,
                "No valid jeff key in the request. Send it as Authorization: Bearer <key>.",
            );
        }
    }
    let mut res = next.run(req).await;
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// Answers only to localhost, IP literals and allowed names, so a page on another domain cannot reach the UI
/// through DNS rebinding.
async fn check_host(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(n, _)| n),
    };
    let name = name.to_ascii_lowercase();
    let ok = name == "localhost"
        || name.parse::<std::net::IpAddr>().is_ok()
        || app.allowed_hosts.contains(&name);
    if !ok {
        return error(
            StatusCode::MISDIRECTED_REQUEST,
            format!("Host '{name}' is not allowed. Start jeff with --allowed-host {name}."),
        );
    }
    // A page on any site can send a plain POST to 127.0.0.1 without a CORS preflight; browsers mark it
    // with that site's Origin, so a write whose Origin is not this host never reaches the handlers.
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if req.method() != Method::GET
        && req.method() != Method::HEAD
        && origin.is_some_and(|o| o.split_once("://").map(|(_, h)| h) != Some(host))
    {
        return error(
            StatusCode::FORBIDDEN,
            "Cross-site request refused. Open the admin page directly.",
        );
    }
    next.run(req).await
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    res
}

fn key_view(k: &keys::StoredKey) -> Value {
    json!({ "id": k.id, "name": k.name, "role": k.role, "prefix": k.prefix, "created_at": k.created_at, "expires_at": k.expires_at })
}

async fn list_keys(
    State(app): State<Arc<App>>,
    session: Option<Extension<SessionKey>>,
    headers: HeaderMap,
) -> Json<Value> {
    let config = app.config.read().await;
    let digest = match session {
        Some(Extension(SessionKey(digest))) => Some(digest),
        None => bearer(&headers).map(keys::sha256),
    };
    let current = config
        .keys
        .iter()
        .find(|k| Some(&k.sha256) == digest.as_ref())
        .map(|k| k.id.clone());
    Json(json!({
        "static_keys": app.static_keys.len(),
        "keys": config.keys.iter().map(key_view).collect::<Vec<_>>(),
        "current_key_id": current,
    }))
}

/// The cookie that carries one half of an admin page session; `SESSION_HEADER` carries the other.
const SESSION_COOKIE: &str = "jeff_session";
const SESSION_HEADER: &str = "x-jeff-session";

/// The SHA-256 of the key a request's session stands for, set by `cookie_auth` for `require_key`.
#[derive(Clone)]
struct SessionKey(String);

/// The cookie half and the header half of the request's session.
fn session_halves(headers: &HeaderMap) -> Option<(&str, &str)> {
    let cookie = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(SESSION_COOKIE)?.strip_prefix('='))?;
    let header = headers.get(SESSION_HEADER)?.to_str().ok()?;
    Some((cookie, header))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Marks a request without `Authorization` with the key its session stands for, so the key check that follows
/// still refuses a revoked or expired key. Only the admin port reads sessions.
async fn cookie_auth(State(app): State<Arc<App>>, mut req: Request, next: Next) -> Response {
    let key = session_halves(req.headers()).and_then(|(cookie, header)| {
        app.sessions
            .lock()
            .unwrap()
            .find(cookie, header, keys::now())
    });
    if !req.headers().contains_key(header::AUTHORIZATION)
        && let Some(key) = key
    {
        req.extensions_mut().insert(SessionKey(key));
    }
    next.run(req).await
}

/// Trades the admin key in `Authorization`, already checked by `require_admin`, for a session. A session cannot
/// start another, so a copied session ends at sign-out.
async fn sign_in(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let Some(key) = bearer(&headers) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "No admin key in the request. Send it as Authorization: Bearer <key>.",
        );
    };
    let known: Vec<String> = app
        .config
        .read()
        .await
        .keys
        .iter()
        .map(|k| k.sha256.clone())
        .chain(app.static_keys.iter().map(|k| keys::sha256(k)))
        .collect();
    let created = app
        .sessions
        .lock()
        .unwrap()
        .create(&keys::sha256(key), keys::now(), |k| {
            known.iter().any(|s| s == k)
        });
    let (cookie, half) = match created {
        Ok(halves) => halves,
        Err(cause_in) => return sessions_not_saved(cause_in),
    };
    // Behind a TLS proxy jeff sees plain http, so the browser's Origin tells whether the page is on https.
    let secure = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o.starts_with("https://"));
    let cookie = format!(
        "{SESSION_COOKIE}={cookie}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        sessions::LIFETIME_SECS,
        if secure { "; Secure" } else { "" }
    );
    (
        [(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap())],
        Json(json!({ "session": half })),
    )
        .into_response()
}

/// The client never sees the server's path.
fn sessions_not_saved(cause_in: &str) -> Response {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Couldn't save sessions.json. Check {cause_in} for the cause."),
    )
}

/// Keeps the cookie when the file cannot be written, so a session left on disk is never hidden from its browser.
async fn sign_out(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Some((cookie, header)) = session_halves(&headers)
        && let Err(cause_in) = app.sessions.lock().unwrap().remove(cookie, header)
    {
        return sessions_not_saved(cause_in);
    }
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            HeaderValue::from_static("jeff_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
    )
        .into_response()
}

#[derive(Deserialize)]
struct KeyInput {
    name: String,
    role: keys::Role,
    /// Unix seconds; `null` never expires.
    expires_at: Option<u64>,
}

async fn create_key(State(app): State<Arc<App>>, Json(input): Json<KeyInput>) -> Response {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return error(
            StatusCode::BAD_REQUEST,
            "The name is empty or longer than 64 characters. Use 1 to 64 characters.",
        );
    }
    if input.expires_at.is_some_and(|t| t <= keys::now()) {
        return error(
            StatusCode::BAD_REQUEST,
            "The expiry is in the past. Pick a future date.",
        );
    }
    let mut config = app.config.write().await;
    if input.role == keys::Role::Client && !app.has_admin(&config.keys) {
        return error(
            StatusCode::BAD_REQUEST,
            "The first key must be an admin key. Create an admin key first, or this page loses access.",
        );
    }
    let (key, record) = keys::generate(name, input.role, input.expires_at);
    let mut next = config.clone();
    next.keys.push(record.clone());
    if let Err(r) = app.save(&next).await {
        return *r;
    }
    *config = next;
    let mut view = key_view(&record);
    view["key"] = json!(key);
    (StatusCode::CREATED, Json(view)).into_response()
}

async fn revoke_key(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let mut config = app.config.write().await;
    let mut next = config.clone();
    next.keys.retain(|k| k.id != id);
    if next.keys.len() == config.keys.len() {
        return error(
            StatusCode::NOT_FOUND,
            format!("No key with id '{id}'. Check the id with jeff keys list."),
        );
    }
    if !next.keys.is_empty() && app.has_admin(&config.keys) && !app.has_admin(&next.keys) {
        return error(
            StatusCode::CONFLICT,
            "This is the last admin key. Create another admin key or revoke the client keys first.",
        );
    }
    if let Err(r) = app.save(&next).await {
        return *r;
    }
    *config = next;
    StatusCode::NO_CONTENT.into_response()
}

async fn list_questions(State(app): State<Arc<App>>) -> Json<questions::Questions> {
    Json(app.config.read().await.questions.clone())
}

async fn get_question(State(app): State<Arc<App>>, Path(key): Path<String>) -> Response {
    match app.config.read().await.questions.get(&key) {
        Some(q) => Json(q.clone()).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            format!("No question '{key}'. Check the key or create the question first."),
        ),
    }
}

/// Applies `change` to a copy of the config and saves it; an `Err` leaves both file and memory as they were.
async fn change_config(
    app: &App,
    change: impl FnOnce(&mut Config) -> Result<(), questions::Change>,
) -> Result<Config, Box<Response>> {
    let mut config = app.config.write().await;
    let mut next = config.clone();
    change(&mut next).map_err(|c| {
        Box::new(match c {
            questions::Change::NotFound(m) => error(StatusCode::NOT_FOUND, m),
            questions::Change::Exists(m) => error(StatusCode::CONFLICT, m),
            questions::Change::Invalid(m) => error(StatusCode::BAD_REQUEST, m),
        })
    })?;
    app.save(&next).await?;
    *config = next;
    Ok(config.clone())
}

async fn create_questions(
    State(app): State<Arc<App>>,
    Json(input): Json<questions::Questions>,
) -> Response {
    match change_config(&app, |c| questions::create(&mut c.questions, input)).await {
        Ok(c) => (StatusCode::CREATED, Json(c.questions)).into_response(),
        Err(r) => *r,
    }
}

async fn update_question(
    State(app): State<Arc<App>>,
    Path(key): Path<String>,
    Json(q): Json<Value>,
) -> Response {
    match change_config(&app, |c| questions::update(&mut c.questions, &key, q)).await {
        Ok(c) => Json(c.questions[&key].clone()).into_response(),
        Err(r) => *r,
    }
}

async fn delete_question(State(app): State<Arc<App>>, Path(key): Path<String>) -> Response {
    match change_config(&app, |c| questions::remove(&mut c.questions, &key)).await {
        Ok(c) => Json(c.questions).into_response(),
        Err(r) => *r,
    }
}

async fn list_classifiers(State(app): State<Arc<App>>) -> Json<classifiers::Classifiers> {
    Json(app.config.read().await.classifiers.clone())
}

fn unknown_classifier(key: &str) -> Response {
    error(
        StatusCode::NOT_FOUND,
        format!("No classifier '{key}'. Check the key or create the classifier first."),
    )
}

async fn get_classifier(State(app): State<Arc<App>>, Path(key): Path<String>) -> Response {
    match app.config.read().await.classifiers.get(&key) {
        Some(c) => Json(c.clone()).into_response(),
        None => unknown_classifier(&key),
    }
}

async fn call_classifier(
    State(app): State<Arc<App>>,
    Path(key): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let config = app.config.read().await.clone();
    let Some(c) = config.classifiers.get(&key) else {
        return unknown_classifier(&key);
    };
    run_classifier(&app, &config, &key, c, body, no_cache(&headers)).await
}

/// Runs an unsaved classifier sent in the body, as the form's Try does; it skips questions exactly as a saved one.
async fn try_classifier(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let config = app.config.read().await.clone();
    match take_draft(body) {
        Ok((c, body)) => run_classifier(&app, &config, "draft", &c, body, no_cache(&headers)).await,
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// Splits `{"classifier": {...}, ...}` into the classifier and the rest of the call body.
fn take_draft(mut body: Value) -> Result<(Value, Value), String> {
    let obj = body
        .as_object_mut()
        .ok_or("The request body is not a JSON object. Send a JSON object.")?;
    match obj.remove("classifier") {
        Some(c @ Value::Object(_)) => Ok((c, body)),
        _ => Err("The classifier is not an object. Send an object with a questions array.".into()),
    }
}

async fn run_classifier(
    app: &App,
    config: &Config,
    key: &str,
    c: &Value,
    body: Value,
    fresh: bool,
) -> Response {
    let (input, skipped) = match classifiers::resolve(key, c, &config.questions) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let body = match call_body(body, input) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    match answer(app, config, body, fresh).await {
        Ok(u) if u.status.is_success() => Upstream {
            bytes: classifiers::with_skipped(u.bytes, &skipped),
            ..u
        }
        .reply(),
        Ok(u) => u.reply(),
        Err(res) => *res,
    }
}

async fn create_classifiers(
    State(app): State<Arc<App>>,
    Json(input): Json<classifiers::Classifiers>,
) -> Response {
    match change_config(&app, |c| classifiers::create(&mut c.classifiers, input)).await {
        Ok(c) => (StatusCode::CREATED, Json(c.classifiers)).into_response(),
        Err(r) => *r,
    }
}

async fn update_classifier(
    State(app): State<Arc<App>>,
    Path(key): Path<String>,
    Json(classifier): Json<Value>,
) -> Response {
    match change_config(&app, |c| {
        classifiers::update(&mut c.classifiers, &key, classifier)
    })
    .await
    {
        Ok(c) => Json(c.classifiers[&key].clone()).into_response(),
        Err(r) => *r,
    }
}

async fn delete_classifier(State(app): State<Arc<App>>, Path(key): Path<String>) -> Response {
    match change_config(&app, |c| classifiers::remove(&mut c.classifiers, &key)).await {
        Ok(c) => Json(c.classifiers).into_response(),
        Err(r) => *r,
    }
}

async fn get_config(State(app): State<Arc<App>>) -> Json<Value> {
    Json(public(&*app.config.read().await))
}

#[derive(Deserialize, Clone)]
struct ProviderInput {
    id: String,
    name: String,
    url: String,
    /// Absent keeps the stored key for this id while its URL stays the same, "" clears it.
    key: Option<String>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    installer: Option<String>,
    /// Absent keeps the stored value for this id; the admin page does not send it.
    #[serde(default)]
    connect_timeout: Option<u64>,
    /// Absent keeps the stored value for this id; the admin page does not send it.
    #[serde(default)]
    max_time: Option<u64>,
    /// Absent keeps the stored description for this id, "" clears it.
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize, Clone)]
struct ConfigInput {
    default_model: String,
    providers: Vec<ProviderInput>,
}

fn build_config(
    current: &Config,
    input: ConfigInput,
    allow_private: bool,
) -> Result<Config, String> {
    let mut providers = Vec::new();
    for p in input.providers {
        let id = p.id.trim().to_owned();
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "Provider id '{id}' has characters other than a-z, 0-9 and '-'. Rename it."
            ));
        }
        if providers.iter().any(|q: &Provider| q.id == id) {
            return Err(format!(
                "Provider id '{id}' is used twice. Rename one of them."
            ));
        }
        let url = p.url.trim().to_owned();
        let parsed = parse_provider_url(&url, allow_private).map_err(|e| format!("{id}: {e}"))?;
        // A saved key follows its provider only to the URL it was entered for, so it cannot be redirected.
        let key = match p.key {
            Some(k) => Some(k.trim().to_owned()).filter(|k| !k.is_empty()),
            None => current
                .providers
                .iter()
                .find(|q| q.id == id && q.url == url)
                .and_then(|q| q.key.clone()),
        };
        if key.is_some() && parsed.scheme() == "http" && !is_loopback_url(&parsed) {
            return Err(format!(
                "Provider '{id}' sends its key over http://. Use https://."
            ));
        }
        let models = p
            .models
            .iter()
            .map(|m| m.trim().to_owned())
            .filter(|m| !m.is_empty())
            .collect();
        let name = Some(p.name.trim().to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone());
        if p.connect_timeout == Some(0) || p.max_time == Some(0) {
            return Err(format!(
                "Provider '{id}' has a timeout of 0. Set connect_timeout and max_time to 1 second or more."
            ));
        }
        let stored = current.providers.iter().find(|q| q.id == id);
        providers.push(Provider {
            connect_timeout: p.connect_timeout.or(stored.and_then(|q| q.connect_timeout)),
            max_time: p.max_time.or(stored.and_then(|q| q.max_time)),
            description: match p.description {
                Some(d) => Some(d.trim().to_owned()).filter(|d| !d.is_empty()),
                None => stored.and_then(|q| q.description.clone()),
            },
            id,
            name,
            url,
            key,
            models,
            installer: p.installer,
        });
    }
    let default_model = input.default_model.trim().to_owned();
    let known = default_model
        .split_once('/')
        .is_some_and(|(id, m)| !m.is_empty() && providers.iter().any(|p| p.id == id));
    if !known {
        return Err("The default model names no configured provider. Use provider/model with a provider from the list.".into());
    }
    Ok(Config {
        default_model,
        providers,
        keys: current.keys.clone(),
        questions: current.questions.clone(),
        classifiers: current.classifiers.clone(),
        cache: current.cache,
    })
}

/// Resolves provider host names at save time, so a refused address is reported here rather than as an
/// unreachable provider later. Names that do not resolve yet are saved; the resolver checks them on every request.
async fn check_provider_hosts(providers: &[Provider], allow_private: bool) -> Result<(), String> {
    for p in providers {
        let Some(host) = reqwest::Url::parse(&p.url)
            .ok()
            .and_then(|u| u.domain().map(str::to_owned))
        else {
            continue;
        };
        let lookup = tokio::net::lookup_host((host.as_str(), 0));
        if let Ok(Ok(addrs)) = tokio::time::timeout(Duration::from_secs(5), lookup).await {
            for a in addrs {
                check_ip(a.ip(), allow_private).map_err(|e| format!("{}: {host}: {e}", p.id))?;
            }
        }
    }
    Ok(())
}

/// Keys, questions and classifiers have their own endpoints; a provider save built from an older snapshot must not undo them.
async fn put_config(State(app): State<Arc<App>>, Json(input): Json<ConfigInput>) -> Response {
    // The DNS checks run without the lock, so a slow resolver does not stall every request meanwhile.
    let snapshot = app.config.read().await.clone();
    let next = match build_config(&snapshot, input.clone(), app.allow_private) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    if let Err(e) = check_provider_hosts(&next.providers, app.allow_private).await {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let mut config = app.config.write().await;
    // Built again on the live config: what the UI does not send, such as keys or timeouts, may have changed meanwhile.
    let next = match build_config(&config, input, app.allow_private) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    if let Err(r) = app.save(&next).await {
        return *r;
    }
    *config = next;
    Json(public(&config)).into_response()
}

async fn install_action(app: Arc<App>, action: String, stack: install::Stack) -> Response {
    let Some(action) = install::Action::parse(&action) else {
        return error(
            StatusCode::NOT_FOUND,
            format!("Unknown action '{action}'. Use install or remove."),
        );
    };
    match app.tasks.start(stack, action) {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

async fn install_status(app: Arc<App>, stack: install::Stack) -> Json<Value> {
    let provider = app
        .config
        .read()
        .await
        .providers
        .iter()
        .find(|p| p.installer.as_deref() == Some(stack.id()))
        .cloned()
        // A provider added on the page but not saved yet still gets its install tracked to the end.
        .or_else(|| {
            Some(Provider {
                id: stack.id().to_owned(),
                name: stack.name().to_owned(),
                url: format!("http://127.0.0.1:{}", stack.port()),
                key: None,
                models: Vec::new(),
                installer: Some(stack.id().to_owned()),
                connect_timeout: None,
                max_time: None,
                description: None,
            })
        });
    let mut loaded = false;
    if let Some(p) = &provider {
        loaded = model_loaded(&app, p, stack).await;
    }
    let warm = &app.warm[stack as usize];
    if !loaded {
        warm.store(0, Ordering::Relaxed);
    } else if let Some(p) = provider.filter(|_| {
        warm.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }) {
        let app2 = app.clone();
        tokio::spawn(async move {
            let body = json!({ "state": "warm-up", "questions": { "ok": { "type": "noul", "instructions": "The state is a warm-up." } } });
            let sent = request(&app2, &p, reqwest::Method::POST, "/v1/systemone")
                .json(&body)
                .timeout(Duration::from_secs(300))
                .send()
                .await;
            let ok = sent.is_ok_and(|r| r.status().is_success());
            app2.warm[stack as usize].store(if ok { 2 } else { 0 }, Ordering::Relaxed);
        });
    }
    let warm = warm.load(Ordering::Relaxed) == 2;
    let steps = tokio::task::spawn_blocking(move || stack.steps(loaded, warm))
        .await
        .unwrap_or_default();
    let mut out = app.tasks.get(stack);
    out["steps"] = json!(steps);
    Json(out)
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// A classifier built on Jev-compatible System One models: one API in front of local and cloud providers.
#[derive(Parser)]
#[command(name = "jeff", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the API and the web UI (the default command)
    Serve(ServeArgs),
    /// Install a local provider in Docker
    Install { stack: install::Stack },
    /// Stop and remove a local provider's containers; downloaded weights are kept
    Remove { stack: install::Stack },
    /// Manage jeff access keys in the config file; a running server picks changes up on the next request
    Keys {
        #[command(subcommand)]
        action: KeysAction,
        /// The config file [default: ~/.jeff/jeff.json]
        #[arg(long, env = "JEFF_CONFIG", global = true)]
        config: Option<String>,
    },
    /// Ask saved or custom questions through a running jeff and print the answer JSON
    Ask(AskArgs),
    /// Manage saved questions through a running jeff; JEFF_API_KEY must hold an admin key
    Questions {
        #[command(subcommand)]
        action: QuestionsAction,
        /// The jeff admin page address, where questions are managed
        #[arg(long, env = "JEFF_ADMIN_URL", default_value = "http://127.0.0.1:8081", value_parser = parse_url, global = true)]
        url: String,
    },
    /// Manage classifiers through a running jeff; JEFF_API_KEY must hold an admin key
    Classifiers {
        #[command(subcommand)]
        action: ClassifiersAction,
        /// The jeff admin page address, where classifiers are managed
        #[arg(long, env = "JEFF_ADMIN_URL", default_value = "http://127.0.0.1:8081", value_parser = parse_url, global = true)]
        url: String,
    },
}

#[derive(Args)]
struct AskArgs {
    /// Saved questions to ask, by key
    #[arg(conflicts_with = "request")]
    keys: Vec<String>,
    /// Call a saved classifier instead of naming questions
    #[arg(long, conflicts_with_all = ["keys", "questions", "questions_file", "request"])]
    classifier: Option<String>,
    /// The text the questions are about
    #[arg(
        long,
        conflicts_with_all = ["state_file", "request"],
        required_unless_present_any = ["state_file", "request"]
    )]
    state: Option<String>,
    /// Read the state from a file, or `-` for stdin
    #[arg(long, conflicts_with = "request")]
    state_file: Option<String>,
    /// A JSON map of key to question; an entry without `type` overrides that saved question
    #[arg(long, conflicts_with_all = ["questions_file", "request"])]
    questions: Option<String>,
    /// Read the --questions map from a file, or `-` for stdin
    #[arg(long, conflicts_with = "request")]
    questions_file: Option<String>,
    /// A whole /v1/systemone body with `state` and `questions`, from a file or `-` for stdin
    #[arg(long)]
    request: Option<String>,
    /// The model for questions without their own; it replaces the `model` of --request
    #[arg(long)]
    model: Option<String>,
    /// The jeff API to ask
    #[arg(long, env = "JEFF_URL", default_value = "http://127.0.0.1:8080", value_parser = parse_url)]
    url: String,
    /// Seconds to wait for the connection to jeff
    #[arg(long, default_value_t = 5)]
    connect_timeout: u64,
    /// Seconds to wait for the whole answer
    #[arg(long, default_value_t = 60)]
    max_time: u64,
    /// Ask the providers again instead of reusing jeff's cached answer
    #[arg(long)]
    no_cache: bool,
    /// Write the answer JSON to this file instead of printing it
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Subcommand)]
enum QuestionsAction {
    /// List saved questions: key, type, model and instructions
    List {
        /// Write the questions as JSON to this file instead
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Print one saved question as JSON
    Get {
        key: String,
        /// Write the question JSON to this file instead of printing it
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Save new questions from a JSON map of key to question; an existing key fails and saves nothing
    Add {
        /// A JSON file, or `-` for stdin
        file: String,
    },
    /// Replace one saved question with the JSON question in a file
    Update {
        key: String,
        /// A JSON file, or `-` for stdin
        file: String,
    },
    /// Delete one saved question
    Remove { key: String },
}

#[derive(Subcommand)]
enum ClassifiersAction {
    /// List classifiers: key, model and questions
    List {
        /// Write the classifiers as JSON to this file instead
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Print one classifier as JSON
    Get {
        key: String,
        /// Write the classifier JSON to this file instead of printing it
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Save a new classifier, or every classifier of a JSON map in --file; an existing key fails
    Add(AddClassifierArgs),
    /// Replace a classifier; without --model its override is removed
    Update(UpdateClassifierArgs),
    /// Delete one classifier
    Remove { key: String },
}

#[derive(Args)]
struct AddClassifierArgs {
    #[arg(required_unless_present = "file")]
    key: Option<String>,
    /// Saved question keys, in the order to ask them
    #[arg(required_unless_present = "file")]
    questions: Vec<String>,
    /// The model for every question of the classifier, in place of their own
    #[arg(long)]
    model: Option<String>,
    /// What the classifier is for
    #[arg(long)]
    description: Option<String>,
    /// A JSON map of key to classifier, or `-` for stdin
    #[arg(long, conflicts_with_all = ["key", "questions", "model", "description"])]
    file: Option<String>,
}

#[derive(Args)]
struct UpdateClassifierArgs {
    key: String,
    /// Saved question keys, in the order to ask them
    #[arg(required_unless_present = "file")]
    questions: Vec<String>,
    /// The model for every question of the classifier, in place of their own
    #[arg(long)]
    model: Option<String>,
    /// What the classifier is for
    #[arg(long)]
    description: Option<String>,
    /// A JSON classifier, or `-` for stdin
    #[arg(long, conflicts_with_all = ["questions", "model", "description"])]
    file: Option<String>,
}

#[derive(Subcommand)]
enum KeysAction {
    /// List keys without revealing them
    List,
    /// Create a key and print it once
    Create {
        /// Who or what uses the key, for example "ci" or "claude-agent"
        #[arg(long)]
        name: String,
        /// `client` reaches /v1 only; `admin` also manages providers and keys through /api and the admin page
        #[arg(long, value_enum, default_value_t = keys::Role::Client)]
        role: keys::Role,
        /// `never`, a number of days such as `30d`, or a date `YYYY-MM-DD` (valid through the end of that day, UTC)
        #[arg(long, default_value = "30d")]
        expires: String,
        /// The admin page address in the sign-in link printed for an admin key
        #[arg(long, env = "JEFF_ADMIN_URL", default_value = "http://127.0.0.1:8081", value_parser = parse_url)]
        admin_url: String,
    },
    /// Revoke a key by its id
    Revoke { id: String },
}

/// Parses `serve` defaults (flags and environment) when no subcommand is given.
#[derive(Parser)]
struct ServeDefaults {
    #[command(flatten)]
    args: ServeArgs,
}

#[derive(Args)]
struct ServeArgs {
    /// Where the API listens: /v1/systemone, /v1/models and /health. ADDR:PORT, or a bare PORT on 127.0.0.1
    #[arg(long, env = "JEFF_API_ADDR", default_value = "127.0.0.1:8080", value_parser = parse_addr)]
    api: String,
    /// Where the web UI listens. It also manages providers, so keep it off public interfaces
    #[arg(long, env = "JEFF_UI_ADDR", default_value = "127.0.0.1:8081", value_parser = parse_addr)]
    ui: String,
    /// Serve the API only
    #[arg(long)]
    no_ui: bool,
    /// Providers and access keys, created on the first save [default: ~/.jeff/jeff.json]
    #[arg(long, env = "JEFF_CONFIG")]
    config: Option<String>,
    /// A key that never expires; repeat the flag or comma-separate the env var for several. With any key here or in
    /// the config, every /v1 and /api request must send `Authorization: Bearer <key>`; the admin page signs in instead
    #[arg(
        long = "api-key",
        env = "JEFF_API_KEY",
        hide_env_values = true,
        value_delimiter = ','
    )]
    api_keys: Vec<String>,
    /// A host name the UI answers to besides localhost and IP addresses, such as the name of a reverse proxy
    #[arg(
        long = "allowed-host",
        env = "JEFF_ALLOWED_HOSTS",
        value_delimiter = ','
    )]
    allowed_hosts: Vec<String>,
    /// Let providers use private network addresses such as 10.0.0.0/8 and 192.168.0.0/16. Loopback is always
    /// allowed, link-local and cloud metadata addresses never are
    #[arg(long, env = "JEFF_ALLOW_PRIVATE_PROVIDERS")]
    allow_private_providers: bool,
}

fn parse_addr(s: &str) -> Result<String, String> {
    if let Ok(port) = s.parse::<u16>() {
        return Ok(format!("127.0.0.1:{port}"));
    }
    s.parse::<std::net::SocketAddr>()
        .map(|_| s.to_owned())
        .map_err(|_| {
            format!("'{s}' is not ADDR:PORT or PORT. Use a form such as 127.0.0.1:8080 or 8080.")
        })
}

/// The key rides in the fragment, which the browser never sends to the server.
fn sign_in_link(admin_url: &str, key: &str) -> String {
    format!("{}/#key={key}", admin_url.trim_end_matches('/'))
}

fn parse_url(s: &str) -> Result<String, String> {
    if s.starts_with("http://") || s.starts_with("https://") {
        Ok(s.to_owned())
    } else {
        Err("The URL does not start with http:// or https://. Start it with https://.".into())
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let args = match cli.command {
        None => ServeDefaults::parse_from(["jeff"]).args,
        Some(Command::Serve(args)) => args,
        Some(Command::Install { stack }) => return run_install(stack, install::Action::Install),
        Some(Command::Remove { stack }) => return run_install(stack, install::Action::Remove),
        Some(Command::Keys { action, config }) => return run_keys(action, &config_file(config)),
        Some(Command::Ask(args)) => return run_ask(args).await,
        Some(Command::Questions { action, url }) => return run_questions(action, &url).await,
        Some(Command::Classifiers { action, url }) => {
            return run_classifiers(action, &url).await;
        }
    };
    serve(args).await;
}

fn config_file(arg: Option<String>) -> String {
    arg.unwrap_or_else(|| {
        install::home()
            .join("jeff.json")
            .to_string_lossy()
            .into_owned()
    })
}

/// Reads a whole file, or stdin for `-`.
fn read_input(file: &str) -> Result<String, String> {
    if file == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|e| e.to_string())?;
        Ok(text)
    } else {
        fs::read_to_string(file).map_err(|e| format!("Couldn't read {file}: {e}. Check the path."))
    }
}

/// Builds the `/v1/systemone` body: each key asks its saved question, and `--questions` entries go on top.
fn ask_body(
    keys: &[String],
    questions: Option<Value>,
    state: String,
    model: Option<String>,
) -> Result<Value, String> {
    let mut map: questions::Questions = keys.iter().map(|k| (k.clone(), json!({}))).collect();
    match questions {
        None => {}
        Some(Value::Object(extra)) => map.extend(extra),
        Some(_) => return Err("--questions is not a JSON map of key to question. Pass an object keyed by question key.".into()),
    }
    if map.is_empty() {
        return Err("No question given. Name a saved question or pass --questions.".into());
    }
    let mut body = json!({ "state": state, "questions": map });
    if let Some(m) = model {
        body["model"] = Value::String(m);
    }
    Ok(body)
}

/// Builds the `/v1/systemone` body from the `jeff ask` arguments.
fn ask_request(
    args: &AskArgs,
    read: impl Fn(&str) -> Result<String, String>,
) -> Result<Value, String> {
    if let Some(file) = &args.request {
        let mut body: Value =
            serde_json::from_str(&read(file)?).map_err(|e| format!("{file}: {e}"))?;
        let Some(obj) = body.as_object_mut() else {
            return Err("--request is not a JSON object with state and questions. Pass an object with both fields.".into());
        };
        if let Some(m) = &args.model {
            obj.insert("model".into(), json!(m));
        }
        return Ok(body);
    }
    if args.state_file.as_deref() == Some("-") && args.questions_file.as_deref() == Some("-") {
        return Err(
            "--state-file and --questions-file both read stdin. Use --request - instead.".into(),
        );
    }
    let state = match (&args.state, &args.state_file) {
        (Some(text), _) => text.clone(),
        (None, Some(file)) => read(file)?,
        (None, None) => unreachable!("clap requires --state or --state-file"),
    };
    let questions = match (&args.questions, &args.questions_file) {
        (Some(json), _) => Some(serde_json::from_str(json).map_err(|e| {
            format!("--questions is not valid JSON: {e}. To read a file, use --questions-file.")
        })?),
        (None, Some(file)) => {
            Some(serde_json::from_str(&read(file)?).map_err(|e| format!("{file}: {e}"))?)
        }
        (None, None) => None,
    };
    if let Some(key) = &args.classifier {
        questions::check_key(key).map_err(|e| format!("classifier '{key}': {e}"))?;
        let mut body = json!({ "state": state });
        if let Some(m) = &args.model {
            body["model"] = json!(m);
        }
        return Ok(body);
    }
    ask_body(&args.keys, questions, state, args.model.clone())
}

/// Writes pretty JSON to `path` and prints the path.
fn write_json(path: &str, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    fs::write(path, text).map_err(|e| format!("Couldn't write {path}: {e}. Check the path."))?;
    println!("{path}");
    Ok(())
}

/// One line per question: a boxed table for a terminal, tab-separated fields for a pipe.
fn questions_table(qs: &questions::Questions, boxed: bool) -> String {
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let rows: Vec<[String; 4]> = qs
        .iter()
        .map(|(key, q)| {
            let field = |f: &str| q.get(f).and_then(Value::as_str).map_or("-".into(), flat);
            [
                key.clone(),
                field("type"),
                field("model"),
                field("instructions"),
            ]
        })
        .collect();
    table(["key", "type", "model", "instructions"], rows, boxed)
}

/// One line per classifier; a question it cannot ask shows as `key (deleted)`.
fn classifiers_table(
    cs: &classifiers::Classifiers,
    saved: &questions::Questions,
    boxed: bool,
) -> String {
    let rows = cs
        .iter()
        .map(|(key, c)| {
            let skipped = classifiers::skipped(c, saved);
            let keys: Vec<String> = c["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|k| match skipped.iter().any(|s| s == k) {
                    true => format!("{k} (deleted)"),
                    false => k.to_owned(),
                })
                .collect();
            let field = |f: &str| c.get(f).and_then(Value::as_str).unwrap_or("-").to_owned();
            let description = field("description")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            [key.clone(), field("model"), keys.join(", "), description]
        })
        .collect();
    table(["key", "model", "questions", "description"], rows, boxed)
}

/// A boxed table for a terminal, tab-separated fields for a pipe. The last column is cut to fit a terminal.
fn table<const N: usize>(header: [&str; N], rows: Vec<[String; N]>, boxed: bool) -> String {
    if !boxed {
        return rows.iter().map(|r| r.join("\t") + "\n").collect();
    }
    const WIDEST: usize = 60;
    let cut = |s: &String| match s.chars().count() > WIDEST {
        true => s.chars().take(WIDEST - 1).chain(['…']).collect(),
        false => s.clone(),
    };
    let rows: Vec<[String; N]> = std::iter::once(header.map(String::from))
        .chain(rows.into_iter().map(|mut r| {
            r[N - 1] = cut(&r[N - 1]);
            r
        }))
        .collect();
    let widths: Vec<usize> = (0..N)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    let rule = |l: &str, m: &str, r: &str| {
        let cells: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        format!("{l}{}{r}\n", cells.join(m))
    };
    let line = |row: &[String; N]| {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(s, w)| format!(" {s}{} ", " ".repeat(w - s.chars().count())))
            .collect();
        format!("│{}│\n", cells.join("│"))
    };
    let mut out = rule("┌", "┬", "┐") + &line(&rows[0]) + &rule("├", "┼", "┤");
    for row in &rows[1..] {
        out += &line(row);
    }
    out + &rule("└", "┴", "┘")
}

/// The server accepts a comma-separated `JEFF_API_KEY`; a client sends the first key.
fn client_key(env_value: Option<String>) -> Option<String> {
    env_value?
        .split(',')
        .map(str::trim)
        .find(|k| !k.is_empty())
        .map(str::to_owned)
}

async fn run_ask(args: AskArgs) {
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1)
    };
    let body = ask_request(&args, read_input).unwrap_or_else(|e| fail(e));
    let base = args.url.trim_end_matches('/');
    let url = format!("{base}{}", ask_target(&args));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(args.connect_timeout))
        .timeout(Duration::from_secs(args.max_time))
        .build()
        .expect("HTTP client");
    let mut req = client.post(&url).json(&body);
    if args.no_cache {
        req = req.header(reqwest::header::CACHE_CONTROL, "no-cache");
    }
    if let Some(key) = client_key(env::var("JEFF_API_KEY").ok()) {
        req = req.bearer_auth(key);
    }
    let failed = |e: reqwest::Error| -> ! {
        if e.is_connect() {
            fail(format!(
                "Couldn't connect to {base}. Start jeff with jeff serve."
            ))
        } else if e.is_timeout() {
            fail(format!(
                "jeff did not answer within {}s. Try again, or check jeff.log.",
                args.max_time
            ))
        } else {
            fail(format!(
                "Couldn't reach {url}: {}. Check that jeff runs and the URL is right.",
                e.without_url()
            ))
        }
    };
    let resp = req.send().await.unwrap_or_else(|e| failed(e));
    let ok = resp.status().is_success();
    let text = resp.text().await.unwrap_or_else(|e| failed(e));
    let answer = serde_json::from_str::<Value>(&text);
    if !ok {
        fail(answer.map_or(text, |v| serde_json::to_string_pretty(&v).unwrap()));
    }
    if let Some(skipped) = answer.as_ref().ok().and_then(|v| v["skipped"].as_array()) {
        let keys: Vec<&str> = skipped.iter().filter_map(Value::as_str).collect();
        eprintln!("skipped: {}", keys.join(", "));
    }
    match (answer, args.output) {
        (Ok(v), Some(path)) => write_json(&path, &v).unwrap_or_else(|e| fail(e)),
        (Ok(v), None) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
        (Err(_), _) => fail(format!(
            "jeff sent an answer that is not JSON: {text}. Check that the URL points at jeff."
        )),
    }
}

fn ask_target(args: &AskArgs) -> String {
    match &args.classifier {
        Some(key) => format!("/v1/classifiers/{key}"),
        None => "/v1/systemone".into(),
    }
}

/// An API error as the CLI prints it: `error[code]: message`, then `log: jeff.log` when the server names its log.
fn error_lines(status: u16, text: &str) -> String {
    let body: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let Some(msg) = body["error"].as_str() else {
        let reason = reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or_default();
        let head = format!("error: HTTP {status} {reason}")
            .trim_end()
            .to_string();
        // A proxy page or an empty body says nothing beyond the status.
        let text = text.trim();
        return match text.is_empty() || text.starts_with('<') {
            true => format!("{head}. Check the URL and jeff.log."),
            false => format!("{head}: {text}"),
        };
    };
    let mut out = match body["code"].as_str() {
        Some(code) => format!("error[{code}]: {msg}"),
        None => format!("error: {msg}"),
    };
    if let Some(log) = body["log"].as_str() {
        out.push_str(&format!("\nlog: {log}"));
    }
    out
}

/// Calls the management API of a running jeff and returns its JSON, or `None` for an empty body. Any failure
/// prints the error and exits.
async fn admin_call(
    base: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Option<Value> {
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1)
    };
    let base = base.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .expect("HTTP client");
    let mut req = client.request(method, format!("{base}{path}"));
    if let Some(body) = body {
        req = req.json(body);
    }
    if let Some(key) = client_key(env::var("JEFF_API_KEY").ok()) {
        req = req.bearer_auth(key);
    }
    let failed = |e: reqwest::Error| -> ! {
        match e.is_connect() {
            true => fail(format!(
                "Couldn't connect to {base}. Start jeff with jeff serve."
            )),
            false => fail(format!(
                "Couldn't reach {base}{path}: {}. Check that jeff runs and the URL is right.",
                e.without_url()
            )),
        }
    };
    let resp = req.send().await.unwrap_or_else(|e| failed(e));
    let status = resp.status();
    let text = resp.text().await.unwrap_or_else(|e| failed(e));
    if !status.is_success() {
        fail(error_lines(status.as_u16(), &text));
    }
    if text.trim().is_empty() {
        return None;
    }
    Some(serde_json::from_str(&text).unwrap_or_else(|_| {
        fail(format!(
            "jeff sent an answer that is not JSON: {text}. Check that the URL points at jeff."
        ))
    }))
}

fn print_json(value: &Value, output: Option<String>) {
    match output {
        Some(path) => write_json(&path, value).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1)
        }),
        None => println!("{}", serde_json::to_string_pretty(value).unwrap()),
    }
}

fn read_json(file: &str) -> Value {
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1)
    };
    let text = read_input(file).unwrap_or_else(|e| fail(e));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| fail(format!("{file}: {e}. Fix the JSON in the file.")))
}

/// A key as one URL path segment; the server refuses keys that need more than this, but the CLI must not build a
/// different path from them.
fn segment(key: &str) -> String {
    url::form_urlencoded::byte_serialize(key.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

async fn saved_questions(url: &str) -> questions::Questions {
    match admin_call(url, reqwest::Method::GET, "/v1/questions", None).await {
        Some(Value::Object(q)) => q,
        _ => questions::Questions::new(),
    }
}

async fn warn_skipped(url: &str, cs: impl IntoIterator<Item = &Value>) {
    let saved = saved_questions(url).await;
    for c in cs {
        for k in classifiers::skipped(c, &saved) {
            eprintln!("{}", classifiers::skip_reason(&k, &saved));
        }
    }
}

fn classifier_body(
    questions: &[String],
    model: Option<String>,
    description: Option<String>,
) -> Value {
    let mut c = json!({ "questions": questions });
    if let Some(m) = model {
        c["model"] = json!(m);
    }
    if let Some(d) = description {
        c["description"] = json!(d);
    }
    c
}

async fn run_classifiers(action: ClassifiersAction, url: &str) {
    use reqwest::Method;
    match action {
        ClassifiersAction::List { output } => {
            let cs = admin_call(url, Method::GET, "/v1/classifiers", None)
                .await
                .unwrap_or_default();
            if output.is_some() {
                return print_json(&cs, output);
            }
            let cs = cs.as_object().cloned().unwrap_or_default();
            if cs.is_empty() {
                eprintln!("no classifiers");
            }
            let saved = saved_questions(url).await;
            let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
            print!("{}", classifiers_table(&cs, &saved, terminal));
        }
        ClassifiersAction::Get { key, output } => {
            let path = format!("/v1/classifiers/{}", segment(&key));
            let c = admin_call(url, Method::GET, &path, None)
                .await
                .unwrap_or_default();
            print_json(&c, output);
        }
        ClassifiersAction::Add(args) => {
            let input = match (args.file, args.key) {
                (Some(file), _) => read_json(&file),
                (None, Some(key)) => {
                    json!({ key: classifier_body(&args.questions, args.model, args.description) })
                }
                (None, None) => unreachable!("clap requires a key without --file"),
            };
            let saved = admin_call(url, Method::POST, "/api/classifiers", Some(&input))
                .await
                .unwrap_or_default();
            print_json(&saved, None);
            warn_skipped(url, input.as_object().into_iter().flat_map(|m| m.values())).await;
        }
        ClassifiersAction::Update(args) => {
            let input = match args.file {
                Some(file) => read_json(&file),
                None => classifier_body(&args.questions, args.model, args.description),
            };
            let path = format!("/api/classifiers/{}", segment(&args.key));
            let saved = admin_call(url, Method::PUT, &path, Some(&input))
                .await
                .unwrap_or_default();
            print_json(&saved, None);
            warn_skipped(url, [&saved]).await;
        }
        ClassifiersAction::Remove { key } => {
            let path = format!("/api/classifiers/{}", segment(&key));
            admin_call(url, Method::DELETE, &path, None).await;
        }
    }
}

async fn run_questions(action: QuestionsAction, url: &str) {
    use reqwest::Method;
    match action {
        QuestionsAction::List { output } => {
            let qs = admin_call(url, Method::GET, "/v1/questions", None)
                .await
                .unwrap_or_default();
            if output.is_some() {
                return print_json(&qs, output);
            }
            let qs = qs.as_object().cloned().unwrap_or_default();
            if qs.is_empty() {
                eprintln!("no questions");
            }
            let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
            print!("{}", questions_table(&qs, terminal));
        }
        QuestionsAction::Get { key, output } => {
            let path = format!("/v1/questions/{}", segment(&key));
            let q = admin_call(url, Method::GET, &path, None)
                .await
                .unwrap_or_default();
            print_json(&q, output);
        }
        QuestionsAction::Add { file } => {
            let input = read_json(&file);
            let saved = admin_call(url, Method::POST, "/api/questions", Some(&input))
                .await
                .unwrap_or_default();
            print_json(&saved, None);
        }
        QuestionsAction::Update { key, file } => {
            let path = format!("/api/questions/{}", segment(&key));
            let saved = admin_call(url, Method::PUT, &path, Some(&read_json(&file)))
                .await
                .unwrap_or_default();
            print_json(&saved, None);
        }
        QuestionsAction::Remove { key } => {
            let path = format!("/api/questions/{}", segment(&key));
            admin_call(url, Method::DELETE, &path, None).await;
        }
    }
}

fn run_keys(action: KeysAction, path: &str) {
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1)
    };
    let mut config = read_config(path).unwrap_or_else(|e| fail(e));
    // The same rules as the admin page, so the CLI cannot lock that page out. A key in JEFF_API_KEY is an
    // admin key for a server started from this environment.
    let static_admin = env::var("JEFF_API_KEY").is_ok_and(|v| !v.trim().is_empty());
    let has_admin = |stored: &[keys::StoredKey]| {
        let now = keys::now();
        static_admin || stored.iter().any(|k| k.active_admin(now))
    };
    let date = |t: Option<u64>| t.map_or("never".to_owned(), |t| format!("{} (unix)", t));
    match action {
        KeysAction::List => {
            let now = keys::now();
            if config.keys.is_empty() {
                println!("no keys in {path}");
            }
            for k in &config.keys {
                let state = if k.expired(now) { "expired" } else { "active" };
                println!(
                    "{}  {:<24} {:<6} {}…  expires {}  {state}",
                    k.id,
                    k.name,
                    k.role.as_str(),
                    k.prefix,
                    date(k.expires_at)
                );
            }
        }
        KeysAction::Create {
            name,
            role,
            expires,
            admin_url,
        } => {
            let name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > 64 {
                fail(
                    "The name is empty or longer than 64 characters. Use 1 to 64 characters."
                        .into(),
                );
            }
            let expires_at = keys::parse_expiry(&expires, keys::now()).unwrap_or_else(|e| fail(e));
            if role == keys::Role::Client && !has_admin(&config.keys) {
                fail(
                    "create an admin key first, or the admin page loses access: \
                     jeff keys create --name NAME --role admin"
                        .into(),
                );
            }
            let (key, record) = keys::generate(&name, role, expires_at);
            config.keys.push(record.clone());
            save_config(path, &config).unwrap_or_else(|e| fail(e));
            println!("{key}");
            eprintln!(
                "id {}. jeff shows this key only once. Store it now.",
                record.id
            );
            if role == keys::Role::Admin {
                eprintln!(
                    "sign in to the admin page while jeff serve runs: {}",
                    sign_in_link(&admin_url, &key)
                );
            }
        }
        KeysAction::Revoke { id } => {
            let before = config.keys.clone();
            config.keys.retain(|k| k.id != id);
            if config.keys.len() == before.len() {
                fail(format!(
                    "No key with id '{id}'. Check the id with jeff keys list."
                ));
            }
            if !config.keys.is_empty() && has_admin(&before) && !has_admin(&config.keys) {
                fail(
                    "This is the last admin key. Create another admin key or revoke the client keys first."
                        .into(),
                );
            }
            save_config(path, &config).unwrap_or_else(|e| fail(e));
            println!("revoked {id}");
        }
    }
}

fn run_install(stack: install::Stack, action: install::Action) {
    let claim = install::Claim::new(stack).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    match claim.run(action, true, &|step| println!("{step}")) {
        Ok(_) if matches!(action, install::Action::Install) => println!(
            "done: {} is at http://127.0.0.1:{}; {}",
            stack.name(),
            stack.port(),
            stack.first_start()
        ),
        Ok(_) => println!("done"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

async fn serve(args: ServeArgs) {
    let config_path = config_file(args.config.clone());
    let config = load_config(&config_path).unwrap_or_else(|problems| {
        let n = problems.len();
        let summary = format!(
            "{config_path}: {}. Fix the file, then start jeff again.",
            count_problems(n)
        );
        log::write(&config_path, "config_invalid", &summary, &problems);
        fail_with(&format!(
            "error[config_invalid]: {}. Fix the file, then start jeff again.\nlog: {}",
            problems_text(&config_path, n),
            log::path(&config_path).display()
        ))
    });
    if let Some(short) = args
        .api_keys
        .iter()
        .map(|k| k.trim())
        .find(|k| !k.is_empty() && k.len() < MIN_STATIC_KEY_LEN)
    {
        fail_with(&format!(
            "--api-key '{}…' is shorter than {MIN_STATIC_KEY_LEN} characters. Create a key with `jeff keys create`.",
            &short[..short.len().min(4)]
        ));
    }
    let allow_private = args.allow_private_providers;
    let loopback = |addr: &str| {
        addr.parse::<SocketAddr>()
            .is_ok_and(|a| a.ip().is_loopback())
    };
    let exposed = !loopback(&args.api) || (!args.no_ui && !loopback(&args.ui));
    let app = Arc::new(App {
        clients: Mutex::new(HashMap::new()),
        config: RwLock::new(config),
        config_path: config_path.clone(),
        listed: RwLock::new(HashMap::new()),
        tasks: Default::default(),
        warm: Default::default(),
        static_keys: args
            .api_keys
            .iter()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())
            .collect(),
        sessions: Mutex::new(sessions::Sessions::new(&config_path)),
        config_mtime: Mutex::new(modified(&config_path)),
        problems: Mutex::new(Vec::new()),
        allowed_hosts: args
            .allowed_hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect(),
        failures: Mutex::new(HashMap::new()),
        exposed,
        allow_private,
        answers: Mutex::new(HashMap::new()),
    });
    if exposed && app.static_keys.is_empty() && app.config.read().await.keys.is_empty() {
        let addr = if loopback(&args.api) {
            &args.ui
        } else {
            &args.api
        };
        fail_with(&format!(
            "Refusing to listen on {addr} without an access key: anyone who reaches it could use the stored \
             provider keys.\nCreate one first with `jeff keys create --name NAME --role admin`, or pass --api-key."
        ));
    }

    // /health stays open for liveness probes; it reveals nothing about providers.
    let api = Router::new()
        .route("/v1/systemone", post(systemone))
        .route("/v1/models", get(models))
        .route("/v1/questions", get(list_questions))
        .route("/v1/questions/{key}", get(get_question))
        .route(
            "/v1/classifiers",
            get(list_classifiers).post(try_classifier),
        )
        .route(
            "/v1/classifiers/{key}",
            get(get_classifier).post(call_classifier),
        )
        .route_layer(middleware::from_fn_with_state(app.clone(), require_client))
        .route("/health", get(health))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES));
    // The UI calls the same /v1 routes on its own origin, so it works whether or not the API port is reachable from the browser.
    let api_addr = args.api.clone();
    let mut manage = Router::new()
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/status", get(status))
        .route("/api/keys", get(list_keys).post(create_key))
        .route("/api/keys/{id}", delete(revoke_key))
        .route("/api/session", post(sign_in))
        .route("/api/questions", post(create_questions))
        .route(
            "/api/questions/{key}",
            axum::routing::put(update_question).delete(delete_question),
        )
        .route("/api/classifiers", post(create_classifiers))
        .route(
            "/api/classifiers/{key}",
            axum::routing::put(update_classifier).delete(delete_classifier),
        );
    for stack in install::Stack::ALL {
        manage = manage
            .route(
                &format!("/api/{}", stack.id()),
                get(move |State(app): State<Arc<App>>| install_status(app, stack)),
            )
            .route(
                &format!("/api/{}/{{action}}", stack.id()),
                post(
                    move |State(app): State<Arc<App>>, Path(action): Path<String>| {
                        install_action(app, action, stack)
                    },
                ),
            );
    }
    let manage = manage
        .route_layer(middleware::from_fn_with_state(app.clone(), require_admin))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES));
    let info_app = app.clone();
    let ui = api
        .clone()
        .merge(manage)
        .route(
            "/",
            get(|| async {
                Html(
                    include_str!("../ui/index.html")
                        .replace("{{version}}", env!("CARGO_PKG_VERSION")),
                )
            }),
        )
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css")],
                    include_str!("../ui/dist/app.css"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript")],
                    include_str!("../ui/app.js"),
                )
            }),
        )
        .route(
            "/api/info",
            get(move || async move {
                info_app.refresh_from_disk().await;
                Json(json!({ "api": api_addr, "api_key_set": info_app.auth_required().await }))
            }),
        )
        .route("/api/session", delete(sign_out))
        .layer(middleware::from_fn_with_state(app.clone(), cookie_auth))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(app.clone(), check_host));

    let api_listener = tokio::net::TcpListener::bind(&args.api)
        .await
        .unwrap_or_else(|e| fail(&args.api, e));
    println!("jeff API on http://{}", args.api);
    let api_server = axum::serve(
        api_listener,
        api.with_state(app.clone())
            .into_make_service_with_connect_info::<SocketAddr>(),
    );
    if args.no_ui {
        api_server.await.unwrap();
        return;
    }
    let ui_listener = tokio::net::TcpListener::bind(&args.ui)
        .await
        .unwrap_or_else(|e| fail(&args.ui, e));
    println!("jeff UI  on http://{}", args.ui);
    let ui_server = axum::serve(
        ui_listener,
        ui.with_state(app)
            .into_make_service_with_connect_info::<SocketAddr>(),
    );
    let (a, u) = tokio::join!(api_server.into_future(), ui_server.into_future());
    a.and(u).unwrap();
}

fn fail(addr: &str, e: std::io::Error) -> ! {
    fail_with(&format!(
        "Couldn't listen on {addr}: {e}. Stop the program on that port or pick another address."
    ))
}

fn fail_with(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_lines_formats_api_errors() {
        assert_eq!(
            error_lines(
                409,
                r#"{"code":"config_invalid","error":"jeff.json has 2 problems. Fix it before changing questions or classifiers.","log":"jeff.log"}"#
            ),
            "error[config_invalid]: jeff.json has 2 problems. Fix it before changing questions or classifiers.\nlog: jeff.log"
        );
        assert_eq!(
            error_lines(
                404,
                r#"{"error":"No classifier 'x'. Check the key or create the classifier first."}"#
            ),
            "error: No classifier 'x'. Check the key or create the classifier first."
        );
        assert_eq!(
            error_lines(
                415,
                "Expected request with `Content-Type: application/json`\n"
            ),
            "error: HTTP 415 Unsupported Media Type: Expected request with `Content-Type: application/json`"
        );
        assert_eq!(
            error_lines(404, ""),
            "error: HTTP 404 Not Found. Check the URL and jeff.log."
        );
        assert_eq!(
            error_lines(502, "<html>\n<body>Bad Gateway</body>\n</html>"),
            "error: HTTP 502 Bad Gateway. Check the URL and jeff.log."
        );
    }

    #[test]
    fn classifiers_add_takes_flags_or_a_file() {
        let parse = |a: &[&str]| Cli::try_parse_from(a).is_ok();
        assert!(parse(&[
            "jeff",
            "classifiers",
            "add",
            "t",
            "a",
            "b",
            "--model",
            "m"
        ]));
        assert!(parse(&["jeff", "classifiers", "add", "--file", "c.json"]));
        assert!(!parse(&[
            "jeff",
            "classifiers",
            "add",
            "t",
            "a",
            "--file",
            "c.json"
        ]));
        assert!(!parse(&["jeff", "classifiers", "add"]));
        assert!(parse(&[
            "jeff",
            "classifiers",
            "update",
            "t",
            "--file",
            "-"
        ]));
        assert!(!parse(&[
            "jeff",
            "classifiers",
            "update",
            "t",
            "--file",
            "-",
            "--model",
            "m"
        ]));
        assert!(!parse(&["jeff", "classifiers", "update", "t"]));
        assert!(parse(&[
            "jeff",
            "classifiers",
            "add",
            "t",
            "a",
            "--description",
            "d"
        ]));
        assert!(!parse(&[
            "jeff",
            "classifiers",
            "update",
            "t",
            "--file",
            "-",
            "--description",
            "d"
        ]));
    }

    fn bearer_headers(key: &str, origin: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}")).unwrap(),
        );
        if let Some(o) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
        }
        h
    }

    fn set_cookie(r: &Response) -> String {
        r.headers()[header::SET_COOKIE].to_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn sign_in_gives_two_halves_and_never_the_key() {
        let dir = env::temp_dir().join(format!("jeff-test-sign-in-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let app = test_app(dir.join("config.json").to_str().unwrap());
        let (key, record) = keys::generate("me", keys::Role::Admin, None);
        app.config.write().await.keys = vec![record];
        let app = Arc::new(app);
        let plain = sign_in(
            State(app.clone()),
            bearer_headers(&key, Some("http://127.0.0.1:8081")),
        )
        .await;
        assert_eq!(plain.status(), StatusCode::OK);
        let cookie = set_cookie(&plain);
        assert!(!cookie.contains(&key));
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Max-Age=2592000") && !cookie.contains("Secure"));
        let cookie_half = cookie["jeff_session=".len()..]
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let header_half = body_of(plain).await["session"].as_str().unwrap().to_owned();
        let digest = app
            .sessions
            .lock()
            .unwrap()
            .find(&cookie_half, &header_half, keys::now());
        assert_eq!(digest, Some(keys::sha256(&key)));

        let tls = sign_in(
            State(app.clone()),
            bearer_headers(&key, Some("https://admin.example")),
        )
        .await;
        assert!(set_cookie(&tls).ends_with("; Secure"));

        let mut session = HeaderMap::new();
        session.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("jeff_session={cookie_half}")).unwrap(),
        );
        session.insert(SESSION_HEADER, HeaderValue::from_str(&header_half).unwrap());
        let again = sign_in(State(app.clone()), session.clone()).await;
        assert_eq!(again.status(), StatusCode::UNAUTHORIZED);

        assert!(set_cookie(&sign_out(State(app.clone()), session).await).contains("Max-Age=0"));
        let found = app
            .sessions
            .lock()
            .unwrap()
            .find(&cookie_half, &header_half, keys::now());
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn a_failed_session_save_names_no_server_path() {
        let dir = env::temp_dir().join(format!("jeff-test-no-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sessions.json"), "{ broken").unwrap();
        let app = test_app(dir.join("config.json").to_str().unwrap());
        let (key, record) = keys::generate("me", keys::Role::Admin, None);
        app.config.write().await.keys = vec![record];
        let r = sign_in(State(Arc::new(app)), bearer_headers(&key, None)).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body_of(r).await["error"],
            "Couldn't save sessions.json. Check jeff.log for the cause."
        );
    }

    #[tokio::test]
    async fn a_session_needs_the_cookie_and_the_header_and_no_authorization() {
        let seen = |req: Request| async move {
            req.extensions()
                .get::<SessionKey>()
                .map(|k| k.0.clone())
                .unwrap_or_default()
        };
        let dir = env::temp_dir().join(format!("jeff-test-halves-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let state = test_app(dir.join("config.json").to_str().unwrap());
        let (c, h) = state
            .sessions
            .lock()
            .unwrap()
            .create("k1", keys::now(), |_| true)
            .unwrap();
        let app = Router::new()
            .route("/", get(seen))
            .layer(middleware::from_fn_with_state(Arc::new(state), cookie_auth));
        let call = |cookie: String, half: Option<&str>, auth: Option<&str>| {
            let mut req = Request::builder().uri("/").header(header::COOKIE, cookie);
            if let Some(v) = half {
                req = req.header(SESSION_HEADER, v);
            }
            if let Some(a) = auth {
                req = req.header(header::AUTHORIZATION, a);
            }
            let app = app.clone();
            let req = req.body(axum::body::Body::empty()).unwrap();
            async move {
                let r = tower::ServiceExt::oneshot(app, req).await.unwrap();
                String::from_utf8(
                    axum::body::to_bytes(r.into_body(), 1024)
                        .await
                        .unwrap()
                        .to_vec(),
                )
                .unwrap()
            }
        };
        assert_eq!(
            call(format!("theme=dark; jeff_session={c}"), Some(&h), None).await,
            "k1"
        );
        assert_eq!(call(format!("jeff_session={c}"), None, None).await, "");
        assert_eq!(call("theme=dark".into(), Some(&h), None).await, "");
        assert_eq!(
            call(format!("jeff_session={c}"), Some("other"), None).await,
            ""
        );
        assert_eq!(
            call(format!("jeff_session={c}"), Some(&h), Some("Bearer jeff_h")).await,
            ""
        );
    }

    #[tokio::test]
    async fn the_key_list_names_the_key_of_the_request() {
        let app = test_app("unused-current.json");
        let (key, record) = keys::generate("me", keys::Role::Admin, None);
        let (_, other) = keys::generate("other", keys::Role::Admin, None);
        app.config.write().await.keys = vec![other, record.clone()];
        let app = Arc::new(app);
        let by_key = list_keys(State(app.clone()), None, bearer_headers(&key, None))
            .await
            .0;
        assert_eq!(by_key["current_key_id"], record.id);
        let by_session = list_keys(
            State(app),
            Some(Extension(SessionKey(record.sha256.clone()))),
            HeaderMap::new(),
        )
        .await
        .0;
        assert_eq!(by_session["current_key_id"], record.id);
    }

    #[test]
    fn the_sign_in_link_carries_the_key_in_the_fragment() {
        assert_eq!(
            sign_in_link("http://127.0.0.1:8081", "jeff_k"),
            "http://127.0.0.1:8081/#key=jeff_k"
        );
        assert_eq!(
            sign_in_link("https://admin.example/", "jeff_k"),
            "https://admin.example/#key=jeff_k"
        );
    }

    fn test_app(config_path: &str) -> App {
        App {
            clients: Mutex::new(HashMap::new()),
            config: RwLock::new(Config::default()),
            config_path: config_path.into(),
            listed: RwLock::new(HashMap::new()),
            tasks: Default::default(),
            warm: Default::default(),
            static_keys: Vec::new(),
            sessions: Mutex::new(sessions::Sessions::new(config_path)),
            problems: Mutex::new(Vec::new()),
            config_mtime: Mutex::new(modified(config_path)),
            allowed_hosts: Vec::new(),
            failures: Mutex::new(HashMap::new()),
            exposed: false,
            allow_private: false,
            answers: Mutex::new(HashMap::new()),
        }
    }

    async fn body_of(r: Response) -> Value {
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn save_refuses_a_file_changed_on_disk() {
        let path = temp_config(
            "changed",
            &serde_json::to_string(&Config::default()).unwrap(),
        );
        let app = test_app(&path);
        *app.config_mtime.lock().unwrap() = Some(SystemTime::UNIX_EPOCH);
        let r = app.save(&Config::default()).await.unwrap_err();
        let _ = fs::remove_file(&path);
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert_eq!(body_of(*r).await["code"], "config_changed");
    }

    #[tokio::test]
    async fn deleting_a_broken_file_clears_its_problems() {
        let path = temp_config("deleted", "{");
        let app = test_app(&path);
        *app.config_mtime.lock().unwrap() = Some(SystemTime::UNIX_EPOCH);
        app.refresh_from_disk().await;
        assert_eq!(app.problems.lock().unwrap().len(), 1);
        fs::remove_file(&path).unwrap();
        app.refresh_from_disk().await;
        assert!(app.problems.lock().unwrap().is_empty());
        assert!(app.save(&Config::default()).await.is_ok());
        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_failed_save_names_no_server_path() {
        let blocker = env::temp_dir().join(format!("jeff-test-blocker-{}", std::process::id()));
        fs::write(&blocker, "").unwrap();
        let path = blocker.join("jeff.json");
        let app = test_app(path.to_str().unwrap());
        let r = app.save(&Config::default()).await.unwrap_err();
        let _ = fs::remove_file(&blocker);
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_of(*r).await;
        assert_eq!(
            body["error"],
            "Couldn't save jeff.json. Check the server console for the cause."
        );
    }

    #[test]
    fn problems_text_counts() {
        assert_eq!(problems_text("x/jeff.json", 1), "jeff.json has 1 problem");
        assert_eq!(problems_text("x/jeff.json", 2), "jeff.json has 2 problems");
    }

    #[tokio::test]
    async fn config_invalid_names_the_log_only() {
        let r = config_invalid("C:/Users/u/.jeff/jeff.json", 2);
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({"code": "config_invalid", "error": "jeff.json has 2 problems. Fix it before changing questions or classifiers.", "log": "jeff.log"})
        );
    }

    #[test]
    fn status_body_shapes() {
        assert_eq!(status_body("x/jeff.json", &[]), json!({"config": "ok"}));
        assert_eq!(
            status_body("x/jeff.json", &["a".into(), "b".into()]),
            json!({"config": "invalid", "file": "jeff.json", "problems": 2, "log": "jeff.log"})
        );
    }

    #[test]
    fn detects_loopback_urls() {
        let loopback = |u: &str| is_loopback_url(&reqwest::Url::parse(u).unwrap());
        assert!(loopback("http://127.0.0.1:8700"));
        assert!(loopback("http://localhost:8000/x"));
        assert!(loopback("http://[::1]:9000"));
        assert!(!loopback("http://10.0.0.5:8000"));
        assert!(!loopback("http://api.example.com"));
        assert!(!loopback("http://127.0.0.1.example.com"));
    }

    #[test]
    fn guards_provider_addresses() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(check_ip(ip("127.0.0.1"), false).is_ok());
        assert!(check_ip(ip("::1"), false).is_ok());
        assert!(check_ip(ip("93.184.216.34"), false).is_ok());
        for refused in [
            "169.254.169.254",
            "fe80::1",
            "fd00:ec2::254",
            "0.0.0.0",
            "::ffff:169.254.169.254",
            "100.100.100.200",
        ] {
            assert!(check_ip(ip(refused), true).is_err(), "{refused}");
        }
        for private in [
            "10.0.0.5",
            "192.168.1.2",
            "172.16.0.1",
            "100.64.0.1",
            "fd12::1",
        ] {
            assert!(check_ip(ip(private), false).is_err(), "{private}");
            assert!(check_ip(ip(private), true).is_ok(), "{private}");
        }
        assert!(parse_provider_url("http://169.254.169.254/latest", true).is_err());
        assert!(parse_provider_url("https://user:pw@api.example.com", false).is_err());
        assert!(parse_provider_url("ftp://api.example.com", false).is_err());
        assert!(parse_provider_url("https://api.example.com/v1", false).is_ok());
    }

    #[test]
    fn parses_listen_addresses() {
        assert_eq!(parse_addr("9000").unwrap(), "127.0.0.1:9000");
        assert_eq!(parse_addr("0.0.0.0:8080").unwrap(), "0.0.0.0:8080");
        assert!(parse_addr("localhost").is_err());
    }

    #[test]
    fn resolves_explicit_and_bare_models() {
        let config = Config::default();
        let mut listed = HashMap::new();
        listed.insert("clm".to_owned(), vec!["clm-raw".to_owned()]);
        let (p, m) = resolve(&config, &listed, "typesafe/jev-1.13.0").unwrap();
        assert_eq!((p.id.as_str(), m.as_str()), ("typesafe", "jev-1.13.0"));
        assert_eq!(resolve(&config, &listed, "clm-raw").unwrap().0.id, "clm");
        assert_eq!(
            resolve(&config, &listed, "jev-latest").unwrap().0.id,
            "typesafe"
        );
        assert!(resolve(&config, &listed, "gpt-4o").is_none());
        assert!(resolve(&config, &listed, "nope/x").is_none());
    }

    #[test]
    fn batches_groups_that_reach_the_same_model() {
        let clm = Config::default().providers[0].clone();
        let qs = |k: &str| {
            Some(
                json!({ k: {"type": "noul", "instructions": "x"} })
                    .as_object()
                    .unwrap()
                    .clone(),
            )
        };
        let batches = batch(vec![
            (clm.clone(), "clm-latest".into(), qs("a")),
            (clm.clone(), "clm-raw".into(), qs("b")),
            (clm.clone(), "clm-latest".into(), qs("c")),
        ]);
        let shape: Vec<_> = batches
            .iter()
            .map(|(p, m, q)| {
                (
                    p.id.as_str(),
                    m.as_str(),
                    q.as_ref().unwrap().keys().cloned().collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("clm", "clm-latest", vec!["a".to_owned(), "c".to_owned()]),
                ("clm", "clm-raw", vec!["b".to_owned()]),
            ]
        );
    }

    #[test]
    fn config_input_keeps_or_clears_keys() {
        let mut current = Config::default();
        current.providers[1].key = Some("secret".into());
        let input = |key: Option<&str>| ConfigInput {
            default_model: "typesafe/jev-latest".into(),
            providers: vec![ProviderInput {
                id: "typesafe".into(),
                name: "".into(),
                url: "https://api.typesafe.ai".into(),
                key: key.map(String::from),
                models: vec![],
                installer: None,
                connect_timeout: None,
                max_time: None,
                description: None,
            }],
        };
        let key_of = |input| {
            build_config(&current, input, false).unwrap().providers[0]
                .key
                .clone()
        };
        assert_eq!(key_of(input(None)).as_deref(), Some("secret"));
        assert!(key_of(input(Some(" "))).is_none());
        let mut moved = input(None);
        moved.providers[0].url = "https://attacker.example".into();
        assert!(key_of(moved).is_none());
        let mut bad = input(None);
        bad.default_model = "clm/clm-latest".into();
        assert!(build_config(&current, bad, false).is_err());
    }

    #[test]
    fn ask_questions_are_json_unless_read_from_a_file() {
        let questions = |args: &[&str]| {
            let args = ask_args(&[&["--state", "s"], args].concat()).map_err(|e| e.to_string())?;
            ask_request(&args, stdin_is(r#"{"f": {}}"#)).map(|b| b["questions"].clone())
        };
        assert_eq!(
            questions(&["--questions", r#"{"u": {}}"#]).unwrap(),
            json!({"u": {}})
        );
        assert_eq!(
            questions(&["--questions-file", "-"]).unwrap(),
            json!({"f": {}})
        );
        let e = questions(&["--questions", "refund.json"]).unwrap_err();
        assert!(
            e.contains("--questions is not valid JSON") && e.contains("--questions-file"),
            "{e}"
        );
        assert!(questions(&["--questions", "{}", "--questions-file", "-"]).is_err());
    }

    fn ask_args(args: &[&str]) -> Result<AskArgs, clap::Error> {
        let argv = ["jeff", "ask"].iter().chain(args).copied();
        match Cli::try_parse_from(argv)?.command {
            Some(Command::Ask(a)) => Ok(a),
            _ => unreachable!(),
        }
    }

    fn stdin_is(text: &'static str) -> impl Fn(&str) -> Result<String, String> {
        move |f: &str| match f {
            "-" => Ok(text.to_owned()),
            _ => fs::read_to_string(f).map_err(|e| format!("{f}: {e}")),
        }
    }

    #[test]
    fn ask_state_is_text_unless_read_from_a_file() {
        let file = env::temp_dir().join(format!("jeff-test-state-{}.txt", std::process::id()));
        fs::write(&file, "from file").unwrap();
        let path = file.to_str().unwrap().to_owned();
        let state = |args: &[&str]| {
            let args = ask_args(args).unwrap();
            ask_request(&args, stdin_is("from stdin")).unwrap()["state"].clone()
        };
        let from_file = state(&["u", "--state-file", &path]);
        let text = state(&["u", "--state", &path]);
        let _ = fs::remove_file(&file);
        assert_eq!(from_file, "from file");
        assert_eq!(text, path.as_str());
        assert_eq!(state(&["u", "--state-file", "-"]), "from stdin");
        assert_eq!(state(&["u", "--state", "-"]), "-");
        assert!(ask_args(&["u"]).is_err());
        assert!(ask_args(&["u", "--state", "x", "--state-file", "y"]).is_err());
    }

    #[test]
    fn ask_reads_a_whole_request() {
        let body = r#"{"state": "s", "questions": ["u"], "model": "p/a"}"#;
        let args = ask_args(&["--request", "-", "--model", "p/b"]).unwrap();
        assert_eq!(
            ask_request(&args, stdin_is(body)).unwrap(),
            json!({"state": "s", "questions": ["u"], "model": "p/b"})
        );
        let args = ask_args(&["--request", "-"]).unwrap();
        assert!(ask_request(&args, stdin_is("[1]")).is_err());
        assert!(ask_args(&["--request", "-", "--state", "x"]).is_err());
        assert!(ask_args(&["u", "--request", "-"]).is_err());
    }

    #[test]
    fn ask_refuses_two_readers_of_stdin() {
        let args = ask_args(&["--state-file", "-", "--questions-file", "-"]).unwrap();
        assert_eq!(
            ask_request(&args, stdin_is("{}")).unwrap_err(),
            "--state-file and --questions-file both read stdin. Use --request - instead."
        );
    }

    #[test]
    fn ask_url_needs_a_scheme() {
        let e = ask_args(&["u", "--state", "x", "--url", "localhost:8080"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            e.contains("The URL does not start with http:// or https://. Start it with https://."),
            "{e}"
        );
        assert!(ask_args(&["u", "--state", "x", "--url", "https://jeff.example"]).is_ok());
    }

    #[test]
    fn lists_questions_one_line_each() {
        let qs = json!({
            "refund": {"type": "noul", "model": "clm/clm-raw", "instructions": "Refund?\nIgnore\texchanges."},
            "urgency": {"type": "score", "instructions": "x".repeat(70), "criteria": ["a", "b"]},
        });
        let qs = qs.as_object().unwrap();
        assert_eq!(
            questions_table(qs, false),
            format!(
                "refund\tnoul\tclm/clm-raw\tRefund? Ignore exchanges.\nurgency\tscore\t-\t{}\n",
                "x".repeat(70)
            )
        );
        let long = format!("{}…", "x".repeat(59));
        let table = questions_table(qs, true);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[1],
            format!("│ key     │ type  │ model       │ {:<60} │", "instructions")
        );
        assert_eq!(
            lines[4],
            format!("│ urgency │ score │ -           │ {long} │")
        );
        assert!(lines[0].starts_with("┌─────────┬") && lines[5].ends_with("┘"));
    }

    #[test]
    fn client_key_takes_the_first_of_a_list() {
        assert_eq!(client_key(Some(" k1 , k2".into())).as_deref(), Some("k1"));
        assert_eq!(client_key(Some(" ".into())), None);
        assert_eq!(client_key(None), None);
    }

    #[test]
    fn cli_reads_a_config_that_serve_refuses() {
        let path = env::temp_dir().join(format!("jeff-test-cli-{}.json", std::process::id()));
        let mut config = serde_json::to_value(Config::default()).unwrap();
        config["questions"] = json!({"a b": {"type": "noul", "instructions": "x"}});
        fs::write(&path, config.to_string()).unwrap();
        let path = path.to_str().unwrap().to_owned();
        let read = read_config(&path);
        let loaded = load_config(&path);
        let _ = fs::remove_file(&path);
        assert!(read.unwrap().questions.contains_key("a b"));
        assert!(loaded.is_err());
    }

    #[test]
    fn ask_body_merges_keys_and_overrides() {
        let body = ask_body(
            &["a".into(), "b".into()],
            Some(json!({"b": {"model": "p/m"}, "c": {"type": "noul", "instructions": "x"}})),
            "s".into(),
            Some("p/n".into()),
        )
        .unwrap();
        assert_eq!(
            body,
            json!({"state": "s", "model": "p/n", "questions": {
                "a": {}, "b": {"model": "p/m"}, "c": {"type": "noul", "instructions": "x"}}})
        );
        let bare = ask_body(&["a".into()], None, "s".into(), None).unwrap();
        assert!(bare.get("model").is_none());
        assert!(ask_body(&[], None, "s".into(), None).is_err());
        assert!(ask_body(&[], Some(json!([1])), "s".into(), None).is_err());
    }

    #[test]
    fn load_config_rejects_invalid_questions() {
        let path = env::temp_dir().join(format!("jeff-test-{}.json", std::process::id()));
        let mut config = serde_json::to_value(Config::default()).unwrap();
        config["questions"] = json!({"a b": {"type": "noul", "instructions": "x"}});
        fs::write(&path, config.to_string()).unwrap();
        let result = load_config(path.to_str().unwrap());
        let _ = fs::remove_file(&path);
        let e = result.err().expect("an invalid question must be refused");
        assert!(e[0].contains("Question 'a b'"), "{e:?}");
    }

    #[test]
    fn load_config_rejects_invalid_classifiers() {
        let path = env::temp_dir().join(format!("jeff-test-c-{}.json", std::process::id()));
        let mut config = serde_json::to_value(Config::default()).unwrap();
        config["classifiers"] = json!({"k": {"questions": []}});
        fs::write(&path, config.to_string()).unwrap();
        let path = path.to_str().unwrap();
        let loaded = load_config(path);
        let read = read_config(path);
        let _ = fs::remove_file(path);
        let e = loaded.err().expect("an invalid classifier must be refused");
        assert!(e[0].contains("Classifier 'k'"), "{e:?}");
        assert!(read.unwrap().classifiers.contains_key("k"));
    }

    fn temp_config(name: &str, text: &str) -> String {
        let path = env::temp_dir().join(format!("jeff-test-{name}-{}.json", std::process::id()));
        fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn load_config_lists_every_problem() {
        let mut config = serde_json::to_value(Config::default()).unwrap();
        config["questions"] = json!({"a b": {"type": "noul", "instructions": "x"}});
        config["classifiers"] = json!({"triage": {"questions": ["x"], "modle": "m"}});
        let path = temp_config("every", &config.to_string());
        let problems = load_config(&path).err().unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("Question 'a b': "), "{problems:?}");
        assert_eq!(
            problems[1],
            "Classifier 'triage': Unknown field 'modle'. Keep only questions, model and description."
        );
    }

    #[test]
    fn load_config_accepts_a_classifier_naming_a_deleted_question() {
        let mut config = serde_json::to_value(Config::default()).unwrap();
        config["classifiers"] = json!({"triage": {"questions": ["gone"]}});
        let path = temp_config("gone", &config.to_string());
        let loaded = load_config(&path);
        let _ = fs::remove_file(&path);
        assert!(loaded.is_ok(), "{:?}", loaded.err());
    }

    #[test]
    fn load_config_reports_bad_json_as_one_problem() {
        let path = temp_config("json", "{");
        let loaded = load_config(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(loaded.err().map(|p| p.len()), Some(1));
    }

    #[test]
    fn config_without_classifiers_still_loads() {
        let mut config = serde_json::to_value(Config::default()).unwrap();
        config.as_object_mut().unwrap().remove("classifiers");
        let config: Config = serde_json::from_value(config).unwrap();
        assert!(config.classifiers.is_empty());
    }

    #[test]
    fn call_body_replaces_client_questions() {
        let body = json!({"state": "s", "model": "p/m", "questions": {"x": {"type": "noul", "instructions": "y"}}});
        assert_eq!(
            call_body(body, json!(["a"])).unwrap(),
            json!({"state": "s", "model": "p/m", "questions": ["a"]})
        );
        assert!(call_body(json!([1]), json!(["a"])).is_err());
    }

    #[test]
    fn provider_save_keeps_timeouts_the_ui_does_not_send() {
        let mut current = Config::default();
        current.providers[0].connect_timeout = Some(3);
        current.providers[0].max_time = Some(120);
        let input = |max_time: Option<u64>| ConfigInput {
            default_model: "clm/clm-latest".into(),
            providers: vec![ProviderInput {
                id: "clm".into(),
                name: "CLM".into(),
                url: "http://127.0.0.1:8700".into(),
                key: None,
                models: vec![],
                installer: None,
                connect_timeout: None,
                max_time,
                description: None,
            }],
        };
        let kept = &build_config(&current, input(None), false)
            .unwrap()
            .providers[0];
        assert_eq!((kept.connect_timeout, kept.max_time), (Some(3), Some(120)));
        let set = &build_config(&current, input(Some(30)), false)
            .unwrap()
            .providers[0];
        assert_eq!(set.max_time, Some(30));
        assert!(build_config(&current, input(Some(0)), false).is_err());
    }

    #[test]
    fn cached_usage_zeroes_tokens() {
        let sent = json!({"answers": {"q": 1}, "usage": {"input_tokens": 73, "output_tokens": 2, "latency_ms": 136.8}});
        let got: Value =
            serde_json::from_slice(&cached_usage(serde_json::to_vec(&sent).unwrap())).unwrap();
        assert_eq!(
            got,
            json!({"answers": {"q": 1}, "usage": {"input_tokens": 0, "output_tokens": 0, "latency_ms": 136.8, "cached": true}})
        );
        assert_eq!(cached_usage(b"not json".to_vec()), b"not json");
    }

    #[test]
    fn provider_save_keeps_a_description_it_does_not_send() {
        let input = |description: Option<&str>| ConfigInput {
            default_model: "clm/clm-latest".into(),
            providers: vec![ProviderInput {
                id: "clm".into(),
                name: "CLM".into(),
                url: "http://127.0.0.1:8700".into(),
                key: None,
                models: vec![],
                installer: None,
                connect_timeout: None,
                max_time: None,
                description: description.map(String::from),
            }],
        };
        let set = build_config(&Config::default(), input(Some(" GPU box ")), false).unwrap();
        assert_eq!(set.providers[0].description.as_deref(), Some("GPU box"));
        let kept = build_config(&set, input(None), false).unwrap();
        assert_eq!(kept.providers[0].description.as_deref(), Some("GPU box"));
        let cleared = build_config(&set, input(Some("")), false).unwrap();
        assert_eq!(cleared.providers[0].description, None);
    }

    #[test]
    fn build_config_keeps_questions() {
        let mut current = Config::default();
        current.questions.insert(
            "urgency".into(),
            json!({"type": "noul", "instructions": "Urgent?"}),
        );
        let input = ConfigInput {
            default_model: "clm/clm-latest".into(),
            providers: vec![ProviderInput {
                id: "clm".into(),
                name: "CLM".into(),
                url: "http://127.0.0.1:8700".into(),
                key: None,
                models: vec![],
                installer: None,
                connect_timeout: None,
                max_time: None,
                description: None,
            }],
        };
        let next = build_config(&current, input, false).unwrap();
        assert_eq!(next.questions, current.questions);
    }

    #[test]
    fn build_config_keeps_classifiers() {
        let mut current = Config::default();
        current
            .classifiers
            .insert("triage".into(), json!({"questions": ["u"]}));
        let input = ConfigInput {
            default_model: "clm/clm-latest".into(),
            providers: vec![ProviderInput {
                id: "clm".into(),
                name: "CLM".into(),
                url: "http://127.0.0.1:8700".into(),
                key: None,
                models: vec![],
                installer: None,
                connect_timeout: None,
                max_time: None,
                description: None,
            }],
        };
        let next = build_config(&current, input, false).unwrap();
        assert_eq!(next.classifiers, current.classifiers);
    }

    #[test]
    fn lists_classifiers_one_line_each() {
        let cs = json!({"triage": {"questions": ["a", "gone"], "model": "p/m", "description": "Routes\n tickets"}, "plain": {"questions": ["a"]}});
        let saved = json!({"a": {"type": "noul", "instructions": "x"}});
        assert_eq!(
            classifiers_table(cs.as_object().unwrap(), saved.as_object().unwrap(), false),
            "plain\t-\ta\t-\ntriage\tp/m\ta, gone (deleted)\tRoutes tickets\n"
        );
    }

    #[test]
    fn ask_calls_a_classifier() {
        let args = ask_args(&["--classifier", "triage", "--state", "s"]).unwrap();
        assert_eq!(ask_target(&args), "/v1/classifiers/triage");
        assert_eq!(
            ask_request(&args, stdin_is("")).unwrap(),
            json!({"state": "s"})
        );
        let args = ask_args(&["--classifier", "triage", "--state", "s", "--model", "p/m"]).unwrap();
        assert_eq!(
            ask_request(&args, stdin_is("")).unwrap(),
            json!({"state": "s", "model": "p/m"})
        );
        assert!(ask_args(&["u", "--classifier", "triage", "--state", "s"]).is_err());
        assert!(ask_args(&["--classifier", "t", "--questions", "{}", "--state", "s"]).is_err());
        assert!(ask_args(&["--classifier", "t", "--request", "-"]).is_err());
        let args = ask_args(&["--classifier", "a/b", "--state", "s"]).unwrap();
        let e = ask_request(&args, stdin_is("")).unwrap_err();
        assert!(e.contains("classifier 'a/b'"), "{e}");
        assert_eq!(
            ask_target(&ask_args(&["u", "--state", "s"]).unwrap()),
            "/v1/systemone"
        );
    }

    #[test]
    fn take_draft_splits_the_classifier_from_the_call() {
        let body = json!({"state": "s", "model": "p/m", "classifier": {"questions": ["a"]}});
        assert_eq!(
            take_draft(body).unwrap(),
            (
                json!({"questions": ["a"]}),
                json!({"state": "s", "model": "p/m"})
            )
        );
        assert!(take_draft(json!({"state": "s"})).is_err());
        assert!(take_draft(json!([1])).is_err());
    }
}
