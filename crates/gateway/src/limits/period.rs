//! Period boundaries on the user's clock, and the shape both kinds of limit
//! answer in.
//!
//! `period_start` hands back the instant a `ts >=` filter wants and the key a
//! notification dedups on; `period_span_secs` hands back the two ends the cost
//! forecast divides by. They are two independent spellings of the same
//! arithmetic — same `reset_period` string set, same `_ => monthly` fallback —
//! so they share this file, where the two cannot drift apart.
//!
//! `PeriodLimit` lives here because the period key in it is the one
//! `period_start` returns.

// `Duration` below is chrono's — the two are named apart so the date maths
// stays readable.
use chrono::{Datelike, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};

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
        // The quota strategy's `period: "day"`. Same boundary as every other
        // period here, because "100 requests a day" means the user's day.
        Some("day") => (Some(midnight_utc(day)), day.format("%Y-%m-%d").to_string()),
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

/// Start and end of the current reset period as epoch seconds, on the same
/// local-calendar boundaries `period_start` draws. `None` for a no-reset
/// limit — there is no end to project toward. The cost-forecast alert divides
/// the period's spend so far by the fraction elapsed, so both ends are its
/// raw material.
pub fn period_span_secs(
    epoch_secs: i64,
    reset_period: Option<&str>,
    tz_offset_minutes: i64,
) -> Option<(i64, i64)> {
    let offset = Duration::minutes(tz_offset_minutes);
    let local = Utc
        .timestamp_opt(epoch_secs, 0)
        .single()
        .unwrap_or_else(Utc::now)
        + offset;
    let day = local.date_naive();
    let midnight_local = |d: NaiveDate| {
        let naive = d.and_hms_opt(0, 0, 0).expect("midnight is a valid time");
        Utc.from_utc_datetime(&(naive - offset)).timestamp()
    };
    let first_of = |y: i32, m: u32| NaiveDate::from_ymd_opt(y, m, 1).expect("the 1st exists");
    let next_month = |y: i32, m: u32| if m == 12 { (y + 1, 1) } else { (y, m + 1) };

    let (start, end) = match reset_period {
        None => return None,
        Some("day") => (day, day + Duration::days(1)),
        Some("weekly") => {
            let monday = day - Duration::days(day.weekday().num_days_from_monday() as i64);
            (monday, monday + Duration::days(7))
        }
        Some("yearly") => (first_of(day.year(), 1), first_of(day.year() + 1, 1)),
        // `monthly` and anything unrecognised — the same fallback period_start makes.
        _ => {
            let (ny, nm) = next_month(day.year(), day.month());
            (first_of(day.year(), day.month()), first_of(ny, nm))
        }
    };
    Some((midnight_local(start), midnight_local(end)))
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
    fn the_quota_day_is_the_local_day() {
        // 2026-09-30T20:00:00Z is already 04:00 on the 1st at UTC+8.
        let t = 1_790_798_400_i64;
        let (since, key) = period_start(t, Some("day"), 480);
        assert_eq!(key, "2026-10-01");
        assert_eq!(
            since.as_deref(),
            Some("2026-09-30T16:00:00Z"),
            "local midnight, in UTC"
        );
        let (_, key) = period_start(t, Some("day"), 0);
        assert_eq!(
            key, "2026-09-30",
            "the same instant is a different day in UTC"
        );
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
