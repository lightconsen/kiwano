//! Endpoint probes: latency, protocol-aware reachability, one prompt round
//! trip, and the live model list.
//!
//! Moved here from `kiwano_core::sidecar` when the daemon took over the probes
//! (`migrate.local.md` §10.15): they are network I/O — the same kind of work
//! the data plane is for — and they write to the health table, which the daemon
//! owns. Nothing else changed: the verdicts, the timeouts and the model parsing
//! are the same.
//!
//! `measure_latency` is the exception that stays a plain function: it is a
//! TCP connect, not an HTTP call, and the CLI calls it too.

use std::net::TcpStream;
use std::time::Duration;

use crate::store::{Provider, Store};
use kiwano_api::agents::PromptLatencyVm;
use kiwano_api::error::ApiError;

pub fn measure_latency(endpoint: &str) -> Result<u64, String> {
    use std::net::ToSocketAddrs;

    let trimmed = endpoint.trim();
    let https = trimmed.starts_with("https://") || !trimmed.contains("://");
    let rest = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return Err("empty endpoint".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| format!("invalid port: {p}"))?,
        ),
        None => (authority.to_string(), if https { 443 } else { 80 }),
    };
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| format!("cannot resolve: {host}"))?;
    let start = std::time::Instant::now();
    TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    Ok(start.elapsed().as_millis() as u64)
}

/// Result of a protocol-aware endpoint probe (GET models on the canonical
/// per-protocol route).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProbeReport {
    /// ok | auth | unsupported | error | unreachable
    pub verdict: String,
    pub status: Option<u16>,
    pub latency_ms: u64,
    /// Human-readable explanation for the UI chip.
    pub detail: String,
}

/// Build the canonical "does this endpoint speak this protocol" URL:
/// the protocol's models list on its well-known path.
pub fn probe_url(protocol: &str, base: &str) -> Result<String, String> {
    let trimmed = base.trim().trim_end_matches('/');
    let root = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    if root.is_empty() {
        return Err("empty endpoint".into());
    }
    let (scheme, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        ("https://", r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        ("http://", r)
    } else {
        ("https://", root)
    };
    // A base that already carries a version segment extends with /models;
    // a bare host gets the protocol's canonical version path.
    let already_versioned =
        rest.ends_with("/v1") || rest.ends_with("/v1beta") || rest.ends_with("/v1alpha");
    let url = match protocol {
        "anthropic" | "openai" | "gemini" if already_versioned => format!("{scheme}{rest}/models"),
        "anthropic" | "openai" => format!("{scheme}{rest}/v1/models"),
        // The native Gemini API lives under /v1beta.
        "gemini" => format!("{scheme}{rest}/v1beta/models"),
        other => return Err(format!("unknown protocol: {other}")),
    };
    Ok(url)
}

/// Attach the protocol's canonical auth headers to a GET request. A blank or
/// missing key is allowed (anonymous probe); an unknown protocol is an error.
fn apply_auth(
    protocol: &str,
    mut req: reqwest::RequestBuilder,
    api_key: Option<&str>,
) -> Result<reqwest::RequestBuilder, String> {
    let key = api_key.map(str::trim).filter(|k| !k.is_empty());
    match protocol {
        "openai" => {
            if let Some(k) = key {
                req = req.bearer_auth(k);
            }
        }
        "anthropic" => {
            if let Some(k) = key {
                req = req.header("x-api-key", k);
            }
            req = req.header("anthropic-version", "2023-06-01");
        }
        "gemini" => {
            if let Some(k) = key {
                req = req.header("x-goog-api-key", k);
            }
        }
        other => return Err(format!("unknown protocol: {other}")),
    }
    Ok(req)
}

/// Probe one endpoint for protocol support: GET the protocol's models route
/// with its canonical auth headers. A 401/403 still proves the route exists
/// (protocol supported, key missing/invalid); only 404/405 means unsupported.
/// Works without an API key.
/// Async on purpose: commands run on the tokio runtime, and a blocking
/// client (which owns its own runtime) panics when dropped inside one.
pub async fn probe_endpoint(
    protocol: &str,
    endpoint: &str,
    api_key: Option<&str>,
) -> Result<ProbeReport, String> {
    let url = probe_url(protocol, endpoint)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let req = apply_auth(protocol, client.get(&url), api_key)?;

    let start = std::time::Instant::now();
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return Ok(ProbeReport {
                verdict: "unreachable".into(),
                status: None,
                latency_ms: start.elapsed().as_millis() as u64,
                detail: format!("connection failed: {e}"),
            });
        }
    };
    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status().as_u16();
    let is_json = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json"));
    let body = resp.text().await.unwrap_or_default();

    let (verdict, detail) = match status {
        s if (200..300).contains(&s) => {
            if !is_json {
                // A marketing page answers 200 on anything — not an API.
                (
                    "error".into(),
                    "endpoint returned HTML, not an API".to_string(),
                )
            } else {
                let models = count_models(&body);
                match models {
                    Some(n) if n > 0 => ("ok".into(), format!("{n} models listed")),
                    _ => ("ok".into(), "route answered".to_string()),
                }
            }
        }
        401 | 403 => (
            "auth".into(),
            "route exists — auth required or key invalid".to_string(),
        ),
        404 | 405 => (
            "unsupported".into(),
            "route not found — protocol not supported".to_string(),
        ),
        s if (500..600).contains(&s) => ("error".into(), format!("upstream error {s}")),
        s => ("error".into(), format!("unexpected status {s}")),
    };
    Ok(ProbeReport {
        verdict,
        status: Some(status),
        latency_ms,
        detail,
    })
}

/// Count models in a models-list body (`data[]`). None when the shape does
/// not match.
pub fn count_models(body: &str) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("data").and_then(|d| d.as_array()).map(|a| a.len())
}

/// Extract the model ids from a models-list body (`data[].id`).
pub fn parse_models(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    // OpenAI-compatible (and Anthropic's compatible list): `data[].id`.
    let openai: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(|x| x.as_str()))
        .map(str::to_string)
        .collect();
    if !openai.is_empty() {
        return openai;
    }
    // Native Gemini: `models[].name`, spelled `models/gemini-2.5-pro` — the
    // prefix is the API's resource path, not part of the model id.
    v.get("models")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("name").and_then(|x| x.as_str()))
        .map(|name| name.strip_prefix("models/").unwrap_or(name).to_string())
        .collect()
}

/// Fetch the live model-name list from a provider endpoint. Requires the API
/// key (cloud providers reject anonymous /models calls). One page: both
/// protocols answer the whole list at once. Sorted and deduped for the
/// dropdown.
pub async fn fetch_model_names(
    protocol: &str,
    endpoint: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API key required".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let url = probe_url(protocol, endpoint)?;
    let req = apply_auth(protocol, client.get(&url), Some(key))?;
    let resp = req
        .send()
        .await
        .map_err(|e| format!("connection failed: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => "auth failed — check the API key".into(),
            404 | 405 => "route not found — protocol not supported".into(),
            s if (500..600).contains(&s) => format!("upstream error {s}"),
            s => format!("unexpected status {s}"),
        });
    }
    let mut names = parse_models(&body);
    names.sort();
    names.dedup();
    Ok(names)
}

/// One prompt round trip, timed.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PromptProbe {
    /// Wall time for the whole exchange: request sent, response read to the end.
    pub latency_ms: u64,
    pub status: u16,
    /// The upstream's own words when it refused. A rejected ping is not a
    /// latency: reporting 180ms for a 401 would read as "fast" on a provider
    /// that never answered.
    pub error: Option<String>,
}

/// The body a latency ping sends: one user turn, one token out.
///
/// Small enough to be cheap on every pricing model, large enough that the answer
/// is a real generation — a zero-token request can be answered from a cache or
/// refused outright. One shape for both flavors, because for a minimal ping they
/// ask the same three fields; the protocols differ in where it goes and how it
/// is authenticated, which is `probe_prompt`'s business, not this one's.
pub fn prompt_body(model: &str) -> String {
    serde_json::json!({
        "model": serde_json::Value::String(model.to_string()),
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "ping" }],
    })
    .to_string()
}

/// The message inside an error body, for both flavors: Anthropic and OpenAI
/// both put it at `error.message`.
pub fn upstream_error_message(body: &str, status: u16) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("upstream answered {status}"))
}

/// Send a prompt to `url` and time the full round trip (headers **and** body).
///
/// Time-to-first-byte is the number a streaming client feels, and it needs a
/// stream to measure; the whole exchange is what a non-streaming one feels and
/// is what this reports — one number, taken the same way for every provider,
/// which is what makes two rows comparable.
pub async fn probe_prompt(
    protocol: &str,
    url: &str,
    api_key: &str,
    model: &str,
) -> Result<PromptProbe, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API key required".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let req = apply_auth(protocol, client.post(url), Some(key))?
        .header("content-type", "application/json")
        .body(prompt_body(model));
    let start = std::time::Instant::now();
    let resp = req
        .send()
        .await
        .map_err(|e| format!("connection failed: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let latency_ms = start.elapsed().as_millis() as u64;
    Ok(PromptProbe {
        latency_ms,
        status,
        error: (!(200..300).contains(&status)).then(|| upstream_error_message(&body, status)),
    })
}

/// What model to ping a provider with: its own default when it has one,
/// otherwise the model its catalog entry publishes a price for — the one model
/// we know the vendor serves.
pub fn prompt_test_model(store: &Store, p: &Provider) -> Option<String> {
    if let Some(m) = p
        .model_default
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return Some(m.to_string());
    }
    let catalog = super::catalog::load_catalog(store);
    let entry = catalog
        .entries
        .iter()
        .find(|e| Some(e.id.as_str()) == p.catalog_id.as_deref())?;
    entry.price_ref.as_ref().map(|r| r.model_id.clone())
}

/// Send one prompt through a provider and time it.
///
/// The URL is composed by the gateway's own function (`compose_upstream`), so
/// the test measures the endpoint the gateway would actually use; the ping is a
/// real completion, because a models-list GET answers from a different code path
/// and a different cache and says nothing about what a request costs in time.
pub async fn test_provider_latency(store: &Store, id: &str) -> Result<PromptLatencyVm, ApiError> {
    let p = store
        .get_provider(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("provider not found: {id}")))?;
    let model = prompt_test_model(store, &p).ok_or_else(|| {
        ApiError::invalid("no model to test with — set a default model on this provider")
    })?;
    let key = p
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| ApiError::invalid("this provider has no API key"))?;
    let protocol = p.protocol.as_str();
    let path = if protocol == "anthropic" {
        "/v1/messages"
    } else {
        "/v1/chat/completions"
    };
    let url = crate::server::data::compose_upstream(&p.base_url, p.api_path.as_deref(), path);
    let probe = probe_prompt(protocol, &url, key, &model).await;
    // What the click measured outlives the click: it is recorded as this
    // provider's verdict, so the Status column can show it. This is the one
    // measurement that works when the provider has no traffic of its own and the
    // prober has not been round — or is not running at all, which happens
    // whenever the daemon up is a build from before the prober existed. The
    // number on the button stays a readout of *this* click; the cell shows the
    // standing verdict, which this just became.
    let (status, latency_ms, error) = test_verdict(&probe);
    store
        .upsert_provider_health(id, status, latency_ms, "test", error.as_deref())
        .map_err(ApiError::failed)?;
    let probe = probe.map_err(ApiError::failed)?;
    Ok(PromptLatencyVm {
        provider_id: id.to_string(),
        model,
        latency_ms: probe.latency_ms,
        status: probe.status,
        error: probe.error,
    })
}

/// What the latency test's result means as a health verdict.
///
/// Any HTTP answer proves reachability — a 401 is the vendor saying no to the
/// key, not the network saying nothing — so a refusal is `reachable` with the
/// vendor's own message beside it, and only a transport failure is `down`. The
/// distinction is the whole reason `provider_health` carries an `error` column:
/// the two read differently on the row, and conflating them sends the reader to
/// the wrong end of the problem.
pub fn test_verdict(probe: &Result<PromptProbe, String>) -> (&'static str, i64, Option<String>) {
    match probe {
        Ok(p) => ("reachable", p.latency_ms as i64, p.error.clone()),
        Err(e) => ("down", 0, Some(e.clone())),
    }
}
