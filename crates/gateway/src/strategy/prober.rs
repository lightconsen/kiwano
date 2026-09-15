//! Reachability probing for the providers the usage table cannot speak for.
//!
//! A probe = GET `{base_url}{api_path}`, **no credentials attached** — what is
//! verified is that something answers at that address, not that the key works.
//! Any HTTP response counts (a 401 is an answer; it is the vendor saying no, not
//! the network saying nothing), and only a transport-level failure is `down`.
//!
//! The prober asks about **enabled providers with no requests of their own in the
//! last day**, and nothing else. That scope is the whole point of it existing
//! again: the Status column shows the latency of the provider's real traffic
//! whenever there is any (`vm::health_vm`), so a provider that has been used is
//! already measured — by its own round trips, with the user's key, which is a
//! better number than this loop can produce. What the loop adds is the case where
//! there is nothing to measure: a provider just added, or one that has been idle
//! since yesterday. Probing those costs a request a minute each; probing *all* of
//! them cost a request per provider per half-minute forever, for rows whose
//! number was already on screen.
//!
//! The verdict is written to `provider_health` (migration v21) and read by the
//! Apps screen. It routes nothing: selection is the circuit breaker's job, fed by
//! real traffic — the honest signal for "should this request go here".

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::store::Store;

/// How often a round runs. A minute is twice the round trip a reader needs to
/// trust the number, and half the requests of the loop that was removed in v18.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(60);

/// How long a provider has to answer before the probe counts as failed.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How far back "has traffic of its own" reaches. One day: long enough that a
/// provider used today is never probed, short enough that the number shown for
/// one used yesterday does not read as today's news.
const TRAFFIC_WINDOW_SECS: u64 = 86_400;

/// Long-running probe loop (spawned by main at startup).
pub async fn run(store: Arc<Store>, interval: Duration) {
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "health prober client init failed");
            return;
        }
    };
    loop {
        probe_once(&store, &client).await;
        tokio::time::sleep(interval).await;
    }
}

/// The providers a round has anything to say about: enabled, and with no request
/// of their own inside the window. A store that cannot be read yields none —
/// a failed read must not turn into probing everything.
fn needs_probing(store: &Store, since: &str) -> Vec<crate::store::Provider> {
    let spoken_for: std::collections::HashSet<String> = store
        .usage_by_provider(None, None, Some(since))
        .map(|rows| rows.into_iter().map(|r| r.provider_id).collect())
        .unwrap_or_default();
    store
        .list_providers()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.enabled && !spoken_for.contains(&p.id))
        .collect()
}

/// The RFC3339 instant the traffic window opens at.
fn window_start() -> String {
    crate::store::rfc3339_from_unix(crate::store::unix_now() - TRAFFIC_WINDOW_SECS as i64)
}

/// One probing round: ask each provider that needs it and persist the verdict.
/// Failures do not abort the loop, and one provider's failure is not another's.
pub async fn probe_once(store: &Store, client: &reqwest::Client) {
    for p in needs_probing(store, &window_start()) {
        // The URL the gateway would forward to, from the function that builds
        // it — not a second composition beside it. A probe that measured a URL
        // the gateway never sends to would report a latency for a request
        // nobody makes, and would report a *failure* for the same reason.
        let url = crate::server::data::compose_upstream(&p.base_url, p.api_path.as_deref(), "");
        let started = Instant::now();
        let (status, latency_ms) = match client.get(&url).send().await {
            Ok(_) => ("reachable", started.elapsed().as_millis() as i64),
            Err(_) => ("down", 0),
        };
        // No error text on a failure: the prober's question is reachability, and
        // `status` already answers it. The reason a *test* failed is the app's to
        // record — see `vm::test_provider_latency`.
        if let Err(e) = store.upsert_provider_health(&p.id, status, latency_ms, "probe", None) {
            tracing::warn!(provider = %p.id, error = %e, "failed to persist provider health");
        } else if status == "down" {
            tracing::warn!(provider = %p.id, url = %url, "health probe: unreachable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{now_rfc3339, Billing, Protocol, Provider};

    fn provider(id: &str) -> Provider {
        Provider {
            id: id.into(),
            name: id.into(),
            catalog_id: None,
            prices: None,
            protocol: Protocol::Anthropic,
            base_url: "http://127.0.0.1:1".into(), // port 1 always fails to connect
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            model_default: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            reset_period: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .connect_timeout(Duration::from_millis(500))
            .build()
            .unwrap()
    }

    /// One request recorded against a provider, `secs` ago.
    fn recorded(store: &Store, provider_id: &str, secs: i64) {
        store
            .record_usage(&crate::store::UsageRecord {
                ts: crate::store::rfc3339_from_unix(crate::store::unix_now() - secs),
                agent: "claude".into(),
                provider_id: provider_id.into(),
                model: Some("m".into()),
                input_tokens: 10,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(120),
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
            })
            .unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_provider_is_marked_down() {
        let store = Store::open_in_memory().unwrap();
        store.insert_provider(&provider("dead")).unwrap();

        probe_once(&store, &client()).await;
        let rows = store.list_provider_health().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "down");
        assert_eq!(rows[0].latency_ms, Some(0));
    }

    #[tokio::test]
    async fn disabled_providers_are_not_probed() {
        let store = Store::open_in_memory().unwrap();
        let mut p = provider("off");
        p.enabled = false;
        store.insert_provider(&p).unwrap();

        probe_once(&store, &client()).await;
        assert!(store.list_provider_health().unwrap().is_empty());
    }

    /// The scope this prober exists for: a provider whose latency the Status
    /// column can already show is not asked about. Silence is not a failure, so
    /// the assertion is on *which* rows were probed, not on how many.
    #[tokio::test]
    async fn a_provider_with_its_own_traffic_is_left_alone() {
        let store = Store::open_in_memory().unwrap();
        store.insert_provider(&provider("busy")).unwrap();
        store.insert_provider(&provider("idle")).unwrap();
        recorded(&store, "busy", 60); // a minute ago
        recorded(&store, "idle", 2 * 86_400); // two days ago — outside the window

        probe_once(&store, &client()).await;
        let probed: Vec<String> = store
            .list_provider_health()
            .unwrap()
            .into_iter()
            .map(|h| h.provider_id)
            .collect();
        assert_eq!(probed, vec!["idle".to_string()]);
    }
}
