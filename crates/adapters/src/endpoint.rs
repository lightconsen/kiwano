//! Splitting a provider endpoint into the two parts the gateway stores.
//!
//! The gateway composes an upstream URL as `base_url + api_path + client path`,
//! and every path it forwards already begins with its version segment —
//! `/v1/messages`, `/v1/chat/completions`, `/v1/responses`. So an endpoint that
//! spells the version too (`https://api.commandcode.ai/provider/v1`, which is
//! how the Hub publishes OpenAI endpoints) would be composed twice:
//! `…/provider/v1/v1/chat/completions` — a 404 from the vendor.
//!
//! The rule here is one sentence: **drop a trailing `/v1`; the rest of the path
//! becomes `api_path`.**
//!
//! ```text
//! https://api.commandcode.ai/provider/v1 → (https://api.commandcode.ai, "/provider")
//! https://api.groq.com/openai/v1         → (https://api.groq.com,        "/openai")
//! https://api.deepseek.com/anthropic     → (https://api.deepseek.com,    "/anthropic")
//! https://api.deepseek.com               → (https://api.deepseek.com,    None)
//! ```
//!
//! Used by both doors an endpoint comes through — the provider form and the
//! cc-switch importer — so the two cannot store the same URL differently.

/// Split `url` into the stored `(base_url, api_path)` pair.
///
/// A URL with no scheme is returned whole with no path: the caller's
/// `absolute_endpoint` supplies the scheme, and guessing at a path here would
/// turn a typo into a different endpoint.
pub fn split_endpoint(url: &str) -> (String, Option<String>) {
    let url = url.trim().trim_end_matches('/');
    let Some(scheme_end) = url.find("://") else {
        return (url.to_string(), None);
    };
    let rest = &url[scheme_end + 3..];
    let Some(slash) = rest.find('/') else {
        return (url.to_string(), None);
    };
    let base = format!("{}://{}", &url[..scheme_end], &rest[..slash]);
    let path = rest[slash..].trim_end_matches('/');
    let path = trim_version(path);
    (base, (!path.is_empty()).then(|| path.to_string()))
}

/// Drop one trailing `/v1` — the segment the client's own path repeats.
///
/// Case-insensitive because a hand-typed endpoint may spell it `V1`, and the
/// comparison is about the segment's meaning rather than its case.
fn trim_version(path: &str) -> &str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with("/v1") {
        &path[..path.len() - 3]
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_the_version_segment_the_client_path_repeats() {
        assert_eq!(
            split_endpoint("https://api.commandcode.ai/provider/v1"),
            (
                "https://api.commandcode.ai".to_string(),
                Some("/provider".to_string())
            )
        );
        // A multi-segment prefix survives whole; only the version goes.
        assert_eq!(
            split_endpoint("https://api.groq.com/openai/v1/"),
            (
                "https://api.groq.com".to_string(),
                Some("/openai".to_string())
            )
        );
        assert_eq!(
            split_endpoint("https://api.commandcode.ai/provider/V1"),
            (
                "https://api.commandcode.ai".to_string(),
                Some("/provider".to_string())
            )
        );
    }

    #[test]
    fn leaves_an_endpoint_without_a_version_alone() {
        assert_eq!(
            split_endpoint("https://api.deepseek.com/anthropic"),
            (
                "https://api.deepseek.com".to_string(),
                Some("/anthropic".to_string())
            )
        );
        assert_eq!(
            split_endpoint("https://api.deepseek.com"),
            ("https://api.deepseek.com".to_string(), None)
        );
        // `/v1` alone is the whole path: nothing left to prefix.
        assert_eq!(
            split_endpoint("https://api.deepseek.com/v1"),
            ("https://api.deepseek.com".to_string(), None)
        );
    }

    #[test]
    fn a_url_without_a_scheme_is_not_guessed_at() {
        assert_eq!(
            split_endpoint("api.example.com/v1"),
            ("api.example.com/v1".to_string(), None)
        );
    }
}
