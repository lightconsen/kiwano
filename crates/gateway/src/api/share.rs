//! Config share: export the whole configuration as JSON, and import one back.
//!
//! Moved here from `kiwano_core::share` when the daemon took over the two
//! library halves (`migrate.local.md` §10.17). The split is the one §7 batch 3
//! describes for this family: **the file is the client's, the rows are the
//! daemon's** — the app reads the path the user picked and writes the bytes it
//! gets back, and everything that touches the store happens here.

use crate::store::{Binding, Provider, Store, StrategyType};
use kiwano_api::error::ApiError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Format version of the exported document.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub providers_added: usize,
    pub providers_kept: usize,
    pub routes_applied: usize,
}

pub fn export_config(store: &Store, include_keys: bool) -> Result<String, ApiError> {
    let mut providers = store.list_providers().map_err(ApiError::failed)?;
    if !include_keys {
        for p in providers.iter_mut() {
            p.api_key = None;
        }
    }
    let mut routes = Vec::new();
    for agent in store.bound_agents().map_err(ApiError::failed)? {
        let (strategy, config) = store
            .get_strategy(&agent)
            .map_err(ApiError::failed)?
            .map(|s| (s.kind.as_str().to_string(), s.config))
            .unwrap_or_else(|| ("single".to_string(), None));
        let candidates = store
            .bindings_for_agent(&agent)
            .map_err(ApiError::failed)?
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
        exported_at: crate::store::now_rfc3339(),
        providers,
        routes,
    };
    serde_json::to_string_pretty(&share).map_err(ApiError::failed)
}

pub fn import_config(store: &Store, json: &str) -> Result<ImportReport, ApiError> {
    let share: ConfigShare = serde_json::from_str(json)
        .map_err(|e| ApiError::invalid(format!("Not a valid Kiwano config file: {e}")))?;
    if share.kiwano_config != FORMAT_VERSION {
        return Err(ApiError::invalid(format!(
            "Unsupported config version {}",
            share.kiwano_config
        )));
    }

    let now = crate::store::now_rfc3339();
    // (name, base_url) → local id
    let mut by_identity: HashMap<(String, String), String> = HashMap::new();
    for p in store.list_providers().map_err(ApiError::failed)? {
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
                if let Some(mut local) = store.get_provider(&local_id).map_err(ApiError::failed)? {
                    if local.api_key.as_deref().unwrap_or("").is_empty() {
                        local.api_key = sp.api_key.clone();
                        local.updated_at = now.clone();
                        store.update_provider(&local).map_err(ApiError::failed)?;
                    }
                }
            }
            remap.insert(sp.id.clone(), local_id);
            kept += 1;
        } else {
            let new_id = format!(
                "{}-{}",
                kiwano_api::ids::slug(&sp.name),
                &uuid::Uuid::new_v4().simple().to_string()[..6]
            );
            store
                .insert_provider(&Provider {
                    id: new_id.clone(),
                    name: sp.name.clone(),
                    // Carried over, unlike the plan-query credentials below: a
                    // catalog id names a Hub entry, not something local to the
                    // exporting install — so the imported provider is priced at
                    // the same entry's rates as the one it came from.
                    catalog_id: sp.catalog_id.clone(),
                    protocol: sp.protocol,
                    openai_wire: sp.openai_wire,
                    base_url: sp.base_url.clone(),
                    api_path: sp.api_path.clone(),
                    endpoints: sp.endpoints.clone(),
                    api_key: sp.api_key.clone(),
                    // A model name, not a credential: it stays useful to
                    // whoever imports the provider.
                    model_default: sp.model_default.clone(),
                    billing: sp.billing,
                    period_limit: sp.period_limit,
                    // Same rule the dialog and the CLI apply: the currency has to be
                    // one this machine can price against, and a shared file is the
                    // one place it can arrive without anyone having said so.
                    limit_unit: super::limits::normalize_limit_unit(
                        sp.limit_unit.as_deref(),
                        sp.period_limit.is_some(),
                        &super::limits::known_limit_currencies(store),
                    )
                    .map_err(ApiError::invalid)?,
                    reset_period: sp.reset_period.clone(),
                    // What the user declared this provider charges, re-validated
                    // the way the limit's unit just above is: it travels with
                    // the row, so a shared provider keeps being costed at its
                    // own rates rather than falling back to the Hub's.
                    prices: super::limits::import_declared_prices(
                        sp.prices.as_deref(),
                        &super::limits::known_limit_currencies(store),
                    )
                    .map_err(ApiError::invalid)?,
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
                .map_err(ApiError::failed)?;
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
            .map_err(ApiError::failed)?;
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
                .map_err(ApiError::failed)?;
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
    use crate::store::{Billing, Provider};

    fn provider(id: &str, name: &str, key: Option<&str>) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            catalog_id: None,
            protocol: crate::store::Protocol::OpenAI,
            openai_wire: crate::store::OpenAiWire::Both,
            base_url: "https://api.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: key.map(str::to_string),
            model_default: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            prices: None,
            reset_period: None,
            enabled: true,
            created_at: crate::store::now_rfc3339(),
            updated_at: crate::store::now_rfc3339(),
        }
    }

    /// The credential opt-in is the whole point of the flag, so both directions
    /// are asserted: the default document carries no key, and the opt-in one
    /// does. A document that leaked a key without being asked would be a silent
    /// export of every credential the user has.
    #[test]
    fn keys_travel_only_when_asked_for() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_provider(&provider("p-1", "P", Some("sk-secret")))
            .unwrap();

        let without = export_config(&store, false).unwrap();
        assert!(!without.contains("sk-secret"), "{without}");
        assert!(without.contains("\"api_key\": null"), "{without}");

        let with = export_config(&store, true).unwrap();
        assert!(with.contains("sk-secret"), "{with}");
    }

    /// A round trip is the property the format exists for: what one install
    /// exports, another can apply. The providers come back, and a second import
    /// of the same document is not a second copy of them.
    #[test]
    fn a_document_round_trips_and_importing_twice_adds_nothing() {
        let source = Store::open_in_memory().unwrap();
        source
            .insert_provider(&provider("p-1", "DeepSeek", Some("sk-a")))
            .unwrap();
        let json = export_config(&source, true).unwrap();

        let target = Store::open_in_memory().unwrap();
        let first = import_config(&target, &json).unwrap();
        assert_eq!(first.providers_added, 1);
        assert_eq!(target.list_providers().unwrap().len(), 1);

        let again = import_config(&target, &json).unwrap();
        assert_eq!(
            again.providers_added, 0,
            "the same provider is not added twice"
        );
        assert_eq!(target.list_providers().unwrap().len(), 1);
    }

    /// The refusals carry the kind the status is picked from, and the sentences
    /// a person reads — including the one that names the version.
    #[test]
    fn the_refusals_are_kinds_and_keep_their_wording() {
        let store = Store::open_in_memory().unwrap();

        let not_json = import_config(&store, "{not json").unwrap_err();
        assert_eq!(not_json.kind(), kiwano_api::error::ApiErrorKind::Invalid);
        assert!(
            not_json
                .message()
                .starts_with("Not a valid Kiwano config file:"),
            "{}",
            not_json.message()
        );

        let wrong_version = import_config(
            &store,
            r#"{"kiwano_config":99,"exported_at":"t","providers":[],"routes":[]}"#,
        )
        .unwrap_err();
        assert_eq!(
            wrong_version.kind(),
            kiwano_api::error::ApiErrorKind::Invalid
        );
        assert_eq!(wrong_version.message(), "Unsupported config version 99");
    }
}
