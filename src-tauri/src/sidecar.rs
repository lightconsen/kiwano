//! Gateway sidecar lifecycle + admin plane client (tech.md §4.1/§4.6).
//!
//! The GUI spawns `kiwano-gateway` as a child process sharing the same
//! SQLite file; control-channel calls are raw loopback HTTP so no extra
//! HTTP client dependency is needed.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// Resolve the gateway binary: explicit env override, or a sibling of the
/// running GUI binary (cargo workspace shares `target/<profile>/`).
fn gateway_bin_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KIWANO_GATEWAY_BIN") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    std::env::current_exe()
        .ok()?
        .parent()?
        .join("kiwano-gateway")
        .into()
}

/// Spawn the sidecar; the child prints a `ready ...` line on stdout once
/// both planes are bound. Callers need not wait — status pings handle it.
pub fn spawn() -> std::io::Result<Child> {
    let Some(bin) = gateway_bin_path() else {
        return Err(std::io::Error::other(
            "gateway binary not found (set KIWANO_GATEWAY_BIN or build the workspace)",
        ));
    };
    Command::new(&bin).spawn().inspect_err(|e| {
        eprintln!(
            "kiwano: cannot spawn gateway sidecar {}: {e}",
            bin.display()
        );
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

/// Probe one endpoint for protocol support: GET the protocol's models route
/// with its canonical auth headers. A 401/403 still proves the route exists
/// (protocol supported, key missing/invalid); only 404/405 means unsupported.
/// Works without an API key.
/// Async on purpose: commands run on the tokio runtime, and a blocking
/// client (which owns its own runtime) panics when dropped inside one.
pub async fn probe_endpoint(protocol: &str, endpoint: &str, api_key: Option<&str>) -> Result<ProbeReport, String> {
    let url = probe_url(protocol, endpoint)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    match protocol {
        "openai" => {
            if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
                req = req.bearer_auth(k.trim());
            }
        }
        "anthropic" => {
            if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
                req = req.header("x-api-key", k.trim());
            }
            req = req.header("anthropic-version", "2023-06-01");
        }
        "gemini" => {
            if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
                req = req.header("x-goog-api-key", k.trim());
            }
        }
        other => return Err(format!("unknown protocol: {other}")),
    }

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
                ("error".into(), "endpoint returned HTML, not an API".to_string())
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
        assert_eq!(
            count_models(r#"{"data":[{"id":"a"},{"id":"b"}]}"#),
            Some(2)
        );
        assert_eq!(
            count_models(r#"{"models":[{"name":"m1"}]}"#),
            Some(1)
        );
        assert_eq!(count_models(r#"{"error":{}}"#), None);
        assert_eq!(count_models("<html>"), None);
    }
}
