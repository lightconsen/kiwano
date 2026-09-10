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

use chrono::{Datelike, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};

use crate::store::{Provider, Store};

/// Start of the current reset period as the RFC3339 UTC instant a `ts >=`
/// filter wants, plus the key a notification dedups on.
///
/// Reset periods follow the user's clock: a monthly limit rolls over at local
/// midnight on the 1st, not at UTC's. `None` means no reset — the limit
/// measures all time, which is also where an unrecognised value lands.
pub fn period_start(
    epoch_secs: i64,
    reset_period: Option<&str>,
    tz_offset_minutes: i64,
) -> (Option<String>, String) {
    let offset = Duration::minutes(tz_offset_minutes);
    // Shift the instant by the offset and read it as UTC: the local wall clock
    // is then the calendar the period boundaries come from.
    let local = Utc
        .timestamp_opt(epoch_secs, 0)
        .single()
        .unwrap_or_else(Utc::now)
        + offset;
    let day = local.date_naive();

    // Local midnight of `d`, written the way `now_rfc3339` writes timestamps —
    // the column is compared lexicographically, so the shape is part of the
    // contract, not cosmetic.
    let midnight_utc = |d: NaiveDate| {
        let naive = d.and_hms_opt(0, 0, 0).expect("midnight is a valid time");
        Utc.from_utc_datetime(&(naive - offset))
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    };
    let first_of = |y: i32, m: u32| NaiveDate::from_ymd_opt(y, m, 1).expect("the 1st exists");

    match reset_period {
        None => (None, "all".into()),
        Some("weekly") => {
            let monday = day - Duration::days(day.weekday().num_days_from_monday() as i64);
            (
                Some(midnight_utc(monday)),
                monday.format("%Y-%m-%d").to_string(),
            )
        }
        Some("yearly") => (
            Some(midnight_utc(first_of(day.year(), 1))),
            format!("{:04}", day.year()),
        ),
        // `monthly` and anything unrecognised.
        _ => (
            Some(midnight_utc(first_of(day.year(), day.month()))),
            format!("{:04}-{:02}", day.year(), day.month()),
        ),
    }
}

/// What a provider's per-period limit is measured in, and how much of it is
/// spent.
pub struct PeriodLimit {
    pub used: f64,
    pub limit: f64,
    /// `requests` | `wan_tokens` | an ISO currency code — the limit's own unit.
    pub unit: String,
    /// The reset period's identity (e.g. `2026-09`), for notification dedup.
    pub period_key: String,
}

/// Read one provider's period limit and how much of it is spent. `None` when
/// there is no limit worth measuring (absent, or zero/negative).
pub fn period_limit_usage(
    store: &Store,
    p: &Provider,
) -> crate::error::Result<Option<PeriodLimit>> {
    let Some(limit) = p.period_limit.filter(|l| *l > 0.0) else {
        return Ok(None);
    };
    // A NULL unit normalizes to requests — the same source the ring percentage
    // reads, so v1 rows keep meaning what they always did.
    let unit = match p.limit_unit.as_deref() {
        Some("wan_tokens") => "wan_tokens",
        // A currency limit compares the period's cost directly: the limit is
        // denominated in the provider's own currency, the one its models are
        // priced in, so no rate is involved. Converting would make the ceiling
        // move with the exchange rate.
        Some(u) if u.len() == 3 => u,
        _ => "requests",
    };
    let (since, period_key) = period_start(
        Utc::now().timestamp(),
        p.reset_period.as_deref(),
        store.ui_tz_offset_minutes(),
    );
    let used = match unit {
        "wan_tokens" => {
            let t = store.usage_totals_for_provider(&p.id, since.as_deref())?;
            (t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_creation_tokens)
                as f64
                / 10_000.0
        }
        u if u.len() == 3 => store
            .usage_cost_by_currency(None, Some(&p.id), since.as_deref())?
            .iter()
            .filter_map(|(currency, cost)| currency.as_deref().map(|_| *cost))
            .sum(),
        _ => {
            store
                .usage_totals_for_provider(&p.id, since.as_deref())?
                .requests as f64
        }
    };
    Ok(Some(PeriodLimit {
        used,
        limit,
        unit: unit.to_string(),
        period_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_start_keys() {
        // 2026-09-07T12:34:56Z (Monday)
        let t = 1_788_784_496_i64;
        let (since, key) = period_start(t, Some("monthly"), 0);
        assert_eq!(since.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert_eq!(key, "2026-09");
        let (since, key) = period_start(t, Some("weekly"), 0);
        assert_eq!(since.as_deref(), Some("2026-09-07T00:00:00Z"));
        assert_eq!(key, "2026-09-07");
        let (since, key) = period_start(t, Some("yearly"), 0);
        assert_eq!(since.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(key, "2026");
        // no reset → all-time totals
        let (since, key) = period_start(t, None, 0);
        assert_eq!(since, None);
        assert_eq!(key, "all");
    }

    #[test]
    fn period_boundaries_follow_the_users_clock() {
        // 2026-09-30T20:00:00Z is already 04:00 on the 1st of October at UTC+8,
        // so that user's month has rolled over while UTC's has not.
        let t = 1_790_798_400_i64;
        let (since, key) = period_start(t, Some("monthly"), 480);
        assert_eq!(key, "2026-10");
        assert_eq!(
            since.as_deref(),
            Some("2026-09-30T16:00:00Z"),
            "local midnight, in UTC"
        );
        let (_, key) = period_start(t, Some("monthly"), 0);
        assert_eq!(key, "2026-09", "the same instant is still September in UTC");
    }
}
