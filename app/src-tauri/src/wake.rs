//! Recover the window after the machine slept.
//!
//! A Kiwano window can come back from sleep blank. It is not a crashed page: the
//! renderer keeps running, the status bar's numbers keep moving, and the gateway
//! keeps answering — but the canvas is white. What died is the WebKit GPU
//! process, the helper that composites the page. Measured on one machine, twice
//! in a week: the display woke, the GPU process was judged unresponsive and
//! killed within three seconds of the wake, its replacement was killed the same
//! way, and the page was left with no rendering backend to ask for pixels from.
//! Whatever it draws afterwards never reaches the window.
//!
//! Nothing in the page can see that state, and this was measured rather than
//! assumed, by killing the GPU process under a live WKWebView: timers and
//! `requestAnimationFrame` keep ticking at 60 Hz, a WebGL context stays current,
//! `fetch` runs, and the document stays "visible". `takeSnapshot` does fail —
//! but only for the second or two before WebKit relaunches the GPU process;
//! afterwards it returns a fresh image while the window is still white.
//!
//! So the trigger is the OS saying the machine woke, which is the one fact the
//! page cannot forge. A reload follows, and it is the same recovery as quitting
//! and reopening: it asks for a rendering backend and a fresh layer tree instead
//! of waiting for a compositor that believes it has both. The route survives
//! (it lives in the URL hash); screen state that does not — a half-typed filter,
//! a scroll position — is the cost, paid only on a real wake.
//!
//! Not on *every* wake, though: the display sleeps after a few idle minutes and
//! wakes the moment the mouse moves, so a wake seconds after a sleep is a
//! machine that never stopped working, and reloading there would only throw away
//! the screen for nothing. The two notifications come in pairs (will-sleep then
//! did-wake), so the length of the absence is known without a timer — and a wake
//! with no matching sleep (an event that never arrived) is treated as the real
//! thing, since the failure it guards against is a white window, not a wasted
//! reload. A system wake raises the screen's note as well as its own, so the
//! notes of one return are timed together (`Episode`) and answer together.
//!
//! Deliberately not a timer: a window that was merely minimised is App Napped,
//! and its timers stop for minutes at a time (measured: a 169 s gap with the
//! document hidden throughout), which any wall-clock heuristic would misread as
//! a sleep and answer by throwing the user's screen state away.
//!
//! macOS only, for now: the incident is the macOS WebKit GPU process. Windows
//! and Linux register nothing and keep the old behaviour.

#[cfg(any(target_os = "macos", test))]
use std::time::SystemTime;

use tauri::WebviewWindow;

/// Away for at least this long and the wake reloads. Above a display that
/// nodded off between two clicks; below any sleep that could strand the page.
#[cfg(any(target_os = "macos", test))]
pub const MIN_ABSENCE_SECS: u64 = 30;

/// Reload `window` when the machine wakes from a sleep long enough to matter.
#[cfg(target_os = "macos")]
pub fn watch(window: WebviewWindow) {
    macos::watch(window);
}

/// Nothing to watch on this platform — see the module note.
#[cfg(not(target_os = "macos"))]
pub fn watch(_window: WebviewWindow) {}

/// Was the machine away long enough for the compositor to have died? `since` is
/// the wall clock at the sleep, which keeps running while the machine does not
/// (a monotonic clock would stop with it and always answer "no time passed").
#[cfg(any(target_os = "macos", test))]
pub fn away_long_enough(since: Option<SystemTime>, now: SystemTime) -> bool {
    since.is_none_or(|t| {
        now.duration_since(t)
            .map(|d| d.as_secs() >= MIN_ABSENCE_SECS)
            // A clock set back mid-sleep: the pair cannot be timed, and the
            // white window is the worse failure.
            .unwrap_or(true)
    })
}

/// Wakes this close together describe the same return: a system wake raises the
/// screen note as well as its own, milliseconds apart.
#[cfg(any(target_os = "macos", test))]
pub const SAME_RETURN_SECS: u64 = 5;

/// One sleep and the wakes that end it, shared by both notification pairs.
///
/// The two pairs must decide together. A system wake raises the screen note too,
/// and if each wake read the sleep stamp on its own the second would find it
/// gone, read that as "no sleep was seen", and reload however short the
/// absence — walking straight through the threshold. So the wakes that describe
/// one return get one answer: the first is timed against the sleep, and its
/// companions are recognised as the same event and stay quiet.
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
pub struct Episode {
    slept_at: Option<SystemTime>,
    /// The last wake that was answered, for spotting its companions.
    answered_at: Option<SystemTime>,
}

#[cfg(any(target_os = "macos", test))]
impl Episode {
    pub fn sleep(&mut self, at: SystemTime) {
        self.slept_at = Some(at);
        self.answered_at = None;
    }

    /// Did this wake end an absence long enough to reload?
    pub fn wake(&mut self, now: SystemTime) -> bool {
        // A wake moments after one already answered is the other note of the
        // same return, not a new absence.
        let same_return = self
            .answered_at
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|d| d.as_secs() < SAME_RETURN_SECS);
        if same_return {
            return false;
        }
        self.answered_at = Some(now);
        away_long_enough(self.slept_at.take(), now)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ptr::NonNull;
    use std::sync::Mutex;
    use std::time::SystemTime;

    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
        NSWorkspaceScreensDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;
    use tauri::WebviewWindow;

    use super::Episode;

    pub fn watch(window: WebviewWindow) {
        // SAFETY: AppKit's power note names are `&'static` constants, safe to
        // hold for the process's life.
        let (will_sleep, did_wake, screens_sleep, screens_wake) = unsafe {
            (
                NSWorkspaceWillSleepNotification,
                NSWorkspaceDidWakeNotification,
                NSWorkspaceScreensDidSleepNotification,
                NSWorkspaceScreensDidWakeNotification,
            )
        };
        // One episode, shared by all four observers: a sleep stamps it, a wake
        // reads it, and the second wake of one return is recognised as that
        // return's companion rather than a new absence.
        //
        // Leaked because the observers are: each one is registered for the
        // process's life, and an `Episode` that outlives the local frame is the
        // shape that lets those blocks hold it without each keeping its own copy
        // of state they must share.
        let episode: &'static Mutex<Episode> = Box::leak(Box::new(Mutex::new(Episode::default())));

        // Both wakes, because the measured incidents were both kinds: once the
        // machine came back from hibernate (`Wake from Hibernate`), and once
        // only the display had been off (`kCGSDisplayDidWake`, the system never
        // slept) — and the GPU process died in each. A system wake raises the
        // screen note as well as its own; `Episode` keeps the pair to one
        // answer.
        for (sleep, wake) in [(will_sleep, did_wake), (screens_sleep, screens_wake)] {
            observe(sleep, move || {
                episode
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .sleep(SystemTime::now());
            });
            let window = window.clone();
            observe(wake, move || {
                let reload_now = episode
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .wake(SystemTime::now());
                if !reload_now {
                    return;
                }
                // Says why a page the user left where it was came back reloaded.
                tracing::info!("reloading the window after a wake");
                reload(&window);
            });
        }
    }

    /// Run `action` on every notification named `name`.
    ///
    /// The closure is `'static` because the registration outlives this call: the
    /// observer it returns is kept by Foundation and leaked below.
    fn observe(name: &'static objc2_foundation::NSNotificationName, action: impl Fn() + 'static) {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| action());
        // The workspace's own centre, not the default one: that is where AppKit
        // posts its power notes. `None` object means "every notification with
        // this name", and `None` queue means the block runs on the thread that
        // posts — which `reload` hops off.
        //
        // SAFETY: the names are AppKit's statics, and the block outlives the
        // call because it is moved into the observer Foundation keeps for it.
        let observer = unsafe {
            NSWorkspace::sharedWorkspace()
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        // Freed with the process: the observer's whole purpose is to still be
        // registered on a wake that may be days away, so there is nothing to
        // drop it from.
        std::mem::forget(observer);
    }

    /// Ask the window for the page again, on the thread that owns it.
    fn reload(window: &WebviewWindow) {
        let inner = window.clone();
        window
            .run_on_main_thread(move || {
                if let Err(e) = inner.reload() {
                    tracing::warn!("could not reload the window after wake: {e}");
                }
            })
            .unwrap_or_else(|e| {
                tracing::warn!("could not reach the main thread after wake: {e}");
            });
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{away_long_enough, Episode, MIN_ABSENCE_SECS};

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
    }

    #[test]
    fn a_display_that_nodded_off_is_not_a_sleep() {
        // The screen sleeps after a few idle minutes and wakes on the first
        // mouse move; reloading there would cost the screen for nothing.
        assert!(!away_long_enough(Some(at(0)), at(2)));
        assert!(!away_long_enough(Some(at(0)), at(MIN_ABSENCE_SECS - 1)));
    }

    #[test]
    fn a_real_sleep_reloads() {
        assert!(away_long_enough(Some(at(0)), at(MIN_ABSENCE_SECS)));
        assert!(away_long_enough(Some(at(0)), at(8 * 60 * 60)));
    }

    #[test]
    fn a_wake_with_no_sleep_notice_reloads() {
        // The guard exists for a white window; a note that never arrived must
        // not be the reason the page stays blank.
        assert!(away_long_enough(None, at(0)));
    }

    #[test]
    fn a_clock_set_back_mid_sleep_reloads() {
        // `duration_since` fails when the later instant is earlier; the pair
        // cannot be timed, and the white window is the worse failure.
        assert!(away_long_enough(Some(at(100)), at(0)));
    }

    #[test]
    fn one_return_answers_once() {
        // A system wake raises the screen note as well, a few tens of
        // milliseconds apart. The first one takes the sleep stamp; if the
        // second read it alone it would see "no sleep was seen" and reload
        // through the threshold — which is exactly what happened on a real
        // three-second sleep before this pairing existed.
        let mut episode = Episode::default();
        episode.sleep(at(0));
        assert!(!episode.wake(at(3)), "a three-second sleep must not reload");
        assert!(!episode.wake(at(3)), "the companion note must stay quiet");
    }

    #[test]
    fn a_later_return_is_a_new_absence() {
        let mut episode = Episode::default();
        episode.sleep(at(0));
        assert!(episode.wake(at(MIN_ABSENCE_SECS)));
        episode.sleep(at(10 * 60));
        assert!(episode.wake(at(10 * 60 + MIN_ABSENCE_SECS)));
    }

    #[test]
    fn a_wake_before_any_sleep_answers_once() {
        // Out of order or missed sleep notes: the first wake reloads (white
        // window is the worse failure) and its companion stays quiet.
        let mut episode = Episode::default();
        assert!(episode.wake(at(0)));
        assert!(!episode.wake(at(0)));
    }
}
