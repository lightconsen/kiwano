//! JSON-value helpers the vendor adapters share: a tolerant number reader
//! and the reset-time forms the endpoints report.

/// Parse a JSON value as f64, accepting numbers and numeric strings.
pub(crate) fn parse_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// Extract a reset time accepting ISO strings and epoch numbers. Numbers are
/// auto-detected as seconds (<1e12) or milliseconds; values <= 0 (e.g.
/// Volcengine's "-1 = no active window") mean "no reset".
pub(crate) fn extract_reset_time(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = value.as_i64() {
        if n <= 0 {
            return None;
        }
        let ms = if n < 1_000_000_000_000 { n * 1000 } else { n };
        return millis_to_iso8601(ms);
    }
    None
}

pub(crate) fn millis_to_iso8601(ms: i64) -> Option<String> {
    let secs = ms.div_euclid(1000);
    let nsecs = (ms.rem_euclid(1000) * 1_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nsecs).map(|dt| dt.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reset_time_accepts_strings_seconds_and_millis() {
        assert_eq!(
            extract_reset_time(&json!("2026-08-26T14:12:03.000Z")).as_deref(),
            Some("2026-08-26T14:12:03.000Z")
        );
        // Seconds < 1e12 are scaled to millis.
        assert!(extract_reset_time(&json!(1_782_057_600_i64)).is_some());
        // Millis pass through.
        assert!(extract_reset_time(&json!(1_782_057_600_000_i64)).is_some());
        // Non-positive means "no active window".
        assert!(extract_reset_time(&json!(0)).is_none());
        assert!(extract_reset_time(&json!(-1)).is_none());
    }
}
