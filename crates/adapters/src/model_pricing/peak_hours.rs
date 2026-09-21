//! When a row's own (peak) rates apply, in the **vendor's** clock.
//!
//! A window is half-open and never wraps midnight; a value that is not a
//! clock time leaves the window unmatched rather than matching broadly,
//! because matching is what turns a typo into a wrong price.
//!
//! A leaf: it reads the `PeakHours` it is handed and nothing else.

use crate::model_pricing::types::PeakHours;

/// Is `at` (unix seconds) inside a peak window?
///
/// Read in the **vendor's** clock: `tz_offset` is minutes east of UTC and the
/// windows are that vendor's business hours, so judging them against the
/// reader's own timezone would pick the wrong rate silently — the one mistake
/// this shape exists to prevent.
///
/// A window is half-open, `[start, end)`, which is what lets two adjacent
/// windows meet without a gap. A window whose times are not clock times at all,
/// or whose day list is empty, never matches: matching is what turns a typo into
/// a wrong price, and the conservative direction is to charge the peak.
///
/// Note that a row carrying tiers is the row's *own* schedule. Since the Hub
/// names the provider on every row it publishes, a reseller that prices a model
/// itself is billed on its own clock; only a model it does not price — billed
/// from another provider's row — puts it on that provider's clock, which is the
/// best available answer rather than a chosen one.
pub fn is_peak(hours: &PeakHours, at: i64) -> bool {
    use chrono::{Datelike, Timelike};

    if hours.windows.is_empty() {
        return false;
    }
    // The shifted DateTime is wrong as an instant and right as a wall clock,
    // which is exactly what a vendor's business hours need. Same idiom as
    // `gateway::limits::period_start`.
    let shifted = chrono::DateTime::from_timestamp(at, 0).unwrap_or_default()
        + chrono::Duration::minutes(hours.tz_offset as i64);
    let today = shifted.weekday().num_days_from_monday() as usize;
    let minutes = (shifted.time().hour() * 60 + shifted.time().minute()) as i64;

    hours.windows.iter().any(|w| {
        if !w.days.iter().any(|d| day_index(d) == Some(today)) {
            return false;
        }
        let (Some(start), Some(end)) = (minutes_of_day(&w.start), minutes_of_day(&w.end)) else {
            return false;
        };
        start <= minutes && minutes < end
    })
}

/// `"HH:MM"` to minutes since midnight, or `None` when it is not that — a
/// malformed time must leave the window unmatched rather than match broadly.
fn minutes_of_day(hhmm: &str) -> Option<i64> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// The weekday names the document uses, in `num_days_from_monday` order.
/// Case-insensitive: a hand-edited `"Mon"` that silently stopped matching would
/// misprice a whole week.
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

fn day_index(day: &str) -> Option<usize> {
    let d = day.trim().to_ascii_lowercase();
    DAYS.iter().position(|x| *x == d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_pricing::test_support::deepseek_hours;
    use crate::model_pricing::types::PeakWindow;

    #[test]
    fn is_peak_reads_the_vendor_clock() {
        let hours = deepseek_hours();
        // Wed 2026-09-09, Beijing: 08:59:59 off, 09:00:00 on (start inclusive),
        // 11:59:59 on, 12:00:00 off (end exclusive), 13:00 the lunch gap off,
        // 14:00 on again, 18:00 off.
        for (epoch, peak) in [
            (1_788_915_599_i64, false),
            (1_788_915_600, true),
            (1_788_919_200, true),
            (1_788_926_399, true),
            (1_788_926_400, false),
            (1_788_930_000, false),
            (1_788_933_600, true),
            (1_788_948_000, false),
            // Sat 2026-09-12 10:00 Beijing: the weekend is never peak.
            (1_789_178_400, false),
        ] {
            assert_eq!(is_peak(&hours, epoch), peak, "at {epoch}");
        }
    }

    /// The same instant, judged on the wrong clock: this is the mistake the
    /// offset exists to prevent, so it gets its own test.
    #[test]
    fn is_peak_ignores_the_readers_timezone() {
        let mut hours = deepseek_hours();
        // Wed 10:00 Beijing is 02:00 UTC — outside the windows if read as UTC.
        assert!(is_peak(&hours, 1_788_919_200));
        hours.tz_offset = 0;
        assert!(!is_peak(&hours, 1_788_919_200));
    }

    /// A schedule must never match broadly: a typo in a field disables the
    /// window, and the peak rate applies.
    #[test]
    fn unparseable_windows_never_match() {
        let at = 1_788_919_200; // Wed 10:00 Beijing
        let with = |window: PeakWindow| PeakHours {
            tz_offset: 480,
            windows: vec![window],
        };
        let days = vec!["wed".to_string()];
        // Case-insensitive, so a hand-edited "Wed" still matches.
        assert!(is_peak(
            &with(PeakWindow {
                days: vec!["Wed".into()],
                start: "09:00".into(),
                end: "12:00".into()
            }),
            at
        ));
        // "9:00" is nine o'clock and matches — leniency here cannot charge the
        // wrong rate. What must not match is a value that is not a clock time.
        assert!(!is_peak(
            &with(PeakWindow {
                days: vec![],
                start: "09:00".into(),
                end: "12:00".into()
            }),
            at
        ));
        assert!(is_peak(
            &with(PeakWindow {
                days: days.clone(),
                start: "9:00".into(),
                end: "12:00".into()
            }),
            at
        ));
        for (start, end) in [("abc", "12:00"), ("09:00", "25:00"), ("09:00", "12:60")] {
            assert!(
                !is_peak(
                    &with(PeakWindow {
                        days: days.clone(),
                        start: start.into(),
                        end: end.into()
                    }),
                    at
                ),
                "{start}-{end} must not match"
            );
        }
        // An empty schedule is the same answer.
        assert!(!is_peak(
            &PeakHours {
                tz_offset: 480,
                windows: vec![]
            },
            at
        ));
    }
}
