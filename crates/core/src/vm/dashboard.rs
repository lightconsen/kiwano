//! The Dashboard — assembled by the daemon.
//!
//! Moved to `kiwanod::api::dashboard` (`migrate.local.md` §10.21): every read it
//! makes is the daemon's — the usage table, the price cache, the settings blob —
//! and none of them is a fact about *this* machine, so the whole thing moved
//! with no input from the client beyond the window the screen is showing.
//!
//! The wrapper below keeps the signature every caller knows. The tests are the
//! screen's promises (local day boundaries, the window arithmetic, the agent
//! breakdown) and they run against the daemon's implementation.

use kiwanod::api::dashboard as daemon;
use kiwanod::store::Store;

pub use kiwano_api::dashboard::{
    AgentDistVm, BlockedProviderVm, CurrencyMetaVm, DashboardVm, FilterOptionVm, FooterStatsVm,
    GatewayStatusVm, ProviderDistVm, TrendVm,
};

pub fn build_dashboard(
    store: &Store,
    _aux: &crate::vm::Aux,
    window: &str,
    provider_id: Option<&str>,
    agent: Option<&str>,
) -> Result<DashboardVm, String> {
    daemon::build_dashboard(store, window, provider_id, agent).map_err(|e| e.to_string())
}

/// The footer's totals — served by the daemon (`kiwanod::api::dashboard`).
pub fn build_footer_stats(store: &Store, version: &str) -> Result<FooterStatsVm, String> {
    daemon::build_footer_stats(store, version).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::settings::update_settings;
    use crate::vm::test_support::{
        no_vars, provider, seed_usage_rows, store, store_and_aux_on_one_file, usage_row,
    };
    use crate::vm::time::{rfc3339, unix_now};
    use crate::vm::Aux;
    use kiwanod::store::time::day_key;
    use kiwanod::store::time::{hh00, hour_key, mmdd};
    use kiwanod::store::{Billing, Store};

    /// Seed `requests` rows `offset_days` back (at 23:00 UTC of that calendar
    /// day, so day-bucket boundaries are unambiguous), each with `tokens` in.
    fn seed_usage_at(s: &Store, offset_days: i64, requests: i64, tokens: i64) {
        let now = unix_now();
        let day = now.div_euclid(86_400) - offset_days;
        seed_usage_rows(s, day * 86_400 + 23 * 3600, requests, tokens);
    }

    #[test]
    fn dashboard_windows_cover_the_right_days() {
        // One file, two handles — as production has it, where the app's `Aux`
        // and the gateway's `Store` open the same SQLite file. It is also the
        // only way this test can see a latency average at all: `avg_latency`
        // reads through the `Aux` connection, which in-memory would be a
        // database of its own.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let s = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();
        // Display conversion uses the Hub's published rates and there is no
        // compiled snapshot behind them, so the fixture publishes the rate the
        // cost assertion below converts with.
        let hub = serde_json::json!({
            "version": 1,
            "exchange_rates": { "USD": 1.0, "CNY": 7.1 },
            "models": []
        })
        .to_string();
        // Seeded through the store: the cache is the daemon's table, and its
        // accessor there is the only one that survives (`migrate.local.md`
        // §10.14).
        s.save_hub_models_cache(1, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
        // Distinct magnitudes per age so a window that is too wide or too
        // narrow cannot cancel out: today 2, 3 days back 4, 10 days back 6,
        // 40 days back 8, plus one row 7 calendar days back.
        seed_usage_at(&s, 0, 2, 1_000);
        seed_usage_at(&s, 3, 4, 2_000);
        seed_usage_at(&s, 10, 6, 3_000);
        seed_usage_at(&s, 40, 8, 4_000);
        seed_usage_at(&s, 7, 1, 5_000);
        // Old enough that "all" has to start saying weeks rather than days.
        seed_usage_at(&s, 100, 3, 900);

        let d = |w: &str| build_dashboard(&s, &aux, w, None, None).unwrap();

        // Today: the current UTC day only — the 23:00 rows of the days before
        // it stay out, and so does the one from 40 days back. The chart splits
        // that day into its 24 hours, so the 23:00 rows land in the last bucket.
        let today = d("today");
        assert_eq!((today.requests, today.input_tokens), (2, 2_000));
        assert_eq!(today.trend.len(), 24, "today plots one bar per hour");
        assert_eq!(today.trend[23].date, "23:00");
        assert_eq!(today.trend[23].requests, 2);
        assert_eq!(today.trend[0].requests, 0, "an idle hour is still a bucket");
        assert_eq!(
            today.trend.iter().map(|p| p.requests).sum::<i64>(),
            today.requests,
            "the hourly bars cover the stat's whole window"
        );

        // 7d is today plus the six days before it, so the chart's seven points
        // cover exactly the same span as the stat above them.
        let week = d("7d");
        assert_eq!((week.requests, week.input_tokens), (6, 10_000));
        assert_eq!(week.trend.len(), 7);
        assert_eq!(
            week.trend.iter().map(|p| p.requests).sum::<i64>(),
            week.requests,
            "the chart covers the stat's whole window"
        );

        // 30d adds the 7- and 10-day-old groups (1 + 6) and still excludes the
        // 40-day-old one: 6 + 7 = 13 requests, 10k + 5k + 18k tokens.
        let month = d("30d");
        assert_eq!((month.requests, month.input_tokens), (13, 33_000));
        assert_eq!(month.trend.len(), 30, "30d is one bar per day");
        assert_eq!(month.trend.iter().map(|p| p.requests).sum::<i64>(), 13);
        // Bar i is the day 29 - i days ago, so each seeded group lands where its
        // own label says it does — no bar covering more than the day it names.
        assert_eq!(month.trend[29].requests, 2, "the last bar is today");
        assert_eq!(month.trend[26].requests, 4, "three days back");
        assert_eq!(month.trend[22].requests, 1, "seven days back");
        assert_eq!(month.trend[19].requests, 6, "ten days back");
        assert_eq!(
            month.trend[18].requests, 0,
            "and the quiet days are bars too"
        );

        // Cost is summed in the window and converted for display (default CNY).
        assert!((month.cost - month.requests as f64 * 0.5 * 7.1).abs() < 0.01);

        // All: everything the store holds, including the row no counted window
        // reaches. Its bars are whole local days like every other window's, and
        // they add up to the stat above them — which is the point of a chart
        // whose left edge is "wherever the oldest row is".
        let all = d("all");
        assert_eq!((all.requests, all.input_tokens), (24, 67_700));
        assert_eq!(
            all.trend.iter().map(|p| p.requests).sum::<i64>(),
            all.requests,
            "the bars cover the stat's whole window"
        );
        assert_eq!(
            all.trend[0].requests, 3,
            "the oldest group is the first bar, not a day count back from today"
        );
        // 101 days no longer fit a bar each, so the rows are summed a week at a
        // time: 15 bars of seven days, the last one clipped at today.
        assert_eq!(all.trend.len(), 15, "a long history is summed by the week");

        // The deltas compare each window with the one before it, over the same
        // source: 7d is six requests this week against seven last week.
        assert_eq!(week.requests_delta_pct, Some(-14));
        // 30d against the 30 days before it: 13 against the 40-day-old group's
        // eight.
        assert_eq!(month.requests_delta_pct, Some(63));
        // Yesterday had no traffic, so today has nothing to be a percentage of.
        // Zero would be the claim that the two days matched.
        assert_eq!(today.requests_delta_pct, None);
        // And "all" has no earlier window at all — it *is* every window.
        assert_eq!(all.requests_delta_pct, None);

        // Every seeded row is 100ms, so the two windows really do have the same
        // average: a flat 0%, which is a finding, not the absence of one.
        assert_eq!(week.latency_delta_pct, Some(0));
        assert_eq!(today.latency_delta_pct, None, "nothing ran yesterday");
    }

    #[test]
    fn footer_today_counts_from_local_midnight() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "tz_offset_minutes": 480 }),
            &no_vars(),
        )
        .unwrap();
        // One row a second after *local* midnight at UTC+8 — 16:00:01Z the day
        // before. It is the first moment of the user's day and the stretch a
        // UTC-midnight boundary silently dropped.
        let now = unix_now();
        let local_day = (now + 480 * 60).div_euclid(86_400);
        seed_usage_rows(&s, local_day * 86_400 - 480 * 60 + 1, 1, 1_000);

        let f = build_footer_stats(&s, "test").unwrap();
        assert_eq!(
            f.today_requests, 1,
            "00:00:01 local is today, not yesterday"
        );
        assert_eq!(f.today_tokens, 1_000);
    }

    #[test]
    fn day_boundaries_follow_the_configured_offset() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        let utc_day = now.div_euclid(86_400);
        let local_day = (now + 480 * 60).div_euclid(86_400);
        // One row the two clocks date differently. Which way it can be built
        // depends on the hour, because the offset only opens a gap once one
        // date has rolled over and the other has not. While UTC's date still
        // matches the local one, the local day's first second (16:00Z the day
        // before) is what UTC calls yesterday; once UTC has caught up, a row in
        // UTC's morning is what the user's clock calls yesterday. Only one of
        // the two exists at any given moment — a fixed "23:00Z yesterday",
        // which is what this used to seed, is yesterday on *both* clocks for
        // the first eight hours of every local day, and the test failed there.
        let (row, in_utc, in_local) = if local_day == utc_day {
            (local_day * 86_400 - 480 * 60 + 1, 0, 1)
        } else {
            (utc_day * 86_400 + 3_600, 1, 0)
        };
        seed_usage_rows(&s, row, 1, 1_000);

        // UTC (the default, and what an older settings blob yields).
        assert_eq!(
            build_dashboard(&s, &aux, "today", None, None)
                .unwrap()
                .requests,
            in_utc
        );

        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "tz_offset_minutes": 480 }),
            &no_vars(),
        )
        .unwrap();
        let shifted = build_dashboard(&s, &aux, "today", None, None).unwrap();
        assert_eq!(shifted.requests, in_local, "the two clocks disagree");
        assert_eq!(shifted.trend.len(), 24);
        assert_eq!(
            shifted.trend.iter().filter(|t| t.requests > 0).count(),
            in_local as usize,
            "and the chart plots the day the stat counts"
        );
    }

    #[test]
    fn dashboard_shape_with_usage() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Prov", Billing::Metered))
            .unwrap();
        let now = rfc3339(unix_now());
        s.record_usage(&kiwanod::store::UsageRecord {
            ts: now.clone(),
            agent: "claude".into(),
            provider_id: "p1".into(),
            client_key_id: None,
            model: None,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cache_creation_tokens: 0,
            latency_ms: Some(1200),
            status: "ok".into(),
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
        })
        .unwrap();
        // Seed the request-log rows the headline counts: the forwarded request
        // above plus a pre-forward failure (usage tables never see the latter).
        let log = |status: i64, tokens: (i64, i64)| kiwanod::store::RequestLogNew {
            ts: now.clone(),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: Some("claude".into()),
            attribution: Some("key".into()),
            provider_id: Some("p1".into()),
            model: None,
            status_code: status,
            error_kind: None,
            error_message: None,
            session_id: None,
            is_streaming: false,
            input_tokens: tokens.0,
            output_tokens: tokens.1,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms: Some(1200),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: None,
            response_body: None,
            request_size: 0,
            response_size: 0,
            truncated: false,
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
            request_notes: None,
        };
        s.insert_request_log(&log(200, (1000, 500))).unwrap();
        s.insert_request_log(&log(503, (0, 0))).unwrap();
        // The aux connection is a separate in-memory DB in tests (one shared
        // file in production); mirror the usage row so avg-latency reads see it.
        {
            let c = aux.conn.lock().unwrap();
            c.execute(
                "CREATE TABLE usage (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     ts TEXT NOT NULL, agent TEXT NOT NULL, provider_id TEXT NOT NULL,
                     model TEXT, input_tokens INTEGER NOT NULL DEFAULT 0,
                     output_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                     latency_ms INTEGER, status TEXT NOT NULL DEFAULT 'ok')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO usage (ts, agent, provider_id, input_tokens, output_tokens,
                                    cache_read_tokens, latency_ms)
                 VALUES (?1, 'claude', 'p1', 1000, 500, 100, 1200)",
                rusqlite::params![now],
            )
            .unwrap();
        }
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        // The headline reads request_logs (Logs-card source): both the
        // forwarded and the failed request count, while the usage-derived
        // totals stay limited to the forwarded one.
        assert_eq!(d.requests, 2);
        assert_eq!(d.input_tokens, 1000);
        assert_eq!(d.latency_ms, 1200);
        assert_eq!(d.by_agent[0].tokens, "2k");
        assert_eq!(d.by_provider[0].pct, 100);
        assert!(d.trend.iter().map(|t| t.requests).sum::<i64>() >= 1);

        // Filters narrow every stat to the matching slice — and zero out on
        // a provider with no traffic.
        let fp = build_dashboard(&s, &aux, "7d", Some("p1"), Some("claude")).unwrap();
        assert_eq!(fp.requests, 2);
        assert_eq!(fp.by_provider.len(), 1);
        assert_eq!(fp.by_provider[0].id, "p1");
        assert_eq!(fp.by_agent.len(), 1);
        let fo = build_dashboard(&s, &aux, "7d", Some("ghost"), None).unwrap();
        assert_eq!(fo.requests, 0);
        assert!(fo.by_provider.is_empty());
        assert!(fo.by_agent.is_empty());
    }

    /// The peak premium is the difference between two sums over the *same* rows,
    /// which is only meaningful if both come out of one roll-up and are converted
    /// the same way. Two currencies in the window make the conversion part of the
    /// assertion rather than an identity, and a row whose model publishes no
    /// schedule (off-peak == cost) contributes nothing to the premium.
    #[test]
    fn dashboard_prices_the_peak_premium_over_one_row_set() {
        let (_dir, s, aux) = store_and_aux_on_one_file();
        // A Hub rate table — the quote is deliberately not the real one; what is
        // under test is that it is applied, not what it says.
        let hub = serde_json::json!({
            "version": 99,
            "exchange_rates": { "USD": 1.0, "CNY": 6.0 },
            "models": []
        })
        .to_string();
        s.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
        for (id, name) in [("p1", "Alpha"), ("p2", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }

        // p1: 10 USD at peak against 4 USD off-peak — a 6 USD premium — plus a
        // row in another currency with no schedule at all (one vendor's off-peak
        // halving applies to some models and not others).
        let mut peak = usage_row("p1");
        peak.cost = Some(10.0);
        peak.cost_currency = Some("USD".into());
        peak.cost_off_peak = Some(4.0);
        s.record_usage(&peak).unwrap();
        let mut flat = usage_row("p1");
        flat.cost = Some(3.0);
        flat.cost_currency = Some("CNY".into());
        flat.cost_off_peak = Some(3.0);
        s.record_usage(&flat).unwrap();
        // p2 is billed the same either way: no premium to report.
        let mut even = usage_row("p2");
        even.cost = Some(2.0);
        even.cost_currency = Some("USD".into());
        even.cost_off_peak = Some(2.0);
        s.record_usage(&even).unwrap();

        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        // (10 + 2) USD × 6 + 3 CNY; off-peak (4 + 2) × 6 + 3.
        assert!((d.cost - 75.0).abs() < 1e-6, "{}", d.cost);
        assert!((d.cost_off_peak - 39.0).abs() < 1e-6, "{}", d.cost_off_peak);
        // What the reader subtracts: 36 CNY of peak rates, in the reader's own
        // currency — the same rows priced the other way, not two populations.
        assert!((d.cost - d.cost_off_peak - 36.0).abs() < 1e-6);

        let p1 = d.by_provider.iter().find(|p| p.id == "p1").unwrap();
        let p2 = d.by_provider.iter().find(|p| p.id == "p2").unwrap();
        assert!((p1.cost - 63.0).abs() < 1e-6, "{}", p1.cost);
        assert!(
            (p1.cost_off_peak - 27.0).abs() < 1e-6,
            "{}",
            p1.cost_off_peak
        );
        assert!((p2.cost - 12.0).abs() < 1e-6);
        assert!(
            (p2.cost_off_peak - 12.0).abs() < 1e-6,
            "no schedule, no premium: {}",
            p2.cost_off_peak
        );
        // The slices add up to the headline they were converted from.
        let sum: f64 = d.by_provider.iter().map(|p| p.cost).sum();
        assert!((sum - d.cost).abs() < 1e-6, "{sum} vs {}", d.cost);
        // …and the per-agent row is the same pair for that agent's rows.
        assert!((d.by_agent[0].cost - 75.0).abs() < 1e-6);
        assert!((d.by_agent[0].cost_off_peak - 39.0).abs() < 1e-6);
    }

    #[test]
    fn the_agent_filter_narrows_its_own_breakdown() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        // Two agents with traffic, so the table has something it could fail to
        // leave out. Only the first gets the request_logs twin the headline
        // counts — the filtered slice's total is all this needs.
        seed_usage_rows(&s, now - 90, 3, 1_000);
        for i in 0..2 {
            s.record_usage(&kiwanod::store::UsageRecord {
                ts: rfc3339(now - 60 - i),
                agent: "codex".into(),
                provider_id: "demo-alpha".into(),
                client_key_id: None,
                model: Some("demo-model".into()),
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(100),
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
            })
            .unwrap();
        }

        let all = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        assert_eq!(all.by_agent.len(), 2, "both agents have traffic");

        let one = build_dashboard(&s, &aux, "7d", None, Some("claude")).unwrap();
        assert_eq!(one.by_agent.len(), 1, "the table narrows with the filter");
        assert_eq!(one.by_agent[0].agent, "claude");
        assert_eq!(one.by_agent[0].requests, 3);
        assert_eq!(
            one.by_agent.iter().map(|a| a.requests).sum::<i64>(),
            one.requests,
            "the breakdown totals the same slice the headline counts"
        );
    }

    #[test]
    fn rfc3339_and_day_helpers() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(mmdd("2026-09-07"), "09-07");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // Hour keys match the `YYYY-MM-DDTHH` shape `usage_hourly` groups by,
        // and roll over at midnight like `day_key` does.
        assert_eq!(hour_key(0), "1970-01-01T00");
        assert_eq!(hour_key(7 * 3_600 + 59 * 60), "1970-01-01T07");
        assert_eq!(hour_key(86_400 + 3_600), "1970-01-02T01");
        assert_eq!(hh00("1970-01-01T07"), "07:00");
    }
}
