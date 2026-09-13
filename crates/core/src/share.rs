//! Config sharing (spec §4.1 P1): one-click export/import of a config scheme.
//!
//! File format kiwano-config v1:
//! `providers` (gateway Provider rows) + `routes` (per-Agent strategy and
//! candidate order). Credentials are **omitted by default**: `api_key` is
//! only written when the caller opts in (`export_config(store, true)`), for
//! the local-backup / cross-device migration case where the file never leaves
//! the machine. An export without keys still imports cleanly — matching is by
//! `(name, base_url)`, and a key is only backfilled into a local row that has
//! none.
//!
//! Import semantics (merge, not overwrite): match existing providers by
//! `(name, base_url)` — on a hit the local row is kept and the key is only
//! backfilled when the local row has no key and the scheme carries one; on a
//! miss a new provider is created (id regenerated to avoid clashing with
//! local ids). Bindings are remapped from exported id → final id and
//! upserted in order (priority = order); strategy rows are upserted
//! directly; bindings of untouched agents are left alone. The caller is
//! responsible for triggering admin /reload.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use kiwano_gateway::store::{Binding, Provider, Store, StrategyType};

use crate::vm;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct ConfigShare {
    kiwano_config: u32,
    exported_at: String,
    providers: Vec<Provider>,
    routes: Vec<ShareRoute>,
}

#[derive(Serialize, Deserialize)]
struct ShareRoute {
    agent: String,
    strategy: String,
    config: Option<String>,
    /// Provider id order at export time (i.e. the priority).
    candidates: Vec<String>,
}

/// Import result (for the frontend to display).
#[derive(Serialize)]
pub struct ImportReport {
    pub providers_added: usize,
    pub providers_kept: usize,
    pub routes_applied: usize,
}

/// Export all providers and per-Agent route schemes as shareable JSON.
///
/// `include_keys` is the explicit opt-in for credentials. It is `false` on the
/// registered `export_config` IPC command — that surface is dormant (no
/// frontend screen calls it) and must not leak when it is finally wired up.
/// Pass `true` only for a local backup that never leaves the machine.
pub fn export_config(store: &Store, include_keys: bool) -> Result<String, String> {
    let mut providers = store.list_providers().map_err(|e| e.to_string())?;
    if !include_keys {
        for p in providers.iter_mut() {
            p.api_key = None;
        }
    }
    let mut routes = Vec::new();
    for agent in store.bound_agents().map_err(|e| e.to_string())? {
        let (strategy, config) = store
            .get_strategy(&agent)
            .map_err(|e| e.to_string())?
            .map(|s| (s.kind.as_str().to_string(), s.config))
            .unwrap_or_else(|| ("single".to_string(), None));
        let candidates = store
            .bindings_for_agent(&agent)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|b| b.provider_id)
            .collect();
        routes.push(ShareRoute {
            agent,
            strategy,
            config,
            candidates,
        });
    }
    let share = ConfigShare {
        kiwano_config: FORMAT_VERSION,
        exported_at: vm::rfc3339(vm::unix_now()),
        providers,
        routes,
    };
    serde_json::to_string_pretty(&share).map_err(|e| e.to_string())
}

/// Import a scheme (semantics in the module doc); returns a count report.
pub fn import_config(store: &Store, json: &str) -> Result<ImportReport, String> {
    let share: ConfigShare =
        serde_json::from_str(json).map_err(|e| format!("Not a valid Kiwano config file: {e}"))?;
    if share.kiwano_config != FORMAT_VERSION {
        return Err(format!(
            "Unsupported config version {}",
            share.kiwano_config
        ));
    }

    let now = vm::rfc3339(vm::unix_now());
    // (name, base_url) → local id
    let mut by_identity: HashMap<(String, String), String> = HashMap::new();
    for p in store.list_providers().map_err(|e| e.to_string())? {
        by_identity.insert((p.name.clone(), p.base_url.clone()), p.id.clone());
    }
    let mut remap: HashMap<String, String> = HashMap::new();
    let mut added = 0usize;
    let mut kept = 0usize;

    for sp in &share.providers {
        let key = (sp.name.clone(), sp.base_url.clone());
        if let Some(local_id) = by_identity.get(&key).cloned() {
            // Already exists locally: only backfill a missing key (all other fields stay local)
            if sp.api_key.as_deref().is_some_and(|k| !k.is_empty()) {
                if let Some(mut local) = store.get_provider(&local_id).map_err(|e| e.to_string())? {
                    if local.api_key.as_deref().unwrap_or("").is_empty() {
                        local.api_key = sp.api_key.clone();
                        local.updated_at = now.clone();
                        store.update_provider(&local).map_err(|e| e.to_string())?;
                    }
                }
            }
            remap.insert(sp.id.clone(), local_id);
            kept += 1;
        } else {
            let new_id = format!(
                "{}-{}",
                vm::slug(&sp.name),
                &uuid::Uuid::new_v4().simple().to_string()[..6]
            );
            store
                .insert_provider(&Provider {
                    id: new_id.clone(),
                    name: sp.name.clone(),
                    protocol: sp.protocol,
                    base_url: sp.base_url.clone(),
                    api_path: sp.api_path.clone(),
                    endpoints: sp.endpoints.clone(),
                    api_key: sp.api_key.clone(),
                    billing: sp.billing,
                    period_limit: sp.period_limit,
                    limit_unit: sp.limit_unit.clone(),
                    reset_period: sp.reset_period.clone(),
                    // Plan-query credentials are intentionally not shared.
                    plan_query: None,
                    // Percent limits carry over (no credentials inside).
                    plan_limits: sp.plan_limits.clone(),
                    timeout_secs: sp.timeout_secs,
                    retries: sp.retries,
                    headers: sp.headers.clone(),
                    enabled: sp.enabled,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                })
                .map_err(|e| e.to_string())?;
            by_identity.insert(key, new_id.clone());
            remap.insert(sp.id.clone(), new_id);
            added += 1;
        }
    }

    let mut routes_applied = 0usize;
    for r in &share.routes {
        let kind = StrategyType::parse_str(&r.strategy).unwrap_or(StrategyType::Single);
        store
            .upsert_strategy(&r.agent, kind, r.config.as_deref())
            .map_err(|e| e.to_string())?;
        for (i, pid) in r.candidates.iter().enumerate() {
            let Some(final_id) = remap.get(pid) else {
                continue; // Scheme references a provider outside the file → skip this candidate
            };
            store
                .upsert_binding(&Binding {
                    agent: r.agent.clone(),
                    provider_id: final_id.clone(),
                    priority: i as i64,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(|e| e.to_string())?;
        }
        routes_applied += 1;
    }
    Ok(ImportReport {
        providers_added: added,
        providers_kept: kept,
        routes_applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwano_gateway::store::{Billing, Protocol};

    fn provider(id: &str, name: &str, base_url: &str, api_key: Option<&str>) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            protocol: Protocol::OpenAI,
            base_url: base_url.into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: api_key.map(Into::into),
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: now_stamp(),
            updated_at: now_stamp(),
        }
    }

    fn now_stamp() -> String {
        vm::rfc3339(vm::unix_now())
    }

    #[test]
    fn export_roundtrip_and_merge_by_identity() {
        let src = Store::open_in_memory().unwrap();
        src.insert_provider(&provider(
            "p1",
            "Alpha",
            "https://a.example.com",
            Some("sk-a"),
        ))
        .unwrap();
        src.insert_provider(&provider(
            "p2",
            "Beta",
            "https://b.example.com",
            Some("sk-b"),
        ))
        .unwrap();
        src.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (pid, pr) in [("p1", 0), ("p2", 1)] {
            src.upsert_binding(&Binding {
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
        let json = export_config(&src, true).unwrap();
        assert!(json.contains("kiwano_config"));

        // Import into an empty DB → everything is created fresh + route remapping takes effect
        let dst = Store::open_in_memory().unwrap();
        let report = import_config(&dst, &json).unwrap();
        assert_eq!(report.providers_added, 2);
        assert_eq!(report.routes_applied, 1);
        assert_eq!(dst.list_providers().unwrap().len(), 2);
        assert!(dst.primary_provider_id("claude").unwrap().is_some());
        let bs = dst.bindings_for_agent("claude").unwrap();
        assert_eq!(bs.len(), 2);
        assert_eq!(
            dst.get_strategy("claude").unwrap().unwrap().kind,
            StrategyType::Failover
        );

        // Import with a local same-name/same-endpoint provider (no key) → keep local + backfill key
        let dst2 = Store::open_in_memory().unwrap();
        dst2.insert_provider(&provider("local-1", "Alpha", "https://a.example.com", None))
            .unwrap();
        let report2 = import_config(&dst2, &json).unwrap();
        assert_eq!(report2.providers_kept, 1);
        assert_eq!(report2.providers_added, 1);
        let local = dst2.get_provider("local-1").unwrap().unwrap();
        assert_eq!(local.api_key.as_deref(), Some("sk-a"));
        // claude primary maps to the local id
        assert_eq!(
            dst2.primary_provider_id("claude").unwrap().as_deref(),
            Some("local-1")
        );
    }

    #[test]
    fn import_rejects_garbage_and_unknown_versions() {
        let s = Store::open_in_memory().unwrap();
        assert!(import_config(&s, "not json").is_err());
        assert!(import_config(&s, r#"{"kiwano_config": 99}"#).is_err());
    }

    /// A store with two keyed providers and one bound agent.
    fn seeded_store() -> Store {
        let src = Store::open_in_memory().unwrap();
        src.insert_provider(&provider(
            "p1",
            "Alpha",
            "https://a.example.com",
            Some("sk-a"),
        ))
        .unwrap();
        src.insert_provider(&provider(
            "p2",
            "Beta",
            "https://b.example.com",
            Some("sk-b"),
        ))
        .unwrap();
        src.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (pid, pr) in [("p1", 0), ("p2", 1)] {
            src.upsert_binding(&Binding {
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
        src
    }

    #[test]
    fn export_omits_credentials_unless_opted_in() {
        let src = seeded_store();

        // Default: no key material anywhere in the file.
        let json = export_config(&src, false).unwrap();
        assert!(!json.contains("sk-a"), "{json}");
        assert!(!json.contains("sk-b"), "{json}");
        // Provider identity and routes still travel.
        assert!(json.contains("Alpha"));
        assert!(json.contains("https://b.example.com"));
        assert!(json.contains("claude"));

        // Opt-in (local backup): keys are present and round-trip.
        let json = export_config(&src, true).unwrap();
        assert!(json.contains("sk-a"));
        assert!(json.contains("sk-b"));

        let dst = Store::open_in_memory().unwrap();
        import_config(&dst, &json).unwrap();
        let by_name = |name: &str| {
            dst.list_providers()
                .unwrap()
                .into_iter()
                .find(|p| p.name == name)
                .unwrap()
                .api_key
        };
        assert_eq!(by_name("Alpha").as_deref(), Some("sk-a"));
        assert_eq!(by_name("Beta").as_deref(), Some("sk-b"));
    }

    #[test]
    fn keyless_export_still_merges() {
        let json = export_config(&seeded_store(), false).unwrap();

        // Fresh DB: rows are created, routes remap, keys are simply absent.
        let dst = Store::open_in_memory().unwrap();
        let report = import_config(&dst, &json).unwrap();
        assert_eq!(report.providers_added, 2);
        assert_eq!(report.routes_applied, 1);
        assert_eq!(dst.bindings_for_agent("claude").unwrap().len(), 2);
        assert!(dst.list_providers().unwrap().iter().all(|p| p
            .api_key
            .as_deref()
            .unwrap_or("")
            .is_empty()));

        // Existing local row *with* a key: the import must not clear it.
        let dst2 = Store::open_in_memory().unwrap();
        dst2.insert_provider(&provider(
            "local-1",
            "Alpha",
            "https://a.example.com",
            Some("sk-local"),
        ))
        .unwrap();
        let report2 = import_config(&dst2, &json).unwrap();
        assert_eq!(report2.providers_kept, 1);
        assert_eq!(report2.providers_added, 1);
        assert_eq!(
            dst2.get_provider("local-1")
                .unwrap()
                .unwrap()
                .api_key
                .as_deref(),
            Some("sk-local")
        );
        assert_eq!(
            dst2.primary_provider_id("claude").unwrap().as_deref(),
            Some("local-1")
        );
    }
}
