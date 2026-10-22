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
///
/// `endpoint` is the **resolved** upstream base URL (an Alternate endpoint's
/// counts, as the protocol does): some rules are a property of one vendor's
/// parser rather than of the protocol, and the URL is what names it. A
/// hand-added provider has no catalog id, so keying on the URL is what lets
/// the same rule reach it.
pub(crate) fn apply_compat_shim(
    body: &Bytes,
    protocol: Protocol,
    endpoint: &str,
) -> Option<(Bytes, String)> {
    if body.is_empty() || protocol == Protocol::Gemini {
        return None;
    }
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let mut notes = match protocol {
        Protocol::Anthropic => kiwano_adapters::proxy::providers::shim::sanitize_anthropic(&mut v),
        Protocol::OpenAI => kiwano_adapters::proxy::providers::shim::sanitize_openai(&mut v),
        Protocol::Gemini => return None,
    };
    // R4: provider-scoped, so it is applied here rather than inside the
    // protocol rules — every other anthropic upstream may accept the role.
    if protocol == Protocol::Anthropic && rejects_system_role(endpoint) {
        notes.extend(kiwano_adapters::proxy::providers::shim::normalize_system_roles(&mut v));
    }
    if notes.is_empty() {
        return None;
    }
    Some((Bytes::from(serde_json::to_vec(&v).ok()?), notes.join("\n")))
}

/// The endpoints that refuse a `system` role inside `messages[]`.
///
/// Command Code's anthropic endpoint answers `400 Invalid input at messages.N.role`
/// for one — both on its Provider API host and on the plan host, whichever
/// spelling the entry uses. The check is on the URL's host so a provider added
/// by hand (no catalog id) gets the rule too.
fn rejects_system_role(endpoint: &str) -> bool {
    let Some((_, rest)) = endpoint.split_once("://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    host == "api.commandcode.ai"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body Claude Code sends: environment/context blocks arrive as
    /// `role: "system"` entries inside `messages[]`, which Command Code's
    /// anthropic endpoint refuses with `400 Invalid input at messages.N.role`.
    fn body() -> Bytes {
        Bytes::from_static(
            br#"{"model":"claude-opus-4-8","messages":[
                {"role":"system","content":"environment: darwin"},
                {"role":"user","content":"hi"}]}"#,
        )
    }

    fn parse(bytes: &Bytes) -> serde_json::Value {
        serde_json::from_slice(bytes).unwrap()
    }

    #[test]
    fn r4_applies_to_the_command_code_endpoint_only() {
        let (out, notes) = apply_compat_shim(
            &body(),
            Protocol::Anthropic,
            "https://api.commandcode.ai/provider",
        )
        .expect("the system role is rewritten");
        assert_eq!(parse(&out)["messages"][0]["role"], "user");
        assert!(notes.contains("system role(s) sent as user"), "{notes}");

        // The same body to any other anthropic upstream is forwarded verbatim:
        // the rule is a property of that parser, not of the protocol.
        assert!(
            apply_compat_shim(&body(), Protocol::Anthropic, "https://api.anthropic.com").is_none()
        );

        // …and the URL check is a host check, so an Alternate endpoint or a
        // hand-added provider with the same host gets it too.
        assert!(apply_compat_shim(
            &body(),
            Protocol::Anthropic,
            "http://api.commandcode.ai/provider"
        )
        .is_some());
        assert!(apply_compat_shim(
            &body(),
            Protocol::Anthropic,
            "https://api.commandcode.ai.evil.example/provider"
        )
        .is_none());
    }

    /// The rules stay additive: R1–R3 still run, and a body that trips only one
    /// of them is still rewritten.
    #[test]
    fn r4_composes_with_the_protocol_rules() {
        let bytes = Bytes::from_static(
            br#"{"model":"m","thinking":{"type":"adaptive"},
                "messages":[{"role":"system","content":"env"},{"role":"user","content":"hi"}]}"#,
        );
        let (out, notes) = apply_compat_shim(
            &bytes,
            Protocol::Anthropic,
            "https://api.commandcode.ai/provider",
        )
        .unwrap();
        let v = parse(&out);
        assert!(v.get("thinking").is_none(), "R1 still ran");
        assert_eq!(v["messages"][0]["role"], "user");
        assert!(notes.contains("thinking"), "{notes}");
        assert!(notes.contains("system role"), "{notes}");
    }
}
