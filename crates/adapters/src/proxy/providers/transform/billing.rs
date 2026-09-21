//! The `x-anthropic-billing-header` scrub.
//!
//! Claude Code can prepend rotating `cch=` metadata to the system prompt. If it
//! were forwarded, the prompt prefix would change on every request and prefix
//! caching would be defeated (#2350). Only a *leading* line is stripped: a later
//! occurrence is user-authored text and is kept.
//!
//! A leaf. It takes a `&str` and returns a slice of it, and `request` is the
//! only caller.

const ANTHROPIC_BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";

/// Strip only a leading Claude Code attribution line from system text.
///
/// Claude Code can send dynamic `x-anthropic-billing-header` metadata at the
/// start of `system`. If forwarded into OpenAI Chat messages or Responses
/// `instructions`, the rotating `cch=` value changes the prompt prefix on every
/// request and prevents prefix cache reuse (#2350). Later occurrences are kept
/// to avoid deleting user-authored prompt text.
pub fn strip_leading_anthropic_billing_header(text: &str) -> &str {
    if !text.starts_with(ANTHROPIC_BILLING_HEADER_PREFIX) {
        return text;
    }

    let Some(line_end) = text
        .as_bytes()
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
    else {
        return "";
    };

    let bytes = text.as_bytes();
    let mut rest_start = line_end + 1;
    if bytes[line_end] == b'\r' && bytes.get(line_end + 1) == Some(&b'\n') {
        rest_start += 1;
    }

    let rest = &text[rest_start..];
    if let Some(stripped) = rest.strip_prefix("\r\n") {
        stripped
    } else if let Some(stripped) = rest.strip_prefix('\n') {
        stripped
    } else if let Some(stripped) = rest.strip_prefix('\r') {
        stripped
    } else {
        rest
    }
}
