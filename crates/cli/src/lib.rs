//! kiwano — manage Kiwano providers without the desktop app.
//!
//! Talks straight to the shared SQLite store (the same file the gateway and the
//! desktop app use) and to the gateway's admin plane for status and hot reload,
//! so it works on a headless server. Everything it does is a call into
//! `kiwano-core`, the crate the desktop app also uses — a provider added here is
//! the same row, written the same way, as one added there.
//!
//! The entry point is [`run_with`], which takes its writers as arguments and
//! never exits the process. `main.rs` is a three-line wrapper around it, and the
//! integration tests drive it directly.

pub mod cli;
mod cmds;
pub mod output;

use std::cell::OnceCell;
use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use kiwano_core::aux::Aux;
use kiwano_core::sidecar::{self, AdminEndpoint};
use kiwanod::store::Store;

use cli::{Cli, Command};
use output::Out;

/// Success.
pub const EXIT_OK: i32 = 0;
/// A negative *answer*, not a failure — `status` with the gateway down. The
/// store was read fine; there is simply no gateway to report on.
pub const EXIT_NEGATIVE: i32 = 1;
/// Usage or validation error. clap's own parse failures already exit 2.
pub const EXIT_USAGE: i32 = 2;
/// Runtime error: store, IO, admin plane, Hub.
pub const EXIT_RUNTIME: i32 = 3;

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub code: i32,
}

impl CliError {
    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_RUNTIME,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_USAGE,
        }
    }
}

/// Core returns `Result<_, String>` throughout; convert at the boundary rather
/// than churning every signature in the core crate.
impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::runtime(message)
    }
}

/// Everything a command needs, resolved once.
///
/// The store and the auxiliary connection are opened on demand and cached: a
/// `status` against a running gateway never needs them, and opening the store
/// would create the database file as a side effect — which a read-only question
/// should not do.
pub struct Ctx<'a> {
    pub db: PathBuf,
    pub admin: AdminEndpoint,
    pub token: Option<String>,
    /// Port an agent's config is pointed at when it is taken over.
    pub data_port: u16,
    /// Root the agent config files live under.
    pub home: PathBuf,
    pub out: Out<'a>,
    reload: bool,
    store: OnceCell<Store>,
    aux: OnceCell<Aux>,
}

impl Ctx<'_> {
    pub fn store(&self) -> Result<&Store, CliError> {
        if self.store.get().is_none() {
            if let Some(dir) = self.db.parent() {
                if !dir.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(dir);
                }
            }
            let store = Store::open(&self.db).map_err(|e| {
                CliError::runtime(format!("cannot open database {}: {e}", self.db.display()))
            })?;
            let _ = self.store.set(store);
        }
        Ok(self.store.get().expect("just set"))
    }

    pub fn aux(&self) -> Result<&Aux, CliError> {
        if self.aux.get().is_none() {
            let aux = Aux::open(&self.db).map_err(|e| {
                CliError::runtime(format!("cannot open database {}: {e}", self.db.display()))
            })?;
            let _ = self.aux.set(aux);
        }
        Ok(self.aux.get().expect("just set"))
    }

    /// Best-effort hot reload of a running gateway after a mutation.
    ///
    /// Reported rather than fatal: a gateway that is not running is a normal
    /// state on a server, and the change is already durable in SQLite. A
    /// *refusal* is called out separately from "not reachable", because it means
    /// the gateway is up but still routing the previous table — saying nothing,
    /// or saying "0 agents", would read as success.
    pub fn after_mutation(&mut self) {
        if !self.reload {
            return;
        }
        match sidecar::reload(&self.admin, self.token.as_deref()) {
            Some(v) if v["ok"].as_bool() == Some(true) => {
                self.out.note(format!(
                    "gateway route table reloaded ({} agents)",
                    v["agents_routed"].as_u64().unwrap_or(0)
                ));
            }
            Some(v) => self.out.note(format!(
                "note: gateway refused the reload ({}); it keeps routing the previous \
                 table until it is restarted",
                v["error"].as_str().unwrap_or("no reason given")
            )),
            None => self.out.note(format!(
                "note: gateway not reachable on {}; changes apply on next start",
                self.admin.describe()
            )),
        }
    }
}

/// Parse `argv` (without the program name) and run it, returning the exit code.
pub fn run_with(argv: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    // argv[0] is what clap prints in usage lines, so this has to be the real
    // binary name rather than the crate's.
    let parsed =
        Cli::try_parse_from(std::iter::once("kiwano".to_string()).chain(argv.iter().cloned()));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(e) => {
            // `--help` and `--version` arrive here too; clap says which stream
            // they belong on and whether they are a failure.
            if e.use_stderr() {
                let _ = write!(stderr, "{e}");
            } else {
                let _ = write!(stdout, "{e}");
            }
            return if e.use_stderr() { EXIT_USAGE } else { EXIT_OK };
        }
    };

    // `--admin-socket` wins, then `KIWANO_ADMIN_SOCKET` (applied by `from_env`),
    // then the default: a socket beside the database. An empty value counts as
    // "not given" rather than as an endpoint named "".
    let db = sidecar::db_path(cli.db.as_deref());
    let admin = match cli
        .admin_socket
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(value) => AdminEndpoint::parse(value),
        None => AdminEndpoint::from_env(&db),
    };
    let token = sidecar::admin_token_for(&db);
    // Same resolution the rest of the codebase uses, so a container with no
    // HOME set behaves the way the gateway does rather than failing.
    let home = cli.home.clone().unwrap_or_else(default_home);

    let mut ctx = Ctx {
        db,
        admin,
        token,
        data_port: cli.data_port,
        home,
        out: Out::new(stdout, stderr, cli.json, cli.quiet),
        reload: !cli.no_reload,
        store: OnceCell::new(),
        aux: OnceCell::new(),
    };

    match dispatch(&cli.command, &mut ctx) {
        Ok(code) => code,
        Err(e) => {
            ctx.out.error(&e.message);
            e.code
        }
    }
}

fn dispatch(command: &Command, ctx: &mut Ctx) -> Result<i32, CliError> {
    match command {
        Command::Status => cmds::status(ctx),
        Command::Reload => cmds::reload(ctx),
        Command::Providers(cmd) => {
            cmds::providers(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Keys(cmd) => {
            cmds::keys(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Usage(args) => {
            cmds::usage(args, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Agents(cmd) => {
            cmds::agents(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Routes(cmd) => {
            cmds::routes(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Logs(cmd) => {
            cmds::logs(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Dashboard(args) => {
            cmds::dashboard(args, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Alerts { mark_notified } => {
            cmds::alerts(*mark_notified, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Gateway(cmd) => {
            cmds::gateway(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Settings(cmd) => {
            cmds::settings(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Config(cmd) => {
            cmds::config(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Catalog(cmd) => {
            cmds::catalog(cmd, ctx)?;
            Ok(EXIT_OK)
        }
        Command::Import(cmd) => {
            cmds::import(cmd, ctx)?;
            Ok(EXIT_OK)
        }
    }
}

/// `$HOME`, or `.` when there is none — matching the gateway's own fallback so
/// a container without HOME set does not fail differently in each process.
fn default_home() -> PathBuf {
    std::env::var("HOME")
        .unwrap_or_else(|_| ".".to_string())
        .into()
}
