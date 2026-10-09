//! Applying a cc-switch import: the **store half** of it.
//!
//! Reading is the client's — cc-switch's files are in the user's home and one of
//! them is a SQLite database (`migrate.local.md` §10.19) — and writing providers
//! is the daemon's. The split falls exactly where the reading stops: the client
//! sends the rows it extracted, and this applies them.
//!
//! `RawProvider` travels as JSON rather than as a shared type, on purpose: it is
//! a *file* shape (cc-switch's), not a wire shape, and putting it in
//! `kiwano-api` would claim it is something both sides have to agree on forever.

use crate::store::{Binding, Protocol, Provider, Store, StrategyType};
use kiwano_api::error::ApiError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One row as a cc-switch file carries it, normalized by the reader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawProvider {
    pub cc_id: String,
    /// One of Kiwano's agent ids.
    pub app: String,
    pub name: String,
    pub base_url: String,
    pub api_path: Option<String>,
    pub api_key: Option<String>,
    pub is_current: bool,
}

/// What an import did, for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReportVm {
    pub imported: usize,
    pub skipped: usize,
    pub detail: Vec<String>,
}

/// Apply the rows a reader extracted — the **store half** of an import.
///
/// Moved here from `kiwano_core::import` (`migrate.local.md` §10.19): reading
/// cc-switch's files is the client's (they are in the user's home, and one of
/// them is a SQLite database), but writing providers is the daemon's, and the
/// split falls exactly where the reading stops. `RawProvider` travels as JSON,
/// so the two halves agree on the shape without sharing a type.
pub fn apply_import(
    store: &Store,
    raws: &[RawProvider],
    skips: &[String],
) -> Result<ImportReportVm, ApiError> {
    let (mut imported, mut skipped) = (0usize, 0usize);
    let mut detail: Vec<String> = Vec::new();
    for line in skips {
        skipped += 1;
        detail.push(line.clone());
    }

    // provider_id by (protocol, origin): a provider that is already here —
    // hand-added, or imported in an earlier run — is the same upstream, so an
    // import reuses it rather than growing a second row beside it.
    let mut by_endpoint: HashMap<(Protocol, String), String> = HashMap::new();
    if !raws.is_empty() {
        for p in store.list_providers().ok().unwrap_or_default() {
            by_endpoint.insert(
                (p.protocol, p.base_url.trim_end_matches('/').to_string()),
                p.id.clone(),
            );
        }
    }

    for raw in raws {
        let (name, protocol) = match raw.app.as_str() {
            "claude" | "claude-desktop" => (raw.name.clone(), Protocol::Anthropic),
            _ => (raw.name.clone(), Protocol::OpenAI),
        };
        // Skip official placeholders / empty configs
        if raw.base_url.is_empty() || raw.api_key.as_deref().unwrap_or("").is_empty() {
            skipped += 1;
            detail.push(format!(
                "skip {}/{}: missing endpoint or key",
                raw.app, raw.cc_id
            ));
            continue;
        }
        let id = format!("ccs-{}-{}", raw.app, kiwano_api::ids::slug(&raw.cc_id));
        // The deterministic id is *ours*: a row already under it is this
        // import's earlier run, refreshed in place. Any other provider with the
        // same origin and protocol — one the user hand-added, or imported from
        // an earlier manager — is the same upstream, and a fresh row beside it
        // would be a duplicate that only the id's prefix distinguishes. Neither
        // is rewritten: the reuse case is somebody else's row, and what it
        // carries is theirs to keep.
        let endpoint_key = (protocol, raw.base_url.trim_end_matches('/').to_string());
        let reused = store
            .get_provider(&id)
            .ok()
            .flatten()
            .map(|_| None)
            .unwrap_or_else(|| by_endpoint.get(&endpoint_key).cloned());
        if let Some(target) = reused {
            imported += 1;
            detail.push(format!(
                "import {id}: {name} ({}) · uses existing provider {target}",
                protocol.as_str()
            ));
            if raw.is_current {
                let agent = raw.app.clone(); // app_type == Kiwano agent id (1:1 for every app)
                let cur = store.primary_provider_id(&agent).ok().flatten();
                match cur {
                    // The agent's routing is the user's, and one is already
                    // theirs — leave the cc-switch choice out of it. Only an
                    // agent with no route yet is imported onto.
                    Some(existing) if existing != target => {
                        skipped += 1;
                        detail.push(format!(
                            "leave {agent} routed to {existing} (cc-switch wanted {target})"
                        ));
                    }
                    _ => {
                        let _ = store.upsert_strategy(&agent, StrategyType::Single, None);
                        let _ = store.upsert_binding(&Binding {
                            agent: agent.to_string(),
                            provider_id: target,
                            priority: 0,
                            weight: 1,
                            win_start: None,
                            win_end: None,
                            enabled: true,
                        });
                    }
                }
            }
            continue;
        }
        let now = crate::store::now_rfc3339();
        let mut provider = Provider {
            id: id.clone(),
            name,
            catalog_id: None,
            protocol,
            base_url: raw.base_url.clone(),
            api_path: raw.api_path.clone(),
            endpoints: Vec::new(),
            api_key: raw.api_key.clone(),
            // The cc-switch shape has no model field to carry.
            model_default: None,
            billing: crate::store::Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            // The cc-switch shape carries no prices to import, and inventing
            // them from another manager's fields is not this importer's job.
            // A row already here keeps the ones the app collected — set below,
            // after the existing row is read.
            prices: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };
        let existing = store.get_provider(&id).ok().flatten();
        // A re-import updates this row wholesale — the endpoint, the key and the
        // billing are what it came to refresh — but the declared prices are not
        // in a cc-switch file at all, and the app is the only place they can be
        // written. Clearing them here would silently re-price the provider's
        // requests and move its spending limit on an import nobody asked to do
        // either.
        if let Some(prev) = &existing {
            provider.prices = prev.prices.clone();
        }
        let existed = existing.is_some();
        let up = if existed {
            store.update_provider(&provider)
        } else {
            store.insert_provider(&provider)
        };
        if let Err(e) = up {
            skipped += 1;
            detail.push(format!("skip {id}: {e}"));
            continue;
        }
        // **The map has to learn this row**, or a second entry in the same file
        // that names the same upstream misses the lookup and inserts a second
        // provider for it — the duplicate the reuse arm exists to prevent. It
        // only ever knew about rows that were here *before* the run.
        by_endpoint.insert(endpoint_key, id.clone());
        imported += 1;
        detail.push(format!(
            "import {id}: {} ({}){}",
            provider.name,
            provider.protocol.as_str(),
            if existed { " · updated" } else { "" },
        ));
        if raw.is_current {
            let agent = raw.app.clone(); // app_type == Kiwano agent id (1:1 for every app)
            if store
                .upsert_strategy(&agent, StrategyType::Single, None)
                .is_ok()
            {
                let prev = store.primary_provider_id(&agent).ok().flatten();
                // Same leave-it-alone rule as the reuse arm: an agent this
                // import would move off its current route is an agent whose
                // routing is already the user's answer.
                match prev {
                    Some(cur) if cur != id => {
                        skipped += 1;
                        detail.push(format!(
                            "leave {agent} routed to {cur} (cc-switch wanted {id})"
                        ));
                    }
                    _ => {
                        let _ = store.upsert_binding(&Binding {
                            agent: agent.to_string(),
                            provider_id: id.clone(),
                            priority: 0,
                            weight: 1,
                            win_start: None,
                            win_end: None,
                            enabled: true,
                        });
                        if let Some(prev) = prev {
                            let _ = store.upsert_binding(&Binding {
                                agent: agent.to_string(),
                                provider_id: prev,
                                priority: 1,
                                weight: 1,
                                win_start: None,
                                win_end: None,
                                enabled: true,
                            });
                        }
                    }
                }
            }
        }
    }
    detail.truncate(30);
    Ok(ImportReportVm {
        imported,
        skipped,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(cc_id: &str, app: &str, base_url: &str) -> RawProvider {
        RawProvider {
            cc_id: cc_id.into(),
            app: app.into(),
            name: format!("{app}-{cc_id}"),
            base_url: base_url.into(),
            api_path: None,
            api_key: Some(format!("sk-{cc_id}")),
            is_current: false,
        }
    }

    /// Two entries in one file naming the same upstream are **one** provider.
    ///
    /// The dedup map is what makes that true, and it is built from the rows that
    /// exist before the run — so an import that adds a row has to tell it about
    /// what it just added. Without that, the second entry misses the lookup and
    /// inserts a duplicate of the first, which is the exact thing the reuse arm
    /// exists to prevent.
    #[test]
    fn two_entries_for_one_upstream_are_one_provider() {
        let store = Store::open_in_memory().unwrap();
        // Same app (so the same protocol) and the same URL, two different
        // cc-switch ids — which is how one upstream appears twice in one file.
        let raws = [
            raw("a", "claude", "https://api.same.example"),
            raw("b", "claude", "https://api.same.example"),
        ];

        let report = apply_import(&store, &raws, &[]).unwrap();

        assert_eq!(report.imported, 2, "both entries are accounted for");
        assert_eq!(
            store.list_providers().unwrap().len(),
            1,
            "but they are one upstream, so one row: {:#?}",
            store.list_providers().unwrap()
        );
        assert!(
            report
                .detail
                .iter()
                .any(|d| d.contains("uses existing provider")),
            "the second entry says it reused one: {:#?}",
            report.detail
        );
    }

    /// A row that is already here is reused rather than duplicated — the case
    /// the map was built for, kept so the fix above cannot regress it.
    #[test]
    fn an_entry_matching_an_existing_provider_reuses_it() {
        let store = Store::open_in_memory().unwrap();
        apply_import(
            &store,
            &[raw("a", "claude", "https://api.here.example")],
            &[],
        )
        .unwrap();
        assert_eq!(store.list_providers().unwrap().len(), 1);

        let again = apply_import(
            &store,
            &[raw("a", "claude", "https://api.here.example")],
            &[],
        )
        .unwrap();
        assert_eq!(again.imported, 1);
        assert_eq!(store.list_providers().unwrap().len(), 1, "still one row");
    }
}
