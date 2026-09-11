//! Full request logging (bodies + metadata) for the data plane.
//!
//! This is a local tool: every data-plane request is captured to
//! `request_logs` + `request_bodies` unless the user turns capture off
//! (`LogConfig`, reloaded from `gateway_settings` alongside the route
//! table). Credential headers are redacted by name and bodies are redacted
//! by shape before storage; bodies are then capped per body and stored
//! lossy-UTF8 (they are JSON/SSE text in practice).

use axum::http::{HeaderMap, StatusCode};

use crate::store::{now_rfc3339, RequestLogNew, Store};

/// Written in place of a redacted credential.
const REDACTED: &str = "[REDACTED]";

/// Header names never written to the log (credentials / cookies).
const REDACTED_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "cookie",
    "proxy-authorization",
];

/// JSON key names whose *scalar* value is a credential, in normalized form
/// (see `normalize_key`): lowercased with `_`/`-` removed, so `api_key`,
/// `apiKey` and `API-KEY` are one entry.
///
/// This is an allowlist of names, not a substring match: `max_tokens` and
/// `token_count` do not normalize to `token` and are left alone. The cost is
/// the mirror image — a credential filed under a name not listed here
/// (`session_token`, `bearer_key`, …) is only caught if its *value* matches a
/// known shape below.
const SECRET_KEYS: &[&str] = &[
    "apikey",
    "xapikey",
    "xgoogapikey",
    "authorization",
    "proxyauthorization",
    "token",
    "accesstoken",
    "refreshtoken",
    "secret",
    "clientsecret",
    "privatekey",
    "password",
    "passwd",
    "cookie",
    "setcookie",
];

/// Credential *shapes* recognized inside free text (a JSON string value, an
/// SSE `data:` payload that is not itself JSON, a plain-text body).
///
/// A pasted key has no JSON key name to match on — it sits inside
/// `messages[].content` as ordinary text — so shape detection is the only
/// thing that catches the reported case. The list is deliberately short and
/// anchored on prefixes that are near-unique to credentials, with a minimum
/// tail length so a passing mention (`Bearer token`, `sk-` in prose) is not
/// rewritten. `(prefix, min_tail)`.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("sk-", 20),   // OpenAI / Anthropic / OpenRouter style
    ("kw-ag-", 8), // kiwano's own agent placeholder keys
    ("ghp_", 20),  // GitHub PATs
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("xoxb-", 20), // Slack
    ("xoxp-", 20),
    ("xoxa-", 20),
    ("xoxr-", 20),
    ("xoxs-", 20),
    ("AKIA", 16), // AWS access key id
    ("Bearer ", 20),
];

/// Normalize a JSON key for comparison: lowercase, `_`/`-` stripped.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// True when a key name means "the value is a credential".
fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.contains(&normalize_key(key).as_str())
}

/// Redact credentials from a parsed JSON value in place.
///
/// Returns `true` when anything changed (so a clean body can be stored
/// verbatim, byte for byte).
///
/// A secret key with a *container* value (`"auth": {"type": "x"}`) recurses
/// instead of being blanked: the container is structure, the credential is a
/// scalar somewhere inside it.
fn scrub_json(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            let mut changed = false;
            for (key, val) in map.iter_mut() {
                if is_secret_key(key) && !val.is_object() && !val.is_array() && !val.is_null() {
                    *val = serde_json::Value::String(REDACTED.to_string());
                    changed = true;
                } else {
                    changed |= scrub_json(val);
                }
            }
            changed
        }
        serde_json::Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                changed |= scrub_json(item);
            }
            changed
        }
        serde_json::Value::String(s) => {
            let scrubbed = scrub_shapes(s);
            let changed = scrubbed != *s;
            if changed {
                *s = scrubbed;
            }
            changed
        }
        _ => false,
    }
}

/// Bytes that may appear inside a credential token. Deliberately excludes `.`,
/// which is only significant for JWTs (see `jwt_len`).
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Length of the run of token bytes at the start of `bytes`.
fn token_run_len(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|b| is_token_byte(**b)).count()
}

/// Length of a JWT at the start of `text`, when there is one.
///
/// `eyJ` is the base64 of `{"`, so a `eyJ…` run carrying two `.` separators
/// and enough material is a JWT with very high confidence — no delimiter
/// needed to avoid false positives on prose.
fn jwt_len(text: &str) -> Option<usize> {
    const HEAD: &str = "eyJ";
    if !text.starts_with(HEAD) {
        return None;
    }
    let bytes = text.as_bytes();
    let (mut i, mut dots, mut end) = (HEAD.len(), 0usize, HEAD.len());
    while i < bytes.len() {
        match bytes[i] {
            b if is_token_byte(b) => end = i + 1,
            b'.' => dots += 1,
            _ => break,
        }
        i += 1;
    }
    (dots >= 2 && end - HEAD.len() >= 30).then_some(end)
}

/// Replace known credential shapes in `text` with [`REDACTED`].
///
/// No regex, and nothing like one: the matches are literal prefixes plus a
/// token-character run, so the only text ever rewritten is a run that looks
/// like a credential.
fn scrub_shapes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (i, _) in text.char_indices() {
        // A match must start a token: `task-sk-…` is a word, not a key.
        if i > 0 && is_token_byte(bytes[i - 1]) {
            continue;
        }
        let rest = &text[i..];
        let mut len = None;
        for &(prefix, min_tail) in TOKEN_PREFIXES {
            if let Some(after) = rest.strip_prefix(prefix) {
                let run = token_run_len(after.as_bytes());
                if run >= min_tail {
                    len = Some(prefix.len() + run);
                }
                break;
            }
        }
        if let Some(len) = len.or_else(|| jwt_len(rest)) {
            spans.push((i, i + len));
        }
    }
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in spans {
        out.push_str(&text[cursor..start]);
        out.push_str(REDACTED);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Redact one line of a non-JSON body: an SSE `data:` line gets its payload
/// parsed and scrubbed structurally, anything else only shape-scanned.
fn scrub_line(line: &str) -> String {
    if let Some(payload) = line.strip_prefix("data:") {
        let (gap, body) = match payload.strip_prefix(' ') {
            Some(rest) => (" ", rest),
            None => ("", payload),
        };
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) {
            if scrub_json(&mut value) {
                if let Ok(json) = serde_json::to_string(&value) {
                    return format!("data:{gap}{json}");
                }
            }
        }
    }
    scrub_shapes(line)
}

/// Redact credentials from a captured body before it is stored.
///
/// What this does:
/// - A body that parses as one JSON document is walked structurally: scalar
///   values under a [`SECRET_KEYS`] name are replaced, and every string value
///   is scanned for a [`TOKEN_PREFIXES`] / JWT shape. That second half is what
///   catches a key pasted *into a prompt*, which no key name would flag.
/// - Anything else (SSE, NDJSON, plain text) is handled line by line: a
///   `data: {…}` payload is parsed and scrubbed structurally when it parses,
///   and every line is shape-scanned.
///
/// What this does not do, stated honestly:
/// - It is an allowlist of names and shapes, not a detector. A credential in
///   an unrecognized shape, under an unrecognized key name, is stored as-is.
///   No amount of regex fixes that — it would only trade misses for false
///   positives, rewriting a request body that is supposed to be a faithful
///   record. That trade is not worth taking, so there is no free-text regex.
/// - The body is capped before redaction (the cap bounds this work), so a
///   credential that straddles the cut or lives only in the discarded tail is
///   not scanned. The stored body is already lossy there.
/// - A body that parses as JSON is re-serialized only when something was
///   actually redacted, so a clean body keeps its exact bytes. A rewritten
///   one is compacted (key order survives: `serde_json` is built with
///   `preserve_order` via `kiwano-adapters`).
fn redact_body(text: &str) -> String {
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) {
        if scrub_json(&mut value) {
            if let Ok(json) = serde_json::to_string(&value) {
                return json;
            }
        }
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&scrub_line(line));
    }
    out
}

/// Serialize headers as JSON with credential headers dropped.
fn redact_headers(headers: &HeaderMap) -> String {
    let mut map = serde_json::Map::new();
    for (name, value) in headers.iter() {
        if REDACTED_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let v = value
            .to_str()
            .map(str::to_string)
            .unwrap_or_else(|_| "<binary>".to_string());
        map.insert(name.as_str().to_string(), serde_json::Value::String(v));
    }
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}

/// Request-side capture, created once per request in `handle()` and carried
/// through the forward leg; `None` while logging is disabled.
#[derive(Debug, Clone)]
pub struct RequestCapture {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub session_id: Option<String>,
    pub request_headers: Option<String>,
    /// Credential-redacted lossy-UTF8 body, truncated to `max_body_bytes`
    /// when capture_bodies is on.
    pub request_body: Option<String>,
    pub request_size: i64,
    pub truncated: bool,
}

impl RequestCapture {
    /// Start a capture for an inbound request; `None` when disabled.
    pub fn start(
        enabled: bool,
        method: &str,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
    ) -> Option<Self> {
        if !enabled {
            return None;
        }
        Some(RequestCapture {
            ts: now_rfc3339(),
            method: method.to_string(),
            path: path.to_string(),
            query: query.map(str::to_string),
            session_id: None,
            request_headers: Some(redact_headers(headers)),
            request_body: None,
            request_size: 0,
            truncated: false,
        })
    }

    /// Attach the (possibly truncated) request body when body capture is on.
    /// The original byte size is always recorded. Credentials are redacted by
    /// [`redact_body`] before the text is stored.
    pub fn set_body(&mut self, body: &[u8], capture_bodies: bool, max_body_bytes: usize) {
        self.request_size = body.len() as i64;
        if !capture_bodies {
            return;
        }
        self.truncated = body.len() > max_body_bytes;
        let capped = &body[..body.len().min(max_body_bytes)];
        self.request_body = Some(redact_body(&String::from_utf8_lossy(capped)));
    }
}

/// Truncate response bytes to the capture cap and lossy-decode them.
pub fn cap_body(bytes: &[u8], max_body_bytes: usize) -> (String, bool) {
    let truncated = bytes.len() > max_body_bytes;
    let capped = &bytes[..bytes.len().min(max_body_bytes)];
    (String::from_utf8_lossy(capped).into_owned(), truncated)
}

/// Persist a request that failed before/inside the forward leg (no usage).
/// Best-effort: logging problems never change the client response.
#[allow(clippy::too_many_arguments)]
pub fn persist_failure(
    store: &Store,
    capture: Option<&RequestCapture>,
    agent: Option<String>,
    attribution: Option<String>,
    provider_id: Option<String>,
    status: StatusCode,
    error_kind: &str,
    error_message: String,
) {
    let Some(c) = capture else {
        return;
    };
    let record = RequestLogNew {
        ts: c.ts.clone(),
        method: c.method.clone(),
        path: c.path.clone(),
        query: c.query.clone(),
        agent,
        attribution,
        provider_id,
        model: None,
        status_code: status.as_u16() as i64,
        error_kind: Some(error_kind.to_string()),
        error_message: Some(error_message),
        session_id: c.session_id.clone(),
        is_streaming: false,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        latency_ms: None,
        first_token_ms: None,
        request_headers: c.request_headers.clone(),
        response_headers: None,
        request_body: c.request_body.clone(),
        response_body: None,
        request_size: c.request_size,
        response_size: 0,
        truncated: c.truncated,
        // Pre-forward failures carry no token usage and hence no cost.
        cost: None,
        cost_currency: None,
    };
    if let Err(e) = store.insert_request_log(&record) {
        tracing::warn!(error = %e, "failed to persist request log");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_headers_drops_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "kw-ag-claude-secret".parse().unwrap());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer sk-upstream".parse().unwrap(),
        );
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("user-agent", "claude-cli/1.0".parse().unwrap());

        let json = redact_headers(&headers);
        assert!(json.contains("content-type"));
        assert!(json.contains("user-agent"));
        assert!(!json.contains("kw-ag-claude-secret"));
        assert!(!json.contains("sk-upstream"));
        assert!(!json.to_lowercase().contains("authorization"));
    }

    #[test]
    fn set_body_respects_cap_and_records_size() {
        let mut c = RequestCapture::start(true, "POST", "/v1/messages", None, &HeaderMap::new())
            .expect("capture");
        c.set_body(b"hello", true, 3);
        assert_eq!(c.request_body.as_deref(), Some("hel"));
        assert!(c.truncated);
        assert_eq!(c.request_size, 5);

        // Body capture off: size recorded, text omitted.
        let mut c =
            RequestCapture::start(true, "POST", "/v1/messages", None, &HeaderMap::new()).unwrap();
        c.set_body(b"hello", false, 1024);
        assert_eq!(c.request_body, None);
        assert!(!c.truncated);
        assert_eq!(c.request_size, 5);

        // Logging disabled → no capture at all.
        assert!(
            RequestCapture::start(false, "POST", "/v1/messages", None, &HeaderMap::new()).is_none()
        );
    }

    fn capture_with_body(body: &str) -> RequestCapture {
        let mut c = RequestCapture::start(true, "POST", "/v1/messages", None, &HeaderMap::new())
            .expect("capture");
        c.set_body(body.as_bytes(), true, 1024 * 1024);
        c
    }

    #[test]
    fn set_body_redacts_secret_key_values() {
        let body = r#"{"model":"claude","api_key":"sk-live-abcdefghijklmnopqrstuvwxyz","temperature":0.2}"#;
        let out = capture_with_body(body).request_body.unwrap();
        assert!(!out.contains("sk-live-abcdefghijklmnopqrstuvwxyz"), "{out}");
        assert!(out.contains(REDACTED));
        // Everything that is not a credential survives.
        assert!(out.contains("claude"));
        assert!(out.contains("temperature"));

        // camelCase and header-style names normalize to the same key.
        let out = capture_with_body(
            r#"{"apiKey":"secretvalue1","Authorization":"Bearer abcdefghijklmnopqrstuvwxyz"}"#,
        )
        .request_body
        .unwrap();
        assert!(!out.contains("secretvalue1"), "{out}");
        assert!(!out.contains("abcdefghijklmnopqrstuvwxyz"), "{out}");

        // A container under a secret-looking name is structure, not a secret:
        // recurse instead of blanking the subtree.
        let out = capture_with_body(r#"{"auth":{"type":"basic"}}"#)
            .request_body
            .unwrap();
        assert!(out.contains("basic"), "{out}");

        // A clean body is stored byte for byte, whitespace and all.
        let pretty = "{\n  \"model\": \"claude\",\n  \"max_tokens\": 16\n}";
        assert_eq!(capture_with_body(pretty).request_body.unwrap(), pretty);

        // A rewritten body is re-serialized but keeps its key order: the log
        // panel shows the request as the client shaped it.
        let out =
            capture_with_body(r#"{"zebra":1,"api_key":"sk-abcdefghijklmnopqrstuvwxyz","alpha":2}"#)
                .request_body
                .unwrap();
        assert!(out.find("zebra") < out.find("alpha"), "{out}");
    }

    #[test]
    fn set_body_redacts_pasted_credentials_in_prompt_text() {
        let body = r#"{"messages":[{"role":"user","content":"my key is sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZZZ, please use it"}]}"#;
        let out = capture_with_body(body).request_body.unwrap();
        assert!(
            !out.contains("sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZZZ"),
            "{out}"
        );
        assert!(out.contains("[REDACTED]"));
        assert!(out.contains("please use it"));

        // `kw-ag-` placeholder keys (kiwano's own) go too.
        let out = capture_with_body(r#"{"note":"use kw-ag-claude-9f3a2b"}"#)
            .request_body
            .unwrap();
        assert!(!out.contains("kw-ag-claude-9f3a2b"), "{out}");

        // Word-boundary guard: `task-sk-…` is prose, not a key.
        let prose = r#"{"note":"the task-sk-abcdefghijklmnopqrstuvwxyz thing"}"#;
        assert!(capture_with_body(prose)
            .request_body
            .unwrap()
            .contains("task-sk-"));

        // Short tails are not credentials; a passing mention survives.
        let prose = r#"{"note":"send a Bearer token next time"}"#;
        assert_eq!(capture_with_body(prose).request_body.unwrap(), prose);
    }

    #[test]
    fn set_body_redacts_sse_and_plain_text() {
        // SSE: a JSON `data:` payload is scrubbed structurally...
        let body = "event: message\ndata: {\"token\":\"abcdef\",\"delta\":\"hi\"}\n\n";
        let out = capture_with_body(body).request_body.unwrap();
        assert!(!out.contains("abcdef"), "{out}");
        assert!(out.contains("delta"));
        assert!(out.starts_with("event: message\ndata: "));
        // ...and a non-JSON payload still gets the shape scan.
        let body = "data: {\"chunk\":\"ghp_ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ\",\n";
        let out = capture_with_body(body).request_body.unwrap();
        assert!(
            !out.contains("ghp_ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ"),
            "{out}"
        );
    }

    #[test]
    fn redaction_keeps_truncation_and_size_accounting() {
        let body =
            r#"{"api_key":"sk-live-abcdefghijklmnopqrstuvwxyz","content":"tail is dropped"}"#;
        let mut c = RequestCapture::start(true, "POST", "/v1/messages", None, &HeaderMap::new())
            .expect("capture");
        c.set_body(body.as_bytes(), true, 60);
        assert_eq!(c.request_size, body.len() as i64);
        assert!(c.truncated);
        let stored = c.request_body.unwrap();
        // The cap cuts the JSON mid-document, so the redaction falls back to
        // the line/shape path — and still catches the key.
        assert!(stored.starts_with("{\"api_key\":\"[REDACTED]"), "{stored}");
        assert!(
            !stored.contains("sk-live-abcdefghijklmnopqrstuvwxyz"),
            "{stored}"
        );
        assert!(stored.len() <= 60);
        // The size recorded is the *original* body, never the redacted text.
        assert!(c.request_size > stored.len() as i64);
    }
}
