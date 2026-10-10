//! The admin plane's transport: a Unix domain socket on unix, a named pipe on
//! Windows. Both ends live here — the listener `main` serves the admin router
//! on, and the blocking connect `src-tauri/src/sidecar.rs` and `crates/cli`
//! drive their hand-written HTTP requests through — so the endpoint is defined
//! once and there is no second place for the two sides to disagree about it.
//!
//! # Why the admin plane is no longer TCP
//!
//! The data plane (:8317) stays on loopback TCP on purpose: third-party agents
//! are configured with a URL (`ANTHROPIC_BASE_URL` and friends) and there is no
//! way to point Claude Code or Codex at a socket. The admin plane has no such
//! constraint — its only clients are this app and this CLI — so it uses the
//! OS's own local channel, whose access control is a filesystem mode and an
//! object DACL the kernel enforces, rather than a port that every process on
//! the machine can open a connection to.
//!
//! The HTTP framing is deliberately unchanged: same routes, same axum handlers,
//! same request and response bodies, same `x-kiwano-admin-token` check. Only the
//! byte pipe underneath moved.
//!
//! # Debugging
//!
//! `curl http://127.0.0.1:8310/status` no longer reaches the admin plane, and
//! that is the transport rather than a broken gateway. On unix the plane is a
//! socket file:
//!
//! ```text
//! curl --unix-socket ~/.kiwano/admin.sock http://localhost/status
//! ```
//!
//! Windows has no `curl` equivalent for a named pipe; use the app's status chip
//! or `kiwano status`, both of which go through this module.
//!
//! # Where the endpoint is
//!
//! On unix, `admin.sock` in the database's directory — `~/.kiwano/admin.sock`
//! by default, so `KIWANO_DB_PATH` moves the database and the plane together —
//! overridable with [`ADMIN_SOCKET_ENV`]. That directory is already `0700`
//! (`crate::store`'s `harden_permissions`), and [`AdminEndpoint::bind`] chmods
//! the socket itself to `0600`, so the filesystem is the access control.
//!
//! On Windows the name cannot follow the database: it is derived from the
//! user's SID instead, because two users on one machine must not land on one
//! pipe (see `user_pipe_name`), and the pipe's DACL — not its name — is what
//! limits who may connect (see `create_pipe_instance`).

use std::path::Path;
use std::time::Duration;

use axum::serve::Listener;

#[cfg(unix)]
use std::path::PathBuf;

/// Environment variable overriding where the admin plane listens.
///
/// The plane is not a port: on unix the value is a filesystem path, on Windows a
/// pipe name (with or without the `\\.\pipe\` prefix). It is what
/// `KIWANO_ADMIN_PORT` named in pre-0.1.8 builds, and nothing reads that
/// variable now.
pub const ADMIN_SOCKET_ENV: &str = "KIWANO_ADMIN_SOCKET";

/// Environment variable that **additionally** binds the admin plane on TCP, so
/// a client on another machine can reach it.
///
/// Unset — the default, and what every install is today — means no TCP listener
/// at all. Separate from [`ADMIN_SOCKET_ENV`] rather than sharing it: that value
/// is a path, colons are legal in paths, and one variable whose meaning depends
/// on what the string looks like is how a socket ends up on a network.
pub const ADMIN_ADDR_ENV: &str = "KIWANO_ADMIN_ADDR";

/// Environment variable an operator sets to say they mean `0.0.0.0`.
///
/// The daemon binds a private address by default, and refuses an
/// all-interfaces one without this. Not because anything checks who connects —
/// see the module's note on TLS — but because `0.0.0.0` is the difference
/// between "my other laptop" and "whoever is on this network", and it should be
/// a decision someone makes rather than a typo someone makes.
pub const ADMIN_ALLOW_ANY_ENV: &str = "KIWANO_ADMIN_ALLOW_ANY";

/// Whether an address is one a daemon may bind without being told twice.
///
/// Private, loopback, and link-local — the ranges a home network and a
/// tailnet live in. Anything else (a public address, an all-interfaces one, a
/// ULA) needs [`ADMIN_ALLOW_ANY_ENV`].
fn is_private(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            // `is_private` is RFC1918 only, and **the tailnet range is not in
            // it**: `100.64.0.0/10` is the carrier-grade NAT block, which is
            // where Tailscale puts every device. It is the single most likely
            // address this listener is configured with, so it is checked by
            // hand rather than assumed to be covered.
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                // `fc00::/7` is every unique-local address, which is where the
                // v6 side of a tailnet lives.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Where the cross-machine listener goes, resolved once at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminTcp {
    pub addr: std::net::SocketAddr,
}

/// Why a configured address was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum AdminAddrError {
    /// Not `host:port` at all.
    Unparseable { value: String, detail: String },
    /// Reachable from anywhere, and nobody said that was meant.
    TooPublic { addr: std::net::SocketAddr },
}

impl std::fmt::Display for AdminAddrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unparseable { value, detail } => write!(
                f,
                "{ADMIN_ADDR_ENV} = \"{value}\" is not an address: {detail}"
            ),
            Self::TooPublic { addr } => write!(
                f,
                "{ADMIN_ADDR_ENV} = {addr} is reachable from any network. The admin plane can \
                 read every provider key and rewrite every route, so binding it where anyone \
                 can reach it is a decision rather than a default — set \
                 {ADMIN_ALLOW_ANY_ENV}=1 if that is what you mean, or bind a private address \
                 (or put the daemon on a VPN)."
            ),
        }
    }
}

impl AdminTcp {
    /// Resolve the listener from the environment. `None` when not configured —
    /// which is every install that has not asked for one.
    ///
    /// `allow_any` is passed in rather than read here so the decision is
    /// testable without an environment.
    pub fn resolve(value: Option<&str>, allow_any: bool) -> Result<Option<Self>, AdminAddrError> {
        let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let addr: std::net::SocketAddr =
            value.parse().map_err(|e| AdminAddrError::Unparseable {
                value: value.to_string(),
                detail: format!("{e} (expected host:port, e.g. 192.168.1.5:8318)"),
            })?;
        if !is_private(addr.ip()) && !allow_any {
            return Err(AdminAddrError::TooPublic { addr });
        }
        Ok(Some(Self { addr }))
    }

    /// [`resolve`](Self::resolve) against the real environment.
    pub fn from_env() -> Result<Option<Self>, AdminAddrError> {
        let value = std::env::var(ADMIN_ADDR_ENV).ok();
        let allow_any = std::env::var(ADMIN_ALLOW_ANY_ENV)
            .map(|v| !v.trim().is_empty() && v.trim() != "0")
            .unwrap_or(false);
        Self::resolve(value.as_deref(), allow_any)
    }
}

/// Default socket file name, in the database's directory.
#[cfg(unix)]
const SOCKET_FILE_NAME: &str = "admin.sock";

/// The named-pipe namespace. A Windows pipe name without this prefix is
/// completed with it.
#[cfg(windows)]
const PIPE_PREFIX: &str = r"\\.\pipe\";

/// Prefix of the per-user pipe name; see `user_pipe_name`.
#[cfg(windows)]
const PIPE_NAME_PREFIX: &str = "kiwano-admin-";

/// Where the admin plane is, and the two ends of it:
/// [`bind`](AdminEndpoint::bind) for the gateway, and
/// [`connect`](AdminEndpoint::connect) for the app and the CLI.
///
/// A value, not a handle: it is cheap to clone and safe to hold for the life of
/// the process, which is what `AppState` does with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminEndpoint {
    /// The socket file beside the database (a named pipe on Windows) — the
    /// local plane, and the only one that exists unless someone asks for TCP.
    #[cfg(unix)]
    Local(PathBuf),
    #[cfg(windows)]
    Local(String),
    /// A cross-machine plane: the address a daemon was told to listen on.
    /// Plaintext, with the token as the credential — see the module's note.
    Tcp(std::net::SocketAddr),
}

impl AdminEndpoint {
    /// The endpoint that belongs to the database at `db_path`: a socket file
    /// beside it, so the two travel together and a caller that knows where the
    /// database is knows where the plane is.
    ///
    /// Windows derives the name from the user instead — a pipe is a kernel
    /// object in a machine-global namespace, so a name derived from a path
    /// would be shared by every user whose database happens to sit in the same
    /// place, and two users' gateways would then fight over one name. The ACL
    /// on each instance is what actually restricts access; the name only has to
    /// keep users apart.
    pub fn beside_db(db_path: &Path) -> Self {
        #[cfg(unix)]
        {
            let dir = db_path.parent().filter(|d| !d.as_os_str().is_empty());
            AdminEndpoint::Local(match dir {
                Some(dir) => dir.join(SOCKET_FILE_NAME),
                None => PathBuf::from(SOCKET_FILE_NAME),
            })
        }
        #[cfg(windows)]
        {
            let _ = db_path;
            AdminEndpoint::Local(user_pipe_name())
        }
    }

    /// The endpoint to use, honouring [`ADMIN_SOCKET_ENV`] and falling back to
    /// [`beside_db`]. The gateway, the app and the CLI all resolve it this way,
    /// which is why an override reaches all three.
    pub fn from_env(db_path: &Path) -> Self {
        match std::env::var(ADMIN_SOCKET_ENV) {
            Ok(v) if !v.trim().is_empty() => Self::parse(v.trim()),
            _ => Self::beside_db(db_path),
        }
    }

    /// An explicitly named endpoint: a socket path on unix, a pipe name on
    /// Windows (`\\.\pipe\` is added when the value does not carry it).
    pub fn parse(value: &str) -> Self {
        #[cfg(unix)]
        {
            AdminEndpoint::Local(PathBuf::from(value))
        }
        #[cfg(windows)]
        {
            AdminEndpoint::Local(if value.starts_with(PIPE_PREFIX) {
                value.to_string()
            } else {
                format!("{PIPE_PREFIX}{value}")
            })
        }
    }

    /// The host a client reaches this plane on: the address it dialled, or
    /// loopback for a local one.
    ///
    /// The companion of [`AdminTcp`]'s address discipline: a client on another
    /// machine composes the data plane's URL from this and the port the daemon
    /// reports, because it knows the host and only the daemon knows the port.
    pub fn host(&self) -> String {
        match self {
            #[cfg(unix)]
            AdminEndpoint::Local(_) => "127.0.0.1".to_string(),
            #[cfg(windows)]
            AdminEndpoint::Local(_) => "127.0.0.1".to_string(),
            AdminEndpoint::Tcp(addr) => addr.ip().to_string(),
        }
    }

    /// The endpoint as a human reads it: a path on unix, a pipe name on
    /// Windows. For logs, the sidecar's ready line and the CLI's status header.
    pub fn describe(&self) -> String {
        match self {
            #[cfg(unix)]
            AdminEndpoint::Local(path) => path.display().to_string(),
            #[cfg(windows)]
            AdminEndpoint::Local(name) => name.clone(),
            AdminEndpoint::Tcp(addr) => addr.to_string(),
        }
    }

    /// The pipe name, on Windows — the counterpart of [`Self::path`].
    ///
    /// `Local` holds it directly there: a pipe is a kernel object whose name
    /// *is* its address, so there is no directory to join it to and no file to
    /// leave behind when the process dies.
    #[cfg(windows)]
    pub fn name(&self) -> &str {
        match self {
            AdminEndpoint::Local(name) => name.as_str(),
            // Only a local plane has a name; see `path`.
            AdminEndpoint::Tcp(_) => "",
        }
    }

    /// The socket file, on unix. Tests assert on its mode; nothing on the
    /// serving side needs it.
    #[cfg(unix)]
    pub fn path(&self) -> &Path {
        match self {
            AdminEndpoint::Local(path) => path,
            // Only a local plane has a file; a test that asks for one on a TCP
            // endpoint is asking the wrong question, and `describe` is the
            // answer it wants.
            AdminEndpoint::Tcp(_) => Path::new(""),
        }
    }

    /// Bind the admin plane, failing when another gateway already holds it.
    ///
    /// On unix a socket *file* left behind by a gateway that died is reclaimed;
    /// a live socket never is. The Windows branch has nothing of the kind to
    /// handle — see the comment there.
    ///
    /// Requires a tokio runtime: the listener registers with the reactor.
    pub async fn bind(&self) -> std::io::Result<AdminListener> {
        #[cfg(unix)]
        let path = match self {
            AdminEndpoint::Local(path) => path,
            // A TCP endpoint is bound by `AdminTcp::bind`, which has the address
            // and the discipline that goes with it.
            AdminEndpoint::Tcp(addr) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{addr} is a TCP endpoint; bind it through AdminTcp"),
                ))
            }
        };
        #[cfg(unix)]
        {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                if !dir.exists() {
                    std::fs::create_dir_all(dir)?;
                    // Same policy the store applies to the database directory:
                    // whatever lives in it is owner-only, and traversal into it
                    // is what makes the socket's own mode meaningful.
                    set_dir_owner_only(dir)?;
                }
            }

            let listener = match tokio::net::UnixListener::bind(path) {
                Ok(listener) => listener,
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    // One error, two situations: a live gateway, or the corpse
                    // of one. Unix does not remove a socket file when its
                    // process dies, so a plain `bind` would keep failing with
                    // EADDRINUSE for the rest of the machine's uptime — the
                    // "gateway will not start after a crash" failure. Tell them
                    // apart by connecting: a live gateway answers (or at least
                    // accepts), a corpse refuses immediately.
                    if tokio::net::UnixStream::connect(path).await.is_ok() {
                        // Really is another gateway. Leave it alone and report
                        // the bind failure as-is.
                        return Err(e);
                    }
                    tracing::warn!(
                        path = %path.display(),
                        "removing a stale admin socket left by a gateway that is no longer running"
                    );
                    std::fs::remove_file(path)?;
                    tokio::net::UnixListener::bind(path)?
                }
                Err(e) => return Err(e),
            };
            // Belt and braces with the `0700` directory above, and the control
            // that survives the directory being moved: a socket is created
            // `0777 & !umask` (0755 for the usual umask 022), so without this
            // any local process could drive `/shutdown` through it.
            set_socket_owner_only(path)?;
            Ok(AdminListener {
                inner: listener,
                describe: self.describe(),
            })
        }
        #[cfg(windows)]
        {
            // No stale-name recovery here, by construction: a named pipe is a
            // kernel object, not a file, so it disappears with its last handle
            // and a crashed gateway leaves nothing behind to collide with. The
            // unix branch's unlink-and-rebind dance has no counterpart.
            Ok(AdminListener {
                inner: PipeListener {
                    name: self.name().to_string(),
                    pending: Some(create_pipe_instance(self.name())?),
                },
                describe: self.describe(),
            })
        }
    }

    /// A blocking connection, for the app and the CLI (both talk HTTP by hand
    /// over this stream).
    ///
    /// `timeout` bounds the connect, and on unix each read and write after it.
    /// On Windows only the connect is bound: a named pipe has no settable
    /// read/write timeout. That is a deliberate trade — the alternative is
    /// overlapped I/O or `PIPE_NOWAIT` polling, a state machine that cannot be
    /// exercised on the machine this was written on — and its failure mode is a
    /// wedged gateway hanging a caller that is only ever reading one small
    /// local response.
    pub fn connect(&self, timeout: Duration) -> std::io::Result<AdminStream> {
        self.connect_with(timeout, Some(timeout))
    }

    /// A blocking connection for a reader that is following a stream rather than
    /// waiting on one small response: the connect is still bounded, the reads
    /// are not.
    ///
    /// [`connect`](AdminEndpoint::connect)'s timeout bounds every read, which is
    /// right for the request/response pairs the app and the CLI send and wrong
    /// for the admin plane's event stream, whose ordinary state is silence — it
    /// would be torn down and reconnected on a loop. Writes stay bounded:
    /// subscribing is one small write.
    ///
    /// On Windows this is [`connect`](AdminEndpoint::connect): a named pipe has
    /// no settable read timeout either way.
    pub fn connect_streaming(&self, timeout: Duration) -> std::io::Result<AdminStream> {
        self.connect_with(timeout, None)
    }

    fn connect_with(
        &self,
        timeout: Duration,
        read_timeout: Option<Duration>,
    ) -> std::io::Result<AdminStream> {
        // A cross-machine plane: the same bytes, over TCP. `TcpStream` has the
        // same settable timeouts the socket branch relies on, which is why the
        // streaming reader keeps working unchanged.
        if let AdminEndpoint::Tcp(addr) = self {
            let stream = std::net::TcpStream::connect_timeout(addr, timeout)?;
            if let Some(read) = read_timeout {
                stream.set_read_timeout(Some(read))?;
            }
            stream.set_write_timeout(Some(timeout))?;
            return Ok(AdminStream::Tcp(stream));
        }
        #[cfg(unix)]
        {
            let AdminEndpoint::Local(path) = self else {
                unreachable!("the TCP case returned above")
            };
            let stream = std::os::unix::net::UnixStream::connect(path)?;
            // Left unset for a streaming reader: `None` means "block until there
            // is something to read".
            if let Some(read) = read_timeout {
                stream.set_read_timeout(Some(read))?;
            }
            stream.set_write_timeout(Some(timeout))?;
            Ok(AdminStream::Local(stream))
        }
        #[cfg(windows)]
        {
            let _ = read_timeout;
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::Foundation::{
                GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
            };
            use windows_sys::Win32::Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
            };
            use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

            let name = wide(self.name());
            // A pipe that exists with every instance busy fails CreateFileW
            // with ERROR_PIPE_BUSY; waiting up to `timeout` for an instance to
            // free up is the only part of this that can block. A pipe that does
            // not exist fails immediately (ERROR_FILE_NOT_FOUND), so "no
            // gateway is running" stays as fast as a refused TCP connect was.
            // The return value is ignored on purpose: on timeout CreateFileW
            // reports the real reason, which is what the caller should see.
            unsafe {
                WaitNamedPipeW(
                    name.as_ptr(),
                    timeout.as_millis().min(u32::MAX as u128) as u32,
                );
            }
            // SAFETY: `name` is NUL-terminated and alive for the call; a null
            // security-attribute pointer is the documented default (the pipe's
            // own DACL governs access, and the client passes no share mode).
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: `handle` is a valid, owned file handle (CreateFileW
            // succeeded), and `File` takes ownership of it from here.
            Ok(AdminStream::Local(unsafe {
                std::fs::File::from_raw_handle(handle)
            }))
        }
    }

    /// Best-effort removal of the endpoint's filesystem entry. The gateway
    /// calls this after its serve loop returns so an orderly stop leaves no
    /// socket file behind; [`bind`](AdminEndpoint::bind) reclaims one that a
    /// crash left anyway.
    pub fn cleanup(&self) {
        #[cfg(unix)]
        {
            let AdminEndpoint::Local(path) = self else {
                // A TCP listener leaves nothing behind to clean up.
                return;
            };
            if let Err(e) = std::fs::remove_file(path) {
                // Not an error worth a warning on the happy path: the file may
                // already be gone, and a leftover is reclaimed on next bind.
                tracing::debug!(path = %path.display(), error = %e, "admin socket not removed");
            }
        }
        #[cfg(windows)]
        {
            // A kernel object: it went away with the last handle.
        }
    }
}

/// Restrict a directory to its owner (unix).
#[cfg(unix)]
fn set_dir_owner_only(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Restrict the socket file to its owner (unix).
#[cfg(unix)]
fn set_socket_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// The connected stream an admin-plane client writes its raw HTTP request to.
///
/// Both platforms' client types are a plain OS handle with `Read`/`Write` on
/// it, so there is nothing to wrap: a unix socket, and the `File` over the
/// `HANDLE` a named pipe is opened through.
#[derive(Debug)]
pub enum AdminStream {
    /// The local plane: a unix socket, or the `File` over the `HANDLE` a named
    /// pipe is opened through.
    #[cfg(unix)]
    Local(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Local(std::fs::File),
    /// A cross-machine plane. Same bytes, same hand-written HTTP on top.
    Tcp(std::net::TcpStream),
}

impl std::io::Read for AdminStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            AdminStream::Local(s) => s.read(buf),
            #[cfg(windows)]
            AdminStream::Local(s) => s.read(buf),
            AdminStream::Tcp(s) => s.read(buf),
        }
    }
}

impl std::io::Write for AdminStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            AdminStream::Local(s) => s.write(buf),
            #[cfg(windows)]
            AdminStream::Local(s) => s.write(buf),
            AdminStream::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            AdminStream::Local(s) => s.flush(),
            #[cfg(windows)]
            AdminStream::Local(s) => s.flush(),
            AdminStream::Tcp(s) => s.flush(),
        }
    }
}

/// The per-connection IO `axum::serve` drives.
#[cfg(unix)]
pub type AdminIo = tokio::net::UnixStream;
#[cfg(windows)]
pub type AdminIo = tokio::net::windows::named_pipe::NamedPipeServer;

/// What [`AdminEndpoint::bind`] hands to `axum::serve`.
///
/// A wrapper rather than a bare listener so `main` is the same code on both
/// platforms: `tokio::net::UnixListener` on unix, a chain of named-pipe
/// instances on Windows.
pub struct AdminListener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    inner: PipeListener,
    /// Reported by `local_addr`; axum logs it and nothing reads it back.
    describe: String,
}

impl Listener for AdminListener {
    type Io = AdminIo;
    type Addr = String;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        #[cfg(unix)]
        {
            // axum's own impl for tokio's listener: it logs transient accept
            // errors and retries with backoff, which the pipe branch below has
            // to reproduce by hand.
            let (io, _addr) = <tokio::net::UnixListener as Listener>::accept(&mut self.inner).await;
            (io, self.describe.clone())
        }
        #[cfg(windows)]
        {
            self.inner.accept().await
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.describe.clone())
    }
}

// ── Windows named pipes ─────────────────────────────────────────────────────

/// A chain of pipe instances, one waiting at a time.
///
/// A named pipe server has to keep an instance *listening* while it serves the
/// previous client, or a client connecting in the gap fails with
/// ERROR_FILE_NOT_FOUND rather than blocking for a moment — see tokio's
/// `named_pipe` module docs, whose loop this is.
#[cfg(windows)]
struct PipeListener {
    name: String,
    /// The instance the next client will land on.
    pending: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

#[cfg(windows)]
impl PipeListener {
    async fn accept(&mut self) -> (AdminIo, String) {
        loop {
            let Some(server) = self.pending.take() else {
                // Only reachable if the replace below itself failed. Rebuild
                // rather than panic: a gateway that cannot create a pipe has
                // nothing to serve, but it can still try again.
                match create_pipe_instance(&self.name) {
                    Ok(next) => {
                        self.pending = Some(next);
                        continue;
                    }
                    Err(e) => {
                        tracing::error!(pipe = %self.name, error = %e, "cannot create the admin pipe; retrying");
                        tokio::time::sleep(RETRY_DELAY).await;
                        continue;
                    }
                }
            };
            match server.connect().await {
                Ok(()) => {
                    // Replace the instance *before* handing this one to axum:
                    // the connection is already established, so the next client
                    // must still find something listening when it arrives.
                    match create_pipe_instance(&self.name) {
                        Ok(next) => self.pending = Some(next),
                        Err(e) => {
                            tracing::error!(
                                pipe = %self.name,
                                error = %e,
                                "cannot queue the next admin pipe instance; \
                                 the next connection may be refused"
                            );
                        }
                    }
                    return (server, self.name.to_string());
                }
                Err(e) => {
                    // A client that vanished between CreateFileW and the connect
                    // is routine; anything else is not, but neither is worth
                    // giving up the accept loop over. Drop this instance — that
                    // is what disconnects it — and let the top of the loop
                    // create a fresh one.
                    tracing::warn!(pipe = %self.name, error = %e, "admin pipe connect failed; retrying");
                    drop(server);
                }
            }
        }
    }
}

#[cfg(windows)]
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// The name of this user's admin pipe.
///
/// Per-user, because the pipe namespace is global to the machine: a fixed
/// `\\.\pipe\kiwano-admin` would be one name for every account on the box, and
/// whichever user created it first would own it. The user's SID bytes are
/// hex-encoded into the name — the SID is what actually identifies an account
/// (an account rename does not change it, and two accounts cannot share one),
/// and the name is not a secret in any case: the DACL set in
/// [`create_pipe_instance`] is what decides who may connect.
///
/// A SID that cannot be read at all yields a name nothing else will pick; the
/// pipe cannot be created without the SID anyway, so the gateway reports that
/// instead of quietly serving a name shared with other users.
#[cfg(windows)]
fn user_pipe_name() -> String {
    match current_user_sid() {
        Some(sid) => {
            let mut hex = String::with_capacity(sid.len() * 2);
            for byte in sid {
                hex.push_str(&format!("{byte:02x}"));
            }
            format!("{PIPE_PREFIX}{PIPE_NAME_PREFIX}{hex}")
        }
        None => format!("{PIPE_PREFIX}{PIPE_NAME_PREFIX}unknown-user"),
    }
}

/// Create one pipe instance whose DACL grants the current user alone.
///
/// # The ACL, explicitly
///
/// `CreateNamedPipeW`'s default is the *process* default DACL — on a normal
/// Windows account that is the user, `SYSTEM` and `Administrators`, but it is
/// inherited from the token and is not something to depend on silently. This
/// builds the DACL instead: one `GRANT_ACCESS` ACE, `GENERIC_READ |
/// GENERIC_WRITE`, trustee = the current user's SID, no inheritance, and the
/// descriptor's DACL marked present-and-explicit. So the ACE list *is* the
/// whole access policy: only the user who started the gateway can connect,
/// whatever the token's default DACL happens to be.
///
/// Built with `EXPLICIT_ACCESS_W` + `SetEntriesInAclW` rather than an SDDL
/// string, the same way (and for the same reason) as
/// `crate::store::harden_permissions_windows`: the SID stays a SID instead of
/// being formatted and parsed back, and the policy is flags the compiler
/// checks. The ACL is freed with `LocalFree` — `SetEntriesInAclW` allocates it
/// with `LocalAlloc`, and it is only needed until `CreateNamedPipeW` has copied
/// the descriptor.
#[cfg(windows)]
fn create_pipe_instance(
    name: &str,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Foundation::{
        LocalFree, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HLOCAL,
    };
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, TRUSTEE_IS_SID,
        TRUSTEE_IS_USER, TRUSTEE_W,
    };
    // `NO_INHERITANCE` is declared in `Win32::Security`, not in the
    // `Authorization` submodule the rest of the ACL API comes from. Getting
    // that wrong is invisible on every platform but Windows, which is the only
    // one that compiles this fn.
    use windows_sys::Win32::Security::{
        InitializeSecurityDescriptor, SetSecurityDescriptorDacl, ACL, NO_INHERITANCE,
        SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
    };

    let Some(sid) = current_user_sid() else {
        return Err(std::io::Error::other(
            "cannot read the current user's SID, so the admin pipe cannot be \
             restricted to that user",
        ));
    };

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_READ | GENERIC_WRITE,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            // Not a string despite the name: with `TRUSTEE_IS_SID` this points
            // at the SID bytes.
            ptstrName: sid.as_ptr() as *mut u16,
        },
    };

    // A NULL old ACL means "this list is the whole DACL".
    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `entry` is a valid EXPLICIT_ACCESS_W that outlives the call,
    // `sid` likewise (the trustee points into it), and `acl` is a valid
    // out-pointer.
    let rc = unsafe { SetEntriesInAclW(1, &entry, std::ptr::null(), &mut acl) };
    if rc != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(rc as i32));
    }

    let created = (|| {
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let descriptor_ptr: *mut std::ffi::c_void =
            (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast::<std::ffi::c_void>();
        // SAFETY: `descriptor` is `size_of::<SECURITY_DESCRIPTOR>()` bytes of
        // correctly aligned, fully-initialized memory.
        let ok =
            unsafe { InitializeSecurityDescriptor(descriptor_ptr, SECURITY_DESCRIPTOR_REVISION) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Present = 1, defaulted = 0: the ACL above is the DACL, and it is not
        // an absent-or-defaulted pointer.
        // SAFETY: `descriptor_ptr` was initialized just above and `acl` is the
        // ACL `SetEntriesInAclW` returned, alive until the `LocalFree` below.
        let ok = unsafe { SetSecurityDescriptorDacl(descriptor_ptr, 1, acl, 0) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let mut attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor_ptr,
            bInheritHandle: 0,
        };
        // Reject remote clients: pipe access is local-only at the OS level,
        // which the DACL would not express on its own. No
        // FILE_FLAG_FIRST_PIPE_INSTANCE: it is only legal on the first instance
        // of a name, and it is the DACL rather than name ownership that keeps
        // other users out.
        let options = {
            let mut options = ServerOptions::new();
            options.reject_remote_clients(true);
            options
        };
        // SAFETY: `name` is a valid pipe name and `attrs` points at a
        // SECURITY_ATTRIBUTES whose descriptor and DACL outlive the call.
        unsafe {
            options.create_with_security_attributes_raw(
                name,
                (&mut attrs as *mut SECURITY_ATTRIBUTES).cast::<std::ffi::c_void>(),
            )
        }
    })();

    // SAFETY: `acl` came from `SetEntriesInAclW` (LocalAlloc) and the pipe has
    // copied the descriptor by now, so this is the last use.
    unsafe {
        LocalFree(acl as HLOCAL);
    }
    created
}

/// `SECURITY_DESCRIPTOR_REVISION`, which windows-sys keeps in
/// `Win32::System::SystemServices` — a module this crate does not otherwise
/// need, so the constant is spelled out here.
#[cfg(windows)]
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// The SID of the user this process runs as, as raw SID bytes.
///
/// Copied out of the `TOKEN_USER` structure rather than borrowed: the SID
/// points into the buffer `GetTokenInformation` filled, which is freed on
/// return. `None` on any failure — every caller treats that as "cannot
/// restrict this to a user" and refuses to proceed, since the fallback would be
/// a pipe anyone may connect to. Same idiom as
/// `crate::store::windows_current_user_sid`.
#[cfg(windows)]
fn current_user_sid() -> Option<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a valid out-pointer; the pseudo-handle from
    // `GetCurrentProcess` needs no closing.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }

    let sid = (|| {
        let mut needed: u32 = 0;
        // Sizes the buffer; the expected outcome of a NULL buffer is
        // ERROR_INSUFFICIENT_BUFFER with `needed` filled in.
        // SAFETY: a NULL buffer with length 0 is the documented sizing call.
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return None;
        }

        // `u64` elements, not `u8`: `TOKEN_USER` holds pointers, so the buffer
        // has to be pointer-aligned to be read through a `*const TOKEN_USER`.
        let mut buf = vec![0u64; needed.div_ceil(8) as usize];
        let cap = (buf.len() * 8) as u32;
        // SAFETY: `buf` is `cap` bytes long and correctly aligned.
        let ok = unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), cap, &mut needed)
        };
        if ok == 0 {
            return None;
        }

        // SAFETY: the call above succeeded, so `buf` holds an aligned,
        // fully-initialized `TOKEN_USER` whose `Sid` points inside it.
        let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
        if user.User.Sid.is_null() {
            return None;
        }
        // SAFETY: a token's SID is a valid SID, so this returns its length.
        let len = unsafe { GetLengthSid(user.User.Sid) } as usize;
        if len == 0 {
            return None;
        }
        // SAFETY: `Sid` points at `len` readable bytes inside `buf`, and the
        // copy happens before `buf` is dropped.
        Some(unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), len) }.to_vec())
    })();

    // SAFETY: `token` was opened by the successful `OpenProcessToken` above.
    unsafe {
        CloseHandle(token);
    }
    sid
}

/// NUL-terminated UTF-16, the encoding every `…W` call wants.
#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Helpers for the tests here and in [`super::admin`], which serves the real
/// admin router over a real endpoint.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A unique endpoint for one test.
    ///
    /// Serve the admin router on a TCP listener, and report the address it got.
    ///
    /// The counterpart of [`serve_in_background`] for the cross-machine plane:
    /// it binds port 0 and hands back the port the OS chose, so tests never race
    /// each other for one. This is what `main` does for the same listener —
    /// `AdminTcp::bind` is deliberately not involved, because it resolves an
    /// address from the environment and a test should not be an environment.
    pub(crate) fn serve_tcp_in_background(app: axum::Router) -> std::net::SocketAddr {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind a loopback port");
                let addr = listener.local_addr().expect("the bound address");
                tx.send(addr).expect("report the address");
                let _ = axum::serve(listener, app).await;
            });
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the TCP listener never came up")
    }

    /// Windows cannot use `beside_db` here: the real name is per-user, so every
    /// test in the process would share one pipe and collide with the others.
    /// The socket path is unique per test by construction (its own tempdir).
    pub(crate) fn test_endpoint(dir: &Path) -> AdminEndpoint {
        #[cfg(unix)]
        {
            AdminEndpoint::beside_db(&dir.join("kiwano.db"))
        }
        #[cfg(windows)]
        {
            let unique = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "endpoint".to_string());
            AdminEndpoint::parse(&format!("kiwano-test-{unique}"))
        }
    }

    /// Serve `app` on `endpoint` from a background thread, returning once the
    /// endpoint answers. The thread is deliberately never joined: `axum::serve`
    /// runs until the process exits, which is exactly what the test wants from
    /// it (the same shape as the upstream stubs in the other test modules).
    pub(crate) fn serve_in_background(endpoint: &AdminEndpoint, app: axum::Router) {
        let bound = endpoint.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");
            runtime.block_on(async move {
                let listener = bound.bind().await.expect("bind the admin endpoint");
                let _ = axum::serve(listener, app).await;
            });
        });
        assert!(
            wait_until_answering(endpoint),
            "the admin plane never came up on {}",
            endpoint.describe()
        );
    }

    /// Poll `endpoint` until a connect succeeds.
    fn wait_until_answering(endpoint: &AdminEndpoint) -> bool {
        for _ in 0..200 {
            if endpoint.connect(Duration::from_millis(50)).is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// One raw HTTP request over the IPC endpoint, read to EOF.
    pub(crate) fn request(endpoint: &AdminEndpoint, request: &str) -> String {
        use std::io::{Read, Write};
        let mut stream = endpoint
            .connect(Duration::from_millis(500))
            .expect("connect");
        stream.write_all(request.as_bytes()).expect("write request");
        let mut raw = String::new();
        stream.read_to_string(&mut raw).expect("read response");
        raw
    }
}

/// The address discipline, which is the substitute for TLS in this design.
///
/// Nothing here checks *who* connects — the token does that. What it checks
/// is the thing an operator can get wrong by accident: an address that
/// reaches further than they meant. A typo (`0.0.0.0` for `127.0.0.1`) is
/// one character and would otherwise silently expose every credential the
/// daemon holds.
#[test]
fn a_configured_listener_address_needs_to_be_a_private_one() {
    // Not configured: no listener. The default, and every install today.
    assert_eq!(AdminTcp::resolve(None, false), Ok(None));
    assert_eq!(AdminTcp::resolve(Some("   "), false), Ok(None));

    // A private address is fine without being asked twice — this is the
    // home network and the tailnet, which is the case that exists.
    for value in [
        "127.0.0.1:8318",
        "192.168.1.5:8318",
        "10.0.0.7:8318",
        "172.16.4.4:8318",
        "100.101.102.103:8318", // tailnet
        "[::1]:8318",
        "[fd7a:115c:a1e0::1]:8318", // tailnet, v6
    ] {
        assert!(
            AdminTcp::resolve(Some(value), false).is_ok(),
            "{value} should be bindable without ALLOW_ANY"
        );
    }

    // All-interfaces and public addresses need the second switch.
    for value in ["0.0.0.0:8318", "[::]:8318", "203.0.113.9:8318"] {
        match AdminTcp::resolve(Some(value), false) {
            Err(AdminAddrError::TooPublic { .. }) => {}
            other => panic!("{value} should need ALLOW_ANY, got {other:?}"),
        }
        assert!(
            AdminTcp::resolve(Some(value), true).is_ok(),
            "{value} is allowed once the operator says so"
        );
    }

    // And the refusal says what to do about it, which a bare "denied" does
    // not — the reader is an operator who just typed an address.
    let err = AdminTcp::resolve(Some("0.0.0.0:8318"), false).unwrap_err();
    let text = err.to_string();
    assert!(text.contains(ADMIN_ALLOW_ANY_ENV), "{text}");
    assert!(text.contains("VPN"), "{text}");

    // A value that is not an address at all names the variable and the
    // shape it wanted, rather than "invalid".
    let err = AdminTcp::resolve(Some("/tmp/admin.sock"), false).unwrap_err();
    assert!(matches!(err, AdminAddrError::Unparseable { .. }));
    assert!(err.to_string().contains("host:port"), "{err}");
}

#[cfg(test)]
mod tests {
    use super::test_support::{request, serve_in_background, test_endpoint};
    use super::*;

    // ── the endpoint itself ─────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn the_endpoint_lives_beside_the_database() {
        let endpoint = AdminEndpoint::beside_db(Path::new("/home/u/.kiwano/kiwano.db"));
        assert_eq!(endpoint.path(), Path::new("/home/u/.kiwano/admin.sock"));

        // An explicit value wins, verbatim.
        let explicit = AdminEndpoint::parse("/tmp/other/admin.sock");
        assert_eq!(explicit.path(), Path::new("/tmp/other/admin.sock"));
        assert_eq!(explicit.describe(), "/tmp/other/admin.sock");
    }

    #[cfg(windows)]
    #[test]
    fn the_endpoint_is_a_per_user_pipe() {
        let endpoint = AdminEndpoint::beside_db(Path::new("C:\\Users\\u\\.kiwano\\kiwano.db"));
        let name = endpoint.name();
        assert!(name.starts_with(r"\\.\pipe\kiwano-admin-"), "{name}");
        // Two derivations agree, and neither is the machine-global name.
        assert_eq!(name, user_pipe_name());
        assert_ne!(name, r"\\.\pipe\kiwano-admin");

        // An explicit value gets the namespace prefix when it lacks one.
        assert_eq!(
            AdminEndpoint::parse("some-pipe").name(),
            r"\\.\pipe\some-pipe"
        );
        assert_eq!(
            AdminEndpoint::parse(r"\\.\pipe\some-pipe").name(),
            r"\\.\pipe\some-pipe"
        );
    }

    #[test]
    fn a_dead_endpoint_refuses_to_connect() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        assert!(
            endpoint.connect(Duration::from_millis(50)).is_err(),
            "nothing is listening, so this must not report a connection"
        );
    }

    // ── serving ─────────────────────────────────────────────────────────

    /// The point of keeping the HTTP framing: the transport carries whole HTTP
    /// requests, so the same router answers over a socket or a pipe as it does
    /// over TCP. GET and POST are both exercised — the admin plane uses both.
    /// The cross-machine plane carries the same requests as the local one.
    ///
    /// This is the whole point of the TCP listener: the app on another machine
    /// speaks the same hand-written HTTP over it, so the only new thing is the
    /// transport underneath. Proven rather than asserted — a router served on a
    /// TCP socket, dialled through `AdminEndpoint::Tcp`, answered.
    #[test]
    fn the_admin_plane_answers_http_over_tcp() {
        use axum::routing::get;

        let app = axum::Router::new().route(
            "/status",
            get(|| async { axum::Json(serde_json::json!({ "ok": true, "version": "test" })) }),
        );
        let addr = super::test_support::serve_tcp_in_background(app);
        let endpoint = AdminEndpoint::Tcp(addr);

        let raw = super::test_support::request(
            &endpoint,
            "GET /status HTTP/1.1\r\nHost: kiwanod\r\nConnection: close\r\n\r\n",
        );
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains(r#""ok":true"#), "{raw}");

        // And the endpoint describes itself as the address, which is what the
        // ready line and the logs print.
        assert_eq!(endpoint.describe(), addr.to_string());
    }

    /// A TCP endpoint is not something `bind` knows how to open — it has no
    /// path, and `AdminTcp` is where the address discipline lives. Asking the
    /// wrong one is a named refusal rather than a panic or a silent no-op.
    #[tokio::test]
    async fn binding_a_tcp_endpoint_through_bind_is_refused() {
        let endpoint = AdminEndpoint::Tcp("127.0.0.1:8318".parse().unwrap());
        let err = match endpoint.bind().await {
            Ok(_) => panic!("a TCP endpoint has no local listener to bind"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("AdminTcp"), "{err}");

        // Cleanup is a no-op for it too: there is no socket file to remove.
        endpoint.cleanup();
    }

    #[test]
    fn the_admin_plane_answers_http_over_the_endpoint() {
        use axum::routing::{get, post};
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let app = axum::Router::new()
            .route("/status", get(|| async { "liveness" }))
            .route("/reload", post(|| async { "reloaded" }));
        serve_in_background(&endpoint, app);

        let status = request(
            &endpoint,
            "GET /status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(status.starts_with("HTTP/1.1 200 OK"), "{status}");
        assert!(status.ends_with("liveness"), "{status}");

        let reload = request(
            &endpoint,
            "POST /reload HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(reload.starts_with("HTTP/1.1 200 OK"), "{reload}");
        assert!(reload.ends_with("reloaded"), "{reload}");
    }

    // ── the stale socket file ───────────────────────────────────────────

    /// The failure this exists for: the gateway dies and its socket file stays,
    /// so the next bind fails with EADDRINUSE forever.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_socket_file_is_reclaimed_on_bind() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());

        // A file that is not a socket at all (a leftover someone dropped there)
        // is the same problem, and is what `remove_file` has to cope with.
        std::fs::write(endpoint.path(), b"not a socket").unwrap();
        let listener = endpoint.bind().await.expect("reclaim a foreign file");
        drop(listener);

        // And the real shape: a socket file whose listener is gone. Dropping a
        // `UnixListener` does not remove the file — exactly what killing the
        // gateway leaves.
        let listener = endpoint.bind().await.expect("reclaim a dead socket");
        drop(listener);
        let listener = endpoint.bind().await.expect("reclaim it again");
        drop(listener);
    }

    /// The other half of that: a gateway that *is* running keeps its socket.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_live_socket_is_not_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let _live = endpoint.bind().await.expect("first bind");

        let second = endpoint.bind().await;
        assert!(
            second.is_err(),
            "a second gateway must not steal the socket from a running one"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_socket_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let _listener = endpoint.bind().await.expect("bind");

        let path = endpoint.path();
        assert_eq!(
            mode(path),
            0o600,
            "the admin plane is reachable by its owner only"
        );

        // The assertion above is only worth something if the measurement can
        // fail. A socket is created `0777 & !umask` — 0755 for the usual umask
        // 022 — so `bind`'s chmod is what takes it to 0600, and the check
        // catches a socket left at the looser default. Take the real mode
        // somewhere else to prove the read follows the file rather than
        // reporting a constant.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode(path), 0o644, "the mode is really being read");
        assert_ne!(
            mode(path),
            0o600,
            "so a socket left group/world-readable would fail the check"
        );
    }

    /// An orderly stop leaves nothing behind for the next bind to trip over.
    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_removes_the_socket_file() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let listener = endpoint.bind().await.expect("bind");
        assert!(endpoint.path().exists());
        drop(listener);

        endpoint.cleanup();
        assert!(
            !endpoint.path().exists(),
            "a stopped gateway does not leave its socket behind"
        );
        // And it is a no-op rather than an error when there is nothing there.
        endpoint.cleanup();
    }
}
