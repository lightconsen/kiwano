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
