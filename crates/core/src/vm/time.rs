//! Time and window arithmetic — UTC instants, local day keys, and the rolling
//! windows a quota or a chart is measured over.
//!
//! Deliberately dependency-free: nothing here reads a `Store` or a setting, so
//! every other module can import it without an ordering worry.

use std::time::{SystemTime, UNIX_EPOCH};

// ── UTC date helpers (no chrono dependency; RFC3339 UTC keeps lexicographic
//    ordering, which is exactly what the store's `ts >= ?` filters expect) ──

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days-since-epoch → (y, m, d), Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn rfc3339(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let secs = epoch_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

pub(crate) fn day_key(epoch_secs: i64) -> String {
    let (y, m, d) = civil_from_days(epoch_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DDTHH` — the bucket key `Store::usage_hourly` groups by, from local
/// time already shifted by the caller's offset.
pub(crate) fn hour_key(local_secs: i64) -> String {
    let (y, m, d) = civil_from_days(local_secs.div_euclid(86_400));
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}",
        local_secs.rem_euclid(86_400) / 3600
    )
}

/// `MM-DD` label for the dashboard trend axis.
pub(crate) fn mmdd(day: &str) -> String {
    day.get(5..10).unwrap_or(day).to_string()
}

/// (y, m, d) → days since epoch: Howard Hinnant's `days_from_civil`, the
/// inverse of [`civil_from_days`].
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The local day index a `YYYY-MM-DD` bucket key names — the inverse of
/// [`local_day_key`], for a chart whose left edge is wherever the oldest bucket
/// is rather than a day count back from today.
pub(crate) fn day_index_of_key(key: &str) -> Option<i64> {
    Some(days_from_civil(
        key.get(0..4)?.parse().ok()?,
        key.get(5..7)?.parse().ok()?,
        key.get(8..10)?.parse().ok()?,
    ))
}

/// `HH:00` label for the dashboard trend axis when it plots hours.
pub(crate) fn hh00(hour: &str) -> String {
    hour.get(11..13)
        .map(|h| format!("{h}:00"))
        .unwrap_or_else(|| hour.to_string())
}

// Day boundaries are the user's, not UTC's: a UTC+8 user's "today" runs from
// 08:00 local yesterday to 08:00 today if we bucket by UTC. Everything that
// asks "which day is this in" goes through these two, with the offset the
// frontend reports (`getTimezoneOffset()`, negated to mean "east of UTC").

/// The local calendar date (`YYYY-MM-DD`) containing a unix timestamp.
pub(crate) fn local_day_key(offset_minutes: i64, epoch_secs: i64) -> String {
    day_key(epoch_secs + offset_minutes * 60)
}

/// The first instant of that local day, as the RFC3339 UTC value a `ts >=`
/// filter needs — stored timestamps are UTC, so the boundary has to be too.
pub(crate) fn local_day_start(offset_minutes: i64, epoch_secs: i64) -> String {
    let local = epoch_secs + offset_minutes * 60;
    local_day_start_from(offset_minutes, local.div_euclid(86_400))
}

/// Same, from a local day index (which is what the calendar math produces).
pub(crate) fn local_day_start_from(offset_minutes: i64, local_day: i64) -> String {
    rfc3339(local_day * 86_400 - offset_minutes * 60)
}

// ── "In use" helpers: which candidate would serve a request issued right now,
//    mirroring the gateway's strategy selection (strategy/mod.rs) minus its
//    runtime state (circuit breakers, roundrobin sticky sessions) ──

/// Minutes-of-day on the user's clock (timewindow windows are the user's local
/// time). Reads the stored `tz_offset_minutes` rather than the host's zone, so
/// the badge and the gateway — which reads the same field — agree on the hour.
pub(crate) fn local_minutes_now(tz_offset_minutes: i64) -> u32 {
    use chrono::Timelike;
    let local = chrono::Utc::now() + chrono::Duration::minutes(tz_offset_minutes);
    let t = local.time();
    t.hour() * 60 + t.minute()
}

/// Whether `now_min` falls inside an "HH:MM" window; inclusive bounds, and a
/// start later than the end wraps midnight (same semantics as the gateway).
pub(crate) fn in_window(now_min: u32, start: &str, end: &str) -> bool {
    let parse = |s: &str| -> Option<u32> {
        let (h, m) = s.trim().split_once(':')?;
        let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
        (h < 24 && m < 60).then_some(h * 60 + m)
    };
    match (parse(start), parse(end)) {
        (Some(s), Some(e)) => {
            if s <= e {
                now_min >= s && now_min <= e
            } else {
                now_min >= s || now_min <= e
            }
        }
        _ => false,
    }
}
