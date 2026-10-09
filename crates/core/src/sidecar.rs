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
//! `/reload`, `/shutdown` and `/events` need the admin token the gateway minted
//! for itself ([`ADMIN_TOKEN_KEY`]); it is read from that same SQLite file, so
//! the two processes agree without any new IPC. The one exception is the
//! liveness probe — see [`ping_admin`].
//!
//! The admin plane was a loopback TCP port before 0.1.8, and this module carried
//! a fallback that looked for — and stopped — a gateway from before that move.
//! It has been removed: no released build ever shipped a gateway on that
//! transport, so the only thing left to find was a hand-built binary on a
//! developer's machine, and the app now speaks IPC alone.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use kiwano_adapters::config::DbPath;
use kiwanod::server::{ADMIN_TOKEN_HEADER, ADMIN_TOKEN_KEY};
use kiwanod::store::Store;

/// The endpoint type itself, so callers in this crate name
/// `sidecar::AdminEndpoint` and do not need to know the gateway crate's module
/// layout.
pub use kiwanod::server::AdminEndpoint;

// The probes moved to the daemon (`kiwanod::api::probe`) — they are network I/O,
// and the health table they write to is the daemon's. Re-exported so the
// `sidecar::probe_*` paths that call them keep resolving.
pub use kiwanod::api::probe::{
    fetch_model_names, measure_latency, probe_endpoint, probe_prompt, ProbeReport, PromptProbe,
};

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

/// The OS null device, for a stream nobody is listening to. Windows only —
/// see `spawn` for why sending a console child's streams here is not a loss.
#[cfg(windows)]
fn null_device() -> std::fs::File {
    std::fs::OpenOptions::new()
        .write(true)
        .open("NUL")
        .expect("the Windows null device opens")
}

/// The command that starts the sidecar, with the database it must open handed
/// to it explicitly.
///
/// The daemon resolves `KIWANO_DB_PATH` itself, and so does this process — the
/// resolver is shared precisely so the two agree (`kiwano_adapters::config::
/// kiwano_db_path`). But "agree on a *rule*" is not "agree on a *value*": the
/// fallback is `$HOME/.kiwano/kiwano.db`, and a child that re-resolves it can
/// land somewhere else the moment its `$HOME` differs — which is exactly the
/// position a service manager launches it from, and how one install ends up
/// with two databases. Passing the resolved path removes the child's reason to
/// look, so the sidecar case cannot diverge at all.
///
/// The port is deliberately *not* passed: it comes from the environment this
/// process already inherited, so the child reads the same value by the same
/// route. The database is the one that has a fallback to disagree about.
fn spawn_command(bin: &Path, db: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env("KIWANO_DB_PATH", db);
    cmd
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
    let mut cmd = spawn_command(&bin, &db_path(None).path);
    #[cfg(windows)]
    {
        // `kiwanod.exe` is a console program, and a console child whose parent
        // has none gets a fresh cmd window on Windows to hold its stdio — which
        // is the window this app's users saw pop up at every launch. Send the
        // streams to NUL instead: no window is ever created, and the daemon
        // still logs to its own files beside the database. Unix inherits as
        // before, where printing to the launching terminal is useful.
        cmd.stdout(null_device()).stderr(null_device());
    }
    cmd.spawn().inspect_err(|e| {
        tracing::error!(binary = %bin.display(), error = %e, "cannot spawn gateway sidecar");
    })
}

/// The SQLite file this app and the gateway share: the caller's own path when it
/// named one, else `KIWANO_DB_PATH`, else `~/.kiwano/kiwano.db`.
///
/// The CLI takes `--db`, the app does not; both then agree on the fallback
/// because it is resolved here rather than in either front end. An explicit path
/// is taken as typed — the user wrote it in this invocation, where a relative
/// path means exactly what it says — while the environment's answer comes back
/// with whatever it had to ignore (see [`kiwano_adapters::config::DbPath`]).
pub fn db_path(explicit: Option<&Path>) -> DbPath {
    match explicit {
        Some(p) => DbPath {
            // Named on the command line, so the variable is not what resolved
            // it — this is only ever read back for a note about the *other*
            // source, which cannot fire in this branch.
            var: "KIWANO_DB_PATH",
            path: p.to_path_buf(),
            ignored: None,
            // Named on the command line in this very invocation, so there is
            // nothing ambient about it — the opposite of the fallback.
            defaulted: false,
        },
        None => kiwano_adapters::config::kiwano_db_path(),
    }
}

/// Environment variable naming a daemon on **another machine**
/// (`migrate.local.md` §13.5).
///
/// Unset — the default, and every install today — means the client resolves the
/// local plane beside its own database, exactly as before. Set, the client dials
/// that address instead.
///
/// It is a **client-side** variable and deliberately not the daemon's own
/// `KIWANO_ADMIN_ADDR`: that one means "listen here", and a machine configured
/// to listen on `0.0.0.0:8318` would, under a shared name, try to *connect* to
/// `0.0.0.0:8318` — which is not an address anything answers on.
pub const DAEMON_ADDR_ENV: &str = "KIWANO_DAEMON_ADDR";

/// Environment variable carrying that daemon's admin token.
///
/// Needed because the usual source is gone: the token lives in the row the
/// daemon minted it into, and on a remote daemon that row is on the other
/// machine (`migrate.local.md` §9.2.1). Where a *local* client reads it, a
/// remote one has to be told — and this is the smallest honest way to be told.
pub const DAEMON_TOKEN_ENV: &str = "KIWANO_DAEMON_TOKEN";

/// Why a configured remote daemon was refused.
#[derive(Debug, PartialEq, Eq)]
pub struct RemoteConfigError(String);

impl std::fmt::Display for RemoteConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The remote daemon a client was pointed at, if any.
///
/// A malformed value is an **error, not a fallback**. Falling back to the local
/// plane would mean a typo silently connects to a different daemon than the one
/// named — the same class of failure as a daemon quietly opening a different
/// database, and it has the same fix: say so and stop.
fn remote_endpoint() -> Result<Option<AdminEndpoint>, RemoteConfigError> {
    let Some(value) = std::env::var(DAEMON_ADDR_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    match value.parse::<std::net::SocketAddr>() {
        Ok(addr) => Ok(Some(AdminEndpoint::Tcp(addr))),
        Err(e) => Err(RemoteConfigError(format!(
            "{DAEMON_ADDR_ENV} = \"{value}\" is not an address: {e} \
             (expected host:port, e.g. 100.64.0.5:8318)"
        ))),
    }
}

/// The admin endpoint a client should dial: a remote daemon when one is named,
/// the local plane otherwise.
///
/// Two forms because the two clients resolve their database differently — the
/// app through the environment, the CLI through `--db` — and the *choice*
/// between remote and local must not differ between them.
pub fn admin_endpoint_resolved() -> Result<AdminEndpoint, RemoteConfigError> {
    admin_endpoint_resolved_for(&db_path(None).path)
}

/// [`admin_endpoint_resolved`] for an explicit database path.
pub fn admin_endpoint_resolved_for(db: &Path) -> Result<AdminEndpoint, RemoteConfigError> {
    match remote_endpoint()? {
        Some(remote) => Ok(remote),
        None => Ok(admin_endpoint_for(db)),
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
    admin_endpoint_for(&db_path(None).path)
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
    let store = Store::open(db).ok()?;
    // **New place first, old place second** (`migrate.local.md` §9.2.1): the
    // token is moving from the app's KV to the daemon's, and the two sides are
    // upgraded independently, so a client has to read whichever row its daemon
    // wrote. The fallback is what makes an old daemon still work; it goes when
    // nothing writes the old row any more.
    store
        .gateway_setting(ADMIN_TOKEN_KEY)
        .or_else(|| store.app_setting(ADMIN_TOKEN_KEY))
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
/// The token this install minted, read from the database — the answer for a
/// client on the same machine as its daemon, which is every client today.
///
/// Deliberately *not* baked into the request helper: a remote client (§13.5)
/// will get its token from somewhere else, and that decision should land here,
/// at the caller, rather than inside a function that looks like it needs no
/// credentials.
pub fn admin_token() -> Option<String> {
    static CACHED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(token) = CACHED.get() {
        return Some(token.clone());
    }
    // A named remote daemon is the one case where the database row is not the
    // answer — it is on the other machine (`migrate.local.md` §13.5). Read the
    // environment first, and only then the local row.
    let token = admin_token_resolved(&db_path(None).path)?;
    let _ = CACHED.set(token.clone());
    Some(token)
}

/// The token for whichever daemon a client is pointed at: the environment's when
/// a remote one is named, the local row otherwise.
///
/// One function rather than two call sites doing it, because the two clients
/// must not disagree about which daemon they are authenticating to.
pub fn admin_token_resolved(db: &Path) -> Option<String> {
    // **No fallback to the local row when a remote is named.** That row belongs
    // to a *different* daemon: sending its token to another host is a credential
    // handed to the wrong place, and the far end would refuse it anyway. A
    // remote client that was not given a token sends none, and the daemon's own
    // 401 is the answer — which says what is missing, unlike a mystery here.
    if remote_is_named() {
        remote_token()
    } else {
        admin_token_for(db)
    }
}

/// Whether the environment names a daemon on another machine.
///
/// Public because the advice differs: a client with no token is told to check
/// its database when the daemon is local, and to set the token variable when it
/// is not — "point `--db` at the shared database" is not something a remote
/// client can do, that database being on the other machine.
pub fn daemon_is_remote() -> bool {
    remote_is_named()
}

fn remote_is_named() -> bool {
    std::env::var(DAEMON_ADDR_ENV)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// The token for a named remote daemon, if one is named.
///
/// `None` when no remote is configured — which is not an error, it is the local
/// case — but also when one *is* configured without a token: the caller then
/// sends an unauthenticated request and gets the daemon's own 401, which is a
/// better error than anything this could invent.
fn remote_token() -> Option<String> {
    std::env::var(DAEMON_TOKEN_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
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

/// The API generation this client speaks (`migrate.local.md` §11.2).
///
/// Sent on every request, including the ones with no token: a daemon that
/// cannot answer this generation should say so **before** anything else, and a
/// client that left the header off would be treated as generation 0 — which is
/// what a build from the middle of the migration is, and not what this is.
fn api_version_header() -> String {
    format!(
        "{}: {}\r\n",
        kiwano_api::version::HEADER,
        kiwano_api::version::CURRENT
    )
}

/// The three admin requests that are one exchange each ([`events_request`] is
/// the fourth, and the only one that is read rather than answered). The framing
/// is raw HTTP written to the endpoint, which is why these are strings rather
/// than calls on a client: [`IPC_HOST`] is arbitrary (the router does no `Host`
/// filtering) but required by HTTP/1.1, and the rest is the same bytes for every
/// call.
fn status_request(token: Option<&str>) -> String {
    format!(
        "GET /status HTTP/1.1\r\nHost: {IPC_HOST}\r\n{}{}Connection: close\r\n\r\n",
        token_header(token),
        api_version_header()
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

/// The fourth admin request: the event stream, which is read rather than
/// answered (see [`watch_events`]).
///
/// No `Connection: close`, unlike its three siblings: this response body is a
/// stream that runs until the gateway stops, so "close when it is finished" is
/// a statement about a moment that has not come. `Accept` is what the gateway's
/// `Sse` answers on, and it is sent because the reader is one.
fn events_request(token: Option<&str>) -> String {
    format!(
        "GET /events HTTP/1.1\r\nHost: {IPC_HOST}\r\n{}{}Accept: text/event-stream\r\n\r\n",
        token_header(token),
        api_version_header()
    )
}

/// Follow the admin plane's event stream, calling `on_tick` for each tick, until
/// the stream ends.
///
/// Blocking, and deliberately returning nothing: a caller runs it on a thread of
/// its own and reconnects when it comes back. Every way out of here — no gateway
/// listening, a refused token, the gateway stopping mid-stream — is the same
/// "try again shortly" to that caller, and none of them is worth an error type
/// nobody would branch on.
///
/// One event per `data:` line: the payload string is handed to `on_event`
/// verbatim — a usage tick (`{"kind":"usage"}`) as well as typed ones (a DLP
/// finding, a limit transition). Each is one line of JSON by the gateway's
/// own contract, which is also why a reader that treats every `data:` line
/// as exactly one event is safe here.
pub fn watch_events(endpoint: &AdminEndpoint, mut on_event: impl FnMut(&str)) {
    let Ok(mut stream) = endpoint.connect_streaming(CONNECT_TIMEOUT) else {
        return;
    };
    if stream
        .write_all(events_request(admin_token().as_deref()).as_bytes())
        .is_err()
    {
        return;
    }
    let mut reader = BufReader::new(stream);
    // The status line first. A gateway that refused the token answers 401 with a
    // JSON body, and reading that as a stream of events would be reading an
    // error message for something it is not.
    let mut status = String::new();
    if !reader
        .read_line(&mut status)
        .map(|n| n > 0)
        .unwrap_or(false)
        || !status.contains("200")
    {
        return;
    }
    // …then the headers, up to the blank line that ends them. `REPLY` is not
    // parsed any further: `Sse` sends the one content type this reader expects.
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => return,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            Ok(_) => {}
        }
    }
    // …then the body, one event per tick. Keep-alive comments (a line starting
    // with `:`) are not events, and neither is any other field.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            // The stream ended: the gateway stopped, or was replaced.
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if let Some(payload) = line.strip_prefix("data:").map(str::trim) {
                    on_event(payload);
                }
            }
        }
    }
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

/// One authorized request from the daemon's resource API, parsed.
///
/// The client half of `migrate.local.md` §7's batch 1: the app and the CLI ask
/// the daemon instead of reading the store, so one process owns the state.
/// Everything the commands need is here — the transport, the token, the error
/// convention — and a resource function is a verb, a path and a type.
///
/// Errors carry the daemon's own message when it sent one (`{"ok":false,
/// "error":…}`), because a refusal a user can act on should not be replaced by
/// a status code retyped here.
fn admin_send<T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<T, String> {
    let (body_headers, payload) = match &body {
        Some(payload) => (
            format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                payload.len()
            ),
            payload.as_str(),
        ),
        None => (String::new(), ""),
    };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {IPC_HOST}\r\n{}{}{body_headers}Connection: close\r\n\r\n{payload}",
        token_header(token),
        api_version_header()
    );
    let mut stream = endpoint
        .connect(CONNECT_TIMEOUT)
        .map_err(|e| format!("gateway is not answering: {e}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("gateway stopped answering: {e}"))?;
    let mut raw = String::new();
    std::io::BufReader::new(stream)
        .read_to_string(&mut raw)
        .map_err(|e| format!("gateway stopped answering: {e}"))?;
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| "gateway answered with no HTTP body".to_string())?;
    if !head.starts_with("HTTP/1.1 200") {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| head.lines().next().unwrap_or("request refused").to_string());
        return Err(message);
    }
    serde_json::from_str(body).map_err(|e| format!("gateway answered something unreadable: {e}"))
}

/// [`admin_send`] for a read.
pub fn admin_get_json<T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    path: &str,
) -> Result<T, String> {
    admin_send(endpoint, token, "GET", path, None)
}

/// [`admin_send`] for a write.
///
/// Separate from the read rather than a mode on it, because the two differ in
/// what a caller has to think about: a GET can be retried by anyone, a POST has
/// to be idempotent at the far end or the retry is a second row
/// (`migrate.local.md` §6.1).
pub fn admin_post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    path: &str,
    body: &B,
) -> Result<T, String> {
    let payload = serde_json::to_string(body).map_err(|e| format!("cannot encode request: {e}"))?;
    admin_send(endpoint, token, "POST", path, Some(payload))
}

/// [`admin_send`] for a replacement: the body *is* the resulting state, so
/// sending it twice leaves the same thing behind. The transport is a POST's —
/// the verb is a promise, and the caller is the one making it.
pub fn admin_put_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    path: &str,
    body: &B,
) -> Result<T, String> {
    let payload = serde_json::to_string(body).map_err(|e| format!("cannot encode request: {e}"))?;
    admin_send(endpoint, token, "PUT", path, Some(payload))
}

/// [`admin_send`] for a partial update: the fields present are the ones that
/// change and an absent one means "leave it alone" — which is what a `PUT`
/// cannot express, since it has no way to tell "unset" from "not mentioned".
pub fn admin_patch_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    path: &str,
    body: &B,
) -> Result<T, String> {
    let payload = serde_json::to_string(body).map_err(|e| format!("cannot encode request: {e}"))?;
    admin_send(endpoint, token, "PATCH", path, Some(payload))
}

/// [`admin_send`] for a body that **is** the document rather than an encoding
/// of one.
///
/// A shared config is JSON, but what travels is its text: the daemon parses and
/// validates it, and a client that had parsed it first would be a second parser
/// to keep in step. `admin_post_json` would send that text as a JSON *string* —
/// quoted and escaped — which is a different document to the far end.
pub fn admin_post_text<T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    path: &str,
    body: &str,
) -> Result<T, String> {
    admin_send(endpoint, token, "POST", path, Some(body.to_string()))
}

/// [`admin_send`] for a deletion. No body: the path names the row.
pub fn admin_delete_json<T: serde::de::DeserializeOwned>(
    endpoint: &AdminEndpoint,
    token: Option<&str>,
    path: &str,
) -> Result<T, String> {
    admin_send(endpoint, token, "DELETE", path, None)
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

/// Whether the gateway answering is on the database *this* process reads.
///
/// There is no error for the case this catches: a daemon started by a service
/// manager resolves `$HOME/.kiwano/kiwano.db` against *its* `$HOME`, so it can
/// come up healthy, answer `/status`, route requests, and be writing to a
/// database nobody is looking at (`migrate.local.md` §13.1). The identity in
/// `/status` is the one thing a client can compare from the outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseAgreement {
    /// Same database: the two ids are the same string.
    Agrees,
    /// Different databases. `ours` is `None` when this side has never seen an
    /// identity at all — which is itself the mismatch: the gateway mints one in
    /// whichever database it opens, so a database without one is not that one.
    Differs,
    /// The gateway did not report an identity — an older build, which predates
    /// this. Unknown rather than agreeing: a check that passes when it could not
    /// run is the state this whole comparison exists to avoid.
    Unknown,
}

/// Compare the identity in a `/status` report against this side's own row.
pub fn database_agreement(status: &serde_json::Value, ours: Option<&str>) -> DatabaseAgreement {
    match status.get("install_id").and_then(|v| v.as_str()) {
        None => DatabaseAgreement::Unknown,
        Some(theirs) if ours == Some(theirs) => DatabaseAgreement::Agrees,
        Some(_) => DatabaseAgreement::Differs,
    }
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
    /// these four requests have to carry it — and must still be well-formed
    /// when the app has none (a fresh install, before the gateway has written
    /// its row), rather than dropping the header line and the blank line with it.
    /// Pointing a client at another machine's daemon, which is what makes the
    /// TCP plane reachable from the app at all (`migrate.local.md` §13.5).
    ///
    /// One test rather than four because the process environment is shared by
    /// every test in this binary: the guards would otherwise race each other,
    /// and a value leaking into a neighbouring test is exactly the failure this
    /// helper exists to avoid.
    #[test]
    fn a_named_remote_daemon_wins_over_the_local_plane() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");
        let store = Store::open(&db).unwrap();
        store.set_app_setting(ADMIN_TOKEN_KEY, "tok-local").unwrap();

        // Not configured: the local plane, and the local row — every install
        // today, and the behaviour that must not change.
        {
            let _addr = crate::test_env::EnvGuard::set(DAEMON_ADDR_ENV, None);
            let _tok = crate::test_env::EnvGuard::set(DAEMON_TOKEN_ENV, None);
            assert_eq!(
                admin_endpoint_resolved_for(&db).unwrap(),
                admin_endpoint_for(&db),
                "no remote named means the local plane"
            );
            assert_eq!(
                admin_token_resolved(&db).as_deref(),
                Some("tok-local"),
                "and the token comes from the local row"
            );
        }

        // A remote named: the client dials it, and the token is the one it was
        // told — the local row is the *other* machine's business now.
        {
            let _addr = crate::test_env::EnvGuard::set(DAEMON_ADDR_ENV, Some("100.64.0.5:8318"));
            let _tok = crate::test_env::EnvGuard::set(DAEMON_TOKEN_ENV, Some("tok-remote"));
            assert_eq!(
                admin_endpoint_resolved_for(&db).unwrap(),
                AdminEndpoint::Tcp("100.64.0.5:8318".parse().unwrap())
            );
            assert_eq!(admin_token_resolved(&db).as_deref(), Some("tok-remote"));
        }

        // A remote named without a token: no token, and the daemon's own 401 is
        // a better answer than anything this could invent.
        {
            let _addr = crate::test_env::EnvGuard::set(DAEMON_ADDR_ENV, Some("100.64.0.5:8318"));
            let _tok = crate::test_env::EnvGuard::set(DAEMON_TOKEN_ENV, None);
            assert_eq!(admin_token_resolved(&db), None);
        }

        // A remote that is not an address: **refused**, not fallen back from. A
        // client that quietly talked to a different daemon than the one named is
        // the same failure as a daemon quietly opening another database.
        {
            let _addr = crate::test_env::EnvGuard::set(DAEMON_ADDR_ENV, Some("daemon.local"));
            let err = admin_endpoint_resolved_for(&db).expect_err("not an address");
            let text = err.to_string();
            assert!(text.contains(DAEMON_ADDR_ENV), "{text}");
            assert!(text.contains("host:port"), "{text}");
        }
    }

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

        let events = events_request(Some("tok-123"));
        assert!(events.starts_with("GET /events HTTP/1.1\r\n"));
        assert!(events.contains(&header));
        assert!(events.contains("Accept: text/event-stream\r\n"));
        assert!(events.ends_with("\r\n\r\n"));
        // No `Connection: close`, unlike its three siblings: this body runs
        // until the gateway stops, so there is no "when it is finished" to
        // state.
        assert!(!events.contains("Connection:"));

        // No token: same requests, no header — a caller with no token to send is
        // answered as one that needs none, so a missing header is not an error.
        for request in [
            status_request(None),
            shutdown_request(None),
            reload_request(None),
            events_request(None),
        ] {
            assert!(!request.contains(ADMIN_TOKEN_HEADER));
            assert!(request.contains(&format!("Host: {IPC_HOST}\r\n")));
            assert!(request.ends_with("\r\n\r\n"));
        }
    }

    /// A stub admin endpoint for `/events`: it answers a `GET /events` with the
    /// status and frames given, then closes — which is what ends a subscription.
    ///
    /// Unix-only for the reason [`LiveGateway`] is: a Windows named pipe cannot
    /// be created from `std`.
    #[cfg(unix)]
    fn events_stub(dir: &Path, status: &str, frames: &str) -> AdminEndpoint {
        use std::os::unix::net::UnixListener;

        let endpoint = AdminEndpoint::parse(&dir.join("events.sock").to_string_lossy());
        let listener = UnixListener::bind(endpoint.path()).expect("bind the stub endpoint");
        let (status, frames) = (status.to_string(), frames.to_string());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                if request.starts_with("GET /events") {
                    let head =
                        format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\n\r\n");
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(frames.as_bytes());
                } else {
                    let body = r#"{"ok":false}"#;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            }
        });
        endpoint
    }

    /// The subscription the app lives on: one call per `data:` frame carrying
    /// that frame's payload, the keep-alive comments ignored, and a return —
    /// rather than a hang — when the stream ends, which is when the caller
    /// reconnects.
    #[cfg(unix)]
    #[test]
    fn watch_events_reports_ticks_and_returns_when_the_stream_ends() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = events_stub(
            dir.path(),
            "200 OK",
            ": keep-alive\n\ndata: {\"kind\":\"usage\"}\n\ndata: {\"kind\":\"dlp_finding\",\"log_id\":7}\n\n",
        );

        let mut events = Vec::new();
        watch_events(&endpoint, |payload| events.push(payload.to_string()));

        assert_eq!(
            events.len(),
            2,
            "one call per event, and none for the comment"
        );
        assert_eq!(events[0], r#"{"kind":"usage"}"#);
        assert_eq!(events[1], r#"{"kind":"dlp_finding","log_id":7}"#);
    }

    /// A gateway that refuses the token answers 401 with a JSON body. Reading
    /// that as a stream of events would be reading an error message for
    /// something it is not, so the subscription ends without a tick.
    #[cfg(unix)]
    #[test]
    fn watch_events_reports_nothing_when_the_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = events_stub(dir.path(), "401 Unauthorized", r#"{"ok":false}"#);

        let mut ticks = 0;
        watch_events(&endpoint, |_| ticks += 1);

        assert_eq!(ticks, 0);
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
            kiwanod::api::probe::probe_url("openai", "https://api.deepseek.com").unwrap(),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            kiwanod::api::probe::probe_url("anthropic", "https://api.deepseek.com/anthropic")
                .unwrap(),
            "https://api.deepseek.com/anthropic/v1/models"
        );
        // versioned bases extend with /models as-is
        assert_eq!(
            kiwanod::api::probe::probe_url("openai", "https://api.moonshot.cn/v1").unwrap(),
            "https://api.moonshot.cn/v1/models"
        );
        // scheme defaults to https when absent; garbage is rejected
        assert_eq!(
            kiwanod::api::probe::probe_url("openai", "api.example.com").unwrap(),
            "https://api.example.com/v1/models"
        );
        assert!(kiwanod::api::probe::probe_url("openai", "").is_err());
        assert!(kiwanod::api::probe::probe_url("grpc", "https://x.example.com").is_err());
    }

    /// The ping body: one user turn, one token out — and the model it names is
    /// the one the caller resolved, not a default of ours.
    #[test]
    fn the_prompt_body_is_one_turn_and_one_token() {
        let v: serde_json::Value =
            serde_json::from_str(&kiwanod::api::probe::prompt_body("kimi-k2")).unwrap();
        assert_eq!(v["model"], "kimi-k2");
        assert_eq!(v["max_tokens"], 1);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "ping");
    }

    /// A refusal is reported with the upstream's words when it has any: "fast"
    /// is the wrong answer for a provider that rejected the request.
    #[test]
    fn an_error_body_is_read_for_its_message() {
        // Both flavors put it at `error.message`.
        assert_eq!(
            kiwanod::api::probe::upstream_error_message(
                r#"{"error":{"message":"model not found","type":"invalid"}}"#,
                404
            ),
            "model not found"
        );
        assert_eq!(
            kiwanod::api::probe::upstream_error_message(
                r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
                401
            ),
            "invalid x-api-key"
        );
        // Anything else falls back to the status rather than to an empty string.
        assert_eq!(
            kiwanod::api::probe::upstream_error_message("<html>nope</html>", 502),
            "upstream answered 502"
        );
        assert_eq!(
            kiwanod::api::probe::upstream_error_message(r#"{"error":{}}"#, 400),
            "upstream answered 400"
        );
    }

    #[test]
    fn count_models_reads_the_list_shape() {
        assert_eq!(
            kiwanod::api::probe::count_models(r#"{"data":[{"id":"a"},{"id":"b"}]}"#),
            Some(2)
        );
        assert_eq!(kiwanod::api::probe::count_models(r#"{"error":{}}"#), None);
        assert_eq!(kiwanod::api::probe::count_models("<html>"), None);
    }

    #[test]
    fn parse_models_reads_ids() {
        assert_eq!(
            kiwanod::api::probe::parse_models(r#"{"data":[{"id":"a"},{"id":"b"}]}"#),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(kiwanod::api::probe::parse_models("<html>").is_empty());
        assert!(kiwanod::api::probe::parse_models(r#"{"data":[{"object":"model"}]}"#).is_empty());
    }

    // The native Gemini list spells the id `models/<id>`; the dropdown wants
    // the id, not the API's resource path.
    #[test]
    fn parse_models_reads_gemini_names() {
        assert_eq!(
            kiwanod::api::probe::parse_models(
                r#"{"models":[{"name":"models/gemini-2.5-pro"},{"name":"models/gemini-2.5-flash"}]}"#
            ),
            vec!["gemini-2.5-pro".to_string(), "gemini-2.5-flash".to_string()]
        );
        assert!(kiwanod::api::probe::parse_models(r#"{"models":[]}"#).is_empty());
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

    /// The sidecar is handed the *resolved* database path, not a rule it could
    /// resolve differently. This is the difference between "both processes read
    /// the same fallback" and "both processes open the same file": a child
    /// launched from a service context has a different `$HOME`, and re-deriving
    /// it there is how one install ends up with two databases.
    #[test]
    fn the_sidecar_is_told_which_database_to_open() {
        let bin = Path::new("/nonexistent/kiwanod");
        let db = Path::new("/tmp/example/kiwano.db");
        let cmd = spawn_command(bin, db);
        let envs: Vec<(String, Option<&std::ffi::OsStr>)> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v))
            .collect();
        assert!(
            envs.iter()
                .any(|(k, v)| k == "KIWANO_DB_PATH" && *v == Some(db.as_os_str())),
            "spawn_command must hand the child its database path: {envs:?}"
        );
    }

    /// The three answers, including the one that is easy to get wrong: an
    /// identity this side has never seen is a *mismatch*, not an unknown —
    /// the gateway mints its id in whichever database it opened.
    #[test]
    fn the_database_identity_comparison_has_three_answers() {
        use DatabaseAgreement::{Agrees, Differs, Unknown};
        let with_id = |id: &str| serde_json::json!({ "ok": true, "install_id": id });

        assert_eq!(database_agreement(&with_id("abc"), Some("abc")), Agrees);
        assert_eq!(database_agreement(&with_id("abc"), Some("xyz")), Differs);
        assert_eq!(database_agreement(&with_id("abc"), None), Differs);
        // An older gateway reports no identity: unknown, never "fine".
        assert_eq!(
            database_agreement(&serde_json::json!({"ok": true}), Some("abc")),
            Unknown
        );
    }
}
