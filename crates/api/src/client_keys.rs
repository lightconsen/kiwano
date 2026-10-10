//! Client-key wire types: the data plane's credential, as a client manages it.
//!
//! Two shapes, and the split is the point. [`ClientKeyVm`] is what a *read*
//! returns, and it carries a **masked** key: a management surface never needs
//! the plaintext of a key that is already live, and every copy of it that is not
//! handed out is one less copy sitting in a webview heap or a shell history.
//! [`ClientKeyCreatedVm`] is the one thing that ever carries the secret, and it
//! is returned by exactly the two operations that mint one (`add`, `rotate`) —
//! the only moments a human has to copy it into an agent's config.
//!
//! The **handle** (`id`, `ck-…`) is what every other operation names a key by:
//! usage and request rows record it, the admin routes are addressed by it, and
//! it survives a rotation. A credential is something a client presents; asking an
//! operator to name a row by the secret is how secrets end up in logs.

use serde::{Deserialize, Serialize};

/// One window of a client key's spend ceiling.
///
/// Deliberately the same three fields as the agent's ceiling
/// (`routes::AgentLimitVm`): the same units, measured against the same period
/// boundaries, so "100 requests a day" means the same thing wherever it is
/// written. What differs is whose it is — this one is the credential's, so two
/// keys of one agent hold it independently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientKeyLimitVm {
    /// `day` | `weekly` | `monthly` | `yearly` | `all`.
    pub period: String,
    pub period_limit: f64,
    /// `requests` (default), `wan_tokens`, or a 3-letter currency code.
    pub limit_unit: Option<String>,
}

/// A client key as the CLI lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientKeyVm {
    /// The handle, and the only name any operation takes.
    pub id: String,
    /// The agent this key routes as. One key names one agent — routing needs it —
    /// while a second key for the same agent is a second budget.
    pub agent: String,
    /// A human name for the client ("office laptop"), free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Display form of the secret (`mask_key`), never a usable credential.
    pub masked: String,
    /// Models this key may name; empty means no restriction.
    #[serde(default)]
    pub model_allow: Vec<String>,
    /// Providers this key may use; empty means no restriction.
    #[serde(default)]
    pub provider_allow: Vec<String>,
    #[serde(default)]
    pub limits: Vec<ClientKeyLimitVm>,
    pub created_at: String,
    /// When this key last carried a request, RFC3339 — `None` when it never has.
    ///
    /// The two states are different and a reader has to render them differently:
    /// a key minted a minute ago and a key minted a year ago have both "never
    /// been used", and neither is a key that stopped being used. Derived from the
    /// metered rows rather than stamped on the key, so nothing writes to the key's
    /// row on the request path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
}

/// The one response that carries a secret: the result of minting one.
///
/// Not a [`ClientKeyVm`] with an extra field, because the two are different
/// promises: a reader of this type knows it is holding something that must be
/// copied now and never read back, and a function that returns it is announcing
/// that it creates credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientKeyCreatedVm {
    pub id: String,
    /// The secret. Printed once, on the operation that made it.
    pub key: String,
    pub agent: String,
}

/// Mint a key for an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewClientKeyInput {
    pub agent: String,
    /// Empty means no restriction, and it is also what clearing writes — nothing
    /// stores an empty list, because a stored `[]` would read as "allow nothing".
    #[serde(default)]
    pub model_allow: Vec<String>,
    #[serde(default)]
    pub provider_allow: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub limits: Vec<ClientKeyLimitVm>,
}

/// Edit everything about a key except its secret: its name, and what it may use.
///
/// One call for both lists rather than one per list, because the caller holds
/// both — the same reason `replace_*_limits` is a set-replace. A window set is
/// edited by its own command (`limits`), since a set with mixed units is a
/// different shape to type on a command line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientKeyPolicyInput {
    #[serde(default)]
    pub model_allow: Vec<String>,
    #[serde(default)]
    pub provider_allow: Vec<String>,
    /// Absent leaves the label alone; `Some("")` clears it.
    #[serde(default)]
    pub label: Option<String>,
}
