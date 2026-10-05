mod install;
mod keys;

use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::delete,
    routing::{get, post},
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env, fs,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
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
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    default_model: String,
    providers: Vec<Provider>,
    /// jeff access keys, hashed.
    #[serde(default)]
    keys: Vec<keys::StoredKey>,
}

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
                },
                Provider {
                    id: "typesafe".into(),
                    name: "TypeSafe Jev".into(),
                    url: env_or("TYPESAFE_URL", "https://api.typesafe.ai"),
                    key: key("TYPESAFE_API_KEY"),
                    models: vec!["jev-latest".into()],
                    installer: None,
                },
            ],
            keys: Vec::new(),
        }
    }
}

/// A missing file gives the defaults. A broken one is an error: falling back would drop every access key and
/// open jeff to anyone.
fn load_config(path: &str) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{path}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

/// Writes the config readable by its owner only: it holds provider keys and access-key hashes. The text goes to a
/// temporary file first, so a crash mid-write cannot leave a truncated config behind.
fn save_config(path: &str, config: &Config) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).unwrap();
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
    http: reqwest::Client,
    config: RwLock<Config>,
    config_path: String,
    /// Model lists last fetched from each provider, keyed by provider id.
    listed: RwLock<HashMap<String, Vec<String>>>,
    clm_task: Mutex<Value>,
    /// 0 cold, 1 warming, 2 warm: whether clm-serve has answered its first, slow request.
    clm_warm: AtomicU8,
    /// Keys from `--api-key` / `JEFF_API_KEY`; they never expire and are managed outside the UI.
    static_keys: Vec<String>,
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

async fn read_capped(mut resp: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if body.len() + chunk.len() > MAX_UPSTREAM_BYTES {
            return Err(format!(
                "response is larger than {MAX_UPSTREAM_BYTES} bytes"
            ));
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
        Err(format!("{ip} is a link-local or metadata address"))
    } else if private && !allow_private {
        Err(format!(
            "{ip} is in a private network; start jeff with --allow-private-providers"
        ))
    } else {
        Ok(())
    }
}

fn parse_provider_url(url: &str, allow_private: bool) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("URL '{url}': {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("URL must start with http:// or https://".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("put the provider key in the key field, not in the URL".into());
    }
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => check_ip(ip.into(), allow_private)?,
        Some(url::Host::Ipv6(ip)) => check_ip(ip.into(), allow_private)?,
        Some(url::Host::Domain(_)) => {}
        None => return Err("URL has no host".into()),
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
    /// A broken file keeps the config in memory, so the access keys stay in force until the file is fixed.
    async fn refresh_from_disk(&self) {
        let now = modified(&self.config_path);
        let seen = *self.config_mtime.lock().unwrap();
        if now.is_some() && now != seen {
            match load_config(&self.config_path) {
                Ok(config) => *self.config.write().await = config,
                Err(e) => eprintln!("{e}; keeping the config loaded before"),
            }
            *self.config_mtime.lock().unwrap() = now;
        }
    }

    async fn save(&self, config: &Config) -> Result<(), String> {
        save_config(&self.config_path, config)?;
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

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn request(
    app: &App,
    p: &Provider,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let req = app
        .http
        .request(method, format!("{}{path}", p.url.trim_end_matches('/')));
    match &p.key {
        Some(key) => req.bearer_auth(key),
        None => req,
    }
}

async fn systemone(State(app): State<Arc<App>>, Json(mut body): Json<Value>) -> Response {
    let config = app.config.read().await.clone();
    let Some(obj) = body.as_object_mut() else {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "body must be a JSON object",
        );
    };
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(&config.default_model)
        .to_owned();
    let mut found = resolve(&config, &*app.listed.read().await, &model);
    if found.is_none() && !model.contains('/') {
        refresh_models(&app, &config).await;
        found = resolve(&config, &*app.listed.read().await, &model);
    }
    let Some((p, upstream_model)) = found else {
        return error(
            StatusCode::BAD_REQUEST,
            format!(
                "unknown model '{model}': use provider/model, for example {}",
                config.default_model
            ),
        );
    };
    obj.insert("model".into(), Value::String(upstream_model));

    let t0 = Instant::now();
    let upstream = request(&app, &p, reqwest::Method::POST, "/v1/systemone")
        .json(&body)
        .timeout(Duration::from_secs(120));
    // Clients get the provider id only; the provider URL and the transport error stay in the server log.
    let resp = match upstream.send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("provider {}: {}", p.id, e.without_url());
            return error(
                StatusCode::BAD_GATEWAY,
                format!("provider '{}' is unreachable", p.id),
            );
        }
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    match read_capped(resp).await {
        Ok(bytes) => {
            let mut res =
                (status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response();
            let headers = res.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&p.id) {
                headers.insert("x-jeff-provider", v);
            }
            headers.insert(
                "x-jeff-upstream-ms",
                HeaderValue::from(t0.elapsed().as_millis() as u64),
            );
            res
        }
        Err(e) => {
            eprintln!("provider {}: {e}", p.id);
            error(
                StatusCode::BAD_GATEWAY,
                format!("provider '{}' sent an unreadable response", p.id),
            )
        }
    }
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
    // clm-serve opens its port only after downloading its head, which takes minutes on first start.
    if !ok
        && p.installer.as_deref() == Some("clm")
        && tokio::task::spawn_blocking(install::clm_running)
            .await
            .unwrap_or(false)
    {
        return json!({ "id": p.id, "name": p.name, "ok": false, "warning": "downloading the model", "models": fallback() });
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
    // clm-serve answers before its encoder has loaded the weights; requests fail with 502 until then.
    if ok && p.installer.as_deref() == Some("clm") {
        let health = request(app, p, reqwest::Method::GET, "/health")
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        if let Ok(r) = health
            && r.json::<Value>()
                .await
                .is_ok_and(|h| h["embedder"] == false)
        {
            out["warning"] = json!("encoder is loading");
        }
    }
    out
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
                "models": p.models, "installer": p.installer,
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
                "too many failed key checks; try again in a minute",
            );
        }
        let auth = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let role = keys::verify(auth, &app.static_keys, &app.config.read().await.keys, now);
        if let Some(role) = role
            && !role.allows(needed)
        {
            return error(
                StatusCode::FORBIDDEN,
                "this is a client key; managing jeff needs an admin key",
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
                "a valid jeff API key is required: Authorization: Bearer <key>",
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
            format!("host '{name}' is not allowed; start jeff with --allowed-host {name}"),
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
        return error(StatusCode::FORBIDDEN, "cross-site requests are not allowed");
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

async fn list_keys(State(app): State<Arc<App>>) -> Json<Value> {
    let config = app.config.read().await;
    Json(
        json!({ "static_keys": app.static_keys.len(), "keys": config.keys.iter().map(key_view).collect::<Vec<_>>() }),
    )
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
        return error(StatusCode::BAD_REQUEST, "name must be 1 to 64 characters");
    }
    if input.expires_at.is_some_and(|t| t <= keys::now()) {
        return error(StatusCode::BAD_REQUEST, "expiry must be in the future");
    }
    let mut config = app.config.write().await;
    if input.role == keys::Role::Client && !app.has_admin(&config.keys) {
        return error(
            StatusCode::BAD_REQUEST,
            "create an admin key first, or this page loses access once the first key exists",
        );
    }
    let (key, record) = keys::generate(name, input.role, input.expires_at);
    let mut next = config.clone();
    next.keys.push(record.clone());
    if let Err(e) = app.save(&next).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e);
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
        return error(StatusCode::NOT_FOUND, format!("no key with id '{id}'"));
    }
    if !next.keys.is_empty() && app.has_admin(&config.keys) && !app.has_admin(&next.keys) {
        return error(
            StatusCode::CONFLICT,
            "this is the last admin key; create another admin key or revoke the client keys first",
        );
    }
    if let Err(e) = app.save(&next).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    *config = next;
    StatusCode::NO_CONTENT.into_response()
}

async fn get_config(State(app): State<Arc<App>>) -> Json<Value> {
    Json(public(&*app.config.read().await))
}

#[derive(Deserialize)]
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
}

#[derive(Deserialize)]
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
            return Err(format!("provider id '{id}' must use a-z, 0-9 and '-'"));
        }
        if providers.iter().any(|q: &Provider| q.id == id) {
            return Err(format!("provider id '{id}' is used twice"));
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
                "{id}: use https:// so the provider key is not sent in clear text"
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
        providers.push(Provider {
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
        return Err("default model must be provider/model with a configured provider".into());
    }
    Ok(Config {
        default_model,
        providers,
        keys: current.keys.clone(),
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

async fn put_config(State(app): State<Arc<App>>, Json(input): Json<ConfigInput>) -> Response {
    // The DNS checks run without the lock, so a slow resolver does not stall every request meanwhile.
    let snapshot = app.config.read().await.clone();
    let next = match build_config(&snapshot, input, app.allow_private) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    if let Err(e) = check_provider_hosts(&next.providers, app.allow_private).await {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let mut config = app.config.write().await;
    let next = Config {
        keys: config.keys.clone(),
        ..next
    };
    if let Err(e) = app.save(&next).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    *config = next;
    Json(public(&config)).into_response()
}

async fn clm_action(State(app): State<Arc<App>>, Path(action): Path<String>) -> Response {
    let Some(action) = install::Action::parse(&action) else {
        return error(StatusCode::NOT_FOUND, "expected install or remove");
    };
    {
        let mut task = app.clm_task.lock().unwrap();
        if task["state"] == "running" {
            return error(StatusCode::CONFLICT, "another CLM action is running");
        }
        *task = json!({ "state": "running" });
    }
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || {
        let progress = |step: &str| {
            *app2.clm_task.lock().unwrap() = json!({ "state": "running", "step": step })
        };
        let result = install::clm(action, false, &progress);
        *app2.clm_task.lock().unwrap() = match result {
            Ok(log) => json!({ "state": "done", "log": log }),
            Err(e) => json!({ "state": "error", "error": e }),
        };
    });
    (
        StatusCode::ACCEPTED,
        Json(app.clm_task.lock().unwrap().clone()),
    )
        .into_response()
}

async fn clm_status(State(app): State<Arc<App>>) -> Json<Value> {
    let clm = app
        .config
        .read()
        .await
        .providers
        .iter()
        .find(|p| p.installer.as_deref() == Some("clm"))
        .cloned();
    let mut loaded = false;
    if let Some(p) = &clm {
        let health = request(&app, p, reqwest::Method::GET, "/health")
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        if let Ok(r) = health {
            loaded = r.json::<Value>().await.is_ok_and(|h| h["embedder"] == true);
        }
    }
    if !loaded {
        app.clm_warm.store(0, Ordering::Relaxed);
    } else if let Some(p) = clm.filter(|_| {
        app.clm_warm
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
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
            let warm = sent.is_ok_and(|r| r.status().is_success());
            app2.clm_warm
                .store(if warm { 2 } else { 0 }, Ordering::Relaxed);
        });
    }
    let warm = app.clm_warm.load(Ordering::Relaxed) == 2;
    let steps = tokio::task::spawn_blocking(move || install::clm_steps(loaded, warm))
        .await
        .unwrap_or_default();
    let mut out = app.clm_task.lock().unwrap().clone();
    out["steps"] = json!(steps);
    Json(out)
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// A classifier built on Jev-compatible System One models: one API in front of local and cloud providers.
#[derive(Parser)]
#[command(name = "jeff", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the API and the web UI (the default command)
    Serve(ServeArgs),
    /// Install a local provider in Docker
    Install { stack: Stack },
    /// Stop and remove a local provider's containers; downloaded weights are kept
    Remove { stack: Stack },
    /// Manage jeff access keys in the config file; a running server picks changes up on the next request
    Keys {
        #[command(subcommand)]
        action: KeysAction,
        /// The config file [default: ~/.jeff/jeff.json]
        #[arg(long, env = "JEFF_CONFIG", global = true)]
        config: Option<String>,
    },
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

#[derive(Clone, Copy, ValueEnum)]
enum Stack {
    Clm,
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
    /// the config, every /v1 and /api request on both ports must send `Authorization: Bearer <key>`
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
        .map_err(|_| format!("'{s}' is not ADDR:PORT or PORT"))
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let args = match cli.command {
        None => ServeDefaults::parse_from(["jeff"]).args,
        Some(Command::Serve(args)) => args,
        Some(Command::Install { stack: Stack::Clm }) => {
            return run_install(install::Action::Install);
        }
        Some(Command::Remove { stack: Stack::Clm }) => return run_install(install::Action::Remove),
        Some(Command::Keys { action, config }) => return run_keys(action, &config_file(config)),
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

fn run_keys(action: KeysAction, path: &str) {
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1)
    };
    let mut config = load_config(path).unwrap_or_else(|e| fail(e));
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
        } => {
            let name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > 64 {
                fail("name must be 1 to 64 characters".into());
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
                "id {}; this key is shown only once, store it now",
                record.id
            );
        }
        KeysAction::Revoke { id } => {
            let before = config.keys.clone();
            config.keys.retain(|k| k.id != id);
            if config.keys.len() == before.len() {
                fail(format!("no key with id '{id}'"));
            }
            if !config.keys.is_empty() && has_admin(&before) && !has_admin(&config.keys) {
                fail(
                    "this is the last admin key; create another admin key or revoke the client keys first"
                        .into(),
                );
            }
            save_config(path, &config).unwrap_or_else(|e| fail(e));
            println!("revoked {id}");
        }
    }
}

fn run_install(action: install::Action) {
    match install::clm(action, true, &|step| println!("{step}")) {
        Ok(_) if matches!(action, install::Action::Install) => {
            println!(
                "done: CLM is at http://127.0.0.1:8700; the encoder downloads ~16 GB on first start"
            )
        }
        Ok(_) => println!("done"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

async fn serve(args: ServeArgs) {
    let config_path = config_file(args.config.clone());
    let config = load_config(&config_path).unwrap_or_else(|e| {
        fail_with(&format!(
            "{e}\nfix or remove the file; jeff does not start on a broken config"
        ))
    });
    if let Some(short) = args
        .api_keys
        .iter()
        .map(|k| k.trim())
        .find(|k| !k.is_empty() && k.len() < MIN_STATIC_KEY_LEN)
    {
        fail_with(&format!(
            "--api-key '{}…' is shorter than {MIN_STATIC_KEY_LEN} characters; generate one with `jeff keys create`",
            &short[..short.len().min(4)]
        ));
    }
    let allow_private = args.allow_private_providers;
    // A provider that redirects could send the provider key on to a host jeff never checked.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(GuardedResolver { allow_private })
        .build()
        .expect("HTTP client");
    let loopback = |addr: &str| {
        addr.parse::<SocketAddr>()
            .is_ok_and(|a| a.ip().is_loopback())
    };
    let exposed = !loopback(&args.api) || (!args.no_ui && !loopback(&args.ui));
    let app = Arc::new(App {
        http,
        config: RwLock::new(config),
        config_path: config_path.clone(),
        listed: RwLock::new(HashMap::new()),
        clm_task: Mutex::new(json!({ "state": "idle" })),
        clm_warm: AtomicU8::new(0),
        static_keys: args
            .api_keys
            .iter()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())
            .collect(),
        config_mtime: Mutex::new(modified(&config_path)),
        allowed_hosts: args
            .allowed_hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect(),
        failures: Mutex::new(HashMap::new()),
        exposed,
        allow_private,
    });
    if exposed && app.static_keys.is_empty() && app.config.read().await.keys.is_empty() {
        let addr = if loopback(&args.api) {
            &args.ui
        } else {
            &args.api
        };
        fail_with(&format!(
            "refusing to listen on {addr} without an access key: anyone who reaches it could use the stored \
             provider keys.\ncreate one first with `jeff keys create --name NAME --role admin`, or pass --api-key"
        ));
    }

    // /health stays open for liveness probes; it reveals nothing about providers.
    let api = Router::new()
        .route("/v1/systemone", post(systemone))
        .route("/v1/models", get(models))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_client))
        .route("/health", get(health))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES));
    // The UI calls the same /v1 routes on its own origin, so it works whether or not the API port is reachable from the browser.
    let api_addr = args.api.clone();
    let manage = Router::new()
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/clm", get(clm_status))
        .route("/api/clm/{action}", post(clm_action))
        .route("/api/keys", get(list_keys).post(create_key))
        .route("/api/keys/{id}", delete(revoke_key))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_admin))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES));
    let info_app = app.clone();
    let ui = api
        .clone()
        .merge(manage)
        .route(
            "/",
            get(|| async { Html(include_str!("../ui/index.html")) }),
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
    fail_with(&format!("cannot listen on {addr}: {e}"))
}

fn fail_with(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
