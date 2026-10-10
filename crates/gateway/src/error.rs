//! Shared error type for the Kiwano gateway.

/// Errors produced by the gateway (store, router, proxy, servers).
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("store error: {0}")]
    Store(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("no enabled provider bound for agent `{0}`")]
    NoBinding(String),

    /// Every provider this agent could use is over a billing limit. Distinct
    /// from `NoBinding`: something is bound, it just must not be spent on.
    #[error("every provider for agent `{agent}` is over its limit: {reasons}")]
    AllOverLimit { agent: String, reasons: String },

    /// The agent has spent its own allowance for the period. Distinct from
    /// `AllOverLimit`, which is about a provider's ceiling: here the agent's
    /// limit is spent, and no provider choice would change that — spending more
    /// elsewhere is exactly what the limit exists to prevent.
    #[error("agent `{agent}` is over its own limit ({reason})")]
    AgentOverLimit { agent: String, reason: String },

    /// The credential has spent its own allowance. A third limit and a third
    /// refusal, because it is the caller's own contract rather than anything
    /// about the agent or a provider: another key for the same agent would be
    /// still be fine, which is exactly what makes it worth naming.
    ///
    /// `retry_after_secs` is the wait until the window it tripped ends, on the
    /// user's clock, and rides the error because only the evaluator knows which
    /// window tripped.
    #[error("client key `{key_id}` is over its own limit ({reason})")]
    ClientOverLimit {
        /// The agent the key routes as, so the refusal is attributed in the log
        /// like every other one. The key is what tripped, but a reader looking
        /// for "what is failing" is looking by agent.
        agent: String,
        key_id: String,
        reason: String,
        retry_after_secs: Option<u64>,
    },

    /// The credential may not name this model. Refused before the upstream is
    /// asked, which is the only place an allowlist can be enforced — after the
    /// call it would be a report rather than a restriction.
    #[error("client key for agent `{agent}` may not use model `{model}` (allowed: {})", allowed.join(", "))]
    ModelNotAllowed {
        agent: String,
        model: String,
        allowed: Vec<String>,
    },

    /// The key carries a model allowlist and the request's model could not be
    /// read. Refused rather than waved through: an allowance that cannot be
    /// checked is not an allowance, and the other reading makes an unparseable
    /// body a way around the list.
    #[error("client key for agent `{agent}` allows only some models and this request names none")]
    ModelUnverifiable { agent: String },

    /// The credential may not use any provider this route offers. The provider
    /// counterpart of `ModelNotAllowed`, and equally terminal.
    #[error("client key for agent `{agent}` may not use any of the providers bound to it (allowed: {})", allowed.join(", "))]
    ProviderNotAllowed { agent: String, allowed: Vec<String> },

    /// The provider's breaker is not admitting requests: it is open, or another
    /// request already holds the single HalfOpen probe permit. Distinct from
    /// `Upstream` on purpose — nothing was sent, and sending again right now is
    /// the one thing the breaker exists to prevent.
    #[error("provider `{provider}` is not admitting requests for agent `{agent}` (circuit open)")]
    CircuitOpen { agent: String, provider: String },

    #[error("provider `{0}` not found")]
    ProviderNotFound(String),

    #[error("unsupported inbound path `{0}`")]
    UnsupportedPath(String),

    /// The inbound request carried no placeholder key, or one this gateway did
    /// not mint, so its agent is unknown. Refused rather than guessed: the
    /// guess would have been forwarded on the operator's upstream credentials.
    #[error("kiwanod: {0}; only agents taken over by Kiwano are routed through this gateway")]
    Unauthorized(String),

    #[error("upstream request failed: {0}")]
    Upstream(String),

    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;
