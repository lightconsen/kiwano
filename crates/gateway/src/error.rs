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

    #[error("provider `{0}` not found")]
    ProviderNotFound(String),

    #[error("unsupported inbound path `{0}`")]
    UnsupportedPath(String),

    #[error("upstream request failed: {0}")]
    Upstream(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;
