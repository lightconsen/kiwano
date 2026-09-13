//! Gateway sidecar lifecycle + admin plane client (tech.md §4.1/§4.6).
//!
//! The GUI spawns `kiwanod` as a child process sharing the same
//! SQLite file. The admin plane is a local IPC endpoint — a unix domain socket,
//! or a per-user named pipe on Windows — not a TCP port, so the control-channel
//! calls are raw HTTP written to that endpoint. Both ends of that transport live
//! in `kiwanod::server::admin_ipc`, which this module re-exports
//! [`AdminEndpoint`] from: the plane is defined once, and the app inherits the
//! endpoint from the database path rather than from a port number. No extra HTTP
//! client dependency is needed, and no other process on the machine can reach
//! the plane to begin with.
//!
//! `/reload` and `/shutdown` need the admin token the gateway minted for
//! itself ([`ADMIN_TOKEN_KEY`]); it is read from that same SQLite file, so the
//! two processes agree without any new IPC. The one exception is the liveness
//! probe — see [`ping_admin`].
//!
//! The admin plane was a loopback TCP port before 0.1.8, and this module carried
//! a fallback that looked for — and stopped — a gateway from before that move.
//! It has been removed: no released build ever shipped a gateway on that
//! transport, so the only thing left to find was a hand-built binary on a
//! developer's machine, and the app now speaks IPC alone.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use kiwanod::server::{ADMIN_TOKEN_HEADER, ADMIN_TOKEN_KEY};
use kiwanod::store::Store;

/// The endpoint type itself, so callers in this crate name
/// `sidecar::AdminEndpoint` and do not need to know the gateway crate's module
/// layout.
pub use kiwanod::server::AdminEndpoint;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// The `Host` header for a request over the IPC endpoint. Its value is
/// arbitrary — the admin router does no `Host` filtering — but HTTP/1.1
/// requires the header to be there.
const IPC_HOST: &str = "localhost";

#[cfg(windows)]
const GATEWAY_BIN_NAMES: &[&str] = &["kiwanod.exe", "kiwanod"];
#[cfg(not(windows))]
const GATEWAY_BIN_NAMES: &[&str] = &["kiwanod"];

/// Resolve the daemon binary: explicit env override, then the sibling of the
/// running GUI binary.
///
/// Sibling is where the bundler puts an `externalBin` on every platform —
/// `Contents/MacOS/` in the .app, `usr/bin/` in the deb/rpm and inside the
/// AppImage, beside `kiwano-app.exe` on Windows — and where cargo leaves it in
/// dev (`target/<profile>/kiwanod`, next to `target/<profile>/kiwano-app`).
///
/// Deliberately no `PATH` search: a `kiwanod` from somewhere else is a
/// different version answering the same admin endpoint, and version skew is
/// handled by replacing it (see `startup_action`), not by quietly running it.
fn gateway_bin_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KIWANO_GATEWAY_BIN") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for name in GATEWAY_BIN_NAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // Without the probe above this returned a path that does not exist, and
    // the only trace was a spawn error that read like a permissions problem.
    // In a packaged app stdout is /dev/null, so the log is the one place a
    // missing sidecar can be diagnosed.
    tracing::error!(
        dir = %dir.display(),
        tried = ?GATEWAY_BIN_NAMES,
        "no kiwanod beside the app binary — this bundle is missing its sidecar"
    );
    None
}

/// Spawn the sidecar; the child prints a `ready ...` line on stdout once
/// both planes are bound. Callers need not wait — status pings handle it.
pub fn spawn() -> std::io::Result<Child> {
    let Some(bin) = gateway_bin_path() else {
        return Err(std::io::Error::other(
            "gateway sidecar not found beside the app binary — reinstall Kiwano, \
             or set KIWANO_GATEWAY_BIN to a kiwanod build",
        ));
    };
    Command::new(&bin).spawn().inspect_err(|e| {
        tracing::error!(binary = %bin.display(), error = %e, "cannot spawn gateway sidecar");
    })
}

/// The SQLite file this app and the gateway share. Resolution matches the
/// gateway's `db_path_from_env` and the CLI's `--db` default: `KIWANO_DB_PATH`,
/// else `~/.kiwano/kiwano.db`. The app sets no environment for the child, so
/// the spawned gateway inherits `KIWANO_DB_PATH` and the two agree by
/// construction.
pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("KIWANO_DB_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".kiwano").join("kiwano.db")
}

/// [`default_db_path`], unless the caller named one.
///
/// The CLI takes `--db`, the app does not; both then agree on the fallback
/// because it is resolved here rather than in either front end.
pub fn db_path(explicit: Option<&Path>) -> PathBuf {
    match explicit {
        Some(p) => p.to_path_buf(),
        None => default_db_path(),
    }
}

/// The admin plane endpoint this app and its gateway share.
///
/// Resolved exactly the way the gateway resolves it — the same function, on the
/// same database path — so the two cannot disagree, and `KIWANO_ADMIN_SOCKET`
/// moves both at once. The child needs no argument for it: it inherits this
/// process's environment and derives the same default from the same file
/// location.
pub fn admin_endpoint() -> AdminEndpoint {
    admin_endpoint_for(&default_db_path())
}

/// [`admin_endpoint`] for an explicit database path.
///
/// The CLI takes `--db`, so it cannot resolve through [`default_db_path`]'s
/// environment lookup. The resolution is otherwise identical — the same
/// [`AdminEndpoint::from_env`] on the same kind of path — which is what keeps
/// two clients from disagreeing about where the plane is.
pub fn admin_endpoint_for(db: &Path) -> AdminEndpoint {
    AdminEndpoint::from_env(db)
}

/// The admin token, read out of the row the gateway minted it into.
///
/// `None` when there is no database yet or the row is not there: on a fresh
/// install the app is up before the gateway has ever run. Deliberately never
/// creates the file — an empty database conjured up by a token lookup would
/// then be migrated by whichever process got there first.
pub fn admin_token_for(db: &Path) -> Option<String> {
    if !db.is_file() {
        return None;
    }
    Store::open(db)
        .ok()?
        .app_setting(ADMIN_TOKEN_KEY)
        .filter(|t| !t.trim().is_empty())
}

/// [`admin_token_for`] against the shared database, cached once found.
///
/// The row does not change under a running gateway, so the first success is
/// kept; a miss is retried, because the gateway may simply not have started
/// yet. Two opens at most, in the ordinary case where the app starts first.
///
/// The cache is why this is the app's form and not the CLI's: a CLI process
/// resolves its own `--db` once and reads the token from there.
fn admin_token() -> Option<String> {
    static CACHED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(token) = CACHED.get() {
        return Some(token.clone());
    }
    let token = admin_token_for(&default_db_path())?;
    let _ = CACHED.set(token.clone());
    Some(token)
}

/// The token header for one raw request, or nothing when there is no token to
/// send. A caller that has none is still answered — liveness-only `/status`, 401
/// from `/reload` (see `server::admin`) — so a missing token is not an error
/// here. That is the fresh-install case, before the gateway has written its row.
fn token_header(token: Option<&str>) -> String {
    match token {
        Some(token) => format!("{ADMIN_TOKEN_HEADER}: {token}\r\n"),
        None => String::new(),
    }
}

/// The three admin requests. The framing is raw HTTP written to the endpoint,
/// which is why these are strings rather than calls on a client: [`IPC_HOST`] is
/// arbitrary (the router does no `Host` filtering) but required by HTTP/1.1, and
/// the rest is the same bytes for every call.
fn status_request(token: Option<&str>) -> String {
    format!(
        "GET /status HTTP/1.1\r\nHost: {IPC_HOST}\r\n{}Connection: close\r\n\r\n",
        token_header(token)
    )
}

fn shutdown_request(token: Option<&str>) -> String {
    format!(
        "POST /shutdown HTTP/1.1\r\nHost: {IPC_HOST}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        token_header(token)
    )
}

fn reload_request(token: Option<&str>) -> String {
    format!(
        "POST /reload HTTP/1.1\r\nHost: {IPC_HOST}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        token_header(token)
    )
}

/// One raw HTTP request over the admin endpoint, read to EOF.
///
/// `None` when the endpoint does not answer at all (nothing is listening, or the
/// path/pipe is not there) — the same "no gateway" answer a refused TCP connect
/// used to be. The connect is bounded by [`CONNECT_TIMEOUT`], and on unix so is
/// every read and write after it (see `AdminEndpoint::connect`).
fn endpoint_body(endpoint: &AdminEndpoint, request: &str) -> Option<String> {
    let mut stream = endpoint.connect(CONNECT_TIMEOUT).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = String::new();
    BufReader::new(stream).read_to_string(&mut raw).ok()?;
    // Drop the status line and headers: the body follows the blank line.
    raw.split_once("\r\n\r\n").map(|(_, body)| body.to_string())
}

/// The response's first line only — all the liveness probe and the
/// "did it answer 200" checks need, without waiting on a body.
fn endpoint_http(endpoint: &AdminEndpoint, request: &str) -> Option<String> {
    let mut stream = endpoint.connect(CONNECT_TIMEOUT).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    Some(line)
}

/// The gateway's own `/status` report, off the admin endpoint. None when it is
/// not answering, or says something we cannot read — the caller then shows less,
/// never a guess.
///
/// The token, when there is one, buys the full report; without it the gateway
/// answers with the liveness subset (identity, version, uptime), which the
/// status chip renders as a gateway that is up but not described. This is also
/// the *startup* question — "is a gateway of ours already running, and which
/// version?" — answered by the same call.
pub fn gateway_status(endpoint: &AdminEndpoint) -> Option<serde_json::Value> {
    status_for(endpoint, admin_token().as_deref())
}

/// [`gateway_status`] with the token supplied by the caller.
///
/// A CLI reads its token out of its own `--db`, so the cached, env-resolved
/// [`admin_token`] would be the wrong one to send. `None` simply omits the
/// header, and `/status` answers with the liveness subset.
pub fn status_for(endpoint: &AdminEndpoint, token: Option<&str>) -> Option<serde_json::Value> {
    let body = endpoint_body(endpoint, &status_request(token))?;
    serde_json::from_str(&body).ok()
}

/// `GET /status` — true when the admin plane answers.
///
/// Deliberately sends no token. `/status` answers 200 to an unauthenticated
/// caller by design (see `server::admin`), and this is the probe the watchdog
/// runs every few seconds: it must be able to tell "a gateway is here" from
/// "nothing is here" without a readable database, and without depending on a
/// row that only exists once a token-aware gateway has started.
pub fn ping_admin(endpoint: &AdminEndpoint) -> bool {
    endpoint_http(endpoint, &status_request(None)).is_some_and(|line| line.contains("200"))
}

/// `POST /shutdown` — ask a running gateway to stop, then wait for it to let go
/// of the endpoint. False means it could not be stopped, which the caller must
/// not read as "it is stopped".
///
/// A gateway that refuses the token (401 — what an unreadable `app_settings` row
/// looks like) lands in the same `false`, as does one that does not answer at
/// all. [`restart`] treats all of them the same way.
pub fn request_shutdown(endpoint: &AdminEndpoint) -> bool {
    shutdown_for(endpoint, admin_token().as_deref())
}

/// [`request_shutdown`] with the token supplied by the caller.
pub fn shutdown_for(endpoint: &AdminEndpoint, token: Option<&str>) -> bool {
    let accepted =
        endpoint_http(endpoint, &shutdown_request(token)).is_some_and(|line| line.contains("200"));
    if !accepted {
        return false;
    }
    // The reply is written before the serve loop returns, but the listener is
    // not necessarily closed the instant the status line arrives — and the
    // replacement has to bind the same endpoint.
    for _ in 0..50 {
        if !ping_admin(endpoint) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// What to do about a gateway that may already be serving the admin plane.
///
/// Factored out because the mismatch branch cannot be reached by hand without
/// an old binary, and it is the branch that decides whether the app runs beside
/// a daemon that disagrees with it about the database schema.
///
/// The decision reads `name` + `version` out of `GET /status`, and those are in
/// the liveness subset the gateway returns to an unauthenticated caller — so a
/// token whose row cannot be read still lets us tell whose daemon this is. The
/// replace path then gets `/shutdown` — see [`restart`].
#[derive(Debug, PartialEq, Eq)]
pub enum StartupAction {
    /// Same version — leave it alone and adopt it.
    Adopt,
    /// Ours, wrong version — stop it and start the bundled one.
    Restart,
    /// Nothing there, or something that is not a kiwanod.
    Spawn,
}

pub fn startup_action(status: Option<&serde_json::Value>, ours: &str) -> StartupAction {
    let Some(status) = status else {
        return StartupAction::Spawn;
    };
    // The endpoint may be held by something else entirely — a pipe or socket
    // another program happens to own. Not ours to stop; the spawn that follows
    // will fail to bind, and say so.
    if status.get("name").and_then(|n| n.as_str()) != Some("kiwanod") {
        tracing::error!(
            "the admin endpoint is held by something that is not kiwanod; leaving it alone"
        );
        return StartupAction::Spawn;
    }
    match status.get("version").and_then(|v| v.as_str()) {
        Some(v) if v == ours => StartupAction::Adopt,
        // Includes a gateway that could not tell us its version, which is by
        // definition not the one we ship.
        _ => StartupAction::Restart,
    }
}

/// Stop the gateway serving the admin plane and start the bundled one in its
/// place. Returns `Err` when the old one would not stop, rather than starting a
/// second gateway that cannot bind the data port.
///
/// `/shutdown` over IPC is the only way to ask; the trace below says why there
/// is no second one.
///
/// # Why there is no signal-based fallback
///
/// There used to be a `force_stop`, which found the gateway through
/// `lsof -iTCP:<port> -sTCP:LISTEN` and sent it SIGTERM. It existed for a
/// gateway built before `POST /shutdown` existed. Three things made it the wrong
/// thing to keep:
///
/// - The population it served was empty in practice: every gateway this app
///   shipped answers `/shutdown`, and a daemon that does not is one built from
///   the workspace by hand.
/// - The rework it would have taken — `lsof -U` on the socket path — is strictly
///   worse than the TCP form it replaced. `-U` matches *every* unix socket the
///   process holds (including client connections), and the path a listening
///   socket was bound to is not reliably in `lsof`'s output; the TCP form could
///   match on a port that only one process can hold. A last-resort path that
///   sometimes kills the wrong thing, or nothing, is worse than a clear error.
/// - And it was never the safe path anyway: it existed to reach a daemon whose
///   write-ahead log the graceful stop is what checkpoints. When nothing can ask
///   the gateway to stop, the honest outcome is to say so and let the operator
///   stop it by hand, which is what the error below does.
///
/// It is worth adding back only if a gateway that answers neither `/shutdown`
/// nor anything else is ever shipped, and the intended lesson from the paths
/// above is that it will not be.
pub fn restart(endpoint: &AdminEndpoint) -> std::io::Result<Child> {
    if !request_shutdown(endpoint) {
        tracing::error!(
            endpoint = %endpoint.describe(),
            "the running gateway would not stop; stop it by hand (pkill -f kiwanod)"
        );
        return Err(std::io::Error::other(
            "the running gateway would not stop; not starting a second one on the same ports",
        ));
    }
    spawn()
}

/// `POST /reload` — ask the gateway to rebuild its route table from SQLite.
/// Fire-and-forget: route hot-reload failures surface in gateway logs, and a
/// refusal (no token readable) means the gateway keeps routing what it has
/// until the next start, which is why this returns nothing to check.
pub fn notify_reload(endpoint: &AdminEndpoint) {
    let _ = reload(endpoint, admin_token().as_deref());
}

/// [`notify_reload`] with the token supplied by the caller, returning the
/// gateway's answer.
///
/// The app can afford to discard the reply; a command-line caller cannot. It
/// prints `agents_routed` on success or the refusal reason on failure, and its
/// exit code depends on which it got — so this reads to EOF for the body rather
/// than stopping at the status line.
pub fn reload(endpoint: &AdminEndpoint, token: Option<&str>) -> Option<serde_json::Value> {
    let body = endpoint_body(endpoint, &reload_request(token))?;
    serde_json::from_str(&body).ok()
}

/// TCP connect-time probe of an endpoint (host or URL). Measures the
/// handshake only — enough for the inline latency readout in the add dialog.
pub fn measure_latency(endpoint: &str) -> Result<u64, String> {
    use std::net::ToSocketAddrs;

    let trimmed = endpoint.trim();
    let https = trimmed.starts_with("https://") || !trimmed.contains("://");
    let rest = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return Err("empty endpoint".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| format!("invalid port: {p}"))?,
        ),
        None => (authority.to_string(), if https { 443 } else { 80 }),
    };
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| format!("cannot resolve: {host}"))?;
    let start = std::time::Instant::now();
    TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    Ok(start.elapsed().as_millis() as u64)
}

/// Result of a protocol-aware endpoint probe (GET models on the canonical
/// per-protocol route).
#[derive(serde::Serialize)]
pub struct ProbeReport {
    /// ok | auth | unsupported | error | unreachable
    pub verdict: String,
    pub status: Option<u16>,
    pub latency_ms: u64,
    /// Human-readable explanation for the UI chip.
    pub detail: String,
}

/// Build the canonical "does this endpoint speak this protocol" URL:
/// the protocol's models list on its well-known path.
fn probe_url(protocol: &str, base: &str) -> Result<String, String> {
    let trimmed = base.trim().trim_end_matches('/');
    let root = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    if root.is_empty() {
        return Err("empty endpoint".into());
    }
    let (scheme, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        ("https://", r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        ("http://", r)
    } else {
        ("https://", root)
    };
    // A base that already carries a version segment extends with /models;
    // a bare host gets the protocol's canonical version path.
    let already_versioned =
        rest.ends_with("/v1") || rest.ends_with("/v1beta") || rest.ends_with("/v1alpha");
    let url = match protocol {
        "anthropic" | "openai" if already_versioned => format!("{scheme}{rest}/models"),
        "anthropic" | "openai" => format!("{scheme}{rest}/v1/models"),
        "gemini" if already_versioned => format!("{scheme}{rest}/models"),
        "gemini" => format!("{scheme}{rest}/v1beta/models"),
        other => return Err(format!("unknown protocol: {other}")),
    };
    Ok(url)
}

/// Attach the protocol's canonical auth headers to a GET request. A blank or
/// missing key is allowed (anonymous probe); an unknown protocol is an error.
fn apply_auth(
    protocol: &str,
    mut req: reqwest::RequestBuilder,
    api_key: Option<&str>,
) -> Result<reqwest::RequestBuilder, String> {
    let key = api_key.map(str::trim).filter(|k| !k.is_empty());
    match protocol {
        "openai" => {
            if let Some(k) = key {
                req = req.bearer_auth(k);
            }
        }
        "anthropic" => {
            if let Some(k) = key {
                req = req.header("x-api-key", k);
            }
            req = req.header("anthropic-version", "2023-06-01");
        }
        "gemini" => {
            if let Some(k) = key {
                req = req.header("x-goog-api-key", k);
            }
        }
        other => return Err(format!("unknown protocol: {other}")),
    }
    Ok(req)
}

/// Probe one endpoint for protocol support: GET the protocol's models route
/// with its canonical auth headers. A 401/403 still proves the route exists
/// (protocol supported, key missing/invalid); only 404/405 means unsupported.
/// Works without an API key.
/// Async on purpose: commands run on the tokio runtime, and a blocking
/// client (which owns its own runtime) panics when dropped inside one.
pub async fn probe_endpoint(
    protocol: &str,
    endpoint: &str,
    api_key: Option<&str>,
) -> Result<ProbeReport, String> {
    let url = probe_url(protocol, endpoint)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let req = apply_auth(protocol, client.get(&url), api_key)?;

    let start = std::time::Instant::now();
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return Ok(ProbeReport {
                verdict: "unreachable".into(),
                status: None,
                latency_ms: start.elapsed().as_millis() as u64,
                detail: format!("connection failed: {e}"),
            });
        }
    };
    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status().as_u16();
    let is_json = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json"));
    let body = resp.text().await.unwrap_or_default();

    let (verdict, detail) = match status {
        s if (200..300).contains(&s) => {
            if !is_json {
                // A marketing page answers 200 on anything — not an API.
                (
                    "error".into(),
                    "endpoint returned HTML, not an API".to_string(),
                )
            } else {
                let models = count_models(&body);
                match models {
                    Some(n) if n > 0 => ("ok".into(), format!("{n} models listed")),
                    _ => ("ok".into(), "route answered".to_string()),
                }
            }
        }
        401 | 403 => (
            "auth".into(),
            "route exists — auth required or key invalid".to_string(),
        ),
        404 | 405 => (
            "unsupported".into(),
            "route not found — protocol not supported".to_string(),
        ),
        s if (500..600).contains(&s) => ("error".into(), format!("upstream error {s}")),
        s => ("error".into(), format!("unexpected status {s}")),
    };
    Ok(ProbeReport {
        verdict,
        status: Some(status),
        latency_ms,
        detail,
    })
}

/// Count models in a models-list body (openai/anthropic: `data[]`,
/// gemini: `models[]`). None when the shape doesn't match.
fn count_models(body: &str) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
        return Some(arr.len());
    }
    if let Some(arr) = v.get("models").and_then(|d| d.as_array()) {
        return Some(arr.len());
    }
    None
}

/// Extract the model ids from a models-list body (openai/anthropic: `data[].id`,
/// gemini: `models[].name` with a `models/` prefix) plus gemini's pagination
/// token when more pages follow.
fn parse_models(body: &str) -> (Vec<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return (Vec::new(), None);
    };
    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| v.get("models").and_then(|d| d.as_array()));
    let names = arr
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m
                .get("id")
                .and_then(|x| x.as_str())
                .or_else(|| m.get("name").and_then(|x| x.as_str()))?;
            Some(id.strip_prefix("models/").unwrap_or(id).to_string())
        })
        .collect();
    let token = v
        .get("nextPageToken")
        .and_then(|t| t.as_str())
        .map(String::from);
    (names, token)
}

/// Fetch the live model-name list from a provider endpoint. Requires the API
/// key (cloud providers reject anonymous /models calls). Follows gemini's
/// nextPageToken pagination; openai/anthropic answer in one page. Sorted and
/// deduped for the dropdown.
pub async fn fetch_model_names(
    protocol: &str,
    endpoint: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API key required".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let mut names: Vec<String> = Vec::new();
    let mut page_token: Option<String> = None;
    // Gemini pages at ~50 entries; 5 pages = 250 models, plenty for a picker
    for _ in 0..5 {
        let mut url = probe_url(protocol, endpoint)?;
        if let Some(t) = &page_token {
            url.push_str(&format!("?pageToken={t}"));
        }
        let req = apply_auth(protocol, client.get(&url), Some(key))?;
        let resp = req
            .send()
            .await
            .map_err(|e| format!("connection failed: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(match status {
                401 | 403 => "auth failed — check the API key".into(),
                404 | 405 => "route not found — protocol not supported".into(),
                s if (500..600).contains(&s) => format!("upstream error {s}"),
                s => format!("unexpected status {s}"),
            });
        }
        let (page, token) = parse_models(&body);
        names.extend(page);
        page_token = token;
        if page_token.is_none() {
            break;
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(unix)]
    use std::sync::{Arc, Mutex};

    /// An endpoint nothing is serving, unique to this test.
    ///
    /// `parse`, not `beside_db`: on Windows `beside_db` resolves to the real
    /// per-user pipe name, which the gateway this developer is running right now
    /// may well be holding — the test would then be talking to it.
    fn dead_endpoint(dir: &Path) -> AdminEndpoint {
        let unique = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "endpoint".to_string());
        AdminEndpoint::parse(&format!("kiwano-test-dead-{unique}"))
    }

    /// The admin plane refuses `/reload` and `/shutdown` without the token, so
    /// these three requests have to carry it — and must still be well-formed
    /// when the app has none (a fresh install, before the gateway has written
    /// its row), rather than dropping the header line and the blank line with it.
    #[test]
    fn admin_requests_carry_the_token_when_we_have_one() {
        let header = format!("{ADMIN_TOKEN_HEADER}: tok-123\r\n");

        let status = status_request(Some("tok-123"));
        assert!(status.starts_with("GET /status HTTP/1.1\r\n"));
        assert!(status.contains(&header));
        assert!(status.contains(&format!("Host: {IPC_HOST}\r\n")));
        assert!(status.ends_with("Connection: close\r\n\r\n"));

        let shutdown = shutdown_request(Some("tok-123"));
        assert!(shutdown.starts_with("POST /shutdown HTTP/1.1\r\n"));
        assert!(shutdown.contains(&header));

        let reload = reload_request(Some("tok-123"));
        assert!(reload.starts_with("POST /reload HTTP/1.1\r\n"));
        assert!(reload.contains(&header));

        // No token: same requests, no header — a caller with no token to send is
        // answered as one that needs none, so a missing header is not an error.
        for request in [
            status_request(None),
            shutdown_request(None),
            reload_request(None),
        ] {
            assert!(!request.contains(ADMIN_TOKEN_HEADER));
            assert!(request.contains(&format!("Host: {IPC_HOST}\r\n")));
            assert!(request.ends_with("\r\n\r\n"));
        }
    }

    /// The token comes out of the row the gateway writes, in the database the
    /// two processes share. A database without one — or without a file — is
    /// None, not an error and not a new empty database.
    #[test]
    fn admin_token_reads_the_gateway_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(admin_token_for(&path), None);
        store.set_app_setting(ADMIN_TOKEN_KEY, "tok-abc").unwrap();
        drop(store);
        assert_eq!(admin_token_for(&path), Some("tok-abc".to_string()));

        let missing = dir.path().join("not-yet.db");
        assert_eq!(admin_token_for(&missing), None);
        assert!(
            !missing.exists(),
            "a token lookup must not bring a database into being"
        );
    }

    /// Nothing is listening on the endpoint, so every admin call has to report
    /// "no gateway" rather than panic or invent one. This is the shape the old
    /// `ping_admin_refuses_dead_port` had, and the failure it guards is a client
    /// that treats an unreachable endpoint as an error instead of an answer.
    #[test]
    fn a_dead_endpoint_reads_as_no_gateway() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = dead_endpoint(dir.path());

        assert!(!ping_admin(&endpoint), "the liveness probe answers false");
        assert!(
            gateway_status(&endpoint).is_none(),
            "and there is no report to read"
        );
        assert!(
            !request_shutdown(&endpoint),
            "nothing was stopped, so this is not a success"
        );
        notify_reload(&endpoint); // must not panic
    }

    /// `restart` must not start a second gateway next to one it failed to stop —
    /// the two would fight over the data port. With nothing to stop it has to
    /// return an error and spawn nothing.
    #[test]
    fn restart_refuses_when_nothing_can_be_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let err = restart(&dead_endpoint(dir.path()))
            .expect_err("nothing was stopped, so nothing may be started");
        assert!(err.to_string().contains("would not stop"), "{err}");
    }

    /// The mismatch branch is the one that cannot be reached by hand without an
    /// old binary, so it is pinned here rather than left to the dev machine.
    #[test]
    fn startup_action_adopts_only_its_own_version() {
        let gw = |version: serde_json::Value| serde_json::json!({ "ok": true, "name": "kiwanod", "version": version });

        // Nothing answering: start one.
        assert_eq!(startup_action(None, "0.1.8"), StartupAction::Spawn);

        // Ours: adopt, do not interrupt it.
        assert_eq!(
            startup_action(Some(&gw("0.1.8".into())), "0.1.8"),
            StartupAction::Adopt
        );

        // An upgrade meeting its own predecessor — the case that matters.
        assert_eq!(
            startup_action(Some(&gw("0.1.7".into())), "0.1.8"),
            StartupAction::Restart
        );

        // A gateway that cannot say its version is not the one we ship.
        assert_eq!(
            startup_action(Some(&gw(serde_json::Value::Null)), "0.1.8"),
            StartupAction::Restart
        );

        // Someone else's listener: start ours anyway (it will fail to bind and
        // log why) rather than stopping a process that is not ours to stop.
        let stranger = serde_json::json!({ "ok": true, "name": "not-kiwano" });
        assert_eq!(
            startup_action(Some(&stranger), "0.1.8"),
            StartupAction::Spawn
        );
    }

    #[test]
    fn latency_parse_rejects_garbage() {
        assert!(measure_latency("").is_err());
        assert!(measure_latency("https://host:notaport").is_err());
        assert!(measure_latency("https://definitely-not-a-host.invalid:443").is_err());
    }

    #[test]
    fn probe_urls_follow_protocol_conventions() {
        // bare hosts get the canonical version path per protocol
        assert_eq!(
            probe_url("openai", "https://api.deepseek.com").unwrap(),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            probe_url("anthropic", "https://api.deepseek.com/anthropic").unwrap(),
            "https://api.deepseek.com/anthropic/v1/models"
        );
        assert_eq!(
            probe_url("gemini", "https://generativelanguage.googleapis.com").unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
        // versioned bases extend with /models as-is
        assert_eq!(
            probe_url("openai", "https://api.moonshot.cn/v1").unwrap(),
            "https://api.moonshot.cn/v1/models"
        );
        assert_eq!(
            probe_url("gemini", "https://x.example.com/v1beta").unwrap(),
            "https://x.example.com/v1beta/models"
        );
        // scheme defaults to https when absent; garbage is rejected
        assert_eq!(
            probe_url("openai", "api.example.com").unwrap(),
            "https://api.example.com/v1/models"
        );
        assert!(probe_url("openai", "").is_err());
        assert!(probe_url("grpc", "https://x.example.com").is_err());
    }

    #[test]
    fn count_models_reads_both_shapes() {
        assert_eq!(count_models(r#"{"data":[{"id":"a"},{"id":"b"}]}"#), Some(2));
        assert_eq!(count_models(r#"{"models":[{"name":"m1"}]}"#), Some(1));
        assert_eq!(count_models(r#"{"error":{}}"#), None);
        assert_eq!(count_models("<html>"), None);
    }

    #[test]
    fn parse_models_reads_ids_names_and_pagination() {
        let (names, token) = parse_models(r#"{"data":[{"id":"a"},{"id":"b"}]}"#);
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(token, None);
        let (names, token) =
            parse_models(r#"{"models":[{"name":"models/gemini-2.0-flash"}],"nextPageToken":"Pg"}"#);
        assert_eq!(names, vec!["gemini-2.0-flash".to_string()]);
        assert_eq!(token, Some("Pg".to_string()));
        let (names, token) = parse_models("<html>");
        assert!(names.is_empty());
        assert_eq!(token, None);
    }

    /// An in-process stand-in for a real admin plane, serving the three routes
    /// by hand over `endpoint`.
    ///
    /// Hand-rolled rather than an axum router, because this crate has no
    /// HTTP-server dependency and does not need one: what is under test here is
    /// the *client* — the exact bytes it writes and how it reads the answer — so
    /// a stub that answers raw bytes is the more honest fixture. The server end
    /// of this transport is exercised by `kiwanod`'s own
    /// `admin_routes_answer_over_the_ipc_endpoint`, which runs on both CI
    /// platforms including Windows; this stub is unix-only because a Windows
    /// named pipe cannot be created from `std`.
    #[cfg(unix)]
    struct LiveGateway {
        endpoint: AdminEndpoint,
        reloads: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    #[cfg(unix)]
    impl LiveGateway {
        fn start(dir: &Path, version: &str) -> LiveGateway {
            use std::os::unix::net::UnixListener;

            let endpoint = AdminEndpoint::parse(&dir.join("admin.sock").to_string_lossy());
            let listener = UnixListener::bind(endpoint.path()).expect("bind the stub endpoint");
            let reloads = Arc::new(AtomicUsize::new(0));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let (reload_count, seen) = (reloads.clone(), requests.clone());
            let version = version.to_string();

            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                    seen.lock().unwrap().push(request.clone());

                    // `/shutdown` is the interesting one: the reply goes out and
                    // the listener is then dropped, which is exactly what the
                    // real gateway does to the endpoint — and what
                    // `request_shutdown` waits to observe.
                    let shutdown = request.starts_with("POST /shutdown");
                    let body = if shutdown {
                        r#"{"ok":true}"#.to_string()
                    } else if request.starts_with("POST /reload") {
                        reload_count.fetch_add(1, Ordering::SeqCst);
                        r#"{"ok":true,"agents_routed":1}"#.to_string()
                    } else {
                        format!(
                            r#"{{"ok":true,"name":"kiwanod","version":"{version}","uptime_secs":1}}"#
                        )
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    if shutdown {
                        return;
                    }
                }
            });

            let gateway = LiveGateway {
                endpoint,
                reloads,
                requests,
            };
            for _ in 0..100 {
                if ping_admin(&gateway.endpoint) {
                    return gateway;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("the stub admin plane never came up");
        }

        /// The first request the stub saw that starts with `prefix`.
        fn request_starting_with(&self, prefix: &str) -> Option<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.starts_with(prefix))
                .cloned()
        }
    }

    /// The whole admin client against a live endpoint: the liveness probe, the
    /// status report, the reload, and the shutdown that takes the endpoint down.
    #[cfg(unix)]
    #[test]
    fn the_admin_client_talks_to_a_live_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let gw = LiveGateway::start(dir.path(), "0.1.8");

        assert!(ping_admin(&gw.endpoint), "the liveness probe answers");

        let status = gateway_status(&gw.endpoint).expect("the status report");
        assert_eq!(status["name"], "kiwanod");
        assert_eq!(status["version"], "0.1.8");

        notify_reload(&gw.endpoint);
        assert_eq!(
            gw.reloads.load(Ordering::SeqCst),
            1,
            "the reload request reached the gateway"
        );

        // The body-returning form is what a command-line caller needs: the app
        // can discard the reply, but the CLI prints `agents_routed` and exits
        // non-zero on a refusal, so `reload` has to read one.
        let reply = reload(&gw.endpoint, None).expect("the reload reply");
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["agents_routed"], 1);
        assert_eq!(gw.reloads.load(Ordering::SeqCst), 2);

        // The request on the wire is a complete, well-formed reload. What it
        // carries *besides* the path is not asserted here: the token comes out
        // of the database this machine happens to have, and
        // `admin_requests_carry_the_token_when_we_have_one` is where that is
        // pinned. This test is about the transport.
        let reload = gw
            .request_starting_with("POST /reload")
            .expect("the reload request was sent");
        assert!(reload.contains("Host: localhost\r\n"), "{reload}");
        assert!(reload.ends_with("\r\n\r\n"), "{reload}");

        assert!(
            request_shutdown(&gw.endpoint),
            "the gateway accepted /shutdown"
        );
        assert!(
            !ping_admin(&gw.endpoint),
            "and stopped serving the endpoint"
        );
    }
}
