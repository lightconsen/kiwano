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
}
