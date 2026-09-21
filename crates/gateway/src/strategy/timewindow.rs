//! `timewindow`: the binding's local `HH:MM` window, and the clock it is read on.
//!
//! The window decides where a conversation *starts* — one already running keeps
//! its provider when the window closes — and the time it is matched against is
//! the stored `tz_offset_minutes`, not the daemon host's zone: it is the user's,
//! and it is the same offset the quota day boundary is read from, so a window
//! and a "today's quota" reset cannot disagree about what time it is.

use crate::error::Result;
use crate::router::AgentRoute;

use super::StrategyEngine;

/// Parse "HH:MM" into minutes-of-day.
fn hhmm_minutes(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Minutes-of-day on the user's clock — the same one the quota day boundary is
/// read from (`limits::period_start`), so a window and a "today's quota" reset
/// cannot disagree about what time it is.
///
/// The offset is the stored `tz_offset_minutes` (minutes east of UTC), not the
/// daemon host's timezone. A gateway run on a UTC host for a UTC+8 user used to
/// peak and go off-peak at the wrong hours; it is fixed, so a session spanning a
/// DST transition keeps the offset it started with (`ui_tz_offset_minutes` is
/// rewritten by the app at launch).
fn local_minutes_of_day(tz_offset_minutes: i64) -> u32 {
    use chrono::Timelike;
    let local = chrono::Utc::now() + chrono::Duration::minutes(tz_offset_minutes);
    let t = local.time();
    t.hour() * 60 + t.minute()
}

/// Whether now falls inside [start, end] (start > end means an overnight window).
fn in_window(now_min: u32, start: &str, end: &str) -> bool {
    match (hhmm_minutes(start), hhmm_minutes(end)) {
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

impl StrategyEngine {
    /// timewindow: match the user's local time windows in candidate order
    /// (overnight supported); falls back to the primary on no match.
    pub(crate) async fn select_timewindow(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
        tz_offset_minutes: i64,
    ) -> Result<crate::router::UpstreamProvider> {
        // The window decides where a conversation *starts*. One that is already
        // running keeps its provider when the window closes, rather than being
        // handed to the other candidate mid-conversation.
        if let Some(pinned) = self.drain(route, session).await {
            return Ok(pinned);
        }
        let now_min = local_minutes_of_day(tz_offset_minutes);
        let chosen = route
            .candidates
            .iter()
            .find(|c| match (&c.win_start, &c.win_end) {
                (Some(s), Some(e)) => in_window(now_min, s, e),
                _ => false,
            })
            .cloned()
            .unwrap_or_else(|| route.candidates[0].clone());
        self.assign_drained(route, session, &chosen);
        Ok(chosen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StrategyType;
    use crate::strategy::test_support::{candidate, route, store};

    #[tokio::test]
    async fn timewindow_matches_by_priority_and_falls_back() {
        let engine = StrategyEngine::new();
        let s = store();
        // Candidate b holds an all-day window; a has none → b matches
        let r = route(
            StrategyType::Timewindow,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, Some(("00:00", "23:59"))),
            ],
        );
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "b"
        );

        // No window contains the current time (e.g. a narrow window a minute ahead) → back to primary
        let (s2, e2) = narrow_future_window();
        let r2 = route(
            StrategyType::Timewindow,
            vec![candidate("a", 1, None), candidate("b", 1, Some((s2, e2)))],
        );
        assert_eq!(
            engine
                .select(&s, &r2, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "a"
        );
    }

    /// Build an [start,end) window guaranteed not to contain the current local
    /// time (now+2 to now+3 minutes). The offset is the one the callers pass —
    /// `LimitState::default()`, i.e. UTC — so the window is built from the same
    /// clock the engine reads and the test does not depend on the host's zone.
    fn narrow_future_window() -> (&'static str, &'static str) {
        // Clock drift does not affect the assertions: the window holds only two marks within
        // the coming minute, so as long as the test finishes within the same minute, now < start always holds.
        let now_min = local_minutes_of_day(0);
        let s = now_min + 2;
        let e = now_min + 3;
        // Midnight wraparound remains a valid window; format as static HH:MM strings
        let fmt = |m: u32| {
            let m = m % (24 * 60);
            format!("{:02}:{:02}", m / 60, m % 60)
        };
        (
            Box::leak(fmt(s).into_boxed_str()),
            Box::leak(fmt(e).into_boxed_str()),
        )
    }

    #[tokio::test]
    async fn timewindow_supports_overnight_window() {
        let now_min = local_minutes_of_day(0);
        // Overnight window [23:00, 06:00]: now after 23:00 or before 06:00 must match
        let late = now_min >= 23 * 60;
        let early = now_min <= 6 * 60;
        assert_eq!(in_window(now_min, "23:00", "06:00"), late || early);
    }

    /// A ±30-minute window around the user's clock at `offset`, leaked to satisfy
    /// `candidate`'s `&'static str`. Wide enough that the minute ticking over
    /// mid-test cannot move `now` out of it.
    fn window_around(offset: i64) -> (&'static str, &'static str) {
        let now = local_minutes_of_day(offset);
        let fmt = |m: u32| {
            let m = m % (24 * 60);
            format!("{:02}:{:02}", m / 60, m % 60)
        };
        (
            Box::leak(fmt(now + 1440 - 30).into_boxed_str()),
            Box::leak(fmt(now + 30).into_boxed_str()),
        )
    }

    /// The window is matched against the stored offset, not the daemon's host
    /// zone: one instant, two offsets 12 hours apart, opposite picks. Before the
    /// offset was threaded through, both halves read `chrono::Local` and this
    /// could not be stated at all.
    #[tokio::test]
    async fn timewindow_reads_the_stored_offset_not_the_host_zone() {
        let engine = StrategyEngine::new();
        let s = store();
        let (start, end) = window_around(480);
        let r = route(
            StrategyType::Timewindow,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, Some((start, end))),
            ],
        );

        // The window was built on the UTC+8 user's clock → it is the one serving.
        let at_utc8 = engine
            .select(&s, &r, None, &crate::limits::LimitState::with_offset(480))
            .await
            .unwrap();
        assert_eq!(at_utc8.id, "b");

        // The same instant read at UTC-4 is 12 hours away from that window.
        let at_utc_minus4 = engine
            .select(&s, &r, None, &crate::limits::LimitState::with_offset(-240))
            .await
            .unwrap();
        assert_eq!(at_utc_minus4.id, "a");
    }

    /// The window decides where a conversation starts; closing it does not pull
    /// a running one across.
    #[tokio::test]
    async fn timewindow_drains_a_running_session_and_moves_the_next() {
        let engine = StrategyEngine::new();
        let s = store();
        let limits = crate::limits::LimitState::default();

        // A wide-open window on `b`, which wins over `a` while it holds — so the
        // first conversation starts there, and the primary is what a
        // out-of-window request falls back to.
        let mut r = route(
            StrategyType::Timewindow,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, Some(("00:00", "23:59"))),
            ],
        );
        assert_eq!(
            engine.select(&s, &r, Some("s1"), &limits).await.unwrap().id,
            "b"
        );

        // The window closes — the candidates are what the engine reads, so
        // replacing `b`'s window with one that is not now is what "the clock
        // moved on" looks like here.
        let (start, end) = narrow_future_window();
        r.candidates[1].win_start = Some(start.into());
        r.candidates[1].win_end = Some(end.into());

        assert_eq!(
            engine.select(&s, &r, Some("s1"), &limits).await.unwrap().id,
            "b",
            "a running conversation is not moved by a window closing"
        );
        assert_eq!(
            engine.select(&s, &r, Some("s2"), &limits).await.unwrap().id,
            "a",
            "a conversation starting now follows the window — and the fallback"
        );
    }

    #[test]
    fn window_parsing_and_containment() {
        assert_eq!(hhmm_minutes("09:30"), Some(570));
        assert_eq!(hhmm_minutes("24:00"), None);
        assert!(in_window(600, "09:00", "18:00"));
        assert!(!in_window(100, "09:00", "18:00"));
        // Crossing midnight
        assert!(in_window(60, "23:00", "06:00"));
        assert!(in_window(23 * 60 + 30, "23:00", "06:00"));
        assert!(!in_window(12 * 60, "23:00", "06:00"));
    }
}
