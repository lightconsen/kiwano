//! Which protocol and endpoint an inbound request resolves to (migration v7).

use crate::protocol::OpenAiWirePath;
use crate::router::UpstreamProvider;
use crate::store::{OpenAiWire, Protocol};

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
    /// Responses `/v1/responses` inbound on a provider that serves only Chat
    /// Completions (migration v30): convert, also in the adapters sublayer.
    ConvertResponsesToChat,
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
        // The OpenAI family is two wires, and a provider can be declared as
        // serving only one of them. `Both` — every provider that predates the
        // column — takes this branch as it always did: whatever arrives is
        // passed through, and the vendor is the authority on what it serves.
        if inbound_proto == Protocol::OpenAI {
            match (
                crate::protocol::openai_wire_of_path(path),
                provider.openai_wire,
            ) {
                (Some(OpenAiWirePath::Responses), OpenAiWire::Chat) => {
                    return InboundResolution::ConvertResponsesToChat;
                }
                (Some(OpenAiWirePath::Chat), OpenAiWire::Responses) => {
                    return InboundResolution::Mismatch {
                        message: format!(
                            "provider `{}` serves only the OpenAI Responses API, and a \
                             `/v1/chat/completions` request cannot be converted to it — that \
                             direction is not implemented. Bind a provider that speaks Chat \
                             Completions, or register one as an endpoint on this provider",
                            provider.id
                        ),
                    };
                }
                _ => {}
            }
        }
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
            "provider `{}` speaks `{}` but path `{}` is `{}`; no `{}` endpoint is configured on the \
             provider — the conversions that exist are Anthropic `/v1/messages` -> OpenAI and \
             OpenAI `/v1/responses` -> Chat Completions",
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

    /// The OpenAI family is two wires, and this is the whole of what the column
    /// decides: which pairing converts, which passes through, and which is
    /// refused outright.
    ///
    /// `Both` is the case that has to stay dull — it is what every provider that
    /// predates the column reads as, so anything but `Native` here would change
    /// behaviour for installs that never asked for a wire.
    #[test]
    fn the_openai_wire_decides_between_conversion_passthrough_and_refusal() {
        let with_wire = |wire| {
            let mut p = provider(Protocol::OpenAI, None);
            p.openai_wire = wire;
            p
        };

        // Chat-only: a Responses request is the cell that used to 404 upstream.
        assert!(matches!(
            resolve_inbound(
                &with_wire(OpenAiWire::Chat),
                Some(Protocol::OpenAI),
                "/v1/responses"
            ),
            InboundResolution::ConvertResponsesToChat
        ));
        // …and the wire it does serve is untouched.
        assert!(matches!(
            resolve_inbound(
                &with_wire(OpenAiWire::Chat),
                Some(Protocol::OpenAI),
                "/v1/chat/completions"
            ),
            InboundResolution::Native
        ));

        // Responses-only: the reverse direction is refused with a reason, and
        // never converted.
        match resolve_inbound(
            &with_wire(OpenAiWire::Responses),
            Some(Protocol::OpenAI),
            "/v1/chat/completions",
        ) {
            InboundResolution::Mismatch { message } => {
                assert!(message.contains("Responses"), "{message}");
                assert!(message.contains("not implemented"), "{message}");
            }
            _ => panic!("expected a mismatch"),
        }
        assert!(matches!(
            resolve_inbound(
                &with_wire(OpenAiWire::Responses),
                Some(Protocol::OpenAI),
                "/v1/responses"
            ),
            InboundResolution::Native
        ));

        // Both (and so every pre-v30 row): pass through, either wire.
        for path in ["/v1/responses", "/v1/chat/completions"] {
            assert!(
                matches!(
                    resolve_inbound(&with_wire(OpenAiWire::Both), Some(Protocol::OpenAI), path),
                    InboundResolution::Native
                ),
                "{path} passes through when the provider serves both"
            );
        }

        // An OpenAI-family path that is neither wire — an embedding — has no
        // Responses shape to convert and is not spoken about by the column: a
        // chat-only provider still gets it, as it always did.
        assert!(matches!(
            resolve_inbound(
                &with_wire(OpenAiWire::Chat),
                Some(Protocol::OpenAI),
                "/v1/embeddings"
            ),
            InboundResolution::Native
        ));

        // The wire belongs to the OpenAI family alone: an Anthropic provider
        // carrying a stray declaration is not consulted for it.
        let mut anth = provider(Protocol::Anthropic, None);
        anth.openai_wire = OpenAiWire::Chat;
        assert!(matches!(
            resolve_inbound(&anth, Some(Protocol::Anthropic), "/v1/messages"),
            InboundResolution::Native
        ));
    }
}
