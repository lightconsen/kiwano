//! The Status cell's one read: a provider's own round trips when it has any,
//! the prober's verdict (or the Apps screen's own test) when it does not, and
//! neither for a parked provider.

use super::types::HealthVm;
use crate::vm::Aux;
use kiwanod::store::Provider;

/// What the row says about a provider nobody has just asked.
///
/// The gateway used to keep a background probe's verdict here (every 30s, one
/// HTTP request per provider, written to `provider_health`), and the row showed
/// it as a green dot and a latency. That signal never routed anything — the
/// breaker, fed by real traffic, is what decides — so it was a continuous
/// background request per provider for a badge, and it is gone. An enabled
/// provider now reads as neutral rather than as healthy, which is the honest
/// answer to "is it up?" when the way to find out is to ask it: the row's Test
/// button measures one, and the request log is where failures show up.
/// What the Status column shows, and where the number came from.
///
/// Two sources, in this order, because they answer the same question with
/// different authority:
///
/// 1. **The provider's own requests** inside `since`. A round trip through the
///    gateway, with the user's key, to the model they actually route to — the
///    number is already in the usage table, so showing it costs nothing and it
///    is the most honest of the two.
/// 2. **The prober's verdict**, for a provider with nothing of its own to
///    measure (just added, or idle since yesterday). An unsigned GET: it says
///    whether something answers at that endpoint, never whether the key works.
///    The `source` field is what keeps the two apart downstream.
///
/// A parked provider answers neither question — it is out of every route, and
/// that is the fact worth showing.
pub(crate) fn health_vm(
    aux: &Aux,
    p: &Provider,
    since: &str,
    probe: Option<&kiwanod::store::ProviderHealth>,
) -> HealthVm {
    if !p.enabled {
        return HealthVm {
            state: "off".into(),
            latency_ms: None,
            note: Some("Disabled".into()),
            source: None,
            checked_at: None,
            error: None,
        };
    }
    if let Some(ms) = aux.avg_latency(Some(&p.id), None, Some(since), None) {
        return HealthVm {
            state: "ok".into(),
            latency_ms: Some(ms),
            note: None,
            source: Some("traffic".into()),
            checked_at: None,
            error: None,
        };
    }
    match probe {
        // Answered. The number is a round trip, and `source` says whose: the
        // prober's unsigned GET, or the Apps screen's own test — which sent a
        // real prompt with the provider's key, and is therefore the stronger
        // claim of the two.
        Some(h) if h.status == "reachable" => HealthVm {
            // A refusal is reachability too: the vendor answered, and what it
            // said was no. The cell reads that as the key rather than as
            // silence, which is the difference the tooltip carries.
            state: if h.error.is_some() {
                "error".into()
            } else {
                "ok".into()
            },
            latency_ms: h.latency_ms,
            note: None,
            source: Some(h.source.clone()),
            checked_at: Some(h.checked_at.clone()),
            error: h.error.clone(),
        },
        Some(h) => HealthVm {
            // No answer at all. Not a latency to print but a fact to show: the
            // endpoint did not respond when it was last asked.
            state: "error".into(),
            latency_ms: None,
            note: None,
            source: Some(h.source.clone()),
            checked_at: Some(h.checked_at.clone()),
            error: h.error.clone(),
        },
        // Nothing measured it yet — a provider added a moment ago, or one whose
        // first probe has not come round. Nothing to say, which is what the
        // blank cell has always meant.
        None => HealthVm {
            state: "idle".into(),
            latency_ms: None,
            note: None,
            source: None,
            checked_at: None,
            error: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::providers::build_provider_vms;
    use crate::vm::test_support::{no_vars, provider, usage_row};
    use kiwanod::store::{Billing, Store};

    /// The Status column reads two sources and must not confuse them: a
    /// provider's own round trips when it has any, the prober's unsigned GET
    /// when it does not, and neither for a parked one.
    #[test]
    fn the_status_column_shows_a_providers_own_latency_before_a_probe() {
        // The store and the aux have to share one file here: the traffic average
        // comes off the aux's connection, which in production is the same
        // database the gateway writes usage rows into.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let s = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();

        let mut parked = provider("parked", "Parked", Billing::Metered);
        parked.enabled = false;
        for p in [
            provider("busy", "Busy", Billing::Metered),
            provider("idle", "Idle", Billing::Metered),
            provider("dead", "Dead", Billing::Metered),
            provider("fresh", "Fresh", Billing::Metered),
            provider("tested", "Tested", Billing::Metered),
            provider("refused", "Refused", Billing::Metered),
            parked,
        ] {
            s.insert_provider(&p).unwrap();
        }

        // The prober has an opinion about all of them — including `busy`, whose
        // verdict is stale (it had no traffic when the probe ran). That stale row
        // is exactly what the order has to get right.
        s.upsert_provider_health("busy", "reachable", 500, "probe", None)
            .unwrap();
        s.upsert_provider_health("idle", "reachable", 12, "probe", None)
            .unwrap();
        s.upsert_provider_health("dead", "down", 0, "probe", None)
            .unwrap();
        s.upsert_provider_health("parked", "reachable", 3, "probe", None)
            .unwrap();
        // The Apps screen's own test: a real prompt with the key. One answered,
        // one refused — and a refusal is reachability with a reason, not silence.
        s.upsert_provider_health("tested", "reachable", 218, "test", None)
            .unwrap();
        s.upsert_provider_health("refused", "reachable", 60, "test", Some("invalid API key"))
            .unwrap();

        // Two requests of its own inside the window: 200ms on average.
        for ms in [180, 220] {
            let mut row = usage_row("busy");
            row.latency_ms = Some(ms);
            s.record_usage(&row).unwrap();
        }

        let vms = build_provider_vms(&s, &aux, std::path::Path::new("/tmp"), &no_vars()).unwrap();
        let health = |id: &str| &vms.iter().find(|v| v.id == id).unwrap().health;

        let busy = health("busy");
        assert_eq!(
            busy.source.as_deref(),
            Some("traffic"),
            "its own round trips outrank a verdict that predates them"
        );
        assert_eq!(
            busy.latency_ms,
            Some(200),
            "…and it is their average, not the probe's 500"
        );

        let idle = health("idle");
        assert_eq!(idle.source.as_deref(), Some("probe"));
        assert_eq!(idle.latency_ms, Some(12));
        assert!(
            idle.checked_at.is_some(),
            "a probe is dated; a traffic average is a window"
        );

        let dead = health("dead");
        assert_eq!(dead.state, "error", "an endpoint that did not answer");
        assert_eq!(dead.latency_ms, None, "no answer is not a latency");
        assert_eq!(dead.source.as_deref(), Some("probe"));

        let parked_health = health("parked");
        assert_eq!(parked_health.note.as_deref(), Some("Disabled"));
        assert_eq!(
            parked_health.latency_ms, None,
            "out of every route outranks both"
        );

        // A verdict the app took itself. The number and its provenance both
        // travel: the cell says "you tested it", not "the gateway asked".
        let tested = health("tested");
        assert_eq!(tested.source.as_deref(), Some("test"));
        assert_eq!(tested.latency_ms, Some(218));
        assert_eq!(tested.state, "ok");
        assert_eq!(tested.error, None);

        // Answered and refused: reachable, with the vendor's reason. Read as an
        // error state so the cell cannot pass it off as a working provider.
        let refused = health("refused");
        assert_eq!(refused.source.as_deref(), Some("test"));
        assert_eq!(refused.state, "error");
        assert_eq!(refused.error.as_deref(), Some("invalid API key"));
        assert_eq!(refused.latency_ms, Some(60), "it did answer, in 60ms");

        let fresh = health("fresh");
        assert_eq!(
            fresh.source, None,
            "never probed and never used: nothing to say"
        );
        assert_eq!(fresh.latency_ms, None);
        assert_eq!(fresh.state, "idle");
    }
}
