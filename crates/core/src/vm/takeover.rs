//! The view-model half of `crate::takeover`: enabling an agent takeover,
//! rebuilding its route, and importing a provider from a live agent config.

use crate::daemon_api::DaemonApi;
use crate::detect::ShellVars;
use crate::vm::agents::AGENTS;
use crate::vm::{e2s, Aux};
use kiwanod::api::takeover::CurrentCreds;
use kiwanod::store::Store;

#[allow(clippy::too_many_arguments)] // the phases' inputs, one each
pub fn set_agent_takeover(
    aux: &Aux,
    agent: &str,
    enabled: bool,
    // `gateway` is where an agent on this machine reaches the data plane —
    // `http://127.0.0.1:8317` locally, or another machine's address. Composed
    // by the caller, which is the side that knows where its daemon is
    // (`migrate.local.md` §10.30).
    gateway: &str,
    home: &std::path::Path,
    vars: &ShellVars,
    state_half: StateHalf<'_>,
) -> Result<(), String> {
    if !AGENTS.iter().any(|(a, _)| *a == agent) {
        return Err(format!("unknown agent: {agent}"));
    }
    if enabled {
        // The import of the agent's current provider, and the binding that
        // gives the gateway a route on day one, are part of `phase_state` now:
        // they are store writes, and the daemon owns those. What this side
        // contributes is the credential, read out of the agent's own config.
        //
        // Three phases, in an order that cannot be reversed: the store rows
        // first, the agent's config second, the "applied" mark last. Between
        // any two of them the process can die, and every one of those windows
        // leaves something *recoverable* rather than something broken — the
        // agent is either untouched or pointing at a gateway that knows its
        // key. See `reconcile_takeovers`, which closes them.
        let prepared = phase_state(aux, agent, home, None, state_half)?;
        if let Err(e) = crate::takeover::enable(aux, agent, &prepared.key, gateway, home, vars) {
            // The file half never landed: take the store half back out.
            undo_state(aux, agent, &prepared.key, state_half);
            return Err(e);
        }
        // A failure here is not a failed takeover: the files are in place and
        // the row is merely still `pending`, which is exactly the state
        // `reconcile_takeovers` finishes. Reported, not propagated.
        if let Err(e) = mark_applied(aux, agent, &prepared.op_id) {
            eprintln!("kiwano: {agent} takeover applied but not marked: {e}");
        }
    } else {
        // The provider the gateway serves for this agent, handed to restore as
        // the rebuild tier: losing the backup must not strand the agent at
        // loopback if there is a provider to point it back at.
        let fallback = state_half.rebuild_route(agent)?;
        let report = crate::takeover::disable(aux, agent, home, fallback.as_ref(), vars)?;
        // A restore that could not hand the original config back changed the
        // agent's config in a way the user did not ask for: it is not an error
        // (the agent is whole and no longer points at loopback), but it must be
        // visible in the log rather than only in the degraded UI state.
        if report.outcome == crate::takeover::RestoreOutcome::RebuiltFromProvider {
            eprintln!(
                "kiwano: {agent} takeover backup was unusable; its config was rebuilt from the gateway's current provider"
            );
        }
        // A cleanup step that did not finish is a log line too, not a failure:
        // the agent is already whole, and failing the command would tell the
        // user the restore broke when it did not.
        if let Some(warning) = report.warning {
            eprintln!("kiwano: {agent} takeover restore: {warning}");
        }
        // The registration, the bindings and the strategy are dropped whatever
        // the restore outcome — one call, because they are one decision. Leave
        // them and the UI keeps offering a key the config no longer carries,
        // and an agent that sends us nothing keeps reading as *bound*.
        // …and so does the route, for the same reason one step further out: an
        // agent that has its own config back sends us nothing, so its candidate
        // list and strategy are stale the moment the restore lands — invisible
        // in the agent tab (which shows the takeover onboarding again) but not
        // in the Apps screen, which kept reading the rows as "bound" and even
        // as "In use" (see `live_bound_agents`). The *providers* stay: they are
        // the user's own rows, with their keys, plans and usage history, and
        // they are what a later takeover re-imports and binds again.
        state_half.teardown(agent)?;
        // No operation is in flight once the agent has its own config back;
        // leaving the row would put the agent on the reconcile pass's list for
        // a takeover that has been deliberately undone.
        let _ = aux.clear_takeover_op(agent);
    }
    Ok(())
}

/// How the store half of a takeover is reached — the **one** place that decides
/// it.
///
/// It used to be a marker beside a `store` parameter the function took anyway,
/// so the caller always held a database handle whether or not the branch it took
/// used one. That is what it took for the CLI to stop opening the shared
/// database here (`migrate.local.md` §10.43): the handle is now *inside* the
/// variant, and a caller that goes over the wire cannot name one.
#[derive(Clone, Copy)]
pub enum StateHalf<'a> {
    /// In-process: the caller shares the database and may write it. What the
    /// crash tests do — they drive the phases against a real store on purpose.
    InProcess(&'a Store),
    /// Over the wire: the caller is a client and must not write the shared
    /// database. What the app and the CLI do.
    Via(&'a DaemonApi),
}

impl StateHalf<'_> {
    /// The placeholder key already registered for this agent, if any.
    ///
    /// Looked up before a key is minted: a replay must register the one it
    /// already has, or the config the file half may yet read would carry a key
    /// the gateway never sees.
    fn registered_key(&self, agent: &str) -> Result<Option<String>, String> {
        match self {
            StateHalf::InProcess(store) => Ok(store
                .list_placeholder_keys()
                .map_err(e2s)?
                .into_iter()
                .find(|k| k.agent == agent)
                .map(|k| k.key)),
            StateHalf::Via(api) => Ok(api.takeover_state(agent)?.key),
        }
    }

    /// Where restore should point the agent when its own backup cannot be
    /// written back.
    fn rebuild_route(&self, agent: &str) -> Result<Option<crate::takeover::ProviderRoute>, String> {
        let route = match self {
            StateHalf::InProcess(store) => {
                kiwanod::api::takeover::rebuild_route(store, agent).map_err(|e| e.to_string())?
            }
            StateHalf::Via(api) => api.takeover_state(agent)?.rebuild,
        };
        Ok(route.map(|v| crate::takeover::ProviderRoute {
            base_url: v.base_url,
            api_key: v.api_key,
        }))
    }

    /// Register the key against the agent — the rest of phase one.
    fn register(&self, agent: &str, key: &str, creds: Option<&CurrentCreds>) -> Result<(), String> {
        match self {
            StateHalf::InProcess(store) => {
                kiwanod::api::takeover::phase_state(store, agent, key, creds)
                    .map_err(|e| e.to_string())
            }
            StateHalf::Via(api) => api.takeover_register(agent, key, creds),
        }
    }

    /// Take back one registration, leaving the rest of the state alone.
    fn unregister_key(&self, agent: &str) -> Result<(), String> {
        match self {
            StateHalf::InProcess(store) => kiwanod::api::takeover::unregister_key(store, agent)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            StateHalf::Via(api) => api.takeover_unregister(agent).map(|_| ()),
        }
    }

    /// Undo the store half: the registration, the bindings and the strategy.
    fn teardown(&self, agent: &str) -> Result<(), String> {
        match self {
            StateHalf::InProcess(store) => {
                kiwanod::api::takeover::teardown(store, agent).map_err(|e| e.to_string())
            }
            StateHalf::Via(api) => api.takeover_teardown(agent),
        }
    }
}

/// What phase one produced, for phase two to use. `op_id` is the operation's
/// identity, which is what makes a replay recognisable as a replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTakeover {
    pub op_id: String,
    pub key: String,
}

/// Phase one — the store half of a takeover, and the operation row that says
/// the file half has not happened yet.
///
/// **The credential is read here and the writing happens on the far side.** The
/// agent's config is this machine's — §5's first constraint forbids paths in the
/// daemon's interface, and the file is the user's — so the client reads it and
/// hands over what it found; `kiwanod::api::takeover::phase_state` does the
/// importing, the binding and the key registration.
///
/// **The key is minted here**, which is what makes a replay safe without an
/// operation table on the daemon's side: a replay finds the key already
/// registered for this agent and reuses it, rather than minting a second one
/// that the config the file half may yet read would never carry.
///
/// **How the store half is reached is the caller's choice, and it is the one
/// difference between the app and the CLI.** The app must not write the shared
/// database — it is a client — so it goes over the wire; the CLI opens the
/// database itself for every command it has, and `InProcess` is that same
/// position. Making the branch explicit here keeps the *sequence* in one place:
/// §8's ordering is the part that must not be written twice.
pub fn phase_state(
    aux: &Aux,
    agent: &str,
    home: &std::path::Path,
    replay_of: Option<&str>,
    state_half: StateHalf<'_>,
) -> Result<PreparedTakeover, String> {
    // A replay keeps the key it already registered: minting a second one would
    // leave the first orphaned in the config the file half may yet read.
    if let Some(op_id) = replay_of {
        if let Some(existing) = state_half.registered_key(agent)? {
            return Ok(PreparedTakeover {
                op_id: op_id.to_string(),
                key: existing,
            });
        }
    }

    let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
    let key = format!("kw-ag-{agent}-{rand}");
    // The credential the agent's own config carries, as the daemon's shape —
    // read here because the file is this machine's.
    let creds = crate::creds::read_current_creds(agent, home).map(|c| CurrentCreds {
        base_url: c.base_url,
        api_key: c.api_key,
        name: c.name,
        protocol: c.protocol.to_string(),
    });
    state_half.register(agent, &key, creds.as_ref())?;

    let op_id = uuid::Uuid::new_v4().simple().to_string();
    aux.start_takeover_op(agent, &op_id).map_err(e2s)?;
    Ok(PreparedTakeover { op_id, key })
}

/// Phase three — mark the operation applied. Separate from the file writes on
/// purpose: the window between them is real, and it is the one
/// `reconcile_takeovers` finishes for free.
pub fn mark_applied(aux: &Aux, agent: &str, op_id: &str) -> Result<(), String> {
    aux.apply_takeover_op(agent, op_id).map_err(e2s).map(|_| ())
}

/// Take phase one back out, and whatever landed of phase two with it.
///
/// The order is forced by what each half costs to get wrong: a key registered
/// for a config that does not carry it is untidy (the UI would offer a key
/// nothing uses), while a half-rewritten agent config is the user's tool
/// pointing at something that no longer answers. So the *files* come back from
/// the backup first — and only when that backup is a restorable one, which is
/// the same test `disable` applies, because a backup holding our own route is
/// not the user's original.
///
/// A no-op when the file half never started: `restorable_backup` is false once
/// the backup row is gone, which is what an in-process failure leaves behind.
fn undo_state(aux: &Aux, agent: &str, key: &str, state_half: StateHalf<'_>) {
    if crate::takeover::restorable_backup(aux, agent) {
        if let Some((_, files)) = aux.load_takeover_backup(agent) {
            let _ = crate::takeover::restore_backup(aux, agent, &files, None);
        }
    }
    // **One registration**, not the whole state: this runs when the file half
    // failed, and a full teardown would delete a route the user had before
    // Kiwano was involved. `key` names what phase one registered — kept in the
    // signature so the intent survives the move to the wire, where the daemon
    // resolves it by agent (`migrate.local.md` §10.43).
    let _ = key;
    let _ = state_half.unregister_key(agent);
    let _ = aux.clear_takeover_op(agent);
}

/// A takeover that stopped between its phases, and what it converged to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverFinding {
    /// The store half landed, the file half never did: a key is registered for
    /// an agent whose config does not carry it. Converged by revoking the key.
    StoreWithoutConfig { agent: String, op_id: String },
    /// The file half landed, the applied mark never did. Converged by marking
    /// it — the work is already done, so this window costs nothing but the row.
    ConfigWithoutMark { agent: String, op_id: String },
    /// Marked applied, and the config no longer carries the key: the file was
    /// put back by hand or by another tool. Converged by revoking the key and
    /// forgetting the operation.
    MarkWithoutConfig { agent: String, op_id: String },
}

/// Close every takeover that stopped between phases.
///
/// Idempotent — a second run finds nothing unless a new window was opened — and
/// deliberately blind to agents with no operation row: every install that
/// predates the row has keys and backups without one, and reporting those would
/// invent a problem out of a missing record. The three outcomes are the three
/// ways the two halves can disagree; there is no fourth, because an operation
/// row only ever exists once phase one has run.
pub fn reconcile_takeovers(
    aux: &Aux,
    home: &std::path::Path,
    vars: &ShellVars,
    state_half: StateHalf<'_>,
) -> Result<Vec<TakeoverFinding>, String> {
    let mut findings = Vec::new();
    for (agent, _) in AGENTS {
        let Some(op) = aux.load_takeover_op(agent) else {
            continue;
        };
        let carries = crate::takeover::live_placeholder_key(agent, home, vars).is_some();
        let applied = op.state == crate::vm::TakeoverOpState::Applied;
        match (applied, carries) {
            // `applied` and the config agrees: nothing in flight.
            (true, true) => {}
            (true, false) => {
                state_half.unregister_key(agent)?;
                let _ = aux.clear_takeover_op(agent);
                findings.push(TakeoverFinding::MarkWithoutConfig {
                    agent: agent.to_string(),
                    op_id: op.op_id.clone(),
                });
            }
            (false, false) => {
                // The store half landed and the files did not (or landed
                // partially): put the user's files back before letting the key
                // go, or the agent is left pointing at a gateway that will
                // refuse it.
                match state_half.registered_key(agent)? {
                    Some(key) => undo_state(aux, agent, &key, state_half),
                    None => {
                        let _ = aux.clear_takeover_op(agent);
                    }
                }
                findings.push(TakeoverFinding::StoreWithoutConfig {
                    agent: agent.to_string(),
                    op_id: op.op_id.clone(),
                });
            }
            (false, true) => {
                let _ = aux.apply_takeover_op(agent, &op.op_id);
                findings.push(TakeoverFinding::ConfigWithoutMark {
                    agent: agent.to_string(),
                    op_id: op.op_id.clone(),
                });
            }
        }
    }
    Ok(findings)
}

// `import_current_provider` moved to `kiwanod::api::takeover` with the rest of
// the store half: the daemon does the importing, because the row is its to
// write. The client's job is to hand over the credential it read out of the
// agent's own config.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::providers::build_provider_vms;
    use crate::vm::settings::build_settings_with_home;
    use crate::vm::test_support::{no_vars, provider, store};
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Binding, StrategyType};

    /// Turning a takeover off hands the agent its own config back — and its
    /// route with it. The providers themselves stay: they are the user's rows,
    /// carrying the key, the plan and the usage history, and they are what a
    /// later takeover re-imports and binds again.
    #[test]
    fn disabling_a_takeover_drops_the_route_and_keeps_the_providers() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();

        // Two providers serving claude, the second standing by in a failover
        // queue: a route with something in it to lose.
        for (id, name) in [("p1", "One"), ("p2", "Two")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        s.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (id, priority) in [("p1", 0), ("p2", 1)] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: id.into(),
                priority,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }

        set_agent_takeover(
            &aux,
            "claude",
            true,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();
        assert_eq!(s.bindings_for_agent("claude").unwrap().len(), 2);
        set_agent_takeover(
            &aux,
            "claude",
            false,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();

        assert!(
            s.bindings_for_agent("claude").unwrap().is_empty(),
            "the route goes with the takeover"
        );
        assert!(
            s.get_strategy("claude").unwrap().is_none(),
            "…including the strategy it routed by"
        );
        // The rows survive, and read as unbound rather than as served.
        let vms = build_provider_vms(&s, &aux, tmp.path(), &no_vars()).unwrap();
        for id in ["p1", "p2"] {
            let vm = vms
                .iter()
                .find(|v| v.id == id)
                .unwrap_or_else(|| panic!("{id} was deleted with its binding"));
            assert!(vm.agents.is_empty(), "{id} still claims an agent");
            assert!(!vm.is_current, "{id} still reads as in use");
        }
    }

    /// The state that prompted this: an agent Kiwano still holds a takeover
    /// backup and a placeholder-key row for, whose config another tool has since
    /// rewritten to point at the provider directly. It reads as *not* taken
    /// over, because nothing of ours is in the file — a toggle that said
    /// otherwise claimed traffic the gateway never sees, and the provider list
    /// (rightly) showed the provider unbound.
    #[test]
    fn a_config_reverted_behind_our_back_is_not_taken_over() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-real"}}"#,
        )
        .unwrap();
        s.insert_provider(&provider("p1", "Relay", Billing::Metered))
            .unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "p1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        set_agent_takeover(
            &aux,
            "claude",
            true,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();
        assert!(crate::takeover::live_placeholder_key("claude", tmp.path(), &no_vars()).is_some());

        // Another tool puts the agent's own config back. The backup row and the
        // key row stay — Kiwano has no way to know it was not us, and nothing
        // here pretends it did.
        let reverted = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.deepseek.com/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-other"}}"#;
        std::fs::write(&settings, reverted).unwrap();
        assert!(aux.load_takeover_backup("claude").is_some());
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .any(|k| k.agent == "claude"));

        let v = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(!claude.enabled, "the file no longer routes through us");
        assert!(claude.placeholder_key.is_none());
        // …and the provider list agrees: the binding is still in the store, but
        // nothing of ours is serving it.
        let vms = build_provider_vms(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert!(vms.iter().all(|p| p.agents.is_empty() && !p.is_current));

        // Taking it over again captures what is there *now*: the stale backup
        // would otherwise be what restore writes back, over a config this
        // takeover is not replacing.
        set_agent_takeover(
            &aux,
            "claude",
            true,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();
        set_agent_takeover(
            &aux,
            "claude",
            false,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), reverted);
    }

    #[test]
    fn lost_backup_restores_the_agent_onto_its_primary_provider() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#,
        )
        .unwrap();

        // A provider the gateway serves for claude, with a path prefix.
        let mut p = provider("p-claude", "Relay", Billing::Metered);
        p.base_url = "https://relay.example.com".into();
        p.api_path = Some("/anthropic".into());
        p.api_key = Some("sk-real".into());
        s.insert_provider(&p).unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "p-claude".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        set_agent_takeover(
            &aux,
            "claude",
            true,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();
        // Lose the backup: the escape hatch is gone, so restore has to fall
        // back to the provider instead of reporting a success it did not have.
        aux.delete_takeover_backup("claude").unwrap();
        set_agent_takeover(
            &aux,
            "claude",
            false,
            "http://127.0.0.1:8317",
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess(&s),
        )
        .unwrap();

        let env: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            env["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com/anthropic"
        );
        assert_eq!(env["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-real");
    }
}
