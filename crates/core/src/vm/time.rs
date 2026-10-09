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

// Day boundaries are the user's, not UTC's: a UTC+8 user's "today" runs from
// 08:00 local yesterday to 08:00 today if we bucket by UTC. Everything that
// asks "which day is this in" goes through these two, with the offset the
// frontend reports (`getTimezoneOffset()`, negated to mean "east of UTC").

// ── "In use" helpers: which candidate would serve a request issued right now,
//    mirroring the gateway's strategy selection (strategy/mod.rs) minus its
//    runtime state (circuit breakers, roundrobin sticky sessions) ──

// The local-day arithmetic and the window helpers moved to `kiwanod::store::time`
// with the provider view (`migrate.local.md` §10.21): the aggregation is their
// only caller and the aggregation is the daemon's now. Re-exported so the paths
// here are unchanged.
