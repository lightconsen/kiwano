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
    } else if path == "/v1beta/models" || path.starts_with("/v1beta/models/") {
        // Gemini API (Gemini CLI): the `GET /v1beta/models` list plus
        // `POST /v1beta/models/{model}:{method}` — generateContent,
        // streamGenerateContent, countTokens — where the `{model}:{method}`
        // segment is a single path component.
        PathProtocol::Fixed(Protocol::Gemini)
    } else if path == "/v1/models" {
        PathProtocol::Ambiguous
    } else {
        PathProtocol::Unknown
    }
}

/// Which of the OpenAI family's two wires an inbound path speaks.
///
/// The family is decided by [`classify_path`], and it is not enough to route by:
/// `/v1/chat/completions` and `/v1/responses` are different request and response
/// shapes, so a provider that serves one and not the other needs the difference
/// named before a request is sent to it (`forward::inbound`).
///
/// `None` for anything that is not an OpenAI-family path. The router wires only
/// `POST /v1/responses`, so the `GET`/`{id}` forms of the Responses API do not
/// reach this — and if one did, it would read as `Responses` here and be refused
/// downstream by the converter, which is the right answer for a shape nothing
/// can convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiWirePath {
    Chat,
    Responses,
}

pub fn openai_wire_of_path(path: &str) -> Option<OpenAiWirePath> {
    if path == "/v1/responses" || path.starts_with("/v1/responses/") {
        Some(OpenAiWirePath::Responses)
    } else if path == "/v1/chat/completions" {
        Some(OpenAiWirePath::Chat)
    } else {
        // `/v1/completions` and `/v1/embeddings` are OpenAI-family paths that are
        // neither wire: they have no Responses shape at all, so there is nothing
        // to convert and nothing to refuse — they pass through as they always did.
        None
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

    // Gemini CLI's native API: the model and the action share one path
    // component (`{model}:{method}`), plus the bare model list.
    #[test]
    fn classifies_gemini_paths() {
        assert_eq!(
            classify_path("/v1beta/models"),
            PathProtocol::Fixed(Protocol::Gemini)
        );
        assert_eq!(
            classify_path("/v1beta/models/gemini-2.5-pro:generateContent"),
            PathProtocol::Fixed(Protocol::Gemini)
        );
        assert_eq!(
            classify_path("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
            PathProtocol::Fixed(Protocol::Gemini)
        );
        assert_eq!(
            classify_path("/v1beta/models/gemini-2.5-pro:countTokens"),
            PathProtocol::Fixed(Protocol::Gemini)
        );
        // A version prefix alone is not enough: the shape is `/v1beta/models…`.
        assert_eq!(classify_path("/v1beta/corpora"), PathProtocol::Unknown);
    }
}
