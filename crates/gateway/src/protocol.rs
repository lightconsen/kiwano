//! Inbound path → protocol classification (tech.md §4.6).
//!
//! Paths are stable and non-overlapping across agent families, so the URL
//! alone decides which protocol family a request speaks. `GET /v1/models`
//! exists in both Anthropic and OpenAI flavors, so it is classified as
//! ambiguous and resolved through the agent attribution instead.

use crate::store::Protocol;

/// Classification result for an inbound request path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProtocol {
    /// Path maps to exactly one protocol family.
    Fixed(Protocol),
    /// Path exists in both families (`GET /v1/models`); resolve via agent.
    Ambiguous,
    /// Not a supported gateway path.
    Unknown,
}

/// Classify an inbound request path (query string excluded).
pub fn classify_path(path: &str) -> PathProtocol {
    if path == "/v1/messages" || path.starts_with("/v1/messages/") || path == "/v1/complete" {
        PathProtocol::Fixed(Protocol::Anthropic)
    } else if path == "/v1/chat/completions"
        || path == "/v1/responses"
        || path.starts_with("/v1/responses/")
        || path == "/v1/completions"
        || path == "/v1/embeddings"
    {
        PathProtocol::Fixed(Protocol::OpenAI)
    } else if path == "/v1/models" {
        PathProtocol::Ambiguous
    } else {
        PathProtocol::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_anthropic_paths() {
        assert_eq!(
            classify_path("/v1/messages"),
            PathProtocol::Fixed(Protocol::Anthropic)
        );
        assert_eq!(
            classify_path("/v1/messages/count_tokens"),
            PathProtocol::Fixed(Protocol::Anthropic)
        );
        assert_eq!(
            classify_path("/v1/complete"),
            PathProtocol::Fixed(Protocol::Anthropic)
        );
    }

    #[test]
    fn classifies_openai_paths() {
        assert_eq!(
            classify_path("/v1/chat/completions"),
            PathProtocol::Fixed(Protocol::OpenAI)
        );
        assert_eq!(
            classify_path("/v1/responses"),
            PathProtocol::Fixed(Protocol::OpenAI)
        );
        assert_eq!(
            classify_path("/v1/responses/input"),
            PathProtocol::Fixed(Protocol::OpenAI)
        );
        assert_eq!(
            classify_path("/v1/completions"),
            PathProtocol::Fixed(Protocol::OpenAI)
        );
        assert_eq!(
            classify_path("/v1/embeddings"),
            PathProtocol::Fixed(Protocol::OpenAI)
        );
    }

    #[test]
    fn classifies_ambiguous_and_unknown_paths() {
        assert_eq!(classify_path("/v1/models"), PathProtocol::Ambiguous);
        assert_eq!(classify_path("/v1/unknown"), PathProtocol::Unknown);
        assert_eq!(classify_path("/"), PathProtocol::Unknown);
        assert_eq!(classify_path("/v1/messagesbogus"), PathProtocol::Unknown);
    }
}
