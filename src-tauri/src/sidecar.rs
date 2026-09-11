//! Gateway sidecar lifecycle + admin plane client (tech.md §4.1/§4.6).
//!
//! The GUI spawns `kiwano-gateway` as a child process sharing the same
//! SQLite file; control-channel calls are raw loopback HTTP so no extra
//! HTTP client dependency is needed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

#[cfg(windows)]
const GATEWAY_BIN_NAMES: &[&str] = &["kiwano-gateway.exe", "kiwano-gateway"];
#[cfg(not(windows))]
const GATEWAY_BIN_NAMES: &[&str] = &["kiwano-gateway"];

/// Resolve the gateway binary: explicit env override, then the sibling of the
/// running GUI binary.
///
/// Sibling is where the bundler puts an `externalBin` on every platform —
/// `Contents/MacOS/` in the .app, `usr/bin/` in the deb/rpm and inside the
/// AppImage, beside `kiwano.exe` on Windows — and where cargo leaves it in dev
/// (`target/<profile>/kiwano-gateway`, next to `target/<profile>/kiwano`).
///
/// Deliberately no `PATH` search: a `kiwano-gateway` from somewhere else is a
/// different version answering the same port, and version skew is handled by
/// replacing it (see `startup_action`), not by quietly running it.
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
        "no kiwano-gateway beside the app binary — this bundle is missing its sidecar"
    );
    None
}

/// Spawn the sidecar; the child prints a `ready ...` line on stdout once
/// both planes are bound. Callers need not wait — status pings handle it.
pub fn spawn() -> std::io::Result<Child> {
    let Some(bin) = gateway_bin_path() else {
        return Err(std::io::Error::other(
            "gateway sidecar not found beside the app binary — reinstall Kiwano, \
             or set KIWANO_GATEWAY_BIN to a kiwano-gateway build",
        ));
    };
    Command::new(&bin).spawn().inspect_err(|e| {
        tracing::error!(binary = %bin.display(), error = %e, "cannot spawn gateway sidecar");
    })
}

fn loopback_http(port: u16, request: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT)).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    Some(line)
}

/// `GET /status`, payload included. The liveness check below reads one line by
/// design; this one wants the body, so it reads to EOF (`Connection: close`).
fn loopback_body(port: u16, request: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT)).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = String::new();
    BufReader::new(stream).read_to_string(&mut raw).ok()?;
    // Drop the status line and headers: the body follows the blank line.
    raw.split_once("\r\n\r\n").map(|(_, body)| body.to_string())
}

/// The gateway's own `/status` report. None when it is not answering, or says
/// something we cannot read — the caller then shows less, never a guess.
pub fn gateway_status(admin_port: u16) -> Option<serde_json::Value> {
    let body = loopback_body(
        admin_port,
        &format!(
            "GET /status HTTP/1.1\r\nHost: 127.0.0.1:{admin_port}\r\nConnection: close\r\n\r\n"
        ),
    )?;
    serde_json::from_str(&body).ok()
}

/// `GET /status` — true when the admin plane answers.
pub fn ping_admin(admin_port: u16) -> bool {
    loopback_http(
        admin_port,
        &format!(
            "GET /status HTTP/1.1\r\nHost: 127.0.0.1:{admin_port}\r\nConnection: close\r\n\r\n"
        ),
    )
    .is_some_and(|l| l.contains("200"))
}

/// `POST /shutdown` — ask a running gateway to stop, then wait for it to let go
/// of the admin port. False means it could not be stopped, which the caller
/// must not read as "it is stopped".
///
/// A gateway older than this endpoint answers 404, so this is genuinely a
/// capability probe as much as a request.
pub fn request_shutdown(admin_port: u16) -> bool {
    let accepted = loopback_http(
        admin_port,
        &format!(
            "POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1:{admin_port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ),
    )
    .is_some_and(|line| line.contains("200"));
    if !accepted {
        return false;
    }
    // The reply is written before the serve loop returns, but the listener is
    // not necessarily closed the instant the status line arrives — and the
    // replacement has to bind the same port.
    for _ in 0..50 {
        if !ping_admin(admin_port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// What to do about a gateway that may already be answering the admin port.
///
/// Factored out because the mismatch branch cannot be reached by hand without
/// an old binary, and it is the branch that decides whether the app runs beside
/// a daemon that disagrees with it about the database schema.
#[derive(Debug, PartialEq, Eq)]
pub enum StartupAction {
    /// Same version — leave it alone and adopt it.
    Adopt,
    /// Ours, wrong version — stop it and start the bundled one.
    Restart,
    /// Nothing there, or something that is not a kiwano-gateway.
    Spawn,
}

pub fn startup_action(status: Option<&serde_json::Value>, ours: &str) -> StartupAction {
    let Some(status) = status else {
        return StartupAction::Spawn;
    };
    // The port may be held by something else entirely. Not ours to stop; the
    // spawn that follows will fail to bind, and say so.
    if status.get("name").and_then(|n| n.as_str()) != Some("kiwano-gateway") {
        tracing::error!(
            "the admin port is held by something that is not kiwano-gateway; leaving it alone"
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

/// Stop the gateway on `admin_port` and start the bundled one in its place.
pub fn restart(admin_port: u16) -> std::io::Result<Child> {
    if !request_shutdown(admin_port) && !force_stop(admin_port) {
        return Err(std::io::Error::other(
            "the running gateway would not stop; not starting a second one on the same ports",
        ));
    }
    spawn()
}

/// Last resort for a gateway predating `POST /shutdown`.
///
/// No *shipped* build before this one contains a gateway at all, so the only
/// machines that reach here are ones where someone built the workspace by hand
/// — a stale `target/debug/kiwano-gateway` left over from `cargo test`. That is
/// common enough in development to be worth handling, and the `lsof`/`kill`
/// pair needs no new dependency. Unix only: the gateway watches SIGTERM there
/// and ctrl-C alone on Windows.
#[cfg(unix)]
fn force_stop(admin_port: u16) -> bool {
    let out = match Command::new("lsof")
        .args(["-nP", "-t", &format!("-iTCP:{admin_port}"), "-sTCP:LISTEN"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "lsof unavailable; cannot stop a gateway without /shutdown");
            return false;
        }
    };
    let pids: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if pids.is_empty() {
        return true; // nothing listening any more
    }
    tracing::warn!(
        ?pids,
        port = admin_port,
        "gateway predates /shutdown; sending SIGTERM"
    );
    for pid in &pids {
        let _ = Command::new("kill").arg(pid).status();
    }
    for _ in 0..30 {
        if !ping_admin(admin_port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::error!(
        ?pids,
        "gateway ignored SIGTERM; stop it by hand (pkill -f kiwano-gateway)"
    );
    false
}

#[cfg(not(unix))]
fn force_stop(_admin_port: u16) -> bool {
    tracing::error!("a running gateway predates /shutdown and cannot be stopped from here");
    false
}

/// `POST /reload` — ask the gateway to rebuild its route table from SQLite.
/// Fire-and-forget: route hot-reload failures surface in gateway logs.
pub fn notify_reload(admin_port: u16) {
    let _ = loopback_http(
        admin_port,
        &format!("POST /reload HTTP/1.1\r\nHost: 127.0.0.1:{admin_port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    );
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

    #[test]
    fn ping_admin_refuses_dead_port() {
        assert!(!ping_admin(1));
    }

    #[test]
    fn reload_on_dead_port_is_silent() {
        // must not panic
        notify_reload(1);
    }

    /// The mismatch branch is the one that cannot be reached by hand without an
    /// old binary, so it is pinned here rather than left to the dev machine.
    #[test]
    fn startup_action_adopts_only_its_own_version() {
        let gw = |version: serde_json::Value| serde_json::json!({ "ok": true, "name": "kiwano-gateway", "version": version });

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
    fn shutdown_on_dead_port_reports_failure() {
        // No gateway to stop is not the same as having stopped one.
        assert!(!request_shutdown(1));
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

    /// Re-exec'd by the test below as a stand-in for a gateway built before
    /// `POST /shutdown` existed: it answers `GET /status` and 404s everything
    /// else, so `request_shutdown` cannot stop it and `force_stop` has to.
    ///
    /// Its own process on purpose — `force_stop` finds its target through
    /// `lsof`, and a listener inside the test runner would name the runner.
    /// Returns immediately in an ordinary run, where the env var is unset.
    #[test]
    fn stub_pre_shutdown_gateway() {
        use std::net::TcpListener;

        let Ok(port) = std::env::var("KIWANO_TEST_STUB_PORT") else {
            return;
        };
        let listener = TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let (code, reason, body) = if String::from_utf8_lossy(&buf).starts_with("GET /status") {
                (
                    200,
                    "OK",
                    r#"{"ok":true,"name":"kiwano-gateway","version":"0.0.1"}"#,
                )
            } else {
                (404, "Not Found", r#"{"ok":false}"#)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    }

    /// Kills the stub even when an assertion panics.
    struct StubGuard(std::process::Child);
    impl Drop for StubGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// The fallback that lets a 0.1.8 app replace a gateway that predates
    /// `POST /shutdown`. Reachable in practice only against a binary someone
    /// built by hand, which is exactly the kind of path that otherwise ships
    /// unexecuted.
    #[cfg(unix)]
    #[test]
    fn force_stop_stops_a_gateway_that_cannot_be_asked() {
        use std::net::TcpListener;

        // Have the OS hand us a port nothing else holds, then let it go for the
        // stub. `force_stop` signals whatever `lsof` names on this port, so it
        // has to be one we know is ours. If the stub fails to take it, the
        // "is it up" check below fails and nothing is signalled.
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("probe bind")
            .local_addr()
            .expect("probe addr")
            .port();

        let guard = StubGuard(
            Command::new(std::env::current_exe().expect("test binary"))
                .args(["--exact", "sidecar::tests::stub_pre_shutdown_gateway"])
                .env("KIWANO_TEST_STUB_PORT", port.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn stub"),
        );

        // Check the version, not just liveness: a stranger that happened to
        // take the port must not be mistaken for our stub — and then killed.
        let mut up = false;
        for _ in 0..100 {
            let ours = gateway_status(port)
                .and_then(|s| {
                    let v = s.get("version")?.as_str()?.to_string();
                    Some(v)
                })
                .is_some_and(|v| v == "0.0.1");
            if ours {
                up = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(up, "stub never came up on {port}");

        // The pre-0.1.8 shape: reachable, but no way to ask it to stop.
        assert!(
            !request_shutdown(port),
            "a gateway without /shutdown cannot be asked"
        );
        assert!(
            ping_admin(port),
            "and it is still running after being asked"
        );

        assert!(force_stop(port), "force_stop should stop it");
        assert!(!ping_admin(port), "and release the port");

        drop(guard);
    }
}
