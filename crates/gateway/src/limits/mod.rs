//! Per-provider billing limits: how much of a limit is spent, and which
//! providers are therefore out of service.
//!
//! The reading half lives here because it has to be shared. The gateway
//! enforces these limits (see `crate::strategy`), and the desktop app notifies
//! about them; if each computed the number its own way they could disagree, and
//! a provider would be blocked by one and reported as fine by the other.
//!
//! Two kinds of limit, one shape of answer:
//!
//!   * **amount** — `period_limit` in `requests`, `wan_tokens` or the
//!     provider's own currency, measured over `reset_period`;
//!   * **plan percent** — `plan_limits` ceilings (`five_hour`, `weekly`) against
//!     the utilization the provider's own endpoint reports (see `plan_quota`).
//!
//! The module is split by concern. Every `pub` item keeps the path it had
//! when this was one file (`limits::period_start`): the facade below
//! re-exports it, because `crates/core::vm::alerts`, `crates/gateway/tests`
//! and the sibling modules of this crate all still name it that way.
//!
//! `period` owns the local-calendar arithmetic both kinds of limit measure
//! over; `usage` reads an amount limit back out of the store; `plan` judges
//! a plan window against its ceiling; `state` is the snapshot the routing
//! path reads; `patrol` is the scheduled half — refresh, evaluate, publish —
//! plus the one-time cleanup of the markers the old app left behind.
//!
//! `state` holds `LimitState`, `evaluate` and `without_blocked` in one file
//! rather than three: the struct's fields are private, privacy reaches only
//! the defining module and its descendants, and a sibling split would have
//! to widen them to `pub(crate)` to compile at all.

pub mod patrol;
pub mod period;
pub mod plan;
pub mod state;
pub mod usage;

// ── the public surface, re-exported so every `limits::x` path still resolves ──

pub use patrol::{clear_legacy_disables, publish, refresh_plan_reports, run, LIMIT_INTERVAL};
pub use period::{period_span_secs, period_start, PeriodLimit};
pub use plan::{window_over, PlanLimits, WindowHit};
pub use state::{
    check_client_key, check_model, evaluate, retry_after_for, route_without_disallowed_providers,
    without_blocked, BlockReason, LimitState,
};
pub use usage::{agent_limit_usage, client_key_limit_usage, period_limit_usage};

/// A stored allowlist as the list it stands for.
///
/// `None` — and a list that parses to nothing — means "no restriction", which is
/// the reading every row that predates the columns gets and the only one that
/// makes the migration behaviour-preserving. An **empty** list deliberately reads
/// the same way: nothing writes one (clearing writes NULL), and `[]` as "allow
/// nothing" would be a footgun with no legitimate use.
pub fn allowlist(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw)
        .map(|v| {
            v.into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|e| {
            // A list we cannot read is not a licence to ignore it, but it is also
            // not worth taking the request down for: an unreadable allowlist can
            // only come from a hand-edited database, and the admin API refuses to
            // write one.
            tracing::warn!(error = %e, raw = %raw, "unreadable allowlist; treating it as absent");
            Vec::new()
        })
}

/// Fixtures more than one submodule's tests need.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::store::Store;

    /// A plain enabled provider, no limits and not yet stored — so a test can
    /// shape it before inserting, since `insert_provider` is a plain INSERT.
    pub(crate) fn test_provider(provider_id: &str) -> crate::store::Provider {
        use crate::store::{Billing, Protocol, Provider};
        Provider {
            id: provider_id.into(),
            name: provider_id.into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            model_default: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: crate::store::now_rfc3339(),
            updated_at: crate::store::now_rfc3339(),
        }
    }

    /// A stored provider on a CNY amount limit, with no usage recorded yet.
    pub(crate) fn limited_provider(
        store: &Store,
        provider_id: &str,
        limit: f64,
    ) -> crate::store::Provider {
        let mut p = test_provider(provider_id);
        p.period_limit = Some(limit);
        p.limit_unit = Some("CNY".into());
        p.reset_period = Some("monthly".into());
        store.insert_provider(&p).unwrap();
        p
    }

    /// One metered row, in the currency given.
    pub(crate) fn record_cost(store: &Store, provider_id: &str, cost: f64, currency: &str) {
        use crate::store::UsageRecord;
        store
            .record_usage(&UsageRecord {
                ts: crate::store::now_rfc3339(),
                agent: "claude".into(),
                provider_id: provider_id.into(),
                client_key_id: None,
                model: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: Some(cost),
                cost_currency: Some(currency.into()),
                cost_off_peak: None,
            })
            .unwrap();
    }

    pub(crate) fn agent_limit(
        agent: &str,
        limit: f64,
        unit: Option<&str>,
        period: &str,
    ) -> crate::store::AgentLimit {
        crate::store::AgentLimit {
            agent: agent.into(),
            period: period.into(),
            period_limit: limit,
            limit_unit: unit.map(str::to_string),
            created_at: crate::store::now_rfc3339(),
            updated_at: crate::store::now_rfc3339(),
        }
    }

    /// One agent's ceilings, as the screen would save them.
    pub(crate) fn set_limits(store: &Store, agent: &str, limits: Vec<crate::store::AgentLimit>) {
        store.replace_agent_limits(agent, &limits).unwrap();
    }

    /// A registered client key for `agent`, returning its row — the handle and
    /// the value, since a test needs the handle to write windows against and the
    /// value to present on a request.
    ///
    /// `suffix` distinguishes two keys of one agent, which is the case a
    /// key-scoped ceiling exists for: registration is idempotent on the value, so
    /// the same suffix names the same key.
    pub(crate) fn client_key(store: &Store, agent: &str, suffix: &str) -> crate::store::ClientKey {
        let value = format!("kw-ag-{agent}-{suffix}");
        let id = store.upsert_client_key(&value, agent).unwrap();
        store.get_client_key(&id).unwrap().unwrap()
    }

    /// One key's windows, as `clients limits` would save them.
    pub(crate) fn set_key_limits(
        store: &Store,
        key_id: &str,
        periods: &[(&str, f64, Option<&str>)],
    ) {
        let now = crate::store::now_rfc3339();
        let limits: Vec<crate::store::ClientKeyLimit> = periods
            .iter()
            .map(|(period, limit, unit)| crate::store::ClientKeyLimit {
                key_id: key_id.to_string(),
                period: (*period).to_string(),
                period_limit: *limit,
                limit_unit: unit.map(str::to_string),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .collect();
        store.replace_client_key_limits(key_id, &limits).unwrap();
    }

    /// One metered row attributed to a client key.
    pub(crate) fn record_key_row(store: &Store, agent: &str, key_id: &str, provider_id: &str) {
        use crate::store::UsageRecord;
        store
            .record_usage(&UsageRecord {
                ts: crate::store::now_rfc3339(),
                agent: agent.into(),
                provider_id: provider_id.into(),
                client_key_id: Some(key_id.into()),
                model: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
            })
            .unwrap();
    }

    pub(crate) fn store_with_spend(provider_id: &str, limit: f64, spent: f64) -> Store {
        let s = Store::open_in_memory().unwrap();
        limited_provider(&s, provider_id, limit);
        record_cost(&s, provider_id, spent, "CNY");
        s
    }

    /// A store on a real file with a Hub price document cached beside it — the
    /// shape production has, where the GUI opens `Aux` on the same database the
    /// gateway opens, so the rates the seeder read and the rates a limit reads
    /// back are one row.
    pub(crate) fn store_with_hub_rates(rates: &str) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        // Through the store's own accessor: the cache is the daemon's table
        // (migration v28), so a fixture that built it by hand was keeping a
        // second copy of the schema — the thing this migration removes
        // (`migrate.local.md` §10.14).
        store
            .save_hub_models_cache(
                1,
                &format!(r#"{{"version":1,"exchange_rates":{rates},"models":[]}}"#),
                "sha",
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        (dir, store)
    }
}
