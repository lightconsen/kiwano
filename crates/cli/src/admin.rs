//! Minimal loopback HTTP client for the gateway admin plane (:8310).
//!
//! The CLI reuses the same control channel as the Tauri app (tech.md §4.6):
//! `GET /status` for summaries, `POST /reload` after SQLite mutations. Raw
//! TCP keeps the dependency surface at zero — the payload is tiny JSON.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

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

/// `GET /status` — None when the gateway is unreachable.
pub fn get_status(port: u16) -> Option<serde_json::Value> {
    let response = request(
        port,
        &format!("GET /status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    )?;
    body_of(&response)
}

/// `POST /reload` — ask the gateway to rebuild its route table from SQLite.
pub fn post_reload(port: u16) -> Option<serde_json::Value> {
    let response = request(
        port,
        &format!(
            "POST /reload HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ),
    )?;
    body_of(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_port_answers_none() {
        assert!(get_status(1).is_none());
        assert!(post_reload(1).is_none());
    }
}
