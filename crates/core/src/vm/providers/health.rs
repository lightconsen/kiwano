//! The Status column's contract, tested through the view the daemon assembles.
//!
//! The rule itself (`health_vm`) is the daemon's (`migrate.local.md` §10.7,
//! §10.21) and nothing in this crate calls it. What is here is the *behaviour*
//! the column promises — a provider's own round trips before the prober's
//! verdict, and neither for a parked one — asserted through
//! [`build_provider_vms`], which is the call the app and the CLI make.

#[cfg(test)]
mod tests {
    use crate::vm::providers::build_provider_vms;
    use crate::vm::test_support::{no_vars, provider, usage_row};
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Store};

    /// The Status column reads two sources and must not confuse them: a
    /// provider's own round trips when it has any, the prober's unsigned GET
    /// when it does not, and neither for a parked one.
    #[test]
    fn the_status_column_shows_a_providers_own_latency_before_a_probe() {
        // The traffic average is a read of the `usage` table, which the store
        // owns — it used to go through an auxiliary connection, which is what
        // this fixture had to hand it as well.
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path().join("kiwano.db")).unwrap();
        // `build_provider_vms` still reads the ui settings (the timezone quota
        // windows are measured in), so the page-level fixture still needs one.
        let aux = Aux::open_in_memory().unwrap();

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
