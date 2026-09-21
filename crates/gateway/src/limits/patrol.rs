//! The scheduled half: refresh the cached plan reports, evaluate, publish —
//! the one thing in the gateway that looks at the world on a schedule rather
//! than because a request arrived.
//!
//! `clear_legacy_disables` rides along because it is the same kind of startup
//! work: it runs once, before the loop, and never again.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use crate::limits::state::{evaluate, LimitState};
use crate::server::GatewayState;
use crate::store::Store;

/// The KV markers the desktop app wrote when *it* disabled a provider for
/// exceeding a limit. Nothing produces them any more.
const LEGACY_DISABLE_MARKERS: [&str; 2] = ["plan_limit_disabled:", "spend_limit_disabled:"];

/// Undo the old app-side enforcement, once at startup.
///
/// Those markers were how the app told its own doing from the user's, so it
/// could re-enable safely. Enforcement moved here and no longer disables
/// anything, which means nothing would ever clear them: a provider the old app
/// switched off would stay off, with no marker UI to explain why and no code
/// left to restore it.
pub fn clear_legacy_disables(store: &Store) -> usize {
    let mut restored = 0;
    for prefix in LEGACY_DISABLE_MARKERS {
        let Ok(rows) = store.app_settings_with_prefix(prefix) else {
            continue;
        };
        for (key, _) in rows {
            let Some(id) = key.strip_prefix(prefix) else {
                continue;
            };
            if let Ok(Some(mut p)) = store.get_provider(id) {
                if !p.enabled {
                    p.enabled = true;
                    p.updated_at = crate::store::now_rfc3339();
                    if store.update_provider(&p).is_ok() {
                        restored += 1;
                        tracing::info!(
                            provider = %id,
                            "re-enabled: it was disabled by an older limit patrol, which the gateway now owns"
                        );
                    }
                }
            }
            let _ = store.delete_app_setting(&key);
        }
    }
    restored
}

/// Refresh the cached plan reports for providers that have a query to run.
///
/// It is what keeps `evaluate` free of network calls. A provider whose endpoint
/// is down keeps its previous cached answer (or none), so a flaky endpoint never
/// fabricates a block.
///
/// The queries run concurrently. Each carries its own 15-second client timeout
/// (`plan_quota::client`), so asking them one at a time put the whole batch —
/// and therefore `publish`, which waits on all of them — behind `N × 15s` when
/// N endpoints are unreachable, while requests kept routing on the stale
/// snapshot. Concurrency is safe here: `Store` is `Send + Sync` and locks per
/// call, and each provider caches under its own `app_settings` key, so two
/// queries share nothing.
///
/// Unbounded on purpose: N is the number of providers the operator configured,
/// which is single digits. A deployment with dozens would want
/// `buffer_unordered` to stop N simultaneous requests from going out at once.
pub async fn refresh_plan_reports(store: &Store) {
    let providers = match store.list_providers() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "plan-quota refresh skipped: providers unreadable");
            return;
        }
    };
    let due = providers
        .iter()
        .filter(|p| p.enabled && p.plan_query.is_some());
    // Awaiting the whole batch is the point: `evaluate` below rebuilds the
    // snapshot from the store, so it has to run after every write has landed,
    // and `publish` has to be called exactly once per tick.
    futures_util::future::join_all(due.map(|p| async move {
        if let Err(e) = crate::plan_quota::get_plan_quota_report(store, &p.id, false).await {
            tracing::debug!(provider = %p.id, error = %e, "plan quota refresh failed");
        }
    }))
    .await;
}

/// How often the limits are re-evaluated. The plan half rides a 5-minute
/// upstream cache, so this is about noticing a spending limit promptly rather
/// than about refresh cost.
pub const LIMIT_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Re-evaluate and publish, forever. The one thing in the gateway that looks at
/// the world on a schedule rather than because a request arrived.
pub async fn run(state: Arc<GatewayState>, interval: StdDuration) {
    loop {
        // No blocking island: the quota queries are awaited on the runtime like
        // everything else the daemon does, and `evaluate` is store reads only.
        // (This was a `spawn_blocking` because the refresh used a blocking HTTP
        // client — the one place the daemon had to leave its own runtime to do
        // network I/O.)
        refresh_plan_reports(&state.store).await;
        publish(&state, evaluate(&state.store));
        tokio::time::sleep(interval).await;
    }
}

/// Install a fresh evaluation, logging only the transitions — the line that
/// answers "why did my request start failing?" without a per-tick heartbeat.
///
/// Shared with the reload path on purpose: an edit is the most likely cause of a
/// transition, and a refresh that reported nothing would leave the one change the
/// user just made as the only silent one.
pub fn publish(state: &GatewayState, next: LimitState) {
    let prev = state.limits();
    let current = state.set_limits(next);
    for (id, reason) in current.entries() {
        if prev.blocked(id).is_none() {
            tracing::warn!(
                provider = %id,
                reason = %reason.describe(),
                "provider is over its limit; routing around it"
            );
        }
    }
    for (id, _) in prev.entries() {
        if current.blocked(id).is_none() {
            tracing::info!(provider = %id, "provider is back under its limit");
        }
    }
    // An agent's own ceiling has no "around it" to describe: the request ends.
    for (agent, reason) in current.agents_over_entries() {
        if prev.agent_blocked(agent).is_none() {
            tracing::warn!(
                agent = %agent,
                reason = %reason.describe(),
                "agent is over its own limit; requests will be refused"
            );
        }
    }
    for (agent, _) in prev.agents_over_entries() {
        if current.agent_blocked(agent).is_none() {
            tracing::info!(agent = %agent, "agent is back under its own limit");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::test_support::{store_with_spend, test_provider};

    #[test]
    fn the_legacy_disable_markers_are_undone_once() {
        let s = store_with_spend("payg-1", 30.0, 31.0);
        let mut p = s.get_provider("payg-1").unwrap().unwrap();
        p.enabled = false;
        s.update_provider(&p).unwrap();
        // What the old app-side patrol left behind.
        s.set_app_setting("spend_limit_disabled:payg-1", "31.00/30.00")
            .unwrap();
        s.set_app_setting("plan_limit_disabled:payg-1", "five_hour")
            .unwrap();

        assert_eq!(clear_legacy_disables(&s), 1, "one provider restored");
        assert!(s.get_provider("payg-1").unwrap().unwrap().enabled);
        assert!(s.app_setting("spend_limit_disabled:payg-1").is_none());
        assert!(s.app_setting("plan_limit_disabled:payg-1").is_none());
        // Idempotent — it runs on every gateway start.
        assert_eq!(clear_legacy_disables(&s), 0);
    }

    /// A local endpoint that answers each request after `delay`, counting the
    /// ones it served. Each connection gets its own thread, so the stub does not
    /// serialize what it is being used to prove is concurrent.
    ///
    /// Every plan template but `zenmux` builds its URL from a constant, so
    /// pointing a provider at this instead of the internet is what `zenmux`'s
    /// configurable `quota_url` is for.
    struct SlowUpstream {
        url: String,
        hits: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl SlowUpstream {
        fn start(delay: StdDuration) -> SlowUpstream {
            use std::io::{Read, Write};
            use std::sync::atomic::Ordering;

            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub");
            let port = listener.local_addr().expect("stub addr").port();
            let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = hits.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    counter.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        // Drain before answering: a request spanning more than
                        // one segment otherwise sees the connection reset.
                        let mut buf = [0u8; 8192];
                        let _ = stream.read(&mut buf);
                        std::thread::sleep(delay);
                        // A body the parser rejects is fine — this stub exists
                        // to be *slow*, and the round trip is the measurement.
                        let body = r#"{"success":true,"data":{}}"#;
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    });
                }
            });
            SlowUpstream {
                url: format!("http://127.0.0.1:{port}/usage"),
                hits,
            }
        }
    }

    /// A provider whose plan query runs against `stub`.
    fn provider_querying(store: &Store, id: &str, stub: &SlowUpstream) {
        let mut p = test_provider(id);
        p.api_key = Some("sk-test".into());
        p.plan_query = Some(format!(
            r#"{{"template":"zenmux","fields":{{"quota_url":"{}"}}}}"#,
            stub.url
        ));
        store.insert_provider(&p).expect("insert provider");
    }

    /// The batch is asked concurrently, so N slow endpoints cost roughly one
    /// round trip rather than N.
    ///
    /// Timing is the only way to see this from outside — the queries share no
    /// other observable side effect — so the margin is deliberately wide: three
    /// serial rounds are 3.0× the delay and the bar is 1.8×, which leaves the
    /// measured round trip room to grow on a loaded machine. Verified to fail
    /// against the serial loop (1.15s) before it was made concurrent.
    #[tokio::test]
    async fn plan_quota_queries_do_not_serialize() {
        use std::sync::atomic::Ordering;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("kiwano.db")).unwrap();
        let delay = StdDuration::from_millis(500);
        let stub = SlowUpstream::start(delay);
        for id in ["p1", "p2", "p3"] {
            provider_querying(&store, id, &stub);
        }

        let started = std::time::Instant::now();
        refresh_plan_reports(&store).await;
        let elapsed = started.elapsed();

        assert_eq!(
            stub.hits.load(Ordering::SeqCst),
            3,
            "every provider with a query was asked"
        );
        assert!(
            elapsed < delay * 3 * 3 / 5,
            "three queries took {elapsed:?}; serially they would be {:?} and the \
             concurrent round trip is {delay:?}",
            delay * 3
        );
    }

    /// The batch skips providers that are parked or have no query to run — the
    /// concurrency change must not have widened who gets asked.
    #[tokio::test]
    async fn plan_quota_refresh_skips_parked_and_queryless_providers() {
        use std::sync::atomic::Ordering;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("kiwano.db")).unwrap();
        let stub = SlowUpstream::start(StdDuration::from_millis(0));
        provider_querying(&store, "asked", &stub);

        // A queryless provider.
        store.insert_provider(&test_provider("no-query")).unwrap();
        // A parked one that does carry a query.
        provider_querying(&store, "parked", &stub);
        let mut parked = store.get_provider("parked").unwrap().unwrap();
        parked.enabled = false;
        store.update_provider(&parked).unwrap();

        refresh_plan_reports(&store).await;

        assert_eq!(
            stub.hits.load(Ordering::SeqCst),
            1,
            "only the enabled provider with a query was asked"
        );
    }
}
