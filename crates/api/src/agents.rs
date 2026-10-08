//! User-defined agents: the wire shape of one.

use serde::{Deserialize, Serialize};

/// A user-defined agent's view: a name for a route, the key its traffic is
/// attributed by, and nothing else — there is no config file to report on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAgentVm {
    pub id: String,
    pub label: String,
    pub note: Option<String>,
    /// Always present: the key is minted with the agent and deleted with it, so
    /// unlike a built-in's (read out of its live config), this one cannot be
    /// stale — the row *is* the truth here, and it is the same row the gateway
    /// attributes by.
    pub placeholder_key: Option<String>,
    /// What this agent's clients speak, chosen when it was defined. `None` for
    /// one defined before the field existed — "not said", which is why the UI
    /// reads it as such rather than showing a protocol nobody picked.
    pub protocol: Option<String>,
}
