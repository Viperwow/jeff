mod install;
mod keys;

use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Request, State},
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
    /// Names a local stack jengine can install, such as "clm".
    #[serde(default)]
    installer: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    default_model: String,
    providers: Vec<Provider>,
    /// jengine access keys, hashed.
    #[serde(default)]
    keys: Vec<keys::StoredKey>,
}

impl Default for Config {
    fn default() -> Self {
        let env_or = |var: &str, default: &str| env::var(var).unwrap_or_else(|_| default.into());
        let key = |var: &str| env::var(var).ok().filter(|k| !k.is_empty());
        Config {
            default_model: env_or("JENGINE_DEFAULT_MODEL", "clm/clm-latest"),
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

fn load_config(path: &str) -> Config {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("{path}: {e}; starting with the default providers");
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

/// Writes the config readable by its owner only: it holds provider keys and access-key hashes.
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
    std::io::Write::write_all(
        &mut options.open(path).map_err(|e| format!("{path}: {e}"))?,
        text.as_bytes(),
    )
    .map_err(|e| format!("{path}: {e}"))
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
    /// Keys from `--api-key` / `JENGINE_API_KEY`; they never expire and are managed outside the UI.
    static_keys: Vec<String>,
    /// When the config file was last read, so keys created or revoked by `jengine keys` apply without a restart.
    config_mtime: Mutex<Option<SystemTime>>,
    /// Host names the UI answers to besides localhost and IP literals.
    allowed_hosts: Vec<String>,
    /// Failed key checks per client address in the current window: (count, window start).
    failures: Mutex<HashMap<IpAddr, (u32, u64)>>,
}

/// Failed key checks one address may make per window before it gets 429 until the window ends.
const MAX_FAILURES: u32 = 20;
const FAILURE_WINDOW_SECS: u64 = 60;
/// The largest provider response jengine relays; typed answers are a few kilobytes.
const MAX_UPSTREAM_BYTES: usize = 16 * 1024 * 1024;
/// Shortest `--api-key` accepted; generated keys carry 256 bits.
const MIN_STATIC_KEY_LEN: usize = 32;

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

fn is_loopback_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?']).next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl App {
    async fn refresh_from_disk(&self) {
        let now = modified(&self.config_path);
        let mut seen = *self.config_mtime.lock().unwrap();
        if now.is_some() && now != seen {
            *self.config.write().await = load_config(&self.config_path);
            seen = now;
            *self.config_mtime.lock().unwrap() = seen;
        }
    }

    async fn save(&self, config: &Config) -> Result<(), String> {
        save_config(&self.config_path, config)?;
        *self.config_mtime.lock().unwrap() = modified(&self.config_path);
        Ok(())
    }

    async fn auth_required(&self) -> bool {
        !self.static_keys.is_empty() || !self.config.read().await.keys.is_empty()
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
                headers.insert("x-jengine-provider", v);
            }
            headers.insert(
                "x-jengine-upstream-ms",
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

/// With any jengine key set, even an expired one, every API and management request must carry a valid key.
async fn require_key(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
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
        let ok = keys::verify(auth, &app.static_keys, &app.config.read().await.keys, now);
        if !ok {
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
                "a valid jengine API key is required: Authorization: Bearer <key>",
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
            format!("host '{name}' is not allowed; start jengine with --allowed-host {name}"),
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
    json!({ "id": k.id, "name": k.name, "prefix": k.prefix, "created_at": k.created_at, "expires_at": k.expires_at })
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
    let (key, record) = keys::generate(name, input.expires_at);
    let mut config = app.config.write().await;
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
    /// Absent keeps the stored key for this id, "" clears it.
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

fn build_config(current: &Config, input: ConfigInput) -> Result<Config, String> {
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
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("{id}: URL must start with http:// or https://"));
        }
        let key = match p.key {
            Some(k) => Some(k.trim().to_owned()).filter(|k| !k.is_empty()),
            None => current
                .providers
                .iter()
                .find(|q| q.id == id)
                .and_then(|q| q.key.clone()),
        };
        if key.is_some() && url.starts_with("http://") && !is_loopback_url(&url) {
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

async fn put_config(State(app): State<Arc<App>>, Json(input): Json<ConfigInput>) -> Response {
    let mut config = app.config.write().await;
    let next = match build_config(&config, input) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
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

/// Router for Jev-compatible System One models: one API in front of local and cloud providers.
#[derive(Parser)]
#[command(name = "jengine", version)]
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
    /// Manage jengine access keys in the config file; a running server picks changes up on the next request
    Keys {
        #[command(subcommand)]
        action: KeysAction,
        /// The config file [default: ~/.jengine/jengine.json]
        #[arg(long, env = "JENGINE_CONFIG", global = true)]
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
    #[arg(long, env = "JENGINE_API_ADDR", default_value = "127.0.0.1:8080", value_parser = parse_addr)]
    api: String,
    /// Where the web UI listens. It also manages providers, so keep it off public interfaces
    #[arg(long, env = "JENGINE_UI_ADDR", default_value = "127.0.0.1:8081", value_parser = parse_addr)]
    ui: String,
    /// Serve the API only
    #[arg(long)]
    no_ui: bool,
    /// Providers and access keys, created on the first save [default: ~/.jengine/jengine.json]
    #[arg(long, env = "JENGINE_CONFIG")]
    config: Option<String>,
    /// A key that never expires; repeat the flag or comma-separate the env var for several. With any key here or in
    /// the config, every /v1 and /api request on both ports must send `Authorization: Bearer <key>`
    #[arg(
        long = "api-key",
        env = "JENGINE_API_KEY",
        hide_env_values = true,
        value_delimiter = ','
    )]
    api_keys: Vec<String>,
    /// A host name the UI answers to besides localhost and IP addresses, such as the name of a reverse proxy
    #[arg(
        long = "allowed-host",
        env = "JENGINE_ALLOWED_HOSTS",
        value_delimiter = ','
    )]
    allowed_hosts: Vec<String>,
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
        None => ServeDefaults::parse_from(["jengine"]).args,
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
            .join("jengine.json")
            .to_string_lossy()
            .into_owned()
    })
}

fn run_keys(action: KeysAction, path: &str) {
    let mut config = load_config(path);
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(1)
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
                    "{}  {:<24} {}…  expires {}  {state}",
                    k.id,
                    k.name,
                    k.prefix,
                    date(k.expires_at)
                );
            }
        }
        KeysAction::Create { name, expires } => {
            let name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > 64 {
                fail("name must be 1 to 64 characters".into());
            }
            let expires_at = keys::parse_expiry(&expires, keys::now()).unwrap_or_else(|e| fail(e));
            let (key, record) = keys::generate(&name, expires_at);
            config.keys.push(record.clone());
            save_config(path, &config).unwrap_or_else(|e| fail(e));
            println!("{key}");
            eprintln!(
                "id {}; this key is shown only once, store it now",
                record.id
            );
        }
        KeysAction::Revoke { id } => {
            let before = config.keys.len();
            config.keys.retain(|k| k.id != id);
            if config.keys.len() == before {
                fail(format!("no key with id '{id}'"));
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
    let config = load_config(&config_path);
    if let Some(short) = args
        .api_keys
        .iter()
        .map(|k| k.trim())
        .find(|k| !k.is_empty() && k.len() < MIN_STATIC_KEY_LEN)
    {
        fail_with(&format!(
            "--api-key '{}…' is shorter than {MIN_STATIC_KEY_LEN} characters; generate one with `jengine keys create`",
            &short[..short.len().min(4)]
        ));
    }
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("HTTP client");
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
    });

    // /health stays open for liveness probes; it reveals nothing about providers.
    let api = Router::new()
        .route("/v1/systemone", post(systemone))
        .route("/v1/models", get(models))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_key))
        .route("/health", get(health));
    // The UI calls the same /v1 routes on its own origin, so it works whether or not the API port is reachable from the browser.
    let api_addr = args.api.clone();
    let manage = Router::new()
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/clm", get(clm_status))
        .route("/api/clm/{action}", post(clm_action))
        .route("/api/keys", get(list_keys).post(create_key))
        .route("/api/keys/{id}", delete(revoke_key))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_key));
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
    println!("jengine API on http://{}", args.api);
    let public = !args.api.starts_with("127.") && !args.api.starts_with("[::1]");
    if public && !app.auth_required().await {
        eprintln!(
            "warning: the API listens on {} without --api-key; anyone who reaches it can use the stored provider keys",
            args.api
        );
    }
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
    println!("jengine UI  on http://{}", args.ui);
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
        assert!(is_loopback_url("http://127.0.0.1:8700"));
        assert!(is_loopback_url("http://localhost:8000/x"));
        assert!(is_loopback_url("http://[::1]:9000"));
        assert!(!is_loopback_url("http://10.0.0.5:8000"));
        assert!(!is_loopback_url("http://api.example.com"));
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
        assert_eq!(
            build_config(&current, input(None)).unwrap().providers[0]
                .key
                .as_deref(),
            Some("secret")
        );
        assert!(
            build_config(&current, input(Some(" "))).unwrap().providers[0]
                .key
                .is_none()
        );
        let mut bad = input(None);
        bad.default_model = "clm/clm-latest".into();
        assert!(build_config(&current, bad).is_err());
    }
}
