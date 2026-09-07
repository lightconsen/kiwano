//! 后台健康探测（tech.md §4.7 failover 地基）：周期性探测所有启用的
//! Provider 端点，把粗粒度可达性结论持久化到 `provider_health` 表。
//!
//! 探测 = GET `{base_url}{api_path}`（凭据不携带——验证的是可达性而非
//! 授权；任何 HTTP 应答均视为 healthy，仅传输层失败记 down）。熔断由
//! 策略引擎的 circuit_breaker 实时承担，本表服务于 UI 展示与诊断。

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::store::Store;

/// 默认探测周期。
pub const PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// 单次探测超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 常驻探测循环（main 启动时 spawn）。
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

/// 一轮探测：对所有启用 Provider 逐一探测并落库（失败不中断循环）。
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
            base_url: "http://127.0.0.1:1".into(), // 端口 1 必然连接失败
            api_path: None,
            api_key: None,
            billing: Billing::Metered,
            period_limit: None,
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
