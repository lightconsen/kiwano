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
