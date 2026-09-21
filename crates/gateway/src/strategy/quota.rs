//! `quota`: the strategy, and the config payload written outside the engine.
//!
//! The primary's same-day usage decides where a conversation **starts** — under
//! the threshold the primary, over it a backup — and one already running stays
//! put. The day is the user's, from the snapshot, not UTC's.
//!
//! [`QuotaConfig`] is public because it is authored outside the engine too (the
//! CLI writes it), and two definitions of its spelling is how a config that
//! looks accepted ends up silently ignored. `from_json` is the write path that
//! validates what `parse` — the read path — only warns about.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::router::AgentRoute;
use crate::store::{Store, UsageTotals};

use super::StrategyEngine;

/// Config payload of the quota strategy (agent_strategies.config JSON).
///
/// `period` currently supports only `day` (the user's local day); other values are
/// treated as day and warned about in engine logs. The cost unit opens up once the §8 billing model lands.
///
/// Public, because this payload is authored outside the engine too — the CLI
/// writes it — and two definitions of its spelling is how a config that looks
/// accepted ends up silently ignored.
#[derive(Debug, Serialize, Deserialize)]
pub struct QuotaConfig {
    pub limit: f64,
    #[serde(default = "default_quota_unit")]
    pub unit: String,
    #[serde(default)]
    pub period: String,
}

fn default_quota_unit() -> String {
    "requests".into()
}

impl QuotaConfig {
    /// The units [`QuotaConfig::consumed`] acts on.
    pub const UNITS: [&'static str; 2] = ["requests", "tokens"];

    /// Parse and *validate* a config authored outside the engine.
    ///
    /// Stricter than the private [`QuotaConfig::parse`], which stays lenient for
    /// the read path: a config already in the database must not start failing
    /// requests just because it names a unit this build does not know. A writer,
    /// on the other hand, can act on being told — and an unknown unit silently
    /// falling back to `requests` changes what the threshold counts.
    pub fn from_json(config: &str) -> std::result::Result<QuotaConfig, String> {
        let cfg: QuotaConfig =
            serde_json::from_str(config).map_err(|e| format!("invalid quota config JSON: {e}"))?;
        // `is_finite` first: a NaN limit would compare false against everything
        // and quietly never trip the threshold.
        if !cfg.limit.is_finite() || cfg.limit <= 0.0 {
            return Err(format!(
                "quota limit must be a positive number, got {}",
                cfg.limit
            ));
        }
        if !Self::UNITS.contains(&cfg.unit.as_str()) {
            return Err(format!(
                "unknown quota unit \"{}\" (expected {})",
                cfg.unit,
                Self::UNITS.join("|")
            ));
        }
        if !cfg.period.is_empty() && cfg.period != "day" {
            return Err(format!(
                "unknown quota period \"{}\" (only day is supported)",
                cfg.period
            ));
        }
        Ok(cfg)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    fn parse(config: Option<&str>) -> Option<QuotaConfig> {
        let cfg: QuotaConfig = serde_json::from_str(config?).ok()?;
        if cfg.unit != "requests" && cfg.unit != "tokens" {
            tracing::warn!(unit = %cfg.unit, "quota unit not supported; falling back to requests");
        }
        if cfg.period != "day" && !cfg.period.is_empty() {
            tracing::warn!(period = %cfg.period, "quota period not supported; using day");
        }
        Some(cfg)
    }
}

impl QuotaConfig {
    fn consumed(&self, t: &UsageTotals) -> f64 {
        match self.unit.as_str() {
            "tokens" => (t.input_tokens + t.output_tokens) as f64,
            _ => t.requests as f64,
        }
    }
}

impl StrategyEngine {
    /// quota: the primary's same-day usage decides where a conversation
    /// **starts** — under the threshold the primary, over it a backup. One that
    /// is already running stays where it is (see [`Self::pinned`]).
    ///
    /// That last part is the difference between draining and turning over. The
    /// threshold used to move the whole agent at the moment the number was
    /// crossed, which is mid-conversation for whoever happened to be talking —
    /// and the conversation paid for it by rebuilding a prompt cache the
    /// provider had already charged for once.
    pub(crate) async fn select_quota(
        &self,
        store: &Store,
        route: &AgentRoute,
        limits: &crate::limits::LimitState,
        session: Option<&str>,
    ) -> Result<crate::router::UpstreamProvider> {
        if let Some(pinned) = self.drain(route, session).await {
            return Ok(pinned);
        }
        let Some(cfg) = QuotaConfig::parse(route.config.as_deref()) else {
            tracing::warn!(
                agent = %route.agent,
                "quota strategy without valid config; degrading to primary"
            );
            let primary = Self::primary(route)?;
            self.assign_drained(route, session, &primary);
            return Ok(primary);
        };
        let primary = &route.candidates[0];
        // The user's day, from the snapshot — the same boundary the provider
        // billing limits use. It used to be UTC's, which for a UTC+8 user
        // resets "today's quota" at 08:00 and cannot be reasoned about without
        // knowing that.
        let (since, _) = crate::limits::period_start(
            chrono::Utc::now().timestamp(),
            Some("day"),
            limits.tz_offset_minutes(),
        );
        let totals = store.usage_totals_for_provider(&primary.id, since.as_deref())?;
        if cfg.consumed(&totals) < cfg.limit {
            self.assign_drained(route, session, primary);
            return Ok(primary.clone());
        }
        tracing::info!(
            agent = %route.agent,
            provider = %primary.id,
            used = cfg.consumed(&totals),
            limit = cfg.limit,
            "quota threshold reached; new sessions go to the backups"
        );
        for i in 1..route.candidates.len() {
            if self.candidate_available(route, i).await {
                self.assign_drained(route, session, &route.candidates[i]);
                return Ok(route.candidates[i].clone());
            }
        }
        let primary = Self::primary(route)?;
        self.assign_drained(route, session, &primary);
        Ok(primary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{now_rfc3339, Billing, Protocol, StrategyType, UsageRecord};
    use crate::strategy::test_support::{candidate, route, store};

    fn usage_row(provider_id: &str) -> UsageRecord {
        UsageRecord {
            ts: now_rfc3339(),
            agent: "claude".into(),
            provider_id: provider_id.into(),
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
        }
    }

    #[tokio::test]
    async fn the_quota_day_is_the_users_day() {
        let s = store();
        let engine = StrategyEngine::new();
        // The offset the rest of the product runs on, from the same blob.
        s.set_app_setting("ui", r#"{"tz_offset_minutes":480}"#)
            .unwrap();
        s.insert_provider(&crate::store::Provider {
            id: "a".into(),
            name: "a".into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            model_default: None,
            billing: crate::store::Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        })
        .unwrap();

        let mut r = route(
            StrategyType::Quota,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        r.config = Some(r#"{"limit": 5, "unit": "requests"}"#.into());

        // A row the two boundaries date differently — and which direction that
        // is depends on the hour. While the local date still matches UTC's, the
        // local day's early hours are the previous UTC day, so the user's rule
        // counts a row UTC's does not. Once the local date has rolled over,
        // it is UTC's morning that is local yesterday, and the user's rule
        // counts one fewer. Either way this row is the difference.
        let now = chrono::Utc::now().timestamp();
        let utc_day = now.div_euclid(86_400);
        let local_day = (now + 480 * 60).div_euclid(86_400);
        let (row_ts, expected) = if local_day == utc_day {
            (local_day * 86_400 - 480 * 60 + 3_600, "b") // 01:00 local, UTC yesterday
        } else {
            (utc_day * 86_400 + 12 * 3_600, "a") // UTC noon, local yesterday
        };
        for i in 0..5 {
            let mut row = usage_row("a");
            row.ts = chrono::DateTime::from_timestamp(row_ts + i, 0)
                .expect("a valid instant")
                .to_rfc3339();
            s.record_usage(&row).unwrap();
        }
        assert!(
            crate::limits::period_start(row_ts, Some("day"), 0)
                .0
                .as_deref()
                != crate::limits::period_start(row_ts, Some("day"), 480)
                    .0
                    .as_deref(),
            "the row must be one the two boundaries date differently, or this proves nothing"
        );

        let limits = crate::limits::evaluate(&s);
        assert_eq!(
            limits.tz_offset_minutes(),
            480,
            "the snapshot carries the offset"
        );
        assert_eq!(
            engine.select(&s, &r, None, &limits).await.unwrap().id,
            expected,
            "the quota's day is the user's day, not UTC's"
        );
    }

    #[tokio::test]
    async fn quota_over_limit_sinks_to_backup() {
        let s = store();
        s.insert_provider(&crate::store::Provider {
            id: "a".into(),
            name: "a".into(),
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
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            reset_period: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        })
        .unwrap();

        let engine = StrategyEngine::new();
        let mut r = route(
            StrategyType::Quota,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        r.config = Some(r#"{"limit": 5, "unit": "requests"}"#.into());

        // Under the limit → primary
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "a"
        );

        // Primary records 5 rows → over the limit, sink to backup
        for _ in 0..5 {
            s.record_usage(&usage_row("a")).unwrap();
        }
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "b"
        );
    }

    /// The threshold decides where a conversation *starts*. One that is already
    /// running finishes where it is — the switch is for the next one.
    ///
    /// This is the difference the drain makes: the same usage state used to move
    /// the whole agent, mid-conversation for whoever was talking, and the
    /// conversation paid for it by rebuilding a prefix its provider had already
    /// cached and charged for once.
    #[tokio::test]
    async fn quota_drains_a_running_session_and_moves_the_next() {
        let s = store();
        let engine = StrategyEngine::new();
        let mut r = route(
            StrategyType::Quota,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        r.config = Some(r#"{"limit": 5, "unit": "requests"}"#.into());
        let limits = crate::limits::LimitState::default();

        // A conversation starts while the primary is under its quota.
        assert_eq!(
            engine.select(&s, &r, Some("s1"), &limits).await.unwrap().id,
            "a"
        );

        // The primary crosses the threshold while that conversation is running.
        for _ in 0..5 {
            s.record_usage(&usage_row("a")).unwrap();
        }

        // It stays: what it has cached on the primary is worth more than the
        // quota the switch would spread.
        assert_eq!(
            engine.select(&s, &r, Some("s1"), &limits).await.unwrap().id,
            "a",
            "a running conversation is not moved by a threshold"
        );
        // A conversation *starting* now goes to the backup, which is the whole
        // point of the strategy.
        assert_eq!(
            engine.select(&s, &r, Some("s2"), &limits).await.unwrap().id,
            "b"
        );
    }

    #[test]
    fn quota_config_parsing() {
        let cfg = QuotaConfig::parse(Some(r#"{"limit": 100}"#)).unwrap();
        assert_eq!(cfg.unit, "requests");
        assert!(QuotaConfig::parse(Some("not json")).is_none());
        assert!(QuotaConfig::parse(None).is_none());
    }

    /// The read path stays lenient — a row already in the database must not
    /// start failing requests — while the write path rejects what the engine
    /// would silently reinterpret.
    #[test]
    fn quota_config_validation_is_stricter_than_parsing() {
        // Defaults fill in, and the round trip survives.
        let cfg = QuotaConfig::from_json(r#"{"limit": 100}"#).unwrap();
        assert_eq!(cfg.unit, "requests");
        assert_eq!(QuotaConfig::from_json(&cfg.to_json()).unwrap().limit, 100.0);

        // Lenient: parse accepts a unit it will fall back from.
        assert!(QuotaConfig::parse(Some(r#"{"limit":1,"unit":"cost"}"#)).is_some());
        // Strict: a writer is told instead of silently counting something else.
        let err = QuotaConfig::from_json(r#"{"limit":1,"unit":"cost"}"#).unwrap_err();
        assert!(err.contains("cost"), "{err}");
        assert!(err.contains("requests|tokens"), "{err}");

        for bad in [
            r#"{"limit":0}"#,
            r#"{"limit":-5}"#,
            r#"{"unit":"tokens"}"#,
            "not json",
            r#"{"limit":10,"period":"week"}"#,
        ] {
            assert!(
                QuotaConfig::from_json(bad).is_err(),
                "should have been rejected: {bad}"
            );
        }

        // The period the engine does support stays accepted.
        assert!(QuotaConfig::from_json(r#"{"limit":10,"period":"day"}"#).is_ok());
    }
}
