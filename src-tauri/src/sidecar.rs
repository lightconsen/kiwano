//! Gateway sidecar lifecycle + admin plane client (tech.md §4.1/§4.6).
//!
//! The GUI spawns `kiwano-gateway` as a child process sharing the same
//! SQLite file. The admin plane is a local IPC endpoint — a unix domain socket,
//! or a per-user named pipe on Windows — not a TCP port, so the control-channel
//! calls are raw HTTP written to that endpoint. Both ends of that transport live
//! in `kiwano_gateway::server::admin_ipc`, which this module re-exports
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
//! # The transport change of 0.1.8, and the TCP window around it
//!
//! A gateway built *before* the admin plane moved to IPC listens on loopback TCP
//! `:8310` (or whatever `KIWANO_ADMIN_PORT` named). An upgrade has to be able to
//! recognise and stop one: the daemon outlives the GUI, so the app meets its own
//! predecessor still holding the data port, and a new gateway spawned beside it
//! would fail to bind and exit. So the *startup* path — [`running_gateway_status`]
//! and [`restart`] — tries the IPC endpoint first and falls back to that port,
//! and [`notify_reload`] does the same so an old gateway the app could not
//! replace still picks up route changes. Nothing else consults it; the
//! steady-state admin client is IPC-only.
//!
//! The whole fallback is deletable, and should be deleted once no pre-0.1.8
//! gateway can still be running. No *shipped* release before 0.1.8 contained a
//! gateway binary at all, so the population it reaches is hand-built workspace
//! binaries and a developer's stale `target/debug/kiwano-gateway` — which is
//! exactly why it is worth keeping for one release and not longer. The release
//! after 0.1.8 is the natural point: delete [`legacy_admin_port`],
//! `tcp_gateway_status`, `tcp_ping`, `legacy_request_shutdown`, the legacy
//! branches in [`running_gateway_status`] / [`restart`] / [`notify_reload`], and
//! the tests that cover them.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use kiwano_gateway::server::{ADMIN_TOKEN_HEADER, ADMIN_TOKEN_KEY};
use kiwano_gateway::store::Store;

/// The endpoint type itself, so callers in this crate name
/// `sidecar::AdminEndpoint` and do not need to know the gateway crate's module
/// layout.
pub use kiwano_gateway::server::AdminEndpoint;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// The `Host` header for a request over the IPC endpoint. Its value is
/// arbitrary — the admin router does no `Host` filtering — but HTTP/1.1
/// requires the header to be there.
const IPC_HOST: &str = "localhost";

/// The port a *pre-0.1.8* gateway listens on when `KIWANO_ADMIN_PORT` says
/// nothing else. It was the gateway's own default before the plane moved off
/// TCP, and it is the port the old binary will still have chosen.
const DEFAULT_LEGACY_ADMIN_PORT: u16 = 8310;

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

/// The SQLite file this app and the gateway share. Resolution matches the
/// gateway's `db_path_from_env` and the CLI's `--db` default: `KIWANO_DB_PATH`,
/// else `~/.kiwano/kiwano.db`. The app sets no environment for the child, so
/// the spawned gateway inherits `KIWANO_DB_PATH` and the two agree by
/// construction.
fn shared_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("KIWANO_DB_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".kiwano").join("kiwano.db")
}

/// The admin plane endpoint this app and its gateway share.
///
/// Resolved exactly the way the gateway resolves it — the same function, on the
/// same database path — so the two cannot disagree, and `KIWANO_ADMIN_SOCKET`
/// moves both at once. The child needs no argument for it: it inherits this
/// process's environment and derives the same default from the same file
/// location. `KIWANO_ADMIN_PORT` plays no part here; it survives only as the
/// port a legacy gateway is looked for on.
pub fn admin_endpoint() -> AdminEndpoint {
    AdminEndpoint::from_env(&shared_db_path())
}

/// The admin token, read out of the row the gateway minted it into.
///
/// `None` when there is no database yet or the row is not there: on a fresh
/// install the app is up before the gateway has ever run. Deliberately never
/// creates the file — an empty database conjured up by a token lookup would
/// then be migrated by whichever process got there first.
fn read_admin_token(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    Store::open(path)
        .ok()?
        .app_setting(ADMIN_TOKEN_KEY)
        .filter(|t| !t.trim().is_empty())
}

/// [`read_admin_token`] against the shared database, cached once found.
///
/// The row does not change under a running gateway, so the first success is
/// kept; a miss is retried, because the gateway may simply not have started
/// yet. Two opens at most, in the ordinary case where the app starts first.
fn admin_token() -> Option<String> {
    static CACHED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(token) = CACHED.get() {
        return Some(token.clone());
    }
    let token = read_admin_token(&shared_db_path())?;
    let _ = CACHED.set(token.clone());
    Some(token)
}

/// The token header for one raw request, or nothing when there is no token to
/// send. A gateway that predates the token ignores the header; a caller that
/// has none is answered as an older gateway would answer (liveness-only
/// `/status`, 401 from `/reload`), which is why this is not an error.
fn token_header(token: Option<&str>) -> String {
    match token {
        Some(token) => format!("{ADMIN_TOKEN_HEADER}: {token}\r\n"),
        None => String::new(),
    }
}

/// The three admin requests, built for whichever transport is asking. `host` is
/// [`IPC_HOST`] over the endpoint and `127.0.0.1:{port}` over the legacy port;
/// everything else about the request is transport-independent, which is the
/// point of keeping the HTTP framing over a socket.
fn status_request(host: &str, token: Option<&str>) -> String {
    format!(
        "GET /status HTTP/1.1\r\nHost: {host}\r\n{}Connection: close\r\n\r\n",
        token_header(token)
    )
}

fn shutdown_request(host: &str, token: Option<&str>) -> String {
    format!(
        "POST /shutdown HTTP/1.1\r\nHost: {host}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        token_header(token)
    )
}

fn reload_request(host: &str, token: Option<&str>) -> String {
    format!(
        "POST /reload HTTP/1.1\r\nHost: {host}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        token_header(token)
    )
}

/// The host header for a request over the legacy loopback port.
fn loopback_host(port: u16) -> String {
    format!("127.0.0.1:{port}")
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

/// The gateway's own `/status` report, off the admin endpoint. None when it is
/// not answering, or says something we cannot read — the caller then shows less,
/// never a guess.
///
/// The token, when there is one, buys the full report; without it the gateway
/// answers with the liveness subset (identity, version, uptime), which the
/// status chip renders as a gateway that is up but not described.
pub fn gateway_status(endpoint: &AdminEndpoint) -> Option<serde_json::Value> {
    let body = endpoint_body(
        endpoint,
        &status_request(IPC_HOST, admin_token().as_deref()),
    )?;
    serde_json::from_str(&body).ok()
}

/// The gateway's `/status`, IPC first and the legacy TCP port second.
///
/// This is the *startup* question — "is a gateway of ours already running, and
/// which version?" — and the only place the legacy port is consulted besides
/// [`restart`] and [`notify_reload`]. Both answers are the same JSON, so the
/// caller does not have to know which side of the transport change it met; a
/// legacy gateway is never adopted (its version differs by definition), it is
/// replaced. See the module doc for when this fallback can be deleted.
pub fn running_gateway_status(endpoint: &AdminEndpoint) -> Option<serde_json::Value> {
    if let Some(status) = gateway_status(endpoint) {
        return Some(status);
    }
    let port = legacy_admin_port()?;
    let status = tcp_gateway_status(port);
    if status.is_some() {
        tracing::warn!(
            port,
            "a pre-0.1.8 gateway is answering on TCP; it will be replaced"
        );
    }
    status
}

/// `GET /status` off the legacy loopback port. Split out from
/// [`running_gateway_status`] so the transition has a test that binds its own
/// port instead of depending on what happens to hold `:8310` on the machine.
fn tcp_gateway_status(port: u16) -> Option<serde_json::Value> {
    let body = loopback_body(
        port,
        &status_request(&loopback_host(port), admin_token().as_deref()),
    )?;
    serde_json::from_str(&body).ok()
}

/// `GET /status` — true when the admin plane answers.
///
/// Deliberately sends no token. `/status` answers 200 to an unauthenticated
/// caller by design (see `server::admin`), and this is the probe the watchdog
/// runs every few seconds: it must be able to tell "a gateway is here" from
/// "nothing is here" without a readable database, and without depending on a
/// row that only exists once a token-aware gateway has started.
///
/// IPC-only on purpose, unlike the startup path: the watchdog ticks every five
/// seconds, and a legacy gateway can only survive startup if replacing it
/// failed. Answering "alive" for one here would freeze the version skew in place
/// for the rest of the session; leaving it to the respawn path is louder, and
/// wrong-version-forever is worse than noisy.
pub fn ping_admin(endpoint: &AdminEndpoint) -> bool {
    endpoint_http(endpoint, &status_request(IPC_HOST, None))
        .is_some_and(|line| line.contains("200"))
}

/// `GET /status` off the legacy port — [`ping_admin`]'s TCP twin. Exists so the
/// legacy shutdown below can watch for the port being released.
fn tcp_ping(port: u16) -> bool {
    loopback_http(port, &status_request(&loopback_host(port), None))
        .is_some_and(|line| line.contains("200"))
}

/// `POST /shutdown` — ask a running gateway to stop, then wait for it to let go
/// of the endpoint. False means it could not be stopped, which the caller must
/// not read as "it is stopped".
///
/// A gateway that refuses the token (401 — what an unreadable `app_settings` row
/// looks like) lands in the same `false`, as does one that does not answer at
/// all. [`restart`] treats all of them the same way, and then tries the legacy
/// port before giving up.
pub fn request_shutdown(endpoint: &AdminEndpoint) -> bool {
    let accepted = endpoint_http(
        endpoint,
        &shutdown_request(IPC_HOST, admin_token().as_deref()),
    )
    .is_some_and(|line| line.contains("200"));
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

/// `POST /shutdown` on the legacy loopback port, and wait for the port to be
/// let go. [`request_shutdown`] over TCP, for the startup path only.
///
/// A pre-0.1.8 gateway predating `/shutdown` answers 404 here and this reports
/// `false`; there is no signal-based fallback any more, and `restart` explains
/// why. `None` (no legacy port to look for) is also `false`.
fn legacy_request_shutdown(port: Option<u16>) -> bool {
    let Some(port) = port else {
        return false;
    };
    let accepted = loopback_http(
        port,
        &shutdown_request(&loopback_host(port), admin_token().as_deref()),
    )
    .is_some_and(|line| line.contains("200"));
    if !accepted {
        return false;
    }
    for _ in 0..50 {
        if !tcp_ping(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// The port a pre-0.1.8 gateway is looked for on, from `KIWANO_ADMIN_PORT`.
///
/// This env var no longer names anything the admin plane listens on — the plane
/// is not a port any more, and `KIWANO_ADMIN_SOCKET` is what moves it. It is
/// read for one purpose only: to find the old gateway during the transition
/// described in the module doc, for the app whose operator had pointed
/// `KIWANO_ADMIN_PORT` somewhere non-default. `None` means "do not look for a
/// legacy gateway at all" — a value that is not a port.
fn legacy_admin_port() -> Option<u16> {
    legacy_admin_port_from(std::env::var("KIWANO_ADMIN_PORT").ok().as_deref())
}

/// [`legacy_admin_port`] with the environment passed in, so the parse is
/// testable without setting a process-wide variable.
fn legacy_admin_port_from(value: Option<&str>) -> Option<u16> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        // Unset is the normal case, and it means the default port: the old
        // gateway chose 8310 itself when the operator said nothing.
        return Some(DEFAULT_LEGACY_ADMIN_PORT);
    };
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Some(port),
        _ => {
            tracing::warn!(
                value,
                default = DEFAULT_LEGACY_ADMIN_PORT,
                "invalid KIWANO_ADMIN_PORT; not looking for a legacy gateway"
            );
            None
        }
    }
}

/// What to do about a gateway that may already be serving the admin plane.
///
/// Factored out because the mismatch branch cannot be reached by hand without
/// an old binary, and it is the branch that decides whether the app runs beside
/// a daemon that disagrees with it about the database schema.
///
/// The decision reads `name` + `version` out of `GET /status`, and those are in
/// the liveness subset the gateway returns to an unauthenticated caller — so
/// this works across the transport change in both directions. Meeting a
/// pre-0.1.8 gateway (over TCP, see [`running_gateway_status`]) there is no
/// token row to send and the old build ignores the header regardless; meeting a
/// token-aware one whose row cannot be read, the subset still arrives. The
/// replace path then gets `/shutdown` — over IPC for a new gateway, over TCP for
/// an old one — see [`restart`].
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
    // The endpoint may be held by something else entirely — a pipe or socket
    // another program happens to own, or a stranger on the legacy port. Not ours
    // to stop; the spawn that follows will fail to bind, and say so.
    if status.get("name").and_then(|n| n.as_str()) != Some("kiwano-gateway") {
        tracing::error!(
            "the admin endpoint is held by something that is not kiwano-gateway; leaving it alone"
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
/// `/shutdown` over IPC is the normal path, and the one that matters for every
/// gateway this release ships. The legacy TCP call after it covers an upgrade
/// meeting a pre-0.1.8 gateway, which has no endpoint to be asked on.
///
/// # Why there is no signal-based fallback any more
///
/// There used to be a `force_stop`, which found the gateway through
/// `lsof -iTCP:<port> -sTCP:LISTEN` and sent it SIGTERM. It existed for a
/// gateway built before `POST /shutdown` existed. Three things made it the wrong
/// thing to keep:
///
/// - The population it served shrank to almost nothing. No shipped release
///   before 0.1.8 contained a gateway binary at all, so the only gateway that
///   can neither be asked over IPC nor over TCP is one someone built from the
///   workspace by hand — a stale `target/debug/kiwano-gateway`. The legacy TCP
///   `/shutdown` above covers the realistic upgrade case.
/// - The rework it would have taken — `lsof -U` on the socket path — is strictly
///   worse than the TCP form it replaces. `-U` matches *every* unix socket the
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
    if !request_shutdown(endpoint) && !legacy_request_shutdown(legacy_admin_port()) {
        tracing::error!(
            endpoint = %endpoint.describe(),
            "the running gateway would not stop; stop it by hand (pkill -f kiwano-gateway)"
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
///
/// Falls back to the legacy port when the endpoint does not answer at all: a
/// pre-0.1.8 gateway this app could not replace is still routing the operator's
/// traffic, and a mutation that does not reach it is a mutation the user cannot
/// see taking effect. Only a *silent* endpoint falls through — an answer we do
/// not like is still an answer.
pub fn notify_reload(endpoint: &AdminEndpoint) {
    if endpoint_http(
        endpoint,
        &reload_request(IPC_HOST, admin_token().as_deref()),
    )
    .is_some()
    {
        return;
    }
    let Some(port) = legacy_admin_port() else {
        return;
    };
    let _ = loopback_http(
        port,
        &reload_request(&loopback_host(port), admin_token().as_deref()),
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
    /// when the app has none (a fresh install, or an old gateway's database),
    /// rather than dropping the header line and the blank line with it.
    #[test]
    fn admin_requests_carry_the_token_when_we_have_one() {
        let header = format!("{ADMIN_TOKEN_HEADER}: tok-123\r\n");

        let status = status_request(IPC_HOST, Some("tok-123"));
        assert!(status.starts_with("GET /status HTTP/1.1\r\n"));
        assert!(status.contains(&header));
        assert!(status.ends_with("Connection: close\r\n\r\n"));

        let shutdown = shutdown_request(IPC_HOST, Some("tok-123"));
        assert!(shutdown.starts_with("POST /shutdown HTTP/1.1\r\n"));
        assert!(shutdown.contains(&header));

        let reload = reload_request(IPC_HOST, Some("tok-123"));
        assert!(reload.starts_with("POST /reload HTTP/1.1\r\n"));
        assert!(reload.contains(&header));

        // No token: same requests, no header — a gateway that has no token to
        // check them against is exactly the one that does not need them.
        for request in [
            status_request(IPC_HOST, None),
            shutdown_request(IPC_HOST, None),
            reload_request(IPC_HOST, None),
        ] {
            assert!(!request.contains(ADMIN_TOKEN_HEADER));
            assert!(request.ends_with("\r\n\r\n"));
        }

        // The transport is the only difference: the same three requests over the
        // legacy port carry the same bytes apart from the `Host` header. HTTP/1.1
        // requires that header, and what is in it is all the router ignores.
        let over_tcp = status_request(&loopback_host(8310), Some("tok-123"));
        assert!(over_tcp.contains("Host: 127.0.0.1:8310\r\n"));
        assert!(over_tcp.contains(&header));
        assert_eq!(
            over_tcp.replace("Host: 127.0.0.1:8310", "Host: localhost"),
            status,
            "the request is transport-independent apart from its Host"
        );
    }

    /// The token comes out of the row the gateway writes, in the database the
    /// two processes share. A database without one — or without a file — is
    /// None, not an error and not a new empty database.
    #[test]
    fn admin_token_reads_the_gateway_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(read_admin_token(&path), None);
        store.set_app_setting(ADMIN_TOKEN_KEY, "tok-abc").unwrap();
        drop(store);
        assert_eq!(read_admin_token(&path), Some("tok-abc".to_string()));

        let missing = dir.path().join("not-yet.db");
        assert_eq!(read_admin_token(&missing), None);
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

        // `running_gateway_status` also probes the legacy port, which on a
        // developer's machine may well be a real gateway — so this proves only
        // that a dead endpoint does not make it panic.
        let _ = running_gateway_status(&endpoint);
    }

    /// The legacy port comes from `KIWANO_ADMIN_PORT` when the operator set one
    /// — that is the whole of what the variable still means — and defaults to the
    /// port the pre-0.1.8 gateway chose for itself when they did not. A value
    /// that is not a port means "do not go looking at all".
    #[test]
    fn the_legacy_port_is_read_from_the_environment() {
        assert_eq!(legacy_admin_port_from(None), Some(8310));
        assert_eq!(legacy_admin_port_from(Some("  ")), Some(8310));
        assert_eq!(legacy_admin_port_from(Some("9000")), Some(9000));
        assert_eq!(legacy_admin_port_from(Some(" 9000 ")), Some(9000));
        assert_eq!(legacy_admin_port_from(Some("not-a-port")), None);
        // Port 0 is "any port the OS likes", never something an operator
        // pointed a gateway at.
        assert_eq!(legacy_admin_port_from(Some("0")), None);
    }

    /// `restart` must not start a second gateway next to one it failed to stop —
    /// the two would fight over the data port. With nothing to stop it has to
    /// return an error and spawn nothing.
    ///
    /// Skipped when the legacy port really is answering: that would be a real
    /// gateway of somebody's, and this test would be shutting it down.
    #[test]
    fn restart_refuses_when_nothing_can_be_stopped() {
        if legacy_admin_port().is_some_and(tcp_ping) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let err = restart(&dead_endpoint(dir.path()))
            .expect_err("nothing was stopped, so nothing may be started");
        assert!(err.to_string().contains("would not stop"), "{err}");
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
    /// of this transport is exercised by `kiwano-gateway`'s own
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
                            r#"{{"ok":true,"name":"kiwano-gateway","version":"{version}","uptime_secs":1}}"#
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
    /// This is the coverage the re-exec'd TCP stub used to give the three calls,
    /// now over the transport the plane actually uses.
    #[cfg(unix)]
    #[test]
    fn the_admin_client_talks_to_a_live_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let gw = LiveGateway::start(dir.path(), "0.1.8");

        assert!(ping_admin(&gw.endpoint), "the liveness probe answers");

        let status = gateway_status(&gw.endpoint).expect("the status report");
        assert_eq!(status["name"], "kiwano-gateway");
        assert_eq!(status["version"], "0.1.8");

        // `running_gateway_status` asks the endpoint first, so a live one is
        // found there and the legacy port is never consulted.
        let running = running_gateway_status(&gw.endpoint).expect("the startup probe");
        assert_eq!(running["version"], "0.1.8");

        notify_reload(&gw.endpoint);
        assert_eq!(
            gw.reloads.load(Ordering::SeqCst),
            1,
            "the reload request reached the gateway"
        );

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

    /// A stand-in for a *pre-0.1.8* gateway: the same routes, but on loopback
    /// TCP. The legacy half of the transition, without depending on what happens
    /// to hold `:8310` on this machine — the OS hands us the port.
    struct LegacyGateway {
        port: u16,
    }

    impl LegacyGateway {
        fn start(version: &str) -> LegacyGateway {
            use std::net::TcpListener;

            let listener = TcpListener::bind("127.0.0.1:0").expect("bind the legacy stub");
            let port = listener.local_addr().expect("stub addr").port();
            let version = version.to_string();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let shutdown = request.starts_with("POST /shutdown");
                    let body = if shutdown {
                        r#"{"ok":true}"#.to_string()
                    } else {
                        format!(r#"{{"ok":true,"name":"kiwano-gateway","version":"{version}"}}"#)
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
            LegacyGateway { port }
        }
    }

    /// The upgrade half of the transport change: a gateway from before the IPC
    /// move is found on its loopback port, is still asked to stop the same way,
    /// and lets go of the port when it does.
    #[test]
    fn a_legacy_gateway_is_found_and_stopped_over_tcp() {
        let gw = LegacyGateway::start("0.1.7");

        assert!(tcp_ping(gw.port), "the legacy port answers");
        let status = tcp_gateway_status(gw.port).expect("the legacy status report");
        assert_eq!(status["name"], "kiwano-gateway");
        assert_eq!(status["version"], "0.1.7");

        assert!(
            legacy_request_shutdown(Some(gw.port)),
            "a legacy gateway is stopped over TCP"
        );
        assert!(!tcp_ping(gw.port), "and lets go of its port");
    }

    /// No legacy port to look at is not a gateway that stopped.
    #[test]
    fn a_legacy_shutdown_without_a_port_reports_failure() {
        assert!(!legacy_request_shutdown(None));
    }
}
