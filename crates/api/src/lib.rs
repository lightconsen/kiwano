//! The wire types: what a Kiwano client and the daemon agree on.
//!
//! # Why this crate exists
//!
//! Until the API migration, the "wire" between the app, the CLI and the daemon
//! was a *function call on shared types*: all three linked `kiwano-core`, so
//! the compiler checked both sides and no shape could drift. A separated
//! daemon breaks that — and worse, `crates/gateway` does not depend on
//! `kiwano-core` (it is the other way round), so the daemon **could not name
//! the types it is supposed to serve**. Every command was unmovable until the
//! types had a home both sides can see.
//!
//! That is this crate. It depends on **nothing local** — only serde — which is
//! the property worth keeping: if something here ever needs `kiwano-core` or
//! `kiwanod`, it is not a wire type and does not belong here.
//!
//! # What belongs here, and what does not
//!
//! The rule is *which side serves it*, not whether it ends in `Vm`:
//!
//! - **Here**: the inputs and outputs of commands the daemon will serve —
//!   batch 1's pure-state commands, batch 2's probes, and the state half of
//!   batch 3's mixed ones (`migrate.local.md` §7).
//! - **Not here**: anything whose command stays on the client forever — the
//!   detect family, the updater, `open_url` / `open_log_folder`,
//!   `verify_agent_dir`. Those describe *this machine*, the daemon will never
//!   serve them, and moving them would suggest otherwise.
//! - **Not here either**: the rest of the API's own envelopes (request
//!   wrappers, the version range). Those are the daemon's and the client's to
//!   define together, and nothing needs them yet — writing them now would be
//!   guessing at a shape nothing has needed.
//!
//! The **error** body was on that list until the first endpoint needed it
//! (`error.rs`): the daemon has to pick an HTTP status, and it cannot pick one
//! from a sentence. It is here now for the same reason the rest of the crate
//! is — both sides read it, and neither may depend on the other.
//!
//! # The derives are deliberately less than complete
//!
//! `Serialize` only, as they were in `kiwano-core`. A client that *sends* one of
//! these needs `Deserialize`, and fixtures need both — that arrives with the
//! client, not before, so that what is added is added by something that needs
//! it. `Debug` and `Clone` are here because they cost nothing and their absence
//! was already awkward (a `Result<T, E>` whose `T` is not `Debug` cannot be
//! unwrapped on the error side).

pub mod agents;
pub mod error;
pub mod ids;
pub mod keys;
pub mod logo;
pub mod providers;
pub mod routes;
pub mod settings;
