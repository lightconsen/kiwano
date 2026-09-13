//! The command tree.
//!
//! Globals are `global = true` so they stay position-independent: `kiwano
//! providers list --json` and `kiwano --json providers list` are the same
//! command. The hand-rolled parser accepted them anywhere and scripts in the
//! wild are written both ways, so this is a compatibility requirement rather
//! than a style choice.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "kiwano",
    version,
    about = "Manage Kiwano providers without the desktop app",
    long_about = "Manage Kiwano providers without the desktop app.\n\n\
        Talks straight to the shared SQLite store (the same file the gateway and \
        the desktop app use) and to the gateway's admin plane for status and hot \
        reload, so it works on a headless server.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// SQLite file (default ~/.kiwano/kiwano.db, env KIWANO_DB_PATH)
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// Where the gateway admin plane is: a socket file on unix, a pipe name on
    /// Windows (default: admin.sock beside the database, env KIWANO_ADMIN_SOCKET)
    #[arg(long, global = true, value_name = "TARGET")]
    pub admin_socket: Option<String>,

    /// Machine-readable output on stdout
    #[arg(long, global = true)]
    pub json: bool,

    /// Do not ask a running gateway to reload after a change
    #[arg(long, global = true)]
    pub no_reload: bool,

    /// Suppress informational notes
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Port an agent's config is pointed at when it is taken over
    #[arg(
        long,
        global = true,
        value_name = "PORT",
        env = "KIWANO_DATA_PORT",
        default_value_t = 8317
    )]
    pub data_port: u16,

    /// Root the agent config files live under (default $HOME)
    #[arg(long, global = true, value_name = "PATH")]
    pub home: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Gateway, store and route summary (exit 1 when the gateway is down)
    Status,

    /// Ask a running gateway to hot-reload its route table
    Reload,

    /// Providers: add, inspect, bind, delete
    #[command(subcommand)]
    Providers(ProvidersCmd),

    /// A provider's rotating API keys
    #[command(subcommand)]
    Keys(KeysCmd),

    /// Usage totals
    Usage(UsageArgs),

    /// Coding agents: detect them, and route them through the gateway
    #[command(subcommand)]
    Agents(AgentsCmd),

    /// Agent routes: strategy and candidate order
    #[command(subcommand)]
    Routes(RoutesCmd),

    /// Request logs: what the gateway routed, and what it cost
    #[command(subcommand)]
    Logs(LogsCmd),

    /// Requests, tokens, cost and latency over a window
    Dashboard(DashboardArgs),

    /// Providers that have spent their allowance for the current period
    Alerts {
        /// Record the alert as delivered, which suppresses the desktop
        /// notification. Off by default so a poll cannot eat the user's alert.
        #[arg(long)]
        mark_notified: bool,
    },

    /// The gateway daemon
    #[command(subcommand)]
    Gateway(GatewayCmd),
}

#[derive(Debug, Subcommand)]
pub enum LogsCmd {
    /// List requests, newest first
    List(LogsListArgs),

    /// One request in full, including bodies
    Show {
        /// The row id shown by `logs list`
        id: i64,
    },

    /// Write the matching requests to a CSV file
    Export(LogsExportArgs),

    /// Delete every logged request
    Clear {
        /// Required: this deletes the whole log, not a filtered slice
        #[arg(long)]
        yes: bool,
    },

    /// Print where the gateway keeps its own log files
    Dir,
}

#[derive(Debug, Args)]
pub struct LogsListArgs {
    #[command(flatten)]
    pub filter: LogFilterArgs,

    #[arg(long, default_value_t = 1, value_name = "N")]
    pub page: i64,

    #[arg(long, default_value_t = 20, value_name = "N")]
    pub page_size: i64,
}

#[derive(Debug, Args)]
pub struct LogsExportArgs {
    /// Where to write the CSV. The file is the payload, so this is required.
    #[arg(long, value_name = "PATH")]
    pub out: PathBuf,

    /// Include the request and response bodies (appends two columns)
    #[arg(long)]
    pub include_bodies: bool,

    #[command(flatten)]
    pub filter: LogFilterArgs,
}

/// The shared log filter. `--from` is inclusive and `--to` exclusive, matching
/// the store's half-open range.
#[derive(Debug, Args)]
pub struct LogFilterArgs {
    #[arg(long, value_name = "AGENT")]
    pub agent: Option<String>,

    #[arg(long, value_name = "PROVIDER")]
    pub provider: Option<String>,

    /// ok | error
    #[arg(long, value_name = "STATUS")]
    pub status: Option<String>,

    /// RFC3339, inclusive
    #[arg(long, value_name = "RFC3339")]
    pub from: Option<String>,

    /// RFC3339, exclusive
    #[arg(long, value_name = "RFC3339")]
    pub to: Option<String>,
}

#[derive(Debug, Args)]
pub struct DashboardArgs {
    /// today | 7d | 30d
    #[arg(long, default_value = "7d", value_name = "WINDOW")]
    pub window: String,

    #[arg(long, value_name = "PROVIDER")]
    pub provider: Option<String>,

    #[arg(long, value_name = "AGENT")]
    pub agent: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum GatewayCmd {
    /// Start a gateway, adopting one that is already running
    Start,

    /// Ask the running gateway to stop
    Stop,

    /// Stop the running gateway and start a fresh one
    Restart,
}

#[derive(Debug, Subcommand)]
pub enum RoutesCmd {
    /// Every agent's strategy and candidate order
    List,

    /// Set an agent's selection strategy
    Strategy {
        agent: String,

        /// single | failover | roundrobin | timewindow | quota
        kind: String,

        /// quota: the threshold value
        #[arg(long, value_name = "N")]
        limit: Option<f64>,

        /// quota: what the threshold counts
        #[arg(long, value_name = "UNIT", default_value = "requests")]
        unit: String,
    },

    /// Set the candidate order; the argument order becomes priority 0..n
    Reorder {
        agent: String,

        #[arg(required = true, value_name = "PROVIDER_ID")]
        provider_ids: Vec<String>,
    },

    /// Copy another agent's strategy and candidate order onto this one
    Apply {
        #[arg(long, value_name = "SOURCE")]
        from: String,

        #[arg(long, value_name = "TARGET")]
        to: String,
    },

    /// One candidate binding
    #[command(subcommand)]
    Binding(BindingCmd),
}

#[derive(Debug, Subcommand)]
pub enum BindingCmd {
    /// Add a candidate at the tail of the queue
    Add { agent: String, provider_id: String },

    /// Remove a candidate
    Remove { agent: String, provider_id: String },

    /// Change a candidate's roundrobin weight or timewindow window
    Set {
        agent: String,
        provider_id: String,

        /// roundrobin: relative weight (>= 1)
        #[arg(long, value_name = "N")]
        weight: Option<i64>,

        /// timewindow: local window, HH:MM-HH:MM
        #[arg(long, value_name = "HH:MM-HH:MM")]
        window: Option<String>,

        /// timewindow: drop the window (matches any hour)
        #[arg(long, conflicts_with = "window")]
        no_window: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProbeCmd {
    /// TCP connect time to an endpoint
    Latency { endpoint: String },

    /// Ask an endpoint whether it answers, and how
    Endpoint {
        #[arg(long, value_name = "PROTOCOL")]
        protocol: String,

        #[arg(long, value_name = "URL")]
        endpoint: String,

        /// Optional: without one, 401/403 still proves the route exists
        #[arg(long, value_name = "KEY")]
        key: Option<String>,
    },

    /// The model ids an endpoint advertises (needs a key)
    Models {
        #[arg(long, value_name = "PROTOCOL")]
        protocol: String,

        #[arg(long, value_name = "URL")]
        endpoint: String,

        #[arg(long, value_name = "KEY")]
        key: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentsCmd {
    /// Which agents are installed on this machine
    Detect,

    /// Version strings for the installed CLI agents
    ///
    /// Slow by construction: one `--version` subprocess per agent, each through
    /// the login shell.
    Versions,

    /// Route an agent through the local gateway, backing up its config first
    ///
    /// Mints the agent's placeholder key if it has none — without it the
    /// gateway refuses the agent's requests, because it only routes keys it
    /// issued itself.
    Takeover { agent: String },

    /// Restore an agent's configuration from its takeover backup
    Restore { agent: String },
}

#[derive(Debug, Subcommand)]
pub enum ProvidersCmd {
    /// List providers
    List {
        /// Only providers bound to this agent
        #[arg(long, value_name = "AGENT")]
        agent: Option<String>,
    },

    /// Add a provider
    Add(Box<AddArgs>),

    /// Make a provider the primary for an agent (switches it to single strategy)
    Use {
        provider_id: String,
        #[arg(long, value_name = "AGENT")]
        agent: String,
    },

    /// Delete a provider; a bound agent promotes its next candidate
    Remove { provider_id: String },

    /// Change a provider in place, keeping its id — and so every binding to it
    Edit(Box<EditArgs>),

    /// Make a provider the current route of every agent bound to it
    ///
    /// Unlike `use`, this does not touch the strategy: it reorders candidates
    /// under whatever the agent already has configured.
    Enable { provider_id: String },

    /// Ask an endpoint whether it answers, before committing to it
    #[command(subcommand)]
    Probe(ProbeCmd),
}

/// Every field optional: only what is given is changed. Absent means "keep",
/// which is why this is a read-modify-write against the stored row rather than
/// a fresh `NewProviderInput`.
#[derive(Debug, Args)]
pub struct EditArgs {
    pub provider_id: String,

    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    #[arg(long, value_name = "URL")]
    pub endpoint: Option<String>,

    /// New upstream key. Omit to keep the stored one.
    #[arg(long, value_name = "KEY")]
    pub key: Option<String>,

    #[arg(long, value_name = "PROTOCOL")]
    pub protocol: Option<String>,

    #[arg(long, value_name = "BILLING")]
    pub billing: Option<String>,

    #[arg(long, value_name = "N")]
    pub limit: Option<f64>,

    #[arg(long, value_name = "UNIT")]
    pub unit: Option<String>,

    #[arg(long, value_name = "PERIOD")]
    pub reset: Option<String>,

    /// Replace the bound agents with exactly these (repeatable).
    /// Omit to leave the current set alone.
    #[arg(long = "bind", value_name = "AGENT")]
    pub bind: Vec<String>,

    /// Unbind from every agent
    #[arg(long, conflicts_with = "bind")]
    pub no_bind: bool,
}

/// The flags `providers add` accepts. They map onto the app's
/// `NewProviderInput`, which is why the vocabulary is the app's (`plan`/`payg`/
/// `unl`) rather than the older CLI's — the store rows the two write are then
/// identical.
#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(long, value_name = "NAME")]
    pub name: String,

    #[arg(long, value_name = "URL")]
    pub endpoint: String,

    /// Upstream API key. Omitted means the provider has none yet.
    #[arg(long, value_name = "KEY")]
    pub key: Option<String>,

    /// anthropic | openai | gemini
    #[arg(long, default_value = "openai", value_name = "PROTOCOL")]
    pub protocol: String,

    /// plan | payg | unl (subscription | metered | unlimited also accepted)
    #[arg(long, default_value = "payg", value_name = "BILLING")]
    pub billing: String,

    /// Spending cap for `--billing payg`. Rejected for plan providers, whose
    /// quota is tracked from the plan query instead.
    #[arg(long, value_name = "N")]
    pub limit: Option<f64>,

    /// Unit of `--limit`: requests | wan_tokens | a 3-letter currency code
    #[arg(long, value_name = "UNIT")]
    pub unit: Option<String>,

    /// Reset cycle of `--limit`: monthly | weekly | yearly | none
    #[arg(long, value_name = "PERIOD")]
    pub reset: Option<String>,

    /// Bind the new provider as the primary for this agent (repeatable)
    #[arg(long = "bind", value_name = "AGENT")]
    pub bind: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum KeysCmd {
    /// List a provider's rotating keys (never printed in full)
    List { provider_id: String },

    /// Add a rotating key
    Add {
        provider_id: String,
        #[arg(long, value_name = "KEY")]
        key: String,
        #[arg(long, value_name = "LABEL")]
        label: Option<String>,
    },

    /// Delete a rotating key by its numeric id
    Remove { key_id: i64 },
}

#[derive(Debug, Args)]
pub struct UsageArgs {
    /// Window size in days
    #[arg(long, default_value_t = 7, value_name = "DAYS")]
    pub days: i64,

    /// Only this agent
    #[arg(long, value_name = "AGENT")]
    pub agent: Option<String>,
}
