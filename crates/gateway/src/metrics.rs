//! Liveness (`GET /health`) and Prometheus-format metrics (`GET /metrics`).
//!
//! Both sit on the **data port**, and neither asks for a placeholder key — the
//! only two paths here that do not. That is a deliberate exception with a narrow
//! scope, and the reasoning is worth writing down because the rest of this plane
//! is unforgiving on purpose:
//!
//! - The tools that want them are the ones that cannot hold the app's
//!   credentials. A supervisor probing liveness has to be able to say "the
//!   gateway is up but its store is unreadable"; a probe that needs a credential
//!   read out of that store cannot report that the store is broken. Prometheus
//!   scrapes over TCP, and the admin plane — where `/status` already lives — is a
//!   unix socket, which no ordinary scraper can reach.
//! - What they expose is counts, breaker states and a version. Never a key, never
//!   a request body, never anything that lets a caller spend the operator's
//!   money — which is the threat the key gate exists for (a local process posting
//!   on the operator's upstream credentials).
//!
//! The one thing a same-user process could abuse is the identifiers in the
//! labels: which agents are routed, to which route a breaker is open. That is
//! the machine owner's own screen content, but it does not need to be handed to
//! any process that can reach the loopback. So `/metrics` serves per-identity
//! labels **redacted by default** ([`redact_name`]) — a scrape still tells one
//! agent from another without naming it. Configuring `KIWANO_METRICS_TOKEN`
//! turns the key gate on instead: the endpoint answers with the full
//! exposition only to a caller bearing the token, and 401s everyone else.
//! `/health` never carries identifiers, so it is unchanged either way.
//!
//! Neither is metered or logged: a scrape is not a request an agent made, and a
//! usage row per scrape interval would quietly inflate the dashboard.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use sha2::{Digest, Sha256};

use crate::server::GatewayState;
use crate::strategy::circuit_breaker::CircuitState;

/// Can the gateway serve a request? Deliberately narrow: it is about the parts
/// that answer one — the store it reads its configuration from — and about
/// nothing else. An upstream being down is what the routed request itself
/// reports, and it must not read as the gateway being down.
///
/// Nor does having nothing to route count as unhealthy. A fresh install has no
/// takeovers and no bindings, and reporting that as a failure is the classic way
/// to make a health check useless: a supervisor would restart it forever, and
/// restarting does not enrol an agent.
pub fn health(state: &GatewayState) -> Response {
    let routed_agents = state.route_table().routes.len();
    let store_ok = state
        .store
        .count_request_logs(None, None, None, None)
        .is_ok();
    let body = serde_json::json!({
        "status": if store_ok { "ok" } else { "degraded" },
        // Which half is down, so a probe's output is worth reading.
        "store": if store_ok { "ok" } else { "unreadable" },
        "routed_agents": routed_agents,
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "version": state.version,
    });
    let status = if store_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}

/// Prometheus text exposition (format 0.0.4).
///
/// Gauges throughout, including the ones that name a count: usage and log rows
/// are pruned on a retention schedule, so a `_total` over them would be a counter
/// that resets itself and makes every `rate()` a lie.
///
/// `redact` names the audience, not the data: `true` (no token configured) hides
/// the per-identity labels behind [`redact_name`]; `false` (the caller just
/// presented a valid token) is the full exposition.
pub async fn prometheus(state: &GatewayState, redact: bool) -> String {
    let mut out = String::with_capacity(1024);
    let routes = state.route_table();

    out.push_str("# HELP kiwano_up 1 while the gateway can read its own store\n");
    out.push_str("# TYPE kiwano_up gauge\n");
    let store_ok = state
        .store
        .count_request_logs(None, None, None, None)
        .is_ok();
    out.push_str(&format!("kiwano_up {}\n", u8::from(store_ok)));

    out.push_str("# HELP kiwano_build_info The build the gateway is running\n");
    out.push_str("# TYPE kiwano_build_info gauge\n");
    out.push_str(&format!(
        "kiwano_build_info{{version=\"{}\"}} 1\n",
        escape_label(state.version)
    ));

    out.push_str("# HELP kiwano_uptime_seconds Seconds since the gateway started\n");
    out.push_str("# TYPE kiwano_uptime_seconds gauge\n");
    out.push_str(&format!(
        "kiwano_uptime_seconds {}\n",
        state.started_at.elapsed().as_secs()
    ));

    out.push_str("# HELP kiwano_routed_agents Agents the route table knows\n");
    out.push_str("# TYPE kiwano_routed_agents gauge\n");
    out.push_str(&format!("kiwano_routed_agents {}\n", routes.routes.len()));

    out.push_str("# HELP kiwano_route_candidates Providers bound to an agent\n");
    out.push_str("# TYPE kiwano_route_candidates gauge\n");
    for (agent, route) in &routes.routes {
        let label = redact_label(agent, redact);
        out.push_str(&format!(
            "kiwano_route_candidates{{agent=\"{label}\"}} {}\n",
            route.candidates.len()
        ));
    }

    // The signal this endpoint earns its keep on: which candidates the breakers
    // have taken out of the pool, per agent.
    out.push_str("# HELP kiwano_circuit_open 1 while a provider's breaker is open\n");
    out.push_str("# TYPE kiwano_circuit_open gauge\n");
    for (key, circuit) in state.engine.breaker_snapshot().await {
        let label = redact_label(&key, redact);
        out.push_str(&format!(
            "kiwano_circuit_open{{route=\"{label}\"}} {}\n",
            u8::from(circuit == CircuitState::Open)
        ));
    }

    if let Ok(totals) = state.store.usage_totals(None, None, None) {
        out.push_str("# HELP kiwano_usage_rows Metered requests held\n");
        out.push_str("# TYPE kiwano_usage_rows gauge\n");
        out.push_str(&format!("kiwano_usage_rows {}\n", totals.requests));

        out.push_str("# HELP kiwano_usage_tokens Tokens metered across those rows\n");
        out.push_str("# TYPE kiwano_usage_tokens gauge\n");
        for (kind, value) in [
            ("input", totals.input_tokens),
            ("output", totals.output_tokens),
            ("cache_read", totals.cache_read_tokens),
            ("cache_creation", totals.cache_creation_tokens),
        ] {
            out.push_str(&format!("kiwano_usage_tokens{{kind=\"{kind}\"}} {value}\n"));
        }
    }

    if let Ok(rows) = state.store.count_request_logs(None, None, None, None) {
        out.push_str("# HELP kiwano_request_log_rows Request-log rows held\n");
        out.push_str("# TYPE kiwano_request_log_rows gauge\n");
        out.push_str(&format!("kiwano_request_log_rows {rows}\n"));
    }

    out
}

/// Prometheus label values are double-quoted with `\`, `"` and newline escaped.
/// The values here are user-chosen ids, so this is about a malformed exposition
/// rather than about injecting into anything that runs.
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// A per-identity label value: the hash when the caller is authorized to see
/// names, [`redact_name`] otherwise.
fn redact_label(value: &str, redact: bool) -> String {
    if redact {
        redact_name(value)
    } else {
        escape_label(value)
    }
}

/// A per-identity label that a scrape can aggregate and `rate()` across time
/// without ever naming the id on the wire (KIW-PRIV-001). First four bytes of
/// SHA-256, hex: eight characters a glance cannot read back into "claude" while
/// still distinguishing one agent from another.
fn redact_name(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    digest[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    // An id is user-chosen, so a quote in it must not be able to break the
    // exposition — a label that ends early makes every line after it unparseable.
    #[test]
    fn label_values_are_escaped() {
        assert_eq!(escape_label("claude"), "claude");
        assert_eq!(escape_label("a\"b"), "a\\\"b");
        assert_eq!(escape_label("a\\b"), "a\\\\b");
        assert_eq!(escape_label("a\nb"), "a\\nb");
    }

    // The redaction is what an unsupervised scrape gets: stable across scrapes
    // (so `rate()` over a label works), different per id, and unreadable back
    // to the original by sight.
    #[test]
    fn redact_stays_stable_distinct_and_unreadable() {
        let a = redact_name("claude");
        let b = redact_name("codex");
        assert_eq!(a, redact_name("claude"));
        assert_eq!(a.len(), 8);
        assert_ne!(a, b);
        assert!(!a.contains("claude"));
        assert!(!b.contains("codex"));
        assert_eq!(redact_label("claude", true), a);
        assert_eq!(redact_label("claude", false), "claude");
    }

    // A deterministic fixture so a change to the digest is a deliberate act,
    // not an unnoticed one.
    #[test]
    fn redact_fixture_is_stable() {
        assert_eq!(redact_name("claude"), "c857d09d");
    }
}
