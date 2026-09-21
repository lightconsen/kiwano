//! Which protocol and endpoint an inbound request resolves to (migration v7).

use crate::router::UpstreamProvider;
use crate::store::Protocol;

/// Endpoint/protocol resolution for one inbound request (multi-protocol
/// providers, migration v7).
pub(crate) enum InboundResolution {
    /// Provider's own protocol (or ambiguous inbound): native forward.
    Native,
    /// A registered per-protocol endpoint matches: forward natively as that
    /// protocol via the endpoint's URL (headers + metering follow it).
    Alternate {
        protocol: Protocol,
        base_url: String,
        api_path: Option<String>,
    },
    /// Anthropic `/v1/messages` inbound on an OpenAI provider without an
    /// Anthropic endpoint: convert via the adapters sublayer.
    ConvertAnthropicToOpenAI,
    /// No endpoint and no conversion: fail cleanly with `protocol_mismatch`.
    Mismatch { message: String },
}

pub(crate) fn resolve_inbound(
    provider: &UpstreamProvider,
    inbound: Option<Protocol>,
    path: &str,
) -> InboundResolution {
    let Some(inbound_proto) = inbound else {
        return InboundResolution::Native;
    };
    if inbound_proto == provider.protocol {
        return InboundResolution::Native;
    }
    if let Some(e) = provider.endpoint_for(inbound_proto) {
        return InboundResolution::Alternate {
            protocol: inbound_proto,
            base_url: e.base_url.clone(),
            api_path: e.api_path.clone(),
        };
    }
    if inbound_proto == Protocol::Anthropic
        && provider.protocol == Protocol::OpenAI
        && path == "/v1/messages"
    {
        return InboundResolution::ConvertAnthropicToOpenAI;
    }
    InboundResolution::Mismatch {
        message: format!(
            "provider `{}` speaks `{}` but path `{}` is `{}`; no `{}` endpoint is configured on the provider and only Anthropic `/v1/messages` -> OpenAI conversion is supported",
            provider.id,
            provider.protocol.as_str(),
            path,
            inbound_proto.as_str(),
            inbound_proto.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::provider;

    #[test]
    fn resolve_inbound_prefers_registered_protocol_endpoint() {
        let mut dual = provider(Protocol::OpenAI, None);
        dual.endpoints = vec![crate::store::ProviderEndpoint {
            protocol: Protocol::Anthropic,
            base_url: "https://up.example.com/anthropic".into(),
            api_path: Some("/ant".into()),
        }];

        // Registered per-protocol endpoint wins over conversion.
        match resolve_inbound(&dual, Some(Protocol::Anthropic), "/v1/messages") {
            InboundResolution::Alternate {
                protocol,
                base_url,
                api_path,
            } => {
                assert_eq!(protocol, Protocol::Anthropic);
                assert_eq!(base_url, "https://up.example.com/anthropic");
                assert_eq!(api_path.as_deref(), Some("/ant"));
            }
            _ => panic!("expected Alternate"),
        }

        // Native when the inbound protocol matches the provider's own.
        assert!(matches!(
            resolve_inbound(&dual, Some(Protocol::OpenAI), "/v1/chat/completions"),
            InboundResolution::Native
        ));
        // Ambiguous inbound (/v1/models) stays native.
        assert!(matches!(
            resolve_inbound(&dual, None, "/v1/models"),
            InboundResolution::Native
        ));

        // No anthropic endpoint registered → conversion fallback.
        let plain = provider(Protocol::OpenAI, None);
        assert!(matches!(
            resolve_inbound(&plain, Some(Protocol::Anthropic), "/v1/messages"),
            InboundResolution::ConvertAnthropicToOpenAI
        ));
        // Legacy anthropic paths have no OpenAI equivalent.
        assert!(matches!(
            resolve_inbound(&plain, Some(Protocol::Anthropic), "/v1/complete"),
            InboundResolution::Mismatch { .. }
        ));

        // Gemini conversion is not a thing: a Gemini inbound reaches a Gemini
        // provider natively, and any other pairing is a clean mismatch rather
        // than a silently mistranslated request.
        assert!(matches!(
            resolve_inbound(
                &provider(Protocol::Gemini, None),
                Some(Protocol::Gemini),
                "/v1beta/models/gemini-pro:generateContent"
            ),
            InboundResolution::Native
        ));
        assert!(matches!(
            resolve_inbound(
                &plain,
                Some(Protocol::Gemini),
                "/v1beta/models/gemini-pro:generateContent"
            ),
            InboundResolution::Mismatch { .. }
        ));
    }
}
