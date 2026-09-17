// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/error_mapper.rs
// Copied on 2026-09-07. Modified for Kiwano (unchanged; the trimmed local ProxyError keeps the same variant names).

//! Mapping of error types to HTTP status codes
//!
//! Maps ProxyError to the appropriate HTTP status code, for logging and
//! manually building error responses

use super::ProxyError;

/// Map a ProxyError to an HTTP status code
///
/// Mapping rules:
/// - Upstream error: use the status code returned by upstream as-is
/// - Timeout: 504 Gateway Timeout
/// - Connection failure: 502 Bad Gateway
/// - No available provider: 503 Service Unavailable
/// - Retries exhausted: 503 Service Unavailable
/// - Auth error: 401 Unauthorized
/// - Config/request error: 400 Bad Request
/// - Transform error: 422 Unprocessable Entity
/// - Other errors: 500 Internal Server Error
pub fn map_proxy_error_to_status(error: &ProxyError) -> u16 {
    match error {
        // Service state errors: consistent with IntoResponse
        ProxyError::AlreadyRunning => 409,
        ProxyError::NotRunning => 503,

        // Upstream error: use the actual status code
        ProxyError::UpstreamError { status, .. } => *status,

        // Timeout errors: 504 Gateway Timeout
        ProxyError::Timeout(_) | ProxyError::StreamIdleTimeout(_) => 504,

        // Forward/connection failure: 502 Bad Gateway
        ProxyError::ForwardFailed(_) => 502,

        // No available provider: 503 Service Unavailable
        ProxyError::NoAvailableProvider => 503,

        // All providers circuit-open: 503 Service Unavailable
        ProxyError::AllProvidersCircuitOpen => 503,

        // No providers configured: 503 Service Unavailable
        ProxyError::NoProvidersConfigured => 503,

        // Retries exhausted: 503 Service Unavailable
        ProxyError::MaxRetriesExceeded => 503,

        // Unhealthy provider: 503 Service Unavailable
        ProxyError::ProviderUnhealthy(_) => 503,

        // Config error/invalid request: 400 Bad Request
        ProxyError::ConfigError(_) | ProxyError::InvalidRequest(_) => 400,

        // Auth error: 401 Unauthorized
        ProxyError::AuthError(_) => 401,

        // Database error: 500 Internal Server Error
        ProxyError::DatabaseError(_) => 500,

        // Transform error: 422 Unprocessable Entity
        ProxyError::TransformError(_) => 422,

        // Other unknown errors: 500 Internal Server Error
        _ => 500,
    }
}

/// Convert a ProxyError into a user-friendly error message
pub fn get_error_message(error: &ProxyError) -> String {
    match error {
        ProxyError::UpstreamError { status, body } => {
            if let Some(body) = body {
                format!("upstream error ({status}): {body}")
            } else {
                format!("upstream error ({status})")
            }
        }
        ProxyError::Timeout(msg) => format!("request timed out: {msg}"),
        ProxyError::ForwardFailed(msg) => format!("cannot forward: {msg}"),
        ProxyError::NoAvailableProvider => "no provider available".to_string(),
        ProxyError::AllProvidersCircuitOpen => {
            "every provider is circuit-broken; no channel is left".to_string()
        }
        ProxyError::NoProvidersConfigured => "no provider is configured".to_string(),
        ProxyError::MaxRetriesExceeded => {
            "every provider failed and the retries are spent".to_string()
        }
        ProxyError::ProviderUnhealthy(msg) => format!("provider is not healthy: {msg}"),
        ProxyError::DatabaseError(msg) => format!("database error: {msg}"),
        ProxyError::TransformError(msg) => format!("cannot convert the request or response: {msg}"),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_upstream_error() {
        let error = ProxyError::UpstreamError {
            status: 401,
            body: Some("Unauthorized".to_string()),
        };
        assert_eq!(map_proxy_error_to_status(&error), 401);
    }

    #[test]
    fn test_map_timeout_error() {
        let error = ProxyError::Timeout("Request timeout".to_string());
        assert_eq!(map_proxy_error_to_status(&error), 504);
    }

    #[test]
    fn test_map_connection_error() {
        let error = ProxyError::ForwardFailed("Connection refused".to_string());
        assert_eq!(map_proxy_error_to_status(&error), 502);
    }

    #[test]
    fn test_map_no_provider_error() {
        let error = ProxyError::NoAvailableProvider;
        assert_eq!(map_proxy_error_to_status(&error), 503);
    }

    #[test]
    fn test_map_status_matches_proxy_error_response_semantics() {
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::AuthError("bad token".to_string())),
            401
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::ConfigError("bad config".to_string())),
            400
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::InvalidRequest("bad request".to_string())),
            400
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::TransformError("bad transform".to_string())),
            422
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::StreamIdleTimeout(30)),
            504
        );
    }

    #[test]
    fn test_get_error_message() {
        let error = ProxyError::UpstreamError {
            status: 500,
            body: Some("Internal Server Error".to_string()),
        };
        let msg = get_error_message(&error);
        assert!(msg.contains("upstream error"));
        assert!(msg.contains("500"));
        assert!(msg.contains("Internal Server Error"));
    }
}
