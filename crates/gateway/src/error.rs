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
