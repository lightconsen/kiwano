//! The provider view — assembled by the daemon, with this machine's evidence
//! supplied by the client.
//!
//! Everything that *builds* a row lives in `kiwanod::api::providers_view`
//! (`migrate.local.md` §10.21): the client cannot open the daemon's database
//! when the daemon is on another machine, and the daemon owns the rows. What
//! stays here is the **one machine fact** the view needs — which agents' own
//! configs carry our placeholder key — because §5 #2 keeps that out of the
//! daemon.
//!
//! **Which of those agents count is not this side's question.** It used to be:
//! `live_bound_agents` intersected the config evidence with the bindings and the
//! custom agents, which meant two *shared* reads to answer it. The intersection
//! needs rows, so it belongs where the rows are, and all the client has to say
//! is what it can see (`migrate.local.md` §10.44).

use super::ProviderVm;
use crate::daemon_api::DaemonApi;
use crate::detect::ShellVars;
use crate::vm::e2s;
use crate::vm::AGENTS;
use kiwanod::store::Store;
use std::path::Path;

/// The agents whose own config carries our placeholder key **right now**.
///
/// Turning a takeover off drops that agent's route (`set_agent_takeover`), so
/// what is left is the odd row: an agent whose config was reverted behind
/// Kiwano's back by another tool, a store written by a build that kept dormant
/// routes, a binding an import landed on an agent that was never taken over. In
/// every one of them the agent's traffic goes to its own provider rather than to
/// this gateway, so a provider must not read as bound to it — or, worse, as *in
/// use* by it, which is what the list claimed for every binding row.
///
/// Recognition is by the live file (`kw-ag-<agent>-…` in the config the takeover
/// wrote), never by the `client_keys` table: a key row can outlive its
/// rewrite. The takeover panel reads the same files and counts *more* agents
/// than this on purpose — it also accepts a restorable backup, which is a claim
/// about being able to undo a takeover, not about traffic arriving here.
///
/// Purely local, and deliberately so: it opens config files and touches no
/// database at all.
pub fn carrying_agents(home: &Path, vars: &ShellVars) -> Vec<String> {
    AGENTS
        .iter()
        .map(|(agent, _)| *agent)
        .filter(|agent| crate::takeover::live_placeholder_key(agent, home, vars).is_some())
        .map(str::to_string)
        .collect()
}

/// The Apps list, as a client asks for it: the daemon assembles the rows, this
/// side supplies the evidence it alone can see.
pub fn provider_view(
    api: &DaemonApi,
    home: &Path,
    vars: &ShellVars,
) -> Result<Vec<ProviderVm>, String> {
    api.provider_view(&carrying_agents(home, vars))
}

/// The provider screen, as every existing caller asks for it.
///
/// The live set is computed **here** — it reads this machine's agent configs —
/// and the daemon does the rest. A remote client reaches the same assembly over
/// the API; this form is for a client that shares the daemon's machine, which is
/// every client today.
pub fn build_provider_vms(
    store: &Store,
    _aux: &crate::vm::Aux,
    home: &Path,
    vars: &ShellVars,
) -> Result<Vec<ProviderVm>, String> {
    kiwanod::api::providers_view::build_provider_vms(store, &carrying_agents(home, vars))
        .map_err(e2s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::routes::set_agent_strategy;
    use crate::vm::test_support::{live_home, no_vars, provider, store};
    use crate::vm::time::{rfc3339, unix_now};
    use crate::vm::Aux;
    use kiwanod::store::time::local_minutes_now;
    use kiwanod::store::{Billing, Binding, StrategyType};

    #[test]
    fn provider_vm_maps_catalog_shape() {
        let s = store();
        s.insert_provider(&provider("deepseek-1", "DeepSeek", Billing::Metered))
            .unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "deepseek-1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        assert_eq!(vms.len(), 1);
        let vm = &vms[0];
        let json = serde_json::to_value(vm).unwrap();
        assert_eq!(json["billing"], "payg");
        assert_eq!(json["logo_char"], "D");
        assert_eq!(json["is_current"], true);
        assert_eq!(json["agents"][0], "claude");
        assert_eq!(json["agents_note"], "1 agent(s)");
        assert_eq!(json["endpoint"], "deepseek-1.example.com");
        assert_eq!(json["endpoint_note"], "OpenAI-compatible");
    }

    /// A binding whose agent never took the gateway over is not a route: the
    /// provider list must leave that agent out of the agent column, out of
    /// "In use", and out of the count — the route stays in the store for the
    /// day the takeover is re-enabled, but no traffic reaches us until then.
    #[test]
    fn dormant_agent_bindings_are_not_counted() {
        let s = store();
        s.insert_provider(&provider("p1", "P One", Billing::Metered))
            .unwrap();
        for agent in ["claude", "codex"] {
            s.upsert_binding(&Binding {
                agent: agent.into(),
                provider_id: "p1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let aux = Aux::open_in_memory().unwrap();

        // claude is routed through the gateway, codex is not.
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert_eq!(vm.agents, ["claude"]);
        assert_eq!(vm.serving_agents, ["claude"]);
        assert!(vm.is_current);
        assert_eq!(vm.agents_note.as_deref(), Some("1 agent(s)"));

        // Nothing taken over at all: the provider reads as unbound rather than
        // as serving an agent that has its own config back.
        let none = live_home(&[]);
        let vms = build_provider_vms(&s, &aux, none.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert!(vm.agents.is_empty());
        assert!(vm.serving_agents.is_empty());
        assert!(!vm.is_current);
        assert_eq!(vm.agents_note, None);
    }

    #[test]
    fn backup_binding_gets_badge_and_note() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Subscription))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let alpha = vms.iter().find(|v| v.id == "a1").unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(alpha.is_current);
        assert_eq!(alpha.serving_agents, ["claude"]);
        assert!(!beta.is_current);
        assert!(beta.serving_agents.is_empty());
        // Standby badges are gone; the failover-queue role lives in the note
        assert_eq!(beta.status_badge, None);
        assert_eq!(beta.agents_note.as_deref(), Some("Failover queue"));
    }

    #[test]
    fn standby_flag_follows_strategy() {
        let s = store();
        for (id, name) in [
            ("a1", "Alpha"),
            ("b1", "Beta"),
            ("c1", "Gamma"),
            ("d1", "Delta"),
        ] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        s.upsert_strategy("codex", StrategyType::Roundrobin, None)
            .unwrap();
        s.upsert_strategy("opencode", StrategyType::Timewindow, None)
            .unwrap();
        s.upsert_strategy("hermes", StrategyType::Timewindow, None)
            .unwrap();
        let bind = |agent: &str, pid: &str, priority: i64, win: Option<(&str, &str)>| Binding {
            agent: agent.into(),
            provider_id: pid.into(),
            priority,
            weight: 1,
            win_start: win.map(|w| w.0.into()),
            win_end: win.map(|w| w.1.into()),
            enabled: true,
        };
        // roundrobin tail: takes rotation turns → not a standby
        s.upsert_binding(&bind("codex", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("codex", "b1", 1, None)).unwrap();
        // windowed timewindow tail: serves its own window → not a standby
        s.upsert_binding(&bind("opencode", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("opencode", "c1", 1, Some(("22:00", "06:00"))))
            .unwrap();
        // windowless timewindow tail: never picked → still a standby
        s.upsert_binding(&bind("hermes", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("hermes", "d1", 1, None)).unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["codex", "opencode", "hermes"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(beta.is_current); // roundrobin serves every candidate
        assert_eq!(beta.serving_agents, ["codex"]);
        assert_eq!(beta.status_badge, None);
        assert_eq!(beta.agents_note.as_deref(), Some("1 agent(s)"));
        let gamma = vms.iter().find(|v| v.id == "c1").unwrap();
        assert_eq!(gamma.status_badge, None);
        let delta = vms.iter().find(|v| v.id == "d1").unwrap();
        assert!(!delta.is_current);
        assert_eq!(delta.status_badge, None);
        assert_eq!(delta.agents_note.as_deref(), Some("Failover queue"));
    }

    #[test]
    fn in_use_badge_follows_strategy() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Metered))
            .unwrap();
        for (pid, pr) in [("a1", 0), ("b1", 1)] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: pid.into(),
                priority: pr,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let home = live_home(&["claude"]);
        // (a1 current, b1 current, b1 fallback) — the quota-over case is where
        // the two badge claims split: the backup is first in line, not in use,
        // so it reads as fallback and neither provider reads as current.
        let badges = |s: &Store| -> (bool, bool, Vec<String>) {
            let vms = build_provider_vms(s, &aux, home.path(), &no_vars()).unwrap();
            let p = |id: &str| vms.iter().find(|x| x.id == id).unwrap();
            (
                p("a1").is_current,
                p("b1").is_current,
                p("b1").fallback_agents.clone(),
            )
        };

        // single: only the head serves
        assert_eq!(badges(&s), (true, false, vec![]));

        // roundrobin: every candidate takes rotation turns
        set_agent_strategy(&s, "claude", "roundrobin", None).unwrap();
        assert_eq!(badges(&s), (true, true, vec![]));

        // timewindow: a window containing now moves the badge off the head.
        // Built from the same clock the view model reads — a fresh store has no
        // settings blob, so the offset defaults to UTC — and the case is stated
        // without depending on the host's zone.
        let now = local_minutes_now(s.ui_tz_offset_minutes());
        let hhmm = |min: u32| format!("{:02}:{:02}", min / 60 % 24, min % 60);
        // [now-30, now+30] — wraps midnight safely near the day edges
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: Some(hhmm(now + 1440 - 30)),
            win_end: Some(hhmm(now + 30)),
            enabled: true,
        })
        .unwrap();
        set_agent_strategy(&s, "claude", "timewindow", None).unwrap();
        assert_eq!(badges(&s), (false, true, vec![]));

        // timewindow: no window matching now → the fallback head serves
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            // one-minute window later today — can never contain now
            win_start: Some(hhmm(now + 60)),
            win_end: Some(hhmm(now + 60)),
            enabled: true,
        })
        .unwrap();
        assert_eq!(badges(&s), (true, false, vec![]));

        // quota: under the threshold the head serves; over it the head stops
        // serving and the first backup is badged fallback, not in use (windows
        // ignored).
        set_agent_strategy(
            &s,
            "claude",
            "quota",
            Some(r#"{"limit":5,"unit":"requests"}"#),
        )
        .unwrap();
        assert_eq!(badges(&s), (true, false, vec![]));
        for _ in 0..5 {
            s.record_usage(&kiwanod::store::UsageRecord {
                ts: rfc3339(unix_now()),
                agent: "claude".into(),
                provider_id: Some("a1".into()),
                client_key_id: None,
                model: None,
                input_tokens: 10,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
                project: None,
                session_id: None,
                import_key: None,
            })
            .unwrap();
        }
        // Over: neither reads as current (the gateway may still serve the
        // primary when every backup is down, which is its runtime call), and
        // b1 — the configured first backup — reads as fallback.
        assert_eq!(badges(&s), (false, false, vec!["claude".to_string()]));
    }
}
