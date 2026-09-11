//! Minimal HTTP client for the gateway admin plane (a unix socket, or a
//! per-user named pipe on Windows).
//!
//! The CLI reuses the same control channel as the Tauri app (tech.md §4.6):
//! `GET /status` for summaries, `POST /reload` after SQLite mutations. The
//! transport itself lives in `kiwano_gateway::server::admin_ipc` — the same
//! [`AdminEndpoint`] the gateway binds — so the CLI cannot disagree with the
//! gateway about where the plane is, and the dependency surface stays at zero:
//! the requests are raw bytes and the payload is tiny JSON.
//!
//! The plane is deliberately *not* a loopback port, so
//! `curl http://127.0.0.1:8310/status` no longer reaches it and that is the
//! transport rather than a broken gateway. On unix the equivalent is
//! `curl --unix-socket ~/.kiwano/admin.sock http://localhost/status`; on both
//! platforms `kiwano-cli status` goes through this module.
//!
//! `/reload` is refused without the admin token the gateway minted for itself,
//! so callers pass the one they read out of the shared database; `/status`
//! answers a liveness-only body without it.

use std::io::{BufReader, Read, Write};
use std::time::Duration;

use kiwano_gateway::server::admin_ipc::AdminEndpoint;
use kiwano_gateway::server::ADMIN_TOKEN_HEADER;

const TIMEOUT: Duration = Duration::from_millis(1500);

/// The `Host` header. Arbitrary in value — the admin router does no `Host`
/// filtering — but HTTP/1.1 requires one.
const HOST: &str = "localhost";

fn request(endpoint: &AdminEndpoint, req: &str) -> Option<String> {
    let mut stream = endpoint.connect(TIMEOUT).ok()?;
    stream.write_all(req.as_bytes()).ok()?;
    let mut response = String::new();
    BufReader::new(stream).read_to_string(&mut response).ok()?;
    Some(response)
}

fn body_of(response: &str) -> Option<serde_json::Value> {
    let body = response.split("\r\n\r\n").nth(1)?;
    serde_json::from_str(body).ok()
}

fn token_header(token: Option<&str>) -> String {
    match token {
        Some(token) => format!("{ADMIN_TOKEN_HEADER}: {token}\r\n"),
        None => String::new(),
    }
}

fn get_request(token: Option<&str>) -> String {
    format!(
        "GET /status HTTP/1.1\r\nHost: {HOST}\r\n{}Connection: close\r\n\r\n",
        token_header(token)
    )
}

fn post_request(path: &str, token: Option<&str>) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: {HOST}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        token_header(token)
    )
}

/// `GET /status` — None when the gateway is unreachable. Without a token the
/// gateway still answers: identity, version and uptime, but no route table.
pub fn get_status(endpoint: &AdminEndpoint, token: Option<&str>) -> Option<serde_json::Value> {
    body_of(&request(endpoint, &get_request(token))?)
}

/// `POST /reload` — ask the gateway to rebuild its route table from SQLite.
/// A refusal comes back as `{"ok": false, "error": …}`, not as `None`; the
/// caller checks `ok` (see `cmd_reload`).
pub fn post_reload(endpoint: &AdminEndpoint, token: Option<&str>) -> Option<serde_json::Value> {
    body_of(&request(endpoint, &post_request("/reload", token))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An endpoint nothing is serving, unique to this test. `parse` rather than
    /// `beside_db`: on Windows the latter is the real per-user pipe, which the
    /// gateway this developer is running may well be holding.
    fn dead_endpoint(dir: &std::path::Path) -> AdminEndpoint {
        let unique = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "endpoint".to_string());
        AdminEndpoint::parse(&format!("kiwano-test-dead-{unique}"))
    }

    #[test]
    fn a_dead_endpoint_answers_none() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = dead_endpoint(dir.path());
        assert!(get_status(&endpoint, None).is_none());
        assert!(post_reload(&endpoint, None).is_none());
    }

    /// `/reload` is refused without the token, so the header has to be on the
    /// wire — and a tokenless call must still be a complete request.
    #[test]
    fn requests_carry_the_token_when_there_is_one() {
        let header = format!("{ADMIN_TOKEN_HEADER}: tok-123\r\n");

        let with = post_request("/reload", Some("tok-123"));
        assert!(with.starts_with("POST /reload HTTP/1.1\r\n"));
        assert!(with.contains(&header));

        let status = get_request(Some("tok-123"));
        assert!(status.starts_with("GET /status HTTP/1.1\r\n"));
        assert!(status.contains(&header));

        for request in [post_request("/reload", None), get_request(None)] {
            assert!(!request.contains(ADMIN_TOKEN_HEADER));
            assert!(request.ends_with("\r\n\r\n"));
        }
    }
}
