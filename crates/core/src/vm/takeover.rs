//! The view-model half of `crate::takeover`: enabling an agent takeover,
//! rebuilding its route, and importing a provider from a live agent config.

use crate::detect::ShellVars;
use crate::vm::agents::AGENTS;
use crate::vm::catalog::link_providers;
use crate::vm::provider_edit::absolute_endpoint;
use crate::vm::time::{rfc3339, unix_now};
use crate::vm::{e2s, slug, Aux};
use kiwanod::store::{Binding, Provider, Store, StrategyType};

pub fn set_agent_takeover(
    store: &Store,
    aux: &Aux,
    agent: &str,
    enabled: bool,
    data_port: u16,
    home: &std::path::Path,
    vars: &ShellVars,
) -> Result<(), String> {
    if !AGENTS.iter().any(|(a, _)| *a == agent) {
        return Err(format!("unknown agent: {agent}"));
    }
    if enabled {
        // First-takeover import: pick up the provider the agent is currently
        // using and bind it as the agent's sole candidate, so the gateway has
        // a route on day one (official-login/blank configs yield no creds —
        // the onboarding guide steers those users to manual entry). Import or
        // binding failures never block the takeover itself.
        if let Some(creds) = crate::creds::read_current_creds(agent, home) {
            match import_current_provider(store, &creds) {
                Ok(provider_id) => {
                    if store.bindings_for_agent(agent).map_err(e2s)?.is_empty() {
                        store
                            .upsert_strategy(agent, StrategyType::Single, None)
                            .map_err(e2s)?;
                        store
                            .upsert_binding(&Binding {
                                agent: agent.to_string(),
                                provider_id,
                                priority: 0,
                                weight: 1,
                                win_start: None,
                                win_end: None,
                                enabled: true,
                            })
                            .map_err(e2s)?;
                    }
                }
                Err(e) => eprintln!("kiwano: current-provider import skipped: {e}"),
            }
            // The import leaves the link empty (the agent's config knows
            // nothing about our catalog), and this is the one path where the
            // provider starts carrying traffic before any backfill pass runs —
            // takeover is followed immediately by real requests.
            let _ = link_providers(store, aux);
        }
        let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
        let key = format!("kw-ag-{agent}-{rand}");
        store.upsert_placeholder_key(&key, agent).map_err(e2s)?;
        // Rewrite the Agent config (backup → base_url → placeholder key); on
        // failure roll back the key registration to stay consistent
        if let Err(e) = crate::takeover::enable(aux, agent, &key, data_port, home, vars) {
            let _ = store.delete_placeholder_key(&key);
            return Err(e);
        }
    } else {
        // The provider the gateway serves for this agent, handed to restore as
        // the rebuild tier: losing the backup must not strand the agent at
        // loopback if there is a provider to point it back at.
        let fallback = rebuild_route(store, agent)?;
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
        // The registration is dropped whatever the restore outcome: leave it
        // and the UI keeps offering a key the config no longer carries.
        for k in store.list_placeholder_keys().map_err(e2s)? {
            if k.agent == agent {
                store.delete_placeholder_key(&k.key).map_err(e2s)?;
            }
        }
        // …and so does the route, for the same reason one step further out: an
        // agent that has its own config back sends us nothing, so its candidate
        // list and strategy are stale the moment the restore lands — invisible
        // in the agent tab (which shows the takeover onboarding again) but not
        // in the Apps screen, which kept reading the rows as "bound" and even
        // as "In use" (see `live_bound_agents`). The *providers* stay: they are
        // the user's own rows, with their keys, plans and usage history, and
        // they are what a later takeover re-imports and binds again.
        for b in store.bindings_for_agent(agent).map_err(e2s)? {
            store.delete_binding(agent, &b.provider_id).map_err(e2s)?;
        }
        store.delete_strategy(agent).map_err(e2s)?;
    }
    Ok(())
}

/// The route restore can rebuild an agent's config from: its primary provider,
/// when that provider carries a key and does not itself point at the gateway.
///
/// `None` is a real answer — the strip tier then applies. Only the agents the
/// takeover module can actually rewrite from a provider get looked up at all:
/// the additive agents' `kiwano-gateway` entry and Claude Desktop's
/// configLibrary profile are kiwano-authored projections with no faithful
/// provider-side rebuild.
fn rebuild_route(
    store: &Store,
    agent: &str,
) -> Result<Option<crate::takeover::ProviderRoute>, String> {
    if !crate::takeover::REBUILDABLE_AGENTS.contains(&agent) {
        return Ok(None);
    }
    let Some(id) = store.primary_provider_id(agent).map_err(e2s)? else {
        return Ok(None);
    };
    let Some(provider) = store.get_provider(&id).map_err(e2s)? else {
        return Ok(None);
    };
    let Some(api_key) = provider
        .api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    else {
        return Ok(None);
    };
    // A provider whose own endpoint is the gateway (a card created from an
    // already taken-over config, say) would rebuild the agent straight back
    // onto loopback, which is the one outcome restore must never produce.
    let base_url = join_provider_url(&provider.base_url, provider.api_path.as_deref());
    if kiwano_adapters::codex_config::is_loopback_gateway_url(&base_url) {
        return Ok(None);
    }
    Ok(Some(crate::takeover::ProviderRoute { base_url, api_key }))
}

/// `base_url` with the provider's optional `api_path` prefix appended — the URL
/// the gateway itself forwards to, and therefore the one an agent rebuilt onto
/// this provider has to hold.
fn join_provider_url(base_url: &str, api_path: Option<&str>) -> String {
    let base = base_url.trim().trim_end_matches('/');
    match api_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => format!(
            "{base}/{}",
            path.trim_start_matches('/').trim_end_matches('/')
        ),
        None => base.to_string(),
    }
}

/// Find-or-create a provider for the agent's current credentials: dedup by
/// base_url (trailing slash ignored) reuses the existing row — that shared
/// provider then also serves other agents; otherwise insert a new PAYG row
/// named after the config's provider key (or the URL host).
fn import_current_provider(
    store: &Store,
    creds: &crate::creds::CurrentCreds,
) -> Result<String, String> {
    // An agent's config is another tool's file, and it can hold a bare host —
    // which is how a provider ends up stored as `api.deepseek.com` and unrouted.
    // Normalizing before the dedup lookup also means a config that gains its
    // scheme later still matches the row it made without one.
    let base = absolute_endpoint(creds.base_url.trim_end_matches('/'));
    for p in store.list_providers().map_err(e2s)? {
        if p.base_url.trim().trim_end_matches('/') == base {
            return Ok(p.id);
        }
    }
    let now = rfc3339(unix_now());
    let name = creds.name.clone().unwrap_or_else(|| {
        let host = crate::creds::host_of(&creds.base_url);
        crate::creds::brand_name_for_host(&host)
            .map(String::from)
            .unwrap_or_else(|| {
                if host.is_empty() {
                    "Imported provider".into()
                } else {
                    host
                }
            })
    });
    let provider = Provider {
        // Imported from another manager: no Hub catalog entry behind it.
        catalog_id: None,
        model_default: None,
        id: format!(
            "{}-{}",
            slug(&name),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        ),
        name,
        protocol: kiwanod::store::Protocol::parse_str(creds.protocol)
            .unwrap_or(kiwanod::store::Protocol::OpenAI),
        base_url: base.to_string(),
        api_path: None,
        endpoints: Vec::new(),
        api_key: Some(creds.api_key.clone()),
        billing: kiwanod::store::Billing::Metered,
        period_limit: None,
        limit_unit: None,
        reset_period: None,
        plan_query: None,
        plan_limits: None,
        prices: None,
        timeout_secs: None,
        retries: None,
        headers: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    let id = provider.id.clone();
    store.insert_provider(&provider).map_err(e2s)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::providers::build_provider_vms;
    use crate::vm::settings::build_settings_with_home;
    use crate::vm::test_support::{no_vars, provider, store};
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Binding, StrategyType};

    #[test]
    fn import_current_provider_dedups_by_base_url() {
        let s = store();
        let creds = |base: &str, name: Option<&str>| crate::creds::CurrentCreds {
            base_url: base.into(),
            api_key: "sk-x".into(),
            name: name.map(String::from),
            protocol: "openai",
        };
        // trailing-slash variants dedup to one row
        let id1 =
            import_current_provider(&s, &creds("https://api.deepseek.com/v1/", Some("deepseek")))
                .unwrap();
        let id2 =
            import_current_provider(&s, &creds("https://api.deepseek.com/v1", Some("deepseek")))
                .unwrap();
        assert_eq!(id1, id2);
        let list = s.list_providers().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "deepseek");
        assert_eq!(list[0].api_key.as_deref(), Some("sk-x"));
        // no declared name → brand name inferred from the host
        let id3 = import_current_provider(&s, &creds("https://api.x.ai/v1", None)).unwrap();
        let p = s.get_provider(&id3).unwrap().unwrap();
        assert_eq!(p.name, "xAI");
        assert_ne!(id1, id3);
    }

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

        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        assert_eq!(s.bindings_for_agent("claude").unwrap().len(), 2);
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();

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
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
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
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();
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

        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        // Lose the backup: the escape hatch is gone, so restore has to fall
        // back to the provider instead of reporting a success it did not have.
        aux.delete_takeover_backup("claude").unwrap();
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();

        let env: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            env["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com/anthropic"
        );
        assert_eq!(env["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-real");
    }
}
