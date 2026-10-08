//! Agent routing: which provider serves an agent, in what order the candidates
//! are tried, and the per-agent limits and strategy that ride along.
//!
//! The bodies live in `kiwanod::api::routes` now: the daemon serves this
//! resource and owns the store it is read from, so a copy here would be the
//! second one (`migrate.local.md` §10.8). What stays is the *client-facing*
//! signature — `Result<_, String>`, which is what the app's IPC and the CLI
//! take — and the wire types, re-exported so `crate::vm::AgentRouteVm` and
//! friends still resolve.
//!
//! The tests at the bottom did **not** move with the bodies on purpose: they
//! are the app-facing promises (a rename keeps its id, a route copies whole,
//! a zero ceiling is no ceiling), and through these wrappers they run against
//! the daemon's implementation rather than a copy of it.

use kiwanod::api::routes as daemon;
use kiwanod::store::Store;
use std::path::Path;

pub use kiwano_api::routes::{AgentLimitVm, AgentRouteVm, BindingVm};

pub fn build_agent_routes(store: &Store) -> Result<Vec<AgentRouteVm>, String> {
    daemon::build_agent_routes(store).map_err(|e| e.to_string())
}

/// Set or clear one agent's spend ceilings — the whole set at once, one window
/// per entry. An empty list clears them.
pub fn set_agent_limits(
    store: &Store,
    agent: &str,
    limits: Vec<AgentLimitVm>,
) -> Result<(), String> {
    daemon::set_agent_limits(store, agent, limits).map_err(|e| e.to_string())
}

/// Update an Agent's strategy type (+ optional JSON config); unknown types error.
pub fn set_agent_strategy(
    store: &Store,
    agent: &str,
    strategy: &str,
    config: Option<&str>,
) -> Result<(), String> {
    daemon::set_agent_strategy(store, agent, strategy, config).map_err(|e| e.to_string())
}

/// Candidate reorder: given a provider_id order → rewrite priority 0..n.
pub fn reorder_agent_bindings(
    store: &Store,
    agent: &str,
    provider_ids: &[String],
) -> Result<(), String> {
    daemon::reorder_agent_bindings(store, agent, provider_ids).map_err(|e| e.to_string())
}

/// Patch one binding's strategy parameters (weight / local time window).
pub fn update_agent_binding(
    store: &Store,
    agent: &str,
    provider_id: &str,
    weight: Option<i64>,
    win_start: Option<String>,
    win_end: Option<String>,
) -> Result<(), String> {
    daemon::update_agent_binding(store, agent, provider_id, weight, win_start, win_end)
        .map_err(|e| e.to_string())
}

/// Bind a provider to an agent (appended at the queue tail).
pub fn add_agent_binding(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    daemon::add_agent_binding(store, agent, provider_id).map_err(|e| e.to_string())
}

/// Unbind a provider from one agent (other agents keep theirs).
pub fn remove_agent_binding(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    daemon::remove_agent_binding(store, agent, provider_id).map_err(|e| e.to_string())
}

/// Copy another agent's whole route (strategy + ordered candidates) onto this
/// one, replacing whatever it had.
pub fn apply_agent_route(store: &Store, target: &str, source: &str) -> Result<(), String> {
    daemon::apply_agent_route(store, target, source).map_err(|e| e.to_string())
}

/// A path as the screen should print it: `~` for the home tree, absolute for
/// anything outside it (a `HERMES_HOME` pointing elsewhere is a real path, and
/// rewriting it to `~/…` would name a file that does not exist).
///
/// Joined rather than formatted, so the separator this *adds* is the platform's:
/// gluing a literal `~/` to a Windows path produced `~/.claude\settings.json`, a
/// spelling neither platform uses. What it does not do is rewrite the separators
/// already inside `path` — a path built component by component (as every one of
/// these is) already spells them natively, and one that does not is somebody's
/// own text, which is not this function's to re-punctuate.
pub(crate) fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => Path::new("~").join(rest).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::settings::build_settings_with_home;
    use crate::vm::test_support::{no_vars, provider, store};
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Binding, Store, StrategyType};

    #[test]
    fn apply_agent_route_copies_strategy_and_candidates() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Subscription))
            .unwrap();
        // source: roundrobin with tuned weights and a windowed tail
        s.upsert_strategy("codex", StrategyType::Roundrobin, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 60,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 40,
            win_start: Some("22:00".into()),
            win_end: Some("06:00".into()),
            enabled: true,
        })
        .unwrap();
        // target: an unrelated failover route that gets replaced wholesale
        s.upsert_strategy("opencode", StrategyType::Failover, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "opencode".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        apply_agent_route(&s, "opencode", "opencode").unwrap_err();
        apply_agent_route(&s, "opencode", "claude").unwrap_err(); // no route
        apply_agent_route(&s, "opencode", "codex").unwrap();

        let st = s.get_strategy("opencode").unwrap().unwrap();
        assert_eq!(st.kind, StrategyType::Roundrobin);
        let bs = s.bindings_for_agent("opencode").unwrap();
        assert_eq!(bs.len(), 2);
        assert_eq!((bs[0].provider_id.as_str(), bs[0].weight), ("a1", 60));
        assert_eq!((bs[1].provider_id.as_str(), bs[1].weight), ("b1", 40));
        // weights copied as-is, not re-seeded to an even split
        assert_eq!(bs[1].win_start.as_deref(), Some("22:00"));
        assert_eq!(bs[1].win_end.as_deref(), Some("06:00"));
        // the source agent keeps its own bindings
        assert_eq!(s.bindings_for_agent("codex").unwrap().len(), 2);
    }

    /// Binding appends to the tail of the queue: the standing primary keeps
    /// priority 0, the new candidate takes the next free slot, and a provider
    /// that is already bound is left exactly where it was — the command is
    /// idempotent so a second click cannot reorder the queue.
    #[test]
    fn add_agent_binding_appends_a_candidate_and_never_duplicates_one() {
        let s = store();
        for (id, name) in [("a1", "Alpha"), ("b1", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        add_agent_binding(&s, "claude", "a1").unwrap();
        add_agent_binding(&s, "claude", "b1").unwrap();
        let priorities = |s: &Store| -> Vec<(String, i64)> {
            s.bindings_for_agent("claude")
                .unwrap()
                .into_iter()
                .map(|b| (b.provider_id, b.priority))
                .collect()
        };
        assert_eq!(
            priorities(&s),
            vec![("a1".to_string(), 0), ("b1".to_string(), 1)]
        );
        let fresh = s.bindings_for_agent("claude").unwrap();
        assert!(
            fresh.iter().all(|b| b.enabled && b.weight == 1),
            "a new candidate joins enabled, at an even weight"
        );

        // Re-binding the primary leaves it at 0.
        add_agent_binding(&s, "claude", "a1").unwrap();
        assert_eq!(
            priorities(&s),
            vec![("a1".to_string(), 0), ("b1".to_string(), 1)]
        );

        // An unknown provider is refused rather than bound to nothing.
        assert!(add_agent_binding(&s, "claude", "ghost").is_err());
        assert_eq!(s.bindings_for_agent("claude").unwrap().len(), 2);
    }

    /// Unbinding takes one candidate off one agent's route and touches nothing
    /// else: another agent's binding of the same provider stays, and an empty
    /// route is a legal state (requests fail cleanly with NoBinding until
    /// something is bound again) rather than a reason to keep a stale row.
    #[test]
    fn remove_agent_binding_touches_one_route_only() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        for agent in ["claude", "codex"] {
            add_agent_binding(&s, agent, "a1").unwrap();
        }

        remove_agent_binding(&s, "claude", "a1").unwrap();
        assert!(s.bindings_for_agent("claude").unwrap().is_empty());
        assert!(s.primary_provider_id("claude").unwrap().is_none());
        assert_eq!(s.bindings_for_agent("codex").unwrap().len(), 1);

        // Removing what is not bound is an error, not a silent success — the
        // caller has to be able to tell "it is gone" from "it never was".
        assert!(remove_agent_binding(&s, "claude", "a1").is_err());
        assert!(remove_agent_binding(&s, "claude", "ghost").is_err());
    }

    /// A binding's strategy parameters are patched one field at a time, except
    /// the window: its two bounds are set or cleared together, because half a
    /// window can never match and would quietly turn a rotating candidate into
    /// one that never serves.
    #[test]
    fn update_agent_binding_patches_weight_and_window_together() {
        let s = store();
        for (id, name) in [("a1", "Alpha"), ("b1", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        add_agent_binding(&s, "claude", "a1").unwrap();
        add_agent_binding(&s, "claude", "b1").unwrap();
        let binding = |s: &Store| -> Binding {
            s.bindings_for_agent("claude")
                .unwrap()
                .into_iter()
                .find(|b| b.provider_id == "b1")
                .unwrap()
        };

        update_agent_binding(
            &s,
            "claude",
            "b1",
            Some(7),
            Some("22:00".into()),
            Some("06:00".into()),
        )
        .unwrap();
        let b = binding(&s);
        assert_eq!(b.weight, 7);
        assert_eq!(b.win_start.as_deref(), Some("22:00"));
        assert_eq!(b.win_end.as_deref(), Some("06:00"));

        // A patch that does not mention the window leaves it alone.
        update_agent_binding(&s, "claude", "b1", Some(3), None, None).unwrap();
        let b = binding(&s);
        assert_eq!(b.weight, 3);
        assert_eq!(b.win_start.as_deref(), Some("22:00"));

        // A weight below 1 is floored: 0 would take the candidate out of a
        // weighted rotation while still sitting in the queue.
        update_agent_binding(&s, "claude", "b1", Some(0), None, None).unwrap();
        assert_eq!(binding(&s).weight, 1);

        // Half a window — one bound, or an empty string — clears the pair.
        update_agent_binding(&s, "claude", "b1", None, Some("22:00".into()), None).unwrap();
        let b = binding(&s);
        assert!(b.win_start.is_none() && b.win_end.is_none(), "{b:?}");

        // The other candidate was never touched by any of it.
        let a1 = s
            .bindings_for_agent("claude")
            .unwrap()
            .into_iter()
            .find(|b| b.provider_id == "a1")
            .unwrap();
        assert_eq!((a1.weight, a1.win_start), (1, None));

        // Patching something that is not bound is an error.
        assert!(update_agent_binding(&s, "claude", "ghost", Some(1), None, None).is_err());
    }

    // The screen's half of an agent's ceilings: what it writes is what the gateway
    // measures. Two windows are written at once here because that is the shape the
    // change was for — the zero and empty cases matter for the same reason they
    // did when there was one.
    #[test]
    fn agent_limits_round_trip_as_a_set_of_windows() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let agent = "claude";

        let p = provider("ds-1", "DeepSeek", Billing::Metered);
        s.insert_provider(&p).unwrap();
        s.upsert_binding(&Binding {
            agent: agent.into(),
            provider_id: p.id.clone(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        let route = |s: &Store| {
            build_agent_routes(s)
                .unwrap()
                .into_iter()
                .find(|r| r.agent == agent)
                .expect("the agent has a binding")
        };
        assert!(route(&s).limits.is_empty(), "no rows is no ceiling");

        let day = AgentLimitVm {
            period: "day".into(),
            period_limit: 100.0,
            limit_unit: None,
        };
        let month = AgentLimitVm {
            period: "monthly".into(),
            period_limit: 50.0,
            limit_unit: Some("CNY".into()),
        };
        set_agent_limits(&s, agent, vec![day.clone(), month.clone()]).unwrap();
        assert_eq!(
            route(&s).limits,
            vec![day.clone(), month.clone()],
            "both windows come back, in window order"
        );

        // A window of zero is the absence of that window, not a ceiling of
        // nothing: the gateway would ignore it while the screen showed it.
        set_agent_limits(
            &s,
            agent,
            vec![
                day.clone(),
                AgentLimitVm {
                    period: "weekly".into(),
                    period_limit: 0.0,
                    limit_unit: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            route(&s).limits,
            vec![day.clone()],
            "the zero window is gone"
        );

        // Replacing the set keeps the surviving window's age — it is the same
        // window, not a new one that happens to look the same.
        let first_set = s.agent_limits_for(agent).unwrap()[0].created_at.clone();
        set_agent_limits(&s, agent, vec![day.clone(), month.clone()]).unwrap();
        let rows = s.agent_limits_for(agent).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().find(|l| l.period == "day").unwrap().created_at,
            first_set
        );

        // And an empty set clears them all.
        set_agent_limits(&s, agent, vec![]).unwrap();
        assert!(route(&s).limits.is_empty());

        // A screen that was never asked about limits still builds.
        assert_eq!(
            build_settings_with_home(&s, &aux, tmp.path(), &no_vars())
                .unwrap()
                .language,
            "system"
        );
    }

    #[test]
    fn agent_routes_roundtrip_strategy_and_reorder() {
        let s = store();
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

        // default strategy is single
        let routes = build_agent_routes(&s).unwrap();
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.agent, "claude");
        assert_eq!(r.strategy, "single");
        assert_eq!(r.bindings.len(), 2);
        assert_eq!(r.bindings[0].provider_name, "Alpha");
        assert_eq!(r.bindings[0].logo_char, "A");

        // change strategy + reorder → priorities rewritten, weights preserved
        set_agent_strategy(&s, "claude", "failover", None).unwrap();
        assert!(set_agent_strategy(&s, "claude", "bogus", None).is_err());
        reorder_agent_bindings(&s, "claude", &["b1".into(), "a1".into()]).unwrap();
        assert!(reorder_agent_bindings(&s, "claude", &["nope".into()]).is_err());

        let routes = build_agent_routes(&s).unwrap();
        let r = &routes[0];
        assert_eq!(r.strategy, "failover");
        assert_eq!(r.bindings[0].provider_id, "b1");
        assert_eq!(r.bindings[0].priority, 0);
        assert_eq!(r.bindings[1].provider_id, "a1");
        assert_eq!(r.bindings[1].priority, 1);

        // quota config passes through
        set_agent_strategy(
            &s,
            "claude",
            "quota",
            Some(r#"{"limit":50,"unit":"requests"}"#),
        )
        .unwrap();
        let r = &build_agent_routes(&s).unwrap()[0];
        assert_eq!(r.strategy, "quota");
        assert_eq!(
            r.config.as_deref(),
            Some(r#"{"limit":50,"unit":"requests"}"#)
        );
    }

    #[test]
    fn roundrobin_strategy_seeds_even_weights() {
        let s = store();
        for (pid, name, pr) in [("a1", "Alpha", 0), ("b1", "Beta", 1), ("c1", "Gamma", 2)] {
            s.insert_provider(&provider(pid, name, Billing::Metered))
                .unwrap();
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

        // Entering roundrobin splits 100 across the candidates (remainder to
        // the head of the queue): 3 candidates → 34/33/33
        set_agent_strategy(&s, "claude", "roundrobin", None).unwrap();
        let weights: Vec<i64> = build_agent_routes(&s).unwrap()[0]
            .bindings
            .iter()
            .map(|b| b.weight)
            .collect();
        assert_eq!(weights, vec![34, 33, 33]);

        // Other strategies leave the weights untouched
        set_agent_strategy(&s, "claude", "failover", None).unwrap();
        let weights: Vec<i64> = build_agent_routes(&s).unwrap()[0]
            .bindings
            .iter()
            .map(|b| b.weight)
            .collect();
        assert_eq!(weights, vec![34, 33, 33]);
    }
}
