//! Credentials and headers, both directions: which key one upstream request
//! carries, what the upstream is told, and what the client is told back.

use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;

use crate::error::GatewayError;
use crate::forward::finish::log_failure;
use crate::forward::sample::CompletedLog;
use crate::router::{RoutedRequest, UpstreamProvider};
use crate::server::{error_into_response, GatewayState};
use crate::store::Protocol;

/// Hop-by-hop headers never forwarded in either direction.
fn is_hop_by_hop(name: &axum::http::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "transfer-encoding"
            | "keep-alive"
            | "upgrade"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
    )
}

/// Pick the credential for one upstream request: the provider's key pool is
/// `[primary, extras…]`; pools with more than one entry rotate per request
/// (spec §4.1 P1 multi-key rotation, so a single key never trips upstream rate limits).
///
/// A request that names a session is the exception: it stays on the pool entry
/// the conversation started on. The upstream prompt cache is scoped to the
/// account a key belongs to, so a conversation rotated mid-flight would
/// rebuild its prefix on another account at full input price — the same loss
/// the strategy layer pins a provider to avoid, one level down. New sessions
/// still take the rotating cursor, so the spreading the pool exists for
/// happens across sessions instead of across requests. A request that names
/// no session rotates per request, as before.
pub(crate) fn select_upstream_key(
    state: &crate::server::GatewayState,
    provider: &UpstreamProvider,
    session: Option<&str>,
) -> Result<SelectedKey, GatewayError> {
    let pool = provider.key_pool();
    if pool.is_empty() {
        return Err(GatewayError::Upstream(format!(
            "provider `{}` has no API key configured",
            provider.id
        )));
    }
    if let Some(session) = session {
        // A pin that survives: the entry is still in the pool (a removed or
        // disabled key falls through and the session is re-pinned).
        if let Some(handle) = state.pinned_key(&provider.id, session) {
            if let Some(found) = pool.iter().find(|(h, _)| *h == handle) {
                return Ok(SelectedKey::of(found.0, found.1, provider));
            }
        }
        let (handle, key) = pool[state.next_key_index(&provider.id, pool.len())];
        state.pin_key(&provider.id, session, handle);
        return Ok(SelectedKey::of(handle, key, provider));
    }
    let (handle, key) = pool[state.next_key_index(&provider.id, pool.len())];
    Ok(SelectedKey::of(handle, key, provider))
}

/// One chosen pool entry: the credential to send, and the name to remember it
/// by.
///
/// The two are separate on purpose. The credential is a secret that only ever
/// goes upstream; the name is what the ledger and the receipt record, which is
/// why it can never be the key. Built here, at the one place that knows which
/// entry was chosen, so no caller can pair the wrong name with the wrong key.
pub(crate) struct SelectedKey {
    pub(crate) key: String,
    pub(crate) account: String,
}

/// Hand-written so a stray `{:?}` — in a test failure, an assertion message, a
/// log line — cannot print a live credential into a terminal or a file.
impl std::fmt::Debug for SelectedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectedKey")
            .field("key", &"<redacted>")
            .field("account", &self.account)
            .finish()
    }
}

impl SelectedKey {
    fn of(handle: crate::router::PoolHandle, key: &str, provider: &UpstreamProvider) -> Self {
        let account = match handle {
            crate::router::PoolHandle::Primary => "primary".to_string(),
            crate::router::PoolHandle::Extra(id) => provider
                .extra_keys
                .iter()
                .find(|k| k.id == id)
                .map_or_else(|| format!("#{id}"), crate::router::PoolKey::account_name),
        };
        SelectedKey {
            key: key.to_string(),
            account,
        }
    }
}

/// Build upstream request headers: copy the inbound set (minus hop-by-hop and
/// local auth headers), then inject the provider's real credential in its
/// native auth style.
pub(crate) fn build_upstream_headers(
    inbound: &HeaderMap,
    provider: &UpstreamProvider,
    key: &str,
) -> Result<HeaderMap, GatewayError> {
    let mut out = HeaderMap::new();
    for (k, v) in inbound {
        if k == axum::http::header::HOST
            || k == axum::http::header::CONTENT_LENGTH
            || k == axum::http::header::AUTHORIZATION
            || k == "x-api-key"
            || k == "x-goog-api-key"
            || is_hop_by_hop(k)
        {
            continue;
        }
        out.insert(k, v.clone());
    }

    match provider.protocol {
        Protocol::Anthropic => {
            let value =
                HeaderValue::from_str(key).map_err(|e| GatewayError::Upstream(e.to_string()))?;
            out.insert("x-api-key", value);
            if !out.contains_key("anthropic-version") {
                out.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
        }
        Protocol::OpenAI => {
            let value = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| GatewayError::Upstream(e.to_string()))?;
            out.insert(axum::http::header::AUTHORIZATION, value);
        }
        // The native Gemini API takes its credential in `x-goog-api-key`.
        Protocol::Gemini => {
            let value =
                HeaderValue::from_str(key).map_err(|e| GatewayError::Upstream(e.to_string()))?;
            out.insert("x-goog-api-key", value);
        }
    }

    // Custom per-provider headers (migration v8): merged after credential
    // injection with insert-replace semantics, so they can override the
    // injected credentials and defaults (e.g. an Azure `api-key` or an
    // `OpenAI-Organization`). Framing/hop-by-hop names are refused; invalid
    // names or values are skipped with a warning rather than failing the
    // whole forward.
    if let Some(custom) = &provider.headers {
        for (name, value) in custom {
            let Ok(hn) = axum::http::HeaderName::from_bytes(name.as_bytes()) else {
                tracing::warn!(provider = %provider.id, header = %name, "invalid custom header name; skipping");
                continue;
            };
            if is_hop_by_hop(&hn)
                || hn == axum::http::header::HOST
                || hn == axum::http::header::CONTENT_LENGTH
            {
                continue;
            }
            match HeaderValue::from_str(value) {
                Ok(hv) => {
                    out.insert(hn, hv);
                }
                Err(e) => {
                    tracing::warn!(provider = %provider.id, header = %name, error = %e, "invalid custom header value; skipping");
                }
            }
        }
    }
    Ok(out)
}

/// The headers one upstream request goes out with, and the name of the pool
/// entry that carries it — or the client-facing failure to answer with, whose
/// log row is written here.
///
/// The entry's name rides back with the headers rather than being read again
/// later: the choice is made once, and a second read could disagree with it.
#[allow(clippy::result_large_err)] // Err is the client-facing response, not a diagnostic
pub(crate) fn upstream_key_and_headers(
    state: &GatewayState,
    provider: &UpstreamProvider,
    inbound_headers: &HeaderMap,
    inbound: Option<Protocol>,
    log: Option<&CompletedLog>,
    routed: &RoutedRequest,
) -> Result<(HeaderMap, String), Response> {
    let selected = match select_upstream_key(state, provider, routed.session_id.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            let resp = error_into_response(e, inbound);
            log_failure(
                state,
                log,
                provider,
                routed,
                resp.status(),
                "upstream_error",
                "selecting upstream key failed".to_string(),
            );
            return Err(resp);
        }
    };
    match build_upstream_headers(inbound_headers, provider, &selected.key) {
        Ok(h) => Ok((h, selected.account)),
        Err(e) => {
            let resp = error_into_response(e, inbound);
            log_failure(
                state,
                log,
                provider,
                routed,
                resp.status(),
                "upstream_error",
                "building upstream headers failed".to_string(),
            );
            Err(resp)
        }
    }
}

/// The receipt headers a forwarded response carries: which provider answered,
/// and the model the request was metered under. They answer "this response —
/// who actually served it?" at the moment it matters; under failover or a
/// standby queue the agent otherwise learns it only by digging through the
/// Logs screen afterwards.
pub(crate) const RECEIPT_PROVIDER_HEADER: &str = "x-kiwano-provider";
pub(crate) const RECEIPT_MODEL_HEADER: &str = "x-kiwano-model";
pub(crate) const RECEIPT_ACCOUNT_HEADER: &str = "x-kiwano-account";

/// Stamp the attribution receipt onto a client-facing response. The gateway's
/// own statement wins over an upstream header of the same name — a provider
/// could echo one, and only the gateway knows which provider it chose. A value
/// that cannot be a header value is skipped rather than failing the response.
///
/// The account is the pool entry's name (`primary`, a label, or `#3`) and never
/// the key: a receipt is read by clients and written to logs, and neither is a
/// place for a credential.
pub(crate) fn stamp_receipt(
    headers: &mut HeaderMap,
    provider_id: &str,
    model: Option<&str>,
    account: Option<&str>,
) {
    if let Ok(v) = HeaderValue::from_str(provider_id) {
        headers.insert(RECEIPT_PROVIDER_HEADER, v);
    }
    if let Some(model) = model {
        if let Ok(v) = HeaderValue::from_str(model) {
            headers.insert(RECEIPT_MODEL_HEADER, v);
        }
    }
    if let Some(account) = account {
        if let Ok(v) = HeaderValue::from_str(account) {
            headers.insert(RECEIPT_ACCOUNT_HEADER, v);
        }
    }
}

/// Copy upstream response headers onto a gateway response, dropping framing
/// headers that hyper/axum will regenerate for the client side.
pub(crate) fn copy_response_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in src {
        if k == axum::http::header::CONTENT_LENGTH || is_hop_by_hop(k) {
            continue;
        }
        out.insert(k, v.clone());
    }
    out
}

/// Serialize a response header map for the request log.
///
/// The request side's redaction, not a copy of it: an upstream response is not
/// the safer half of the exchange. `set-cookie` is a live session credential
/// and it reaches this map — `copy_response_headers` only drops framing and
/// hop-by-hop names — so the same rule has to run here. Sharing the one
/// function is what keeps the two sides from drifting apart.
pub(crate) fn response_headers_text(headers: &HeaderMap) -> String {
    crate::log_capture::redact_headers(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::provider;

    fn inbound_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("kw-ag-claude-abc"));
        h.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer kw-ag-claude-abc"),
        );
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        h.insert("user-agent", HeaderValue::from_static("claude-code/1.0"));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        h
    }

    #[test]
    fn anthropic_upstream_headers_replace_local_auth() {
        let headers = build_upstream_headers(
            &inbound_headers(),
            &provider(Protocol::Anthropic, None),
            "sk-real-key",
        )
        .unwrap();
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-real-key");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(headers.get("user-agent").unwrap(), "claude-code/1.0");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
        // The placeholder key must never reach the upstream.
        assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
    }

    #[test]
    fn openai_upstream_headers_use_bearer() {
        let headers = build_upstream_headers(
            &inbound_headers(),
            &provider(Protocol::OpenAI, None),
            "sk-real-key",
        )
        .unwrap();
        assert_eq!(
            headers.get(axum::http::header::AUTHORIZATION).unwrap(),
            "Bearer sk-real-key"
        );
        // x-api-key from the inbound request is stripped, not forwarded.
        assert!(headers.get("x-api-key").is_none());
    }

    #[test]
    fn custom_provider_headers_override_injected_credentials() {
        // A vendor (Azure-style) that wants `api-key` plus an overriding
        // x-api-key: custom headers merge after credential injection.
        let mut p = provider(Protocol::Anthropic, None);
        let mut custom = std::collections::BTreeMap::new();
        custom.insert("api-key".to_string(), "azure-key".to_string());
        custom.insert("x-api-key".to_string(), "override".to_string());
        custom.insert("OpenAI-Organization".to_string(), "org-1".to_string());
        p.headers = Some(custom);

        let headers = build_upstream_headers(&inbound_headers(), &p, "sk-real-key").unwrap();
        assert_eq!(headers.get("x-api-key").unwrap(), "override");
        assert_eq!(headers.get("api-key").unwrap(), "azure-key");
        assert_eq!(headers.get("openai-organization").unwrap(), "org-1");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
    }

    #[test]
    fn custom_provider_headers_skip_framing_and_invalid() {
        let mut p = provider(Protocol::OpenAI, None);
        let mut custom = std::collections::BTreeMap::new();
        custom.insert("connection".to_string(), "close".to_string());
        custom.insert("host".to_string(), "evil.example.com".to_string());
        custom.insert("content-length".to_string(), "0".to_string());
        custom.insert("x-good".to_string(), "kept".to_string());
        // Header names are case-insensitive per HTTP; an invalid value is skipped.
        custom.insert("x-bad-value".to_string(), "\n\r inject".to_string());
        p.headers = Some(custom);

        let headers = build_upstream_headers(&inbound_headers(), &p, "sk-real-key").unwrap();
        assert_eq!(headers.get("x-good").unwrap(), "kept");
        assert!(headers.get("connection").is_none());
        // host/content-length are never settable via custom headers
        assert_ne!(
            headers.get("host").map(|v| v.to_str().unwrap()),
            Some("evil.example.com")
        );
        assert!(headers.get("x-bad-value").is_none());
    }

    #[test]
    fn missing_provider_key_fails_cleanly() {
        let mut p = provider(Protocol::Anthropic, None);
        p.api_key = None;
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let err = select_upstream_key(&state, &p, None).unwrap_err();
        assert!(matches!(err, GatewayError::Upstream(_)));
    }

    fn pool_key(id: i64, key: &str) -> crate::router::PoolKey {
        crate::router::PoolKey {
            id,
            key: key.into(),
            label: None,
        }
    }

    #[test]
    fn multi_key_pool_rotates_per_request() {
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.extra_keys = vec![pool_key(2, "sk-two"), pool_key(3, "sk-three")];

        // Single-key pool always returns the primary key
        let mut single = provider(Protocol::OpenAI, None);
        single.extra_keys.clear();
        for _ in 0..3 {
            let picked = select_upstream_key(&state, &single, None).unwrap();
            assert_eq!(picked.key, "sk-real-key");
            assert_eq!(picked.account, "primary");
        }

        // Multi-key pool: primary key → extra keys rotate in turn, then back to the primary key
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(select_upstream_key(&state, &p, None).unwrap().key);
        }
        assert_eq!(seen, vec!["sk-real-key", "sk-two", "sk-three"]);
        let wrapped = select_upstream_key(&state, &p, None).unwrap();
        assert_eq!(wrapped.key, "sk-real-key");
        // An entry the operator did not name is reported by its row id, and
        // never by the key itself.
        assert_eq!(wrapped.account, "primary");
    }

    /// A conversation stays on the pool entry it started on: the upstream
    /// prompt cache is scoped to the account behind the key, and rotating
    /// mid-conversation would rebuild the prefix at full price. New sessions
    /// still spread across the pool.
    #[test]
    fn a_session_keeps_its_key_while_new_sessions_spread() {
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.extra_keys = vec![pool_key(2, "sk-two"), pool_key(3, "sk-three")];

        // New sessions take the rotating cursor, one entry each.
        let first = select_upstream_key(&state, &p, Some("s-1")).unwrap();
        let second = select_upstream_key(&state, &p, Some("s-2")).unwrap();
        assert_eq!(first.key, "sk-real-key");
        assert_eq!(second.key, "sk-two");
        assert_eq!(second.account, "#2");

        // And then hold it, however many requests the conversation makes.
        for _ in 0..5 {
            assert_eq!(
                select_upstream_key(&state, &p, Some("s-1")).unwrap().key,
                "sk-real-key"
            );
            assert_eq!(
                select_upstream_key(&state, &p, Some("s-2")).unwrap().key,
                "sk-two"
            );
        }
    }

    /// A pin does not outlive its key: when the pinned entry leaves the pool
    /// (removed or disabled), the session is re-pinned to one that is still
    /// usable rather than failing on a key that is gone.
    #[test]
    fn a_pin_does_not_outlive_its_key() {
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.extra_keys = vec![pool_key(2, "sk-two"), pool_key(3, "sk-three")];

        // Pin the session to the first extra.
        assert_eq!(
            select_upstream_key(&state, &p, Some("s-1")).unwrap().key,
            "sk-real-key"
        );
        assert_eq!(
            select_upstream_key(&state, &p, Some("s-2")).unwrap().key,
            "sk-two"
        );

        // The pinned key leaves the pool: the session re-pins instead of
        // erroring on a credential that is no longer offered.
        p.extra_keys.retain(|k| k.id != 2);
        let repinned = select_upstream_key(&state, &p, Some("s-2")).unwrap().key;
        assert!(
            repinned == "sk-real-key" || repinned == "sk-three",
            "re-pinned to a live entry, got {repinned}"
        );
        assert_eq!(
            select_upstream_key(&state, &p, Some("s-2")).unwrap().key,
            repinned,
            "and the re-pin holds"
        );
    }

    #[test]
    fn response_headers_drop_framing() {
        let mut src = HeaderMap::new();
        src.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        src.insert(
            axum::http::header::CONTENT_LENGTH,
            HeaderValue::from_static("123"),
        );
        src.insert(
            axum::http::header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        let out = copy_response_headers(&src);
        assert!(out.get(axum::http::header::CONTENT_TYPE).is_some());
        assert!(out.get(axum::http::header::CONTENT_LENGTH).is_none());
        assert!(out.get(axum::http::header::TRANSFER_ENCODING).is_none());
    }

    /// The receipt is the gateway's own statement: it is always stamped, the
    /// model is stamped when the request named one, and it overrides an
    /// upstream header of the same name rather than the other way around.
    #[test]
    fn receipt_stamps_attribution_and_wins_over_an_upstream_spoof() {
        let mut headers = HeaderMap::new();
        headers.insert(
            RECEIPT_PROVIDER_HEADER,
            HeaderValue::from_static("upstream-lie"),
        );
        stamp_receipt(
            &mut headers,
            "p-work",
            Some("claude-haiku-4-5"),
            Some("personal"),
        );
        assert_eq!(headers.get(RECEIPT_PROVIDER_HEADER).unwrap(), "p-work");
        assert_eq!(
            headers.get(RECEIPT_MODEL_HEADER).unwrap(),
            "claude-haiku-4-5"
        );
        assert_eq!(headers.get(RECEIPT_ACCOUNT_HEADER).unwrap(), "personal");

        // A request whose model could not be read still says who served it.
        let mut headers = HeaderMap::new();
        stamp_receipt(&mut headers, "p-work", None, None);
        assert_eq!(headers.get(RECEIPT_PROVIDER_HEADER).unwrap(), "p-work");
        assert!(headers.get(RECEIPT_MODEL_HEADER).is_none());
        assert!(headers.get(RECEIPT_ACCOUNT_HEADER).is_none());
    }

    /// An upstream response carries credentials of its own — a session cookie
    /// is the common one — and this map is what lands in `response_headers`.
    #[test]
    fn response_headers_are_redacted_like_the_request_side() {
        let mut src = HeaderMap::new();
        src.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        src.insert(
            axum::http::header::SET_COOKIE,
            HeaderValue::from_static("session=upstream-secret; HttpOnly"),
        );
        src.insert("x-request-id", HeaderValue::from_static("req-7"));

        let json = response_headers_text(&src);
        assert!(!json.contains("upstream-secret"), "{json}");
        assert!(json.contains(r#""set-cookie":"[REDACTED]""#), "{json}");
        assert!(json.contains("application/json"), "{json}");
        assert!(json.contains("req-7"), "{json}");
    }
}
