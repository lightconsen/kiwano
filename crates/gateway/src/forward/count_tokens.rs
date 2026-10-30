//! Local `count_tokens` answer for a conversion upstream that has no such
//! endpoint.
//!
//! Claude Code calls `POST /v1/messages/count_tokens` before every request to
//! decide whether the context window still has room — and that is the whole
//! question. A conversion upstream (an OpenAI-only provider) has no endpoint to
//! forward it to, so the honest choice used to be a `protocol_mismatch` error.
//! The client tolerates the failure — the conversation carries on — but it then
//! flies blind on its own size, and the log grows a 502 row for something that
//! is not wrong.
//!
//! So the gateway estimates. The shape is Anthropic's own
//! (`{"input_tokens": N}`), the number is an approximation, and the direction of
//! the error is deliberate: **over**-counting makes the client compact a little
//! early, which costs a compacted-turn; under-counting makes it run out of
//! window mid-turn, which costs the turn itself.
//!
//! # How the number is made
//!
//! The request is walked as JSON rather than measured in bytes, because the
//! JSON carries structure the byte count cannot see. The rules, in order of
//! what they cost the estimate:
//!
//! - **An image block** — Anthropic bills it by area, roughly one token per
//!   750 pixels, and a screenshot is on the order of a thousand tokens. The
//!   base64 payload would swamp the estimate if it were counted as text, so
//!   each image counts 1500 and its bytes are skipped.
//! - **A string** — one token per four characters, rounded up. That is the
//!   English-prose rule of thumb; code and CJK text sit on either side of it.
//! - **A number, boolean or null** — one token. They are short and rare enough
//!   in a messages request that the rounding does not matter.
//!
//! Nothing is tokenized. The estimate never pretends to be the upstream's own
//! count, and the caller that needs precision — billing — never sees it: this
//! path is for the context-window check alone.

use serde_json::Value;

/// Flat per-image estimate. Anthropic's own count is area-based
/// (`(width * height) / 750`); a screenshot is on the order of a thousand
/// tokens, and the error direction here is the one that matters — an
/// over-estimate compacts early, an under-estimate runs out of window.
const IMAGE_TOKENS: u64 = 1500;

/// Estimate the input token count of a `/v1/messages/count_tokens` body.
///
/// `None` when the body is not JSON at all — a caller that cannot parse the
/// request has nothing to estimate from, and returning zero would claim the
/// context is empty when the client already knows it is not.
pub(crate) fn estimate(body: &[u8]) -> Option<u64> {
    let value: Value = serde_json::from_slice(body).ok()?;
    Some(walk(&value))
}

/// Sum the tree. Each node counts for what it *is*, not what its byte
/// representation is — the same reason the whole thing is JSON-walked rather
/// than divided by a byte-per-token constant.
fn walk(value: &Value) -> u64 {
    match value {
        Value::Object(map) => {
            // An image block is `{ "type": "image", "source": { ... } }`. Its
            // `source.data` is the whole base64 payload — a screenshot's
            // megabytes would otherwise dominate the estimate by an order of
            // magnitude more than the model actually reads.
            if map.get("type").and_then(Value::as_str) == Some("image") {
                return IMAGE_TOKENS;
            }
            map.values().map(walk).sum()
        }
        Value::Array(items) => items.iter().map(walk).sum(),
        // Four characters per token, rounded up. Code averages denser than
        // prose and CJK sparser; the round-up keeps both on the safe side.
        Value::String(s) => (s.chars().count() as u64).div_ceil(4),
        Value::Number(_) | Value::Bool(_) | Value::Null => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape Claude Code actually sends: a user turn of text.
    #[test]
    fn estimates_a_plain_text_request() {
        // 400 characters of prose is ~100 tokens by the four-per-token rule.
        let body = json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "x".repeat(400) }],
        });
        let estimate = estimate(&serde_json::to_vec(&body).unwrap()).unwrap();
        // The envelope's short fields (`user`, key names, the model id) add a
        // few tokens on top of the 100 the text accounts for.
        assert!((100..=120).contains(&estimate), "estimate was {estimate}");
    }

    /// The case that made a byte-count approach untenable: a base64 image is
    /// megabytes of string that the model bills as ~1500 tokens.
    #[test]
    fn an_image_counts_its_area_not_its_base64() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image", "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        // A megabyte of base64 — the string a screenshot sends.
                        "data": "a".repeat(1_000_000),
                    }},
                    { "type": "text", "text": "describe this" },
                ],
            }],
        });
        let estimate = estimate(&serde_json::to_vec(&body).unwrap()).unwrap();
        // The image's flat estimate plus the caption, not a quarter of the
        // base64's character count.
        assert!(estimate < 2_000, "estimate was {estimate}");
        assert!(estimate >= IMAGE_TOKENS, "estimate was {estimate}");
    }

    /// The estimate is what it is: nothing when the body is not JSON, and the
    /// keys the envelope always carries still count for something.
    #[test]
    fn the_estimate_is_none_for_non_json_and_never_zero() {
        assert_eq!(estimate(b"not json"), None);
        // Even an empty object carries nothing — but a request body always has
        // a model, and that is what the walk counts.
        let body = serde_json::json!({ "model": "m", "messages": [] });
        assert!(estimate(&serde_json::to_vec(&body).unwrap()).unwrap() > 0);
    }

    /// The round-up is the safety margin: three characters are one token, not
    /// zero, and one is one.
    #[test]
    fn short_strings_still_count() {
        let body = json!({ "messages": [{ "role": "user", "content": "hi" }] });
        let estimate = estimate(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(estimate > 0);
    }
}
