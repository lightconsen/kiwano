//! Background health probing (tech.md §4.7 failover groundwork): periodically probe every enabled
//! Provider endpoint, persisting the coarse reachability verdict into the `provider_health` table.
//!
//! A probe = GET `{base_url}{api_path}` (no credentials attached — what is verified is
//! reachability, not authorization; any HTTP response counts as healthy, only transport-level
//! failures mark down). Circuit breaking is handled live by the strategy engine's circuit_breaker;
//! this table serves UI display and diagnostics.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::store::Store;

/// Default probe interval.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Per-probe timeout.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// One probing round: probe each enabled Provider in turn and persist the verdict (failures do not abort the loop).
pub async fn probe_once(store: &Store, client: &reqwest::Client) {
    let providers = match store.list_providers() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "health prober cannot list providers");
            return;
        }
    };
    for p in providers.into_iter().filter(|p| p.enabled) {
        let mut url = p.base_url.trim_end_matches('/').to_string();
        if let Some(prefix) = p.api_path.as_deref().map(|s| s.trim_end_matches('/')) {
            if !prefix.is_empty() {
                url.push_str(prefix);
            }
        }
        let started = Instant::now();
        let (status, latency_ms) = match client.get(&url).send().await {
            Ok(_) => ("healthy", started.elapsed().as_millis() as i64),
            Err(_) => ("down", 0),
        };
        if let Err(e) = store.upsert_provider_health(&p.id, status, latency_ms) {
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
            protocol: Protocol::Anthropic,
            base_url: "http://127.0.0.1:1".into(), // port 1 always fails to connect
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            plan_query: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            reset_period: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    }

    #[tokio::test]
    async fn unreachable_provider_is_marked_down_with_streak() {
        let store = Store::open_in_memory().unwrap();
        store.insert_provider(&provider("dead")).unwrap();
        let client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .connect_timeout(Duration::from_millis(500))
            .build()
            .unwrap();

        probe_once(&store, &client).await;
        let h1 = store.list_provider_health().unwrap();
        assert_eq!(h1.len(), 1);
        assert_eq!(h1[0].status, "down");
        assert_eq!(h1[0].consecutive_failures, 1);

        probe_once(&store, &client).await;
        let h2 = store.list_provider_health().unwrap();
        assert_eq!(h2[0].consecutive_failures, 2);
    }

    #[tokio::test]
    async fn disabled_providers_are_not_probed() {
        let store = Store::open_in_memory().unwrap();
        let mut p = provider("off");
        p.enabled = false;
        store.insert_provider(&p).unwrap();
        let client = reqwest::Client::builder().build().unwrap();

        probe_once(&store, &client).await;
        assert!(store.list_provider_health().unwrap().is_empty());
    }
}
