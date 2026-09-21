//! The two places the gateway rewrites a client's own bytes on the way out:
//! the compat shim, and the request for a usage chunk the meter needs.

use axum::body::Bytes;

use crate::store::Protocol;

/// Add `stream_options.include_usage` to a streaming OpenAI chat request that
/// did not ask for it, so the meter has numbers to read.
///
/// `None` means "leave the bytes alone" — not streaming, already asked for, or
/// not JSON at all. The caller keeps that answer rather than re-parsing: what
/// it decides is whether the usage-only chunk coming back has to be taken out
/// again, and a body this function did not touch cannot produce one.
pub(crate) fn ensure_openai_stream_usage(body: &Bytes) -> Option<Bytes> {
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    if v.get("stream").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let asked = v
        .pointer("/stream_options/include_usage")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    if asked {
        return None;
    }
    kiwano_adapters::proxy::providers::transform::inject_openai_stream_include_usage(&mut v);
    Some(Bytes::from(serde_json::to_vec(&v).ok()?))
}

/// Run the compat shim over a passthrough body and report what it changed.
///
/// `None` means "forward the original bytes": empty body (GET /v1/models),
/// not JSON, Gemini (the one protocol v1 has no rules for), or — the common
/// case by far — nothing matched. That last answer is deliberately an
/// untouched `None` rather than a re-serialization: upstream prompt caches
/// key on the exact prefix, and a body we never needed to edit must not
/// rotate one. `Some` carries the sanitized bytes plus the notes the request
/// log will store.
pub(crate) fn apply_compat_shim(body: &Bytes, protocol: Protocol) -> Option<(Bytes, String)> {
    if body.is_empty() || protocol == Protocol::Gemini {
        return None;
    }
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let notes = match protocol {
        Protocol::Anthropic => kiwano_adapters::proxy::providers::shim::sanitize_anthropic(&mut v),
        Protocol::OpenAI => kiwano_adapters::proxy::providers::shim::sanitize_openai(&mut v),
        Protocol::Gemini => return None,
    };
    if notes.is_empty() {
        return None;
    }
    Some((Bytes::from(serde_json::to_vec(&v).ok()?), notes.join("\n")))
}
