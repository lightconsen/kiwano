//! Provider write paths: the inputs the UI sends, their normalization, and the
//! mutations that each end in an admin `/reload`.

use crate::detect::ShellVars;
use crate::vm::catalog::{catalog_id_for, catalog_snapshot};
use crate::vm::fmt::{logo_char, palette_color};
use crate::vm::limits::{
    known_limit_currencies, normalize_declared_prices, normalize_limit_unit, ProviderPricesInput,
};
use crate::vm::providers::{
    advanced_vm, billing_to_db, billing_to_ui, build_provider_vms, display_endpoint, endpoint_note,
    health_vm, provider_currency, vm_endpoints, ProviderVm,
};
use crate::vm::time::{rfc3339, unix_now};
use crate::vm::{e2s, slug, Aux};
use kiwanod::store::{Binding, Provider, Store, StrategyType};
use serde::Deserialize;
use std::path::Path;

/// Plan-mode percent limits (modal form): utilization ceilings over the
/// vendor's rolling 5h / weekly windows. Both optional; both absent = none.
#[derive(Deserialize, Clone)]
pub struct PlanLimitsInput {
    pub five_hour: Option<f64>,
    pub weekly: Option<f64>,
}

#[derive(Deserialize)]
pub struct BillingConfigInput {
    pub limit_value: Option<f64>,
    #[allow(dead_code)]
    pub limit_unit: Option<String>,
    pub reset_period: Option<String>,
    pub plan_limits: Option<PlanLimitsInput>,
}

/// Serialize the percent limits into the `providers.plan_limits` JSON shape.
/// Non-positive / absent percents are dropped; an empty object reads as NULL.
fn plan_limits_json(input: Option<&PlanLimitsInput>) -> Option<String> {
    let input = input?;
    let mut obj = serde_json::Map::new();
    if let Some(pct) = input.five_hour.filter(|p| *p > 0.0 && *p <= 100.0) {
        obj.insert("five_hour".into(), serde_json::json!(pct));
    }
    if let Some(pct) = input.weekly.filter(|p| *p > 0.0 && *p <= 100.0) {
        obj.insert("weekly".into(), serde_json::json!(pct));
    }
    if obj.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(obj).to_string())
    }
}

/// Per-provider advanced forwarding settings (timeout / retries / custom
/// headers), edited in the provider modal's Advanced section. Custom header
/// names/values are sanitized before they reach the gateway.
#[derive(Deserialize)]
pub struct AdvancedInput {
    pub timeout_secs: Option<i64>,
    pub retries: Option<i64>,
    /// Header name → value; serialized to a JSON object column.
    pub headers: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Deserialize)]
pub struct NewEndpointInput {
    pub protocol: String,
    pub endpoint: String,
}

#[derive(Deserialize)]
pub struct NewProviderInput {
    pub name: String,
    #[allow(dead_code)]
    pub api_key: String,
    pub endpoint: String,
    pub protocol: String,
    /// The model the form collected as this provider's default.
    ///
    /// It was carried here for a long time and read by nothing — stored in no
    /// column and returned by no view, so reopening a provider always showed an
    /// empty box. Persisted since v14 (`providers.model_default`), which is what
    /// lets the edit dialog show what the add dialog asked for.
    ///
    /// Remembered rather than consulted: nothing picks a model from it when
    /// routing, because the model a request uses is the one the agent sent. Empty
    /// stores `NULL`.
    pub model_default: String,
    pub billing: String,
    pub billing_config: BillingConfigInput,
    /// Agents to bind this provider to.
    ///
    /// Expected when adding — a new provider nothing serves is a dead row — and
    /// **absent when editing**, where the bindings belong to the Apps screen's
    /// agent tabs. The distinction has to be in the type: a plain `Vec` cannot
    /// tell "no agents" from "not speaking about agents", and the difference is
    /// whether an edit leaves the bindings alone or unbinds the lot.
    #[serde(default)]
    pub agents: Option<Vec<String>>,
    /// Additional per-protocol endpoints; unknown protocol strings are
    /// skipped (defaulting one to openai could collide with the primary).
    #[serde(default)]
    pub endpoints: Vec<NewEndpointInput>,
    /// Advanced forwarding settings. Absent in an update = keep existing
    /// (mirrors the empty-api_key semantics); a present object is an
    /// authoritative snapshot whose null fields clear values.
    #[serde(default)]
    pub advanced: Option<AdvancedInput>,
    /// Token-plan quota query `{"template":"kimi","fields":{...}}`. Absent in
    /// an update = keep existing; null clears; a present object replaces.
    #[serde(default)]
    pub plan_query: Option<serde_json::Value>,
    /// The prices the user declared for this provider. Absent in an update =
    /// keep what is stored (same semantics as an empty `api_key`); a present
    /// bundle is an authoritative snapshot, so an empty model list clears the
    /// column — which is what switching a provider off pay-as-you-go does.
    #[serde(default)]
    pub prices: Option<ProviderPricesInput>,
    /// The Hub catalog entry this provider is being added from, when the add
    /// came from the shelf. Prices are published per catalog entry, and a local
    /// row's own id is `<slug>-<hex>`, so this is what lets a forwarded request
    /// be costed at its provider's own rate rather than the general one.
    ///
    /// Absent (the hand-added form, and every edit) means "no catalog entry":
    /// on add the provider prices at the general rate, and on update the stored
    /// value is kept — an edit must not silently unlink the provider from its
    /// price row.
    #[serde(default)]
    pub catalog_id: Option<String>,
}

/// Map the advanced input to the three store columns: timeout clamps to
/// 1..=3600 (else unset = gateway defaults), retries to 0..=5 (0 = "no retry"
/// stored as NULL), headers serialize to a sanitized JSON object (dropping
/// empty names/values; empty object → NULL).
fn advanced_columns(adv: &AdvancedInput) -> (Option<i64>, Option<i64>, Option<String>) {
    let timeout_secs = adv.timeout_secs.filter(|s| (1..=3600).contains(s));
    let retries = adv
        .retries
        .filter(|r| (0..=5).contains(r))
        .filter(|&r| r > 0);
    let headers = adv
        .headers
        .as_ref()
        .map(|map| {
            let sanitized: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter(|(k, v)| !k.trim().is_empty() && !v.is_empty())
                .map(|(k, v)| (k.trim().to_string(), serde_json::Value::String(v.clone())))
                .collect();
            (!sanitized.is_empty()).then(|| serde_json::Value::Object(sanitized).to_string())
        })
        .unwrap_or(None);
    (timeout_secs, retries, headers)
}

/// Map the user-supplied additional endpoints to store rows; unknown protocol
/// strings are skipped (defaulting one to openai could collide with the
/// primary's protocol in provider_endpoints' PK).
/// The stored form of an endpoint somebody typed: an absolute URL.
///
/// The dialog is shown `display_endpoint` — the scheme stripped, `api_path`
/// folded in — and hands that back on save, so opening a provider and saving it
/// again rewrote `https://api.deepseek.com` as `api.deepseek.com` and broke
/// routing for that provider from then on. Nothing downstream puts the scheme
/// back: the gateway composes the upstream URL by concatenation
/// (`server::data::compose_upstream`) and `reqwest` refuses a relative one, so
/// every request to it fails at the transport layer while the row still reads
/// like a working provider. The invariant is kept here, at the one function
/// every writer of a provider's endpoints goes through.
///
/// The scheme is `http://` for a loopback host and `https://` otherwise. A local
/// server — Ollama on 11434, an LM Studio port — is both the common case and the
/// one where the obvious guess is wrong, and it is not a case that corrects
/// itself by trying: a TLS handshake against a plaintext listener fails before
/// anything can say why.
pub(crate) fn absolute_endpoint(raw: &str) -> String {
    let endpoint = raw.trim();
    if endpoint.contains("://") {
        return endpoint.to_string();
    }
    let local = matches!(
        crate::creds::host_of(endpoint)
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]"
    );
    format!("{}{endpoint}", if local { "http://" } else { "https://" })
}

fn input_endpoints(input: &NewProviderInput) -> Vec<kiwanod::store::ProviderEndpoint> {
    input
        .endpoints
        .iter()
        .filter_map(|e| {
            kiwanod::store::Protocol::parse_str(&e.protocol).map(|p| {
                kiwanod::store::ProviderEndpoint {
                    protocol: p,
                    base_url: absolute_endpoint(&e.endpoint),
                    api_path: None,
                }
            })
        })
        .collect()
}

pub fn add_provider(
    store: &Store,
    aux: &Aux,
    input: &NewProviderInput,
) -> Result<ProviderVm, String> {
    let now = rfc3339(unix_now());
    let id = format!(
        "{}-{}",
        slug(&input.name),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let reset_period = match input.billing_config.reset_period.as_deref() {
        Some("monthly") | Some("weekly") | Some("yearly") => {
            input.billing_config.reset_period.clone()
        }
        _ => None,
    };
    let (timeout_secs, retries, adv_headers) = input
        .advanced
        .as_ref()
        .map(advanced_columns)
        .unwrap_or((None, None, None));
    let plan_query_json = input
        .plan_query
        .as_ref()
        .filter(|v| !v.is_null())
        .map(|v| v.to_string());
    // Plan rows carry percent limits in plan_limits; the legacy
    // number+unit+reset-cycle columns are left NULL (v10 form dropped them).
    let billing = billing_to_db(&input.billing)?;
    let is_plan = billing == kiwanod::store::Billing::Subscription;
    let mut provider = Provider {
        id: id.clone(),
        name: input.name.trim().to_string(),
        // Which catalog entry this came from, if it was added from the shelf.
        // An empty string is the frontend's "nothing selected"; storing it
        // would be a provider id that names no row.
        catalog_id: input
            .catalog_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        protocol: kiwanod::store::Protocol::parse_str(&input.protocol)
            .unwrap_or(kiwanod::store::Protocol::OpenAI),
        base_url: absolute_endpoint(&input.endpoint),
        api_path: None,
        endpoints: input_endpoints(input),
        api_key: Some(input.api_key.clone()),
        // Collected by the form since it existed, and until now dropped on the
        // floor (see `NewProviderInput::model_default`). Empty is `None`: a
        // cleared box is "not set", not the empty string.
        model_default: Some(input.model_default.trim().to_string()).filter(|m| !m.is_empty()),
        billing,
        period_limit: if is_plan {
            None
        } else {
            input.billing_config.limit_value
        },
        limit_unit: if is_plan {
            None
        } else {
            normalize_limit_unit(
                input.billing_config.limit_unit.as_deref(),
                input.billing_config.limit_value.is_some(),
                &known_limit_currencies(store),
            )?
        },
        plan_query: plan_query_json,
        plan_limits: if is_plan {
            plan_limits_json(input.billing_config.plan_limits.as_ref())
        } else {
            None
        },
        // What this provider charges, when the user said. Validated here rather
        // than in the gateway: a rate that will not parse is a mistake in the
        // form, and the person who made it is the one who can fix it.
        prices: normalize_declared_prices(input.prices.as_ref(), &known_limit_currencies(store))?,
        reset_period: if is_plan { None } else { reset_period },
        timeout_secs,
        retries,
        headers: adv_headers,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    // Nobody named an entry — a hand-added provider, or the CLI — so infer it
    // from the endpoint where that is unambiguous. Worth doing at all because
    // the row's link is what prices its requests at its own rate rather than at
    // whichever entry sorts first (`link_providers` has the long version).
    let catalog_entries = catalog_snapshot(aux).entries;
    if provider.catalog_id.is_none() {
        provider.catalog_id = catalog_id_for(&catalog_entries, &provider);
    }
    store.insert_provider(&provider).map_err(e2s)?;

    // compute view fields before partially moving `provider`
    let vm_name = provider.name.clone();
    let vm_catalog_id = provider.catalog_id.clone();
    let vm_model_default = provider.model_default.clone();
    let vm_endpoint = display_endpoint(&provider);
    let vm_note = endpoint_note(&provider);
    let vm_protocol = provider.protocol.as_str().to_string();
    let vm_endpoints = vm_endpoints(&provider);
    let vm_billing = billing_to_ui(provider.billing).to_string();
    // Nothing has measured this provider yet — it was created a line ago — so
    // `None` is what the Status column starts as, and the prober fills it in.
    let vm_health = health_vm(aux, &provider, &rfc3339(unix_now() - 86_400), None);
    let vm_advanced = advanced_vm(&provider);
    let vm_prices = provider
        .prices
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let vm_currency = provider_currency(&provider, &catalog_entries);

    for agent in input.agents.iter().flatten() {
        // "Save & Enable" → becomes the primary for the chosen agents.
        store
            .upsert_strategy(agent, StrategyType::Single, None)
            .map_err(e2s)?;
        bind_as_primary(store, agent, &id)?;
    }
    Ok(ProviderVm {
        id,
        name: vm_name,
        logo_char: logo_char(&input.name),
        logo_color: palette_color(&input.name).to_string(),
        logo_border: false,
        catalog_id: vm_catalog_id,
        currency: vm_currency,
        endpoint: vm_endpoint,
        protocol: vm_protocol,
        endpoint_note: vm_note,
        endpoints: vm_endpoints,
        billing: vm_billing,
        plan_price: None,
        limit_unit: None,
        model_default: vm_model_default,
        plan_query: input.plan_query.clone(),
        plan_limits: None,
        prices: vm_prices,
        enabled: true,
        agents: input.agents.clone().unwrap_or_default(),
        // Optimistic: strategy serving is only computed by build_provider_vms;
        // the list refetch right after returns the real per-agent state.
        serving_agents: vec![],
        fallback_agents: vec![],
        is_current: input.agents.as_ref().is_some_and(|a| !a.is_empty()),
        status_badge: None,
        agents_note: input
            .agents
            .as_ref()
            .filter(|a| !a.is_empty())
            .map(|a| format!("{} agent(s)", a.len())),
        health: vm_health,
        usage: None,
        advanced: vm_advanced,
    })
}

/// Park a provider, or put it back: `enabled` decides whether this row may
/// serve, and touches no route.
///
/// This is the rung between "in the route" and "deleted". A disabled provider
/// keeps its row, its key, its prices and its usage history, and the gateway's
/// route table skips it (`router::RouteTable::load`) — so every agent bound to it
/// falls through to its next candidate exactly as if the row were gone, without
/// anything being lost. Deleting stays the way to be rid of a provider; this is
/// the way to stop using one for a while.
///
/// The old `enable_provider` did something else with the word: it promoted a
/// provider to primary everywhere it was bound. `providers use --agent` is that
/// operation, one agent at a time, and it keeps its name.
pub fn set_provider_enabled(store: &Store, id: &str, enabled: bool) -> Result<(), String> {
    let mut p = store
        .get_provider(id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider not found: {id}"))?;
    if p.enabled == enabled {
        return Ok(());
    }
    p.enabled = enabled;
    p.updated_at = rfc3339(unix_now());
    store.update_provider(&p).map_err(e2s)
}

/// Make `provider_id` the primary for `agent`: priority 0, every other binding
/// reindexed after it in the order it already had.
///
/// The full reindex is the point. Demoting only the previous primary can leave
/// two bindings claiming priority 1, and `bindings_for_agent` is
/// priority-ordered — so the ambiguity would reach the strategy engine rather
/// than stop here.
///
/// The one place that writes "this provider is now the primary". `add_provider`'s
/// Save & Enable, `update_provider` and the CLI's `providers use` /
/// `providers add --bind` all land here.
pub fn bind_as_primary(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    let mut others: Vec<String> = store
        .bindings_for_agent(agent)
        .map_err(e2s)?
        .into_iter()
        .map(|b| b.provider_id)
        .filter(|p| p != provider_id)
        .collect();
    store
        .upsert_binding(&Binding {
            agent: agent.to_string(),
            provider_id: provider_id.to_string(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .map_err(e2s)?;
    for (i, p) in others.drain(..).enumerate() {
        store
            .upsert_binding(&Binding {
                agent: agent.to_string(),
                provider_id: p,
                priority: i as i64 + 1,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .map_err(e2s)?;
    }
    Ok(())
}

/// Update provider: rewrite the providers row + rebind agents (the new set
/// becomes primary; removed ones are unbound). An empty api_key means keep
/// the existing key. Returns the refreshed VM (re-aggregated so badges and
/// notes stay consistent), read against `home` like [`build_provider_vms`].
pub fn update_provider(
    store: &Store,
    aux: &Aux,
    home: &Path,
    id: &str,
    input: &NewProviderInput,
    vars: &ShellVars,
) -> Result<ProviderVm, String> {
    let mut p = store
        .get_provider(id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider `{id}` not found"))?;

    p.name = input.name.trim().to_string();
    p.base_url = absolute_endpoint(&input.endpoint);
    p.protocol = kiwanod::store::Protocol::parse_str(&input.protocol)
        .unwrap_or(kiwanod::store::Protocol::OpenAI);
    p.endpoints = input_endpoints(input);
    // Authoritative, like `endpoints`: the form is the only caller and always
    // sends it, so clearing the box means "no default" rather than "leave it".
    // Empty stores NULL.
    p.model_default = Some(input.model_default.trim().to_string()).filter(|m| !m.is_empty());
    p.billing = billing_to_db(&input.billing)?;
    // Plan rows carry percent limits in plan_limits and NULL the legacy
    // number+unit+reset-cycle columns (v10 form); payg keeps the old shape.
    let is_plan = p.billing == kiwanod::store::Billing::Subscription;
    p.period_limit = if is_plan {
        None
    } else {
        input.billing_config.limit_value
    };
    p.limit_unit = if is_plan {
        None
    } else {
        normalize_limit_unit(
            input.billing_config.limit_unit.as_deref(),
            input.billing_config.limit_value.is_some(),
            &known_limit_currencies(store),
        )?
    };
    p.plan_limits = if is_plan {
        plan_limits_json(input.billing_config.plan_limits.as_ref())
    } else {
        None
    };
    // Declared prices: same absent-keeps semantics as `advanced` below, and a
    // present bundle is the snapshot — including an empty one, which is how a
    // provider that leaves pay-as-you-go stops carrying prices.
    if input.prices.is_some() {
        p.prices =
            normalize_declared_prices(input.prices.as_ref(), &known_limit_currencies(store))?;
    }
    p.reset_period = if is_plan {
        None
    } else {
        match input.billing_config.reset_period.as_deref() {
            Some("monthly") | Some("weekly") | Some("yearly") => {
                input.billing_config.reset_period.clone()
            }
            _ => None,
        }
    };
    if !input.api_key.trim().is_empty() {
        p.api_key = Some(input.api_key.clone());
    }
    // The catalog entry this provider was added from: absent or empty keeps
    // what is stored. The edit form has no catalog picker, and dropping the
    // link on an unrelated edit would quietly change which price it is costed
    // at — the one thing this column exists to pin down.
    if let Some(catalog_id) = input.catalog_id.as_deref().map(str::trim) {
        if !catalog_id.is_empty() {
            p.catalog_id = Some(catalog_id.to_string());
        }
    }
    // Advanced: absent = keep existing (same semantics as an empty api_key);
    // a present object is an authoritative snapshot — null fields clear values.
    if let Some(adv) = &input.advanced {
        let (timeout_secs, retries, headers) = advanced_columns(adv);
        p.timeout_secs = timeout_secs;
        p.retries = retries;
        p.headers = headers;
    }
    // Plan quota query: same absent-keeps semantics; null clears.
    if let Some(pq) = &input.plan_query {
        p.plan_query = (!pq.is_null()).then(|| pq.to_string());
    }
    p.updated_at = rfc3339(unix_now());
    store.update_provider(&p).map_err(e2s)?;

    // Bindings: absent leaves them exactly as they are, which is what an edit
    // sends. They belong to the Apps screen's agent tabs — that screen binds,
    // unbinds and sets the strategy — and this loop is not a neutral rewrite of
    // "the same set": every agent it names is promoted to *this* provider as
    // primary, and has its strategy flattened to Single. Running it on an
    // unrelated save is how editing a provider's timeout silently reordered an
    // agent's failover queue.
    if let Some(agents) = &input.agents {
        // Unbind old agents not in the new set; new ones use the same primary
        // logic as add.
        let new_set: std::collections::HashSet<&str> = agents.iter().map(String::as_str).collect();
        let old_agents: Vec<String> = store
            .bound_agents()
            .map_err(e2s)?
            .into_iter()
            .filter(|a| {
                store
                    .bindings_for_agent(a)
                    .map(|bs| bs.iter().any(|b| b.provider_id == id))
                    .unwrap_or(false)
            })
            .collect();
        for agent in &old_agents {
            if !new_set.contains(agent.as_str()) {
                store.delete_binding(agent, id).map_err(e2s)?;
            }
        }
        for agent in agents {
            store
                .upsert_strategy(agent, StrategyType::Single, None)
                .map_err(e2s)?;
            bind_as_primary(store, agent, id)?;
        }
    }

    let vms = build_provider_vms(store, aux, home, vars)?;
    vms.into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| "provider vanished after update".to_string())
}

/// Delete a provider. If it is some Agent's primary, the next-best candidate
/// in that Agent's list is promoted automatically.
pub fn delete_provider(store: &Store, id: &str) -> Result<bool, String> {
    let mut affected: Vec<String> = Vec::new();
    for a in store.bound_agents().map_err(e2s)? {
        if store.primary_provider_id(&a).map_err(e2s)?.as_deref() == Some(id) {
            affected.push(a);
        }
    }
    let deleted = store.delete_provider(id).map_err(e2s)?;
    if !deleted {
        return Ok(false);
    }
    for agent in affected {
        let remaining = store.bindings_for_agent(&agent).map_err(e2s)?;
        if let Some(next) = remaining.first() {
            let next_id = next.provider_id.clone();
            store
                .upsert_binding(&Binding {
                    agent: agent.clone(),
                    provider_id: next_id.clone(),
                    priority: 0,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(e2s)?;
            for (i, b) in remaining
                .iter()
                .filter(|b| b.provider_id != next_id)
                .enumerate()
            {
                store
                    .upsert_binding(&Binding {
                        agent: agent.clone(),
                        provider_id: b.provider_id.clone(),
                        priority: i as i64 + 1,
                        weight: b.weight,
                        win_start: b.win_start.clone(),
                        win_end: b.win_end.clone(),
                        enabled: b.enabled,
                    })
                    .map_err(e2s)?;
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::catalog::stored_key_for;
    use crate::vm::providers::build_provider_vms;
    use crate::vm::test_support::{
        catalog_aux, catalog_input, linkless_aux, live_home, no_vars, provider, store,
        stored_catalog_id,
    };
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Binding, StrategyType};

    /// Added from the CLI or by hand, the endpoint still names the entry:
    /// that is what makes the provider's own prices reachable.
    #[test]
    fn add_provider_links_the_unique_catalog_entry() {
        let s = store();
        let aux = catalog_aux();

        // A bare host where the entry names a deeper path.
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("DeepSeek", "https://api.deepseek.com"),
        )
        .unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id).as_deref(), Some("deepseek"));

        // A path the entry advertises among two — still one entry, one link.
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("KFC", "https://api.kimi.com/coding"),
        )
        .unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding")
        );

        // The other direction: the local URL carries the deeper path.
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("Bare", "https://api.bare.example/v1"),
        )
        .unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id).as_deref(), Some("bare-host"));
    }

    /// The default model the form collects survives the trip to storage and back.
    /// It used to be dropped on the floor — collected, sent, ignored — which is
    /// why reopening a provider showed an empty box however carefully it had been
    /// filled in.
    #[test]
    fn add_provider_stores_the_default_model_and_hands_it_back() {
        let s = store();
        let aux = catalog_aux();

        let mut input = catalog_input("DeepSeek", "https://api.deepseek.com");
        input.model_default = "deepseek-v4-pro".into();
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(vm.model_default.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(
            s.get_provider(&vm.id)
                .unwrap()
                .unwrap()
                .model_default
                .as_deref(),
            Some("deepseek-v4-pro"),
            "on the row, not only in the reply"
        );

        // Whitespace is not a model name, and an empty box is "not set" rather
        // than the empty string.
        let mut blank = catalog_input("Blank", "https://api.blank.example/v1");
        blank.model_default = "   ".into();
        let blank_vm = add_provider(&s, &aux, &blank).unwrap();
        assert_eq!(blank_vm.model_default, None);
        assert_eq!(
            s.get_provider(&blank_vm.id).unwrap().unwrap().model_default,
            None
        );

        // An edit is authoritative: clearing the box clears the column, the way
        // emptying the endpoint list rewrites it.
        let mut edit = catalog_input("DeepSeek", "https://api.deepseek.com");
        edit.model_default = String::new();
        update_provider(&s, &aux, live_home(&[]).path(), &vm.id, &edit, &no_vars()).unwrap();
        assert_eq!(s.get_provider(&vm.id).unwrap().unwrap().model_default, None);
    }

    /// An edit that names no agents leaves the bindings exactly as they are.
    ///
    /// Which is not the same as sending the set that is already bound: that loop
    /// promotes *this* provider to primary for every agent it names and flattens
    /// the agent's strategy to Single. Saving an unrelated field therefore used
    /// to reorder an agent's failover queue and change how it routes.
    #[test]
    fn an_edit_that_names_no_agents_leaves_the_bindings_alone() {
        let s = store();
        let aux = catalog_aux();

        // Two providers serving claude. The second one added is primary.
        let mut first = catalog_input("A", "https://a.example.com/v1");
        first.agents = Some(vec!["claude".into()]);
        let a = add_provider(&s, &aux, &first).unwrap();
        let mut second = catalog_input("B", "https://b.example.com/v1");
        second.agents = Some(vec!["claude".into()]);
        let b = add_provider(&s, &aux, &second).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(b.id.as_str())
        );

        // The agent is set up as a failover queue behind that primary.
        s.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();

        // An edit that says nothing about agents: nothing moves.
        let home = live_home(&["claude"]);
        let mut edit = catalog_input("A", "https://a.example.com/v1");
        edit.agents = None;
        update_provider(&s, &aux, home.path(), &a.id, &edit, &no_vars()).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(b.id.as_str()),
            "the edit must not promote itself over the standing primary"
        );
        assert_eq!(
            s.get_strategy("claude").unwrap().unwrap().kind,
            StrategyType::Failover,
            "and must not flatten the agent's strategy"
        );

        // Naming agents still rebinds — that is the one path that may, and the
        // reason the field is an Option rather than a Vec.
        edit.agents = Some(vec!["claude".into()]);
        update_provider(&s, &aux, home.path(), &a.id, &edit, &no_vars()).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(a.id.as_str())
        );
        assert_eq!(
            s.get_strategy("claude").unwrap().unwrap().kind,
            StrategyType::Single
        );
    }

    /// A stored key answers only for the endpoints that provider already answers
    /// on. That guard is the reason the fallback is safe to offer at all: the URL
    /// field is editable, and "use the saved key for whatever is in the box"
    /// would make the Test button a way to post a credential to any host.
    #[test]
    fn a_stored_key_covers_only_the_providers_own_endpoints() {
        let s = store();
        let aux = catalog_aux();
        let mut input = catalog_input("Kimi", "https://api.moonshot.cn");
        input.api_key = "sk-secret".into();
        let vm = add_provider(&s, &aux, &input).unwrap();

        // Its own endpoint, however it is spelled.
        for spelling in [
            "https://api.moonshot.cn",
            "api.moonshot.cn",
            "api.moonshot.cn/",
            "HTTPS://API.MOONSHOT.CN",
        ] {
            assert_eq!(
                stored_key_for(&s, &vm.id, spelling).as_deref(),
                Some("sk-secret"),
                "{spelling} is where this provider answers"
            );
        }

        // Anywhere else: nothing, so the probe goes out anonymous and reports
        // what it finds instead of leaking the key.
        assert_eq!(stored_key_for(&s, &vm.id, "https://evil.example.com"), None);
        assert_eq!(
            stored_key_for(&s, "no-such-provider", "api.moonshot.cn"),
            None
        );
    }

    /// A caller that names an entry is the authority; the inference fills in
    /// what nobody said.
    #[test]
    fn add_provider_keeps_an_explicit_catalog_id() {
        let s = store();
        let aux = catalog_aux();

        let mut input = catalog_input("DeepSeek", "https://api.deepseek.com");
        input.catalog_id = Some("kimi-for-coding".into());
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding"),
            "the caller's answer, not the endpoint's"
        );

        // Even when it names nothing: an id is a statement, not a hint to be
        // corrected into something the endpoint suggests.
        let mut input = catalog_input("Unlisted", "https://api.deepseek.com");
        input.catalog_id = Some("not-a-real-entry".into());
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("not-a-real-entry")
        );
    }

    /// Two entries on one host: the endpoint cannot say which is meant, and a
    /// guess here is a wrong price with nothing to show for it.
    #[test]
    fn add_provider_does_not_guess_between_ambiguous_entries() {
        let s = store();
        let aux = catalog_aux();
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("Ambiguous", "https://api.example.com"),
        )
        .unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id), None);
    }

    /// A genuinely custom provider — self-hosted, an aggregator the Hub does
    /// not list — has no entry to link to, and stays that way.
    #[test]
    fn add_provider_leaves_a_custom_endpoint_unlinked() {
        let s = store();
        let aux = catalog_aux();
        for endpoint in [
            "https://my-own.example.com",
            "https://api.deepseek.com.evil.com",
            "https://api.deepseek.com:8443",
        ] {
            let vm = add_provider(&s, &aux, &catalog_input("Custom", endpoint)).unwrap();
            assert_eq!(stored_catalog_id(&s, &vm.id), None, "{endpoint}");
        }
    }

    /// The provider's extra per-protocol endpoints are part of the match, the
    /// same set the shelf's `added` derives from.
    #[test]
    fn add_provider_links_from_an_extra_endpoint() {
        let s = store();
        let aux = catalog_aux();
        let mut input = catalog_input("KFC", "https://my-own.example.com");
        input.endpoints = vec![NewEndpointInput {
            protocol: "anthropic".into(),
            endpoint: "https://api.kimi.com/coding".into(),
        }];
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding")
        );
    }

    /// Disabling takes a provider out of service without taking anything away:
    /// the binding is still there, the key is still there, and the row says so.
    #[test]
    fn a_disabled_provider_keeps_everything_but_its_place_in_the_route() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("a1", "Alpha", Billing::Metered);
        p.api_key = Some("sk-keep".into());
        s.insert_provider(&p).unwrap();
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

        set_provider_enabled(&s, "a1", false).unwrap();
        let stored = s.get_provider("a1").unwrap().unwrap();
        assert!(!stored.enabled);
        assert_eq!(stored.api_key.as_deref(), Some("sk-keep"));
        assert_eq!(
            s.bindings_for_agent("claude").unwrap().len(),
            1,
            "the route is untouched: what changed is whether this row may serve"
        );
        // The row reads as disabled rather than as healthy, and its agents fall
        // through to whoever is next (nobody, here — the route is empty).
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        assert_eq!(vms.len(), 1);
        assert!(!vms[0].enabled);
        assert!(
            vms[0].serving_agents.is_empty(),
            "a parked provider serves nobody"
        );
        assert_eq!(vms[0].health.note.as_deref(), Some("Disabled"));

        // Back on, and the same route applies again.
        set_provider_enabled(&s, "a1", true).unwrap();
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        assert!(vms[0].enabled);
        assert_eq!(vms[0].serving_agents, ["claude"]);

        // Idempotent, and an unknown id is an error rather than a no-op.
        set_provider_enabled(&s, "a1", true).unwrap();
        assert!(set_provider_enabled(&s, "ghost", false).is_err());
    }

    #[test]
    fn add_provider_becomes_primary_and_demotes_prev() {
        let s = store();
        s.insert_provider(&provider("old1", "Old", Billing::Metered))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "old1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        let input = NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            name: "New Guy".into(),
            api_key: "sk-x".into(),
            endpoint: "https://api.new.example.com".into(),
            protocol: "openai".into(),
            model_default: "new-chat".into(),
            billing: "plan".into(),
            billing_config: BillingConfigInput {
                limit_value: Some(460.0),
                limit_unit: Some("requests".into()),
                reset_period: Some("monthly".into()),
                plan_limits: None,
            },
            agents: Some(vec!["codex".into()]),
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
        };
        let vm = add_provider(&s, &linkless_aux(), &input).unwrap();
        assert_eq!(
            s.primary_provider_id("codex").unwrap().as_deref(),
            Some(vm.id.as_str())
        );
        assert_eq!(vm.billing, "plan");
        let bs = s.bindings_for_agent("codex").unwrap();
        let old = bs.iter().find(|b| b.provider_id == "old1").unwrap();
        assert_eq!(old.priority, 1);
        // plan + limit → request-unit quota
        let s2_quota = serde_json::to_value(&vm).unwrap();
        assert!(s2_quota["is_current"].as_bool().unwrap());
    }

    #[test]
    fn add_provider_persists_endpoints() {
        let s = store();
        let input = NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            name: "Qianfan".into(),
            api_key: "sk-x".into(),
            endpoint: "https://qianfan.baidubce.com/v2/tokenplan/personal".into(),
            protocol: "openai".into(),
            model_default: "qianfan-code-latest".into(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: Some(vec![]),
            endpoints: vec![
                NewEndpointInput {
                    protocol: "anthropic".into(),
                    endpoint: "  https://qianfan.baidubce.com/anthropic/coding  ".into(),
                },
                // unknown protocol → skipped, not defaulted (PK clash guard)
                NewEndpointInput {
                    protocol: "xml".into(),
                    endpoint: "https://x.example.com".into(),
                },
            ],
            advanced: None,
            plan_query: None,
        };
        add_provider(&s, &linkless_aux(), &input).unwrap();

        let rows = s.list_providers().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].endpoints.len(), 1);
        assert_eq!(
            rows[0].endpoints[0].base_url,
            "https://qianfan.baidubce.com/anthropic/coding"
        );
        assert_eq!(
            rows[0].endpoints[0].protocol,
            kiwanod::store::Protocol::Anthropic
        );
    }

    #[test]
    fn update_provider_rebinds_and_keeps_key_when_blank() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "P One", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("p2", "P Two", Billing::Metered))
            .unwrap();
        for id in ["p1", "p2"] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: id.into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "p2".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        let input = NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            name: "P One Renamed".into(),
            api_key: "".into(), // blank = keep existing key
            endpoint: "https://p1.example.com/v2".into(),
            protocol: "openai".into(),
            model_default: String::new(),
            billing: "unl".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: Some(vec!["codex".into()]), // rebind: claude dropped
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
        };
        let vm = update_provider(
            &s,
            &aux,
            live_home(&["codex"]).path(),
            "p1",
            &input,
            &no_vars(),
        )
        .unwrap();
        assert_eq!(vm.name, "P One Renamed");
        assert_eq!(vm.billing, "unl");

        let p = s.get_provider("p1").unwrap().unwrap();
        assert_eq!(p.api_key.as_deref(), Some("sk-test")); // kept
        assert_eq!(
            s.primary_provider_id("codex").unwrap().as_deref(),
            Some("p1")
        );
        // claude binding removed; claude's primary falls back to p2
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some("p2")
        );
        assert!(!s
            .bindings_for_agent("claude")
            .unwrap()
            .iter()
            .any(|b| b.provider_id == "p1"));
    }

    #[test]
    fn delete_provider_promotes_next_candidate() {
        let s = store();
        s.insert_provider(&provider("main", "Main", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("backup", "Backup", Billing::Metered))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "main".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "backup".into(),
            priority: 1,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        assert!(delete_provider(&s, "main").unwrap());
        assert_eq!(s.get_provider("main").unwrap(), None);
        // backup promoted to primary
        assert_eq!(
            s.primary_provider_id("codex").unwrap().as_deref(),
            Some("backup")
        );
        let bs = s.bindings_for_agent("codex").unwrap();
        assert_eq!(bs.len(), 1);
        assert_eq!(bs[0].priority, 0);
    }

    #[test]
    fn delete_unknown_provider_is_noop() {
        let s = store();
        assert!(!delete_provider(&s, "nope").unwrap());
    }
}
