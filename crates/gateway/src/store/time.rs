//! Timestamps. A leaf: it reads no store and no setting, so any module may
//! import it without an ordering worry.
//!
//! `chrono` and not `std::time` because the stored shape is RFC3339 text,
//! which every `ts` filter in this module compares lexicographically.

use chrono::Utc;

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Seconds since the Unix epoch.
pub fn unix_now() -> i64 {
    Utc::now().timestamp()
}

/// RFC3339 for a Unix instant, in the same shape `now_rfc3339` writes.
///
/// The shape matters where the result is compared rather than read: `usage.ts`
/// filters are `ts >= ?` against a string, which sorts correctly only while
/// every writer spells the instant the same way.
pub fn rfc3339_from_unix(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(now_rfc3339)
}

// ── Local-day arithmetic and the "in use" window helpers ──
//
// Moved here from `kiwano_core::vm::time` with the provider view
// (`migrate.local.md` §10.21): every caller of these is the aggregation, and the
// aggregation is the daemon's now. They read no store and no setting — the
// offset is passed in — which is what let them move without dragging anything
// along.

/// The first instant of that local day, as the RFC3339 UTC value a `ts >=`
/// filter needs — stored timestamps are UTC, so the boundary has to be too.
pub fn local_day_start(offset_minutes: i64, epoch_secs: i64) -> String {
    let local = epoch_secs + offset_minutes * 60;
    local_day_start_from(offset_minutes, local.div_euclid(86_400))
}

/// Same, from a local day index (which is what the calendar math produces).
pub fn local_day_start_from(offset_minutes: i64, local_day: i64) -> String {
    rfc3339_from_unix(local_day * 86_400 - offset_minutes * 60)
}

/// Minutes-of-day on the user's clock (timewindow windows are the user's local
/// time). Reads the stored `tz_offset_minutes` rather than the host's zone, so
/// the badge and the gateway — which reads the same field — agree on the hour.
pub fn local_minutes_now(tz_offset_minutes: i64) -> u32 {
    use chrono::Timelike;
    let local = chrono::Utc::now() + chrono::Duration::minutes(tz_offset_minutes);
    let t = local.time();
    t.hour() * 60 + t.minute()
}

/// Whether `now_min` falls inside an "HH:MM" window; inclusive bounds, and a
/// start later than the end wraps midnight (same semantics as the gateway).
pub fn in_window(now_min: u32, start: &str, end: &str) -> bool {
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
