// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/error.rs
// Copied on 2026-09-07. Modified for Kiwano (kept as a lightweight conversion
// error: the axum `IntoResponse` impl and the reqwest-based `categorize_error`
// helper were dropped; variant names preserved so error_mapper.rs stays
// unchanged).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("the upstream response body is over the size limit: {0} bytes")]
    ResponseBodyTooLarge(usize),

    #[error("the server is already running")]
    AlreadyRunning,

    #[error("the server is not running")]
    NotRunning,

    #[error("cannot bind the address: {0}")]
    BindFailed(String),

    #[error("timed out stopping the server")]
    StopTimeout,

    #[error("cannot stop the server: {0}")]
    StopFailed(String),

    #[error("cannot forward the request: {0}")]
    ForwardFailed(String),

    #[error("no provider available")]
    NoAvailableProvider,

    #[error("every provider is circuit-broken; no channel is left")]
    AllProvidersCircuitOpen,

    #[error("no provider is configured")]
    NoProvidersConfigured,

    #[allow(dead_code)]
    #[error("provider is not healthy: {0}")]
    ProviderUnhealthy(String),

    #[error("upstream error (status {status}): {body:?}")]
    UpstreamError { status: u16, body: Option<String> },

    #[error("the retry limit was reached")]
    MaxRetriesExceeded,

    #[error("database error: {0}")]
    DatabaseError(String),

    #[error("config error: {0}")]
    ConfigError(String),

    #[allow(dead_code)]
    #[error("cannot convert between formats: {0}")]
    TransformError(String),

    #[allow(dead_code)]
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("timed out: {0}")]
    Timeout(String),

    /// Streaming response idle timeout
    #[allow(dead_code)]
    #[error("the stream went idle: no data for {0}s")]
    StreamIdleTimeout(u64),

    /// Auth error
    #[error("authentication failed: {0}")]
    AuthError(String),

    #[allow(dead_code)]
    #[error("internal error: {0}")]
    Internal(String),
}

/// Error category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Retryable errors (network issues, 5xx)
    Retryable, // network timeouts, 5xx errors
    /// Non-retryable errors (4xx, auth failures)
    NonRetryable, // auth failures, bad parameters, 4xx errors
    #[allow(dead_code)]
    ClientAbort, // client-initiated abort
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same guard as the adapter errors': this enum is ported, its messages reach
    /// request logs and the UI, and the codebase's rule is English.
    #[test]
    fn the_error_messages_are_english() {
        let errors = [
            ProxyError::ResponseBodyTooLarge(1),
            ProxyError::AlreadyRunning,
            ProxyError::NotRunning,
            ProxyError::BindFailed("x".into()),
            ProxyError::StopTimeout,
            ProxyError::StopFailed("x".into()),
            ProxyError::ForwardFailed("x".into()),
            ProxyError::NoAvailableProvider,
            ProxyError::AllProvidersCircuitOpen,
            ProxyError::NoProvidersConfigured,
            ProxyError::ProviderUnhealthy("x".into()),
            ProxyError::MaxRetriesExceeded,
            ProxyError::DatabaseError("x".into()),
            ProxyError::ConfigError("x".into()),
            ProxyError::TransformError("x".into()),
            ProxyError::InvalidRequest("x".into()),
            ProxyError::Timeout("x".into()),
            ProxyError::StreamIdleTimeout(1),
            ProxyError::AuthError("x".into()),
            ProxyError::Internal("x".into()),
        ];
        for error in errors {
            let message = error.to_string();
            assert!(
                !message
                    .chars()
                    .any(|c| matches!(c, '\u{4e00}'..='\u{9fff}')),
                "{message:?} is not English"
            );
        }
    }
}
