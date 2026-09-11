//! Minimal loopback HTTP client for the gateway admin plane (:8310).
//!
//! The CLI reuses the same control channel as the Tauri app (tech.md §4.6):
//! `GET /status` for summaries, `POST /reload` after SQLite mutations. Raw
//! TCP keeps the dependency surface at zero — the payload is tiny JSON.
//!
//! `/reload` is refused without the admin token the gateway minted for itself,
//! so callers pass the one they read out of the shared database; `/status`
//! answers a liveness-only body without it.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use kiwano_gateway::server::ADMIN_TOKEN_HEADER;

const TIMEOUT: Duration = Duration::from_millis(1500);

fn request(port: u16, req: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
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

fn get_request(port: u16, token: Option<&str>) -> String {
    format!(
        "GET /status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{}Connection: close\r\n\r\n",
        token_header(token)
    )
}

fn post_request(port: u16, path: &str, token: Option<&str>) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        token_header(token)
    )
}

/// `GET /status` — None when the gateway is unreachable. Without a token the
/// gateway still answers: identity, version and uptime, but no route table.
pub fn get_status(port: u16, token: Option<&str>) -> Option<serde_json::Value> {
    body_of(&request(port, &get_request(port, token))?)
}

/// `POST /reload` — ask the gateway to rebuild its route table from SQLite.
/// A refusal comes back as `{"ok": false, "error": …}`, not as `None`; the
/// caller checks `ok` (see `cmd_reload`).
pub fn post_reload(port: u16, token: Option<&str>) -> Option<serde_json::Value> {
    body_of(&request(port, &post_request(port, "/reload", token))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_port_answers_none() {
        assert!(get_status(1, None).is_none());
        assert!(post_reload(1, None).is_none());
    }

    /// `/reload` is refused without the token, so the header has to be on the
    /// wire — and a tokenless call must still be a complete request.
    #[test]
    fn requests_carry_the_token_when_there_is_one() {
        let header = format!("{ADMIN_TOKEN_HEADER}: tok-123\r\n");

        let with = post_request(8310, "/reload", Some("tok-123"));
        assert!(with.starts_with("POST /reload HTTP/1.1\r\n"));
        assert!(with.contains(&header));

        let status = get_request(8310, Some("tok-123"));
        assert!(status.starts_with("GET /status HTTP/1.1\r\n"));
        assert!(status.contains(&header));

        for request in [post_request(8310, "/reload", None), get_request(8310, None)] {
            assert!(!request.contains(ADMIN_TOKEN_HEADER));
            assert!(request.ends_with("\r\n\r\n"));
        }
    }
}
