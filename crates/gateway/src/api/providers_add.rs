//! Adding a provider: the form's normalization, the idempotent insert, and the
//! primary binding.
//!
//! Moved here from `kiwano_core::vm::provider_edit` when the daemon took over
//! `add_provider` (`migrate.local.md` §10.13).
//!
//! # The id is the caller's
//!
//! Nothing else in the request identifies an add — the name is a label, not an
//! id — and the daemon would otherwise have to mint a random one, which is the
//! one case §6.1 reserved for an operation identity. The identity *is* the id:
//! the client mints it (`kiwano_api::ids::mint_provider_id`), and an id that is
//! already in the table *is* this operation's result — so a retried add answers
//! with the provider that exists and writes nothing, exactly the way a custom
//! agent's create does (`api::agents::create_custom_agent`).
//!
//! What did **not** move: `update_provider` (it rebuilds the whole list with the
//! per-machine view, which is batch 3's subject) and the live list itself. The
//! display rules it shares with this add moved to [`super::views`].

use crate::store::{Binding, Provider, Store, StrategyType};
use kiwano_api::error::ApiError;
use kiwano_api::logo::{logo_char, palette_color};
use kiwano_api::providers::{
    AdvancedInput, HealthVm, NewEndpointInput, NewProviderInput, PlanLimitsInput, ProviderPatch,
    ProviderRefVm, ProviderVm,
};

use super::catalog::{catalog_id_for, catalog_snapshot};
use super::limits::{known_limit_currencies, normalize_declared_prices, normalize_limit_unit};
use super::views::{
    advanced_vm, billing_to_db, billing_to_ui, display_endpoint, endpoint_note, provider_currency,
    vm_endpoints,
};

/// Serialize the percent limits into the `providers.plan_limits` JSON shape.
/// Non-positive / absent percents are dropped; an empty object reads as NULL.
pub fn plan_limits_json(input: Option<&PlanLimitsInput>) -> Option<String> {
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

/// Map the advanced input to the three store columns: timeout clamps to
/// 1..=3600 (else unset = gateway defaults), retries to 0..=5 (0 = "no retry"
/// stored as NULL), headers serialize to a sanitized JSON object (dropping
/// empty names/values; empty object → NULL).
pub fn advanced_columns(adv: &AdvancedInput) -> (Option<i64>, Option<i64>, Option<String>) {
    (
        clamped_timeout(adv.timeout_secs),
        clamped_retries(adv.retries),
        adv.headers.as_ref().and_then(sanitized_headers),
    )
}

/// The three column rules, one each — because an edit patch applies them **per
/// field** rather than to a whole object, and a second copy of a clamp is a
/// second place for the range to drift.
fn clamped_timeout(secs: Option<i64>) -> Option<i64> {
    secs.filter(|s| (1..=3600).contains(s))
}

fn clamped_retries(retries: Option<i64>) -> Option<i64> {
    retries.filter(|r| (0..=5).contains(r)).filter(|&r| r > 0)
}

fn sanitized_headers(map: &std::collections::BTreeMap<String, String>) -> Option<String> {
    let sanitized: serde_json::Map<String, serde_json::Value> = map
        .iter()
        .filter(|(k, v)| !k.trim().is_empty() && !v.is_empty())
        .map(|(k, v)| (k.trim().to_string(), serde_json::Value::String(v.clone())))
        .collect();
    (!sanitized.is_empty()).then(|| serde_json::Value::Object(sanitized).to_string())
}

/// An endpoint the gateway can actually route to: keep a URL as given, and
/// add the scheme a bare host or `host:port` needs. A loopback address is the
/// local machine's own service, so `http` — anything else is `https`.
pub fn absolute_endpoint(raw: &str) -> String {
    let endpoint = raw.trim();
    if endpoint.contains("://") {
        return endpoint.to_string();
    }
    let host = endpoint
        .trim_end_matches('/')
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]");
    format!("{}{endpoint}", if local { "http://" } else { "https://" })
}

/// The wire a provider is declared to serve, from what the caller sent.
///
/// Refused rather than ignored when it is given for a provider that is not
/// OpenAI: accepting it would store a restriction that nothing consults, and a
/// user who typed `--openai-wire chat` on an Anthropic provider would have no way
/// to find out it meant nothing. An unrecognised value is refused for the same
/// reason — silently reading it as `both` would turn a typo into "no restriction".
pub(crate) fn resolve_openai_wire(
    raw: Option<&str>,
    protocol: crate::store::Protocol,
) -> Result<crate::store::OpenAiWire, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(crate::store::OpenAiWire::Both);
    };
    if protocol != crate::store::Protocol::OpenAI {
        return Err(format!(
            "`openai_wire` describes which OpenAI wire an endpoint serves, and this provider is \
             `{}` — leave it unset",
            protocol.as_str()
        ));
    }
    crate::store::OpenAiWire::parse_str(raw)
        .ok_or_else(|| format!("`{raw}` is not an OpenAI wire: expected chat, responses or both"))
}

pub fn input_endpoints(input: &NewProviderInput) -> Vec<crate::store::ProviderEndpoint> {
    endpoints_from(&input.endpoints)
}

/// The same mapping for a caller that has the list on its own — an edit patch,
/// which carries endpoints only when it means to replace them.
pub fn endpoints_from(input: &[NewEndpointInput]) -> Vec<crate::store::ProviderEndpoint> {
    input
        .iter()
        .filter_map(|e| {
            crate::store::Protocol::parse_str(&e.protocol).map(|p| crate::store::ProviderEndpoint {
                protocol: p,
                base_url: absolute_endpoint(&e.endpoint),
                api_path: None,
            })
        })
        .collect()
}

/// A `ProviderVm` for a row that is already there — the answer a replay gets.
/// The view is built from the same pieces as the one a fresh insert returns,
/// which is what makes the two paths agree about what "the provider" looks like.
fn provider_vm(
    provider: &Provider,
    catalog_entries: &[super::catalog::CatalogEntryVm],
    input: &NewProviderInput,
) -> ProviderVm {
    let vm_health = HealthVm {
        state: "idle".into(),
        latency_ms: None,
        note: None,
        source: None,
        checked_at: None,
        error: None,
    };
    ProviderVm {
        id: provider.id.clone(),
        name: provider.name.clone(),
        logo_char: logo_char(&provider.name),
        logo_color: palette_color(&provider.name).to_string(),
        logo_border: false,
        catalog_id: provider.catalog_id.clone(),
        currency: provider_currency(provider, catalog_entries),
        endpoint: display_endpoint(provider),
        protocol: provider.protocol.as_str().to_string(),
        openai_wire: (provider.openai_wire != crate::store::OpenAiWire::Both)
            .then(|| provider.openai_wire.as_str().to_string()),
        endpoint_note: endpoint_note(provider),
        endpoints: vm_endpoints(provider),
        billing: billing_to_ui(provider.billing).to_string(),
        plan_price: None,
        limit_unit: None,
        model_default: provider.model_default.clone(),
        plan_query: input.plan_query.clone(),
        plan_limits: None,
        prices: provider
            .prices
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        enabled: provider.enabled,
        agents: input.agents.clone().unwrap_or_default(),
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
        advanced: advanced_vm(provider),
    }
}

/// Apply an edit field by field: what the patch names is set, what it does not
/// is left alone (`migrate.local.md` §10.38).
///
/// Much of this reads as it did because it was **already half a patch** —
/// `api_key`, `catalog_id`, `advanced`, `plan_query` and `prices` kept their
/// stored values when absent. What changed is that `name`, `endpoint`,
/// `protocol`, `model_default`, the endpoint list and the billing block now do
/// too, and that is what removes the client's need to read the row first.
pub fn update_provider(store: &Store, id: &str, patch: &ProviderPatch) -> Result<(), ApiError> {
    let mut p = store
        .get_provider(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("provider `{id}` not found")))?;

    if let Some(name) = &patch.name {
        p.name = name.trim().to_string();
    }
    if let Some(endpoint) = &patch.endpoint {
        p.base_url = absolute_endpoint(endpoint);
    }
    if let Some(protocol) = &patch.protocol {
        p.protocol =
            crate::store::Protocol::parse_str(protocol).unwrap_or(crate::store::Protocol::OpenAI);
    }
    if let Some(wire) = &patch.openai_wire {
        // Resolved against the protocol this row will have *after* the patch:
        // one call that moves a provider to anthropic and sets a wire is a
        // contradiction, and it is refused as one rather than stored half-applied.
        p.openai_wire = resolve_openai_wire(Some(wire), p.protocol).map_err(ApiError::invalid)?;
    }
    if let Some(endpoints) = &patch.endpoints {
        p.endpoints = endpoints_from(endpoints);
    }
    // Empty clears the column rather than storing a blank — the patch's way of
    // saying "no default", which for the form's always-present field was what
    // an empty box meant.
    if let Some(model_default) = &patch.model_default {
        p.model_default = Some(model_default.trim().to_string()).filter(|m| !m.is_empty());
    }

    if let Some(billing) = &patch.billing {
        p.billing = billing_to_db(billing).map_err(ApiError::invalid)?;
    }
    if let Some(cfg) = &patch.billing_config {
        if let Some(limit) = cfg.limit_value {
            p.period_limit = Some(limit);
        }
        if let Some(unit) = &cfg.limit_unit {
            p.limit_unit = if unit.trim().is_empty() {
                None
            } else {
                normalize_limit_unit(Some(unit.as_str()), true, &known_limit_currencies(store))
                    .map_err(ApiError::invalid)?
            };
        }
        if let Some(reset) = &cfg.reset_period {
            p.reset_period =
                matches!(reset.as_str(), "monthly" | "weekly" | "yearly").then(|| reset.clone());
        }
        if let Some(limits) = &cfg.plan_limits {
            // Window by window: naming one keeps the other, which is exactly
            // what `--plan-limit-5h` on its own means. This is the merge the CLI
            // used to do against a row it had read back.
            let stored = stored_plan_limits(&p);
            p.plan_limits = plan_limits_json(Some(&PlanLimitsInput {
                five_hour: limits
                    .five_hour
                    .or_else(|| stored.as_ref().and_then(|s| s.five_hour)),
                weekly: limits
                    .weekly
                    .or_else(|| stored.as_ref().and_then(|s| s.weekly)),
            }));
        }
    }
    // The mode decides *which* columns carry the limits (the v10 form): a plan
    // row has no number+unit+reset cycle, and a pay-as-you-go row has no
    // percent ceilings. Applied after the patch, so that changing the mode on
    // its own still moves them and a stale ceiling cannot survive the switch.
    if p.billing == crate::store::Billing::Subscription {
        p.period_limit = None;
        p.limit_unit = None;
        p.reset_period = None;
    } else {
        p.plan_limits = None;
    }

    // Declared prices: absent keeps, and a present bundle is the snapshot —
    // including an empty one, which is how a provider stops carrying prices.
    if let Some(prices) = &patch.prices {
        p.prices = normalize_declared_prices(Some(prices), &known_limit_currencies(store))
            .map_err(ApiError::invalid)?;
    }
    if let Some(key) = &patch.api_key {
        if !key.trim().is_empty() {
            p.api_key = Some(key.clone());
        }
    }
    // The catalog entry this provider was added from: absent or empty keeps
    // what is stored. Dropping the link on an unrelated edit would quietly
    // change which price it is costed at — the one thing this column pins down.
    if let Some(catalog_id) = patch.catalog_id.as_deref().map(str::trim) {
        if !catalog_id.is_empty() {
            p.catalog_id = Some(catalog_id.to_string());
        }
    }
    // Advanced forwarding: each field on its own, so naming a timeout leaves
    // the retry count and the headers where they were.
    if let Some(adv) = &patch.advanced {
        if let Some(timeout) = adv.timeout_secs {
            p.timeout_secs = clamped_timeout(Some(timeout));
        }
        if let Some(retries) = adv.retries {
            p.retries = clamped_retries(Some(retries));
        }
        if let Some(headers) = &adv.headers {
            p.headers = sanitized_headers(headers);
        }
    }
    // Plan quota query: absent keeps; null clears; an object replaces.
    if let Some(pq) = &patch.plan_query {
        p.plan_query = pq.as_ref().map(|v| v.to_string());
    }
    p.updated_at = crate::store::now_rfc3339();
    store.update_provider(&p).map_err(ApiError::failed)?;

    // Bindings: absent leaves them exactly as they are, which is what an edit
    // sends. They belong to the Apps screen's agent tabs — that screen binds,
    // unbinds and sets the strategy — and this loop is not a neutral rewrite of
    // "the same set": every agent it names is promoted to *this* provider as
    // primary, and has its strategy flattened to Single. Running it on an
    // unrelated save is how editing a provider's timeout silently reordered an
    // agent's failover queue.
    if let Some(agents) = &patch.agents {
        // Unbind old agents not in the new set; new ones use the same primary
        // logic as add.
        let new_set: std::collections::HashSet<&str> = agents.iter().map(String::as_str).collect();
        let old_agents: Vec<String> = store
            .bound_agents()
            .map_err(ApiError::failed)?
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
                store.delete_binding(agent, id).map_err(ApiError::failed)?;
            }
        }
        for agent in agents {
            store
                .upsert_strategy(agent, StrategyType::Single, None)
                .map_err(ApiError::failed)?;
            bind_as_primary(store, agent, id)?;
        }
    }
    Ok(())
}

/// A provider named by id, with the two fields a flag-driven client needs
/// before it can edit one (`migrate.local.md` §10.38): what to call it, and
/// which limits apply to it.
///
/// Deliberately not the row. The client asked to be able to say only what
/// changed; what it still cannot avoid knowing is the **billing mode**, because
/// two of the rules it states are phrased in terms of it — whether
/// `--plan-limit-5h` is legal here at all, and whether a ceiling is a
/// percentage or a spend. The rest of the row stays where the patch leaves it.
pub fn provider_ref(store: &Store, id: &str) -> Result<ProviderRefVm, ApiError> {
    let p = store
        .get_provider(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("provider `{id}` not found")))?;
    Ok(ProviderRefVm {
        id: p.id,
        name: p.name,
        billing: billing_to_ui(p.billing).to_string(),
    })
}

/// The stored percent ceilings, read back out of their column.
///
/// Possible at all only because `PlanLimitsInput`'s fields default: the column
/// is written with a window omitted rather than nulled, so `{"five_hour":80}`
/// is what a provider with one ceiling carries.
fn stored_plan_limits(provider: &Provider) -> Option<PlanLimitsInput> {
    serde_json::from_str(provider.plan_limits.as_deref()?).ok()
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
pub fn bind_as_primary(store: &Store, agent: &str, provider_id: &str) -> Result<(), ApiError> {
    let mut others: Vec<String> = store
        .bindings_for_agent(agent)
        .map_err(ApiError::failed)?
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
        .map_err(ApiError::failed)?;
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
            .map_err(ApiError::failed)?;
    }
    Ok(())
}

pub fn add_provider(
    store: &Store,
    id: &str,
    input: &NewProviderInput,
) -> Result<ProviderVm, ApiError> {
    // The id is the caller's, so it is the operation's identity: an id that is
    // already in the table *is* this request's result, and a replay answers
    // with the provider that exists rather than a second row. Nothing else in
    // the body could stand in for it — the name is a label, not an id
    // (`migrate.local.md` §6.1).
    if let Some(existing) = store.get_provider(id).map_err(ApiError::failed)? {
        return Ok(provider_vm(
            &existing,
            &catalog_snapshot(store).entries,
            input,
        ));
    }
    let now = crate::store::now_rfc3339();
    let protocol = crate::store::Protocol::parse_str(&input.protocol)
        .unwrap_or(crate::store::Protocol::OpenAI);
    let openai_wire =
        resolve_openai_wire(input.openai_wire.as_deref(), protocol).map_err(ApiError::invalid)?;
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
    let billing = billing_to_db(&input.billing).map_err(ApiError::invalid)?;
    let is_plan = billing == crate::store::Billing::Subscription;
    let mut provider = Provider {
        id: id.to_string(),
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
        protocol,
        base_url: absolute_endpoint(&input.endpoint),
        openai_wire,
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
            )
            .map_err(ApiError::invalid)?
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
        prices: normalize_declared_prices(input.prices.as_ref(), &known_limit_currencies(store))
            .map_err(ApiError::invalid)?,
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
    let catalog_entries = catalog_snapshot(store).entries;
    if provider.catalog_id.is_none() {
        provider.catalog_id = catalog_id_for(&catalog_entries, &provider);
    }
    store.insert_provider(&provider).map_err(ApiError::failed)?;

    // compute view fields before partially moving `provider`
    let vm_name = provider.name.clone();
    // The logo is derived from the stored name too — see where it is used.
    let vm_logo_char = logo_char(&vm_name);
    let vm_logo_color = palette_color(&vm_name).to_string();
    let vm_catalog_id = provider.catalog_id.clone();
    let vm_model_default = provider.model_default.clone();
    let vm_endpoint = display_endpoint(&provider);
    let vm_note = endpoint_note(&provider);
    let vm_protocol = provider.protocol.as_str().to_string();
    let vm_endpoints = vm_endpoints(&provider);
    let vm_billing = billing_to_ui(provider.billing).to_string();
    // Nothing has measured this provider yet — it was created a line ago — so
    // `idle` is what the Status column starts as, and the prober fills it in.
    let vm_health = HealthVm {
        state: "idle".into(),
        latency_ms: None,
        note: None,
        source: None,
        checked_at: None,
        error: None,
    };
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
            .map_err(ApiError::failed)?;
        bind_as_primary(store, agent, id)?;
    }
    Ok(ProviderVm {
        id: id.to_string(),
        name: vm_name,
        logo_char: vm_logo_char,
        logo_color: vm_logo_color,
        logo_border: false,
        catalog_id: vm_catalog_id,
        currency: vm_currency,
        endpoint: vm_endpoint,
        protocol: vm_protocol,
        openai_wire: (provider.openai_wire != crate::store::OpenAiWire::Both)
            .then(|| provider.openai_wire.as_str().to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Billing, Protocol};
    use kiwano_api::providers::BillingConfigInput;

    /// The smallest input `add_provider` takes, so a test can vary one field and
    /// leave everything else alone.
    fn input(name: &str, endpoint: &str) -> NewProviderInput {
        NewProviderInput {
            name: name.into(),
            api_key: "sk-test".into(),
            endpoint: endpoint.into(),
            protocol: "openai".into(),
            openai_wire: Default::default(),
            model_default: String::new(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: None,
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
            prices: None,
            catalog_id: None,
        }
    }

    /// A patch sets what it names and leaves everything else where it was.
    ///
    /// The property the whole shape exists for (`migrate.local.md` §10.38): a
    /// client without a form says only what changed. Under the authoritative
    /// rewrite this replaced, every one of these fields would have been
    /// overwritten by whatever the caller happened to send — which is why such
    /// a client had to read the row back first, and why it could not edit a
    /// provider on another machine at all.
    #[test]
    fn a_patch_touches_only_the_fields_it_names() {
        let store = Store::open_in_memory().unwrap();
        let id = kiwano_api::ids::mint_provider_id("Original");
        let mut create = input("Original", "https://one.example");
        create.api_key = "sk-original".into();
        create.protocol = "anthropic".into();
        create.model_default = "m-one".into();
        create.advanced = Some(AdvancedInput {
            timeout_secs: Some(30),
            retries: Some(2),
            headers: Some(
                [("X-Trace".to_string(), "on".to_string())]
                    .into_iter()
                    .collect(),
            ),
        });
        create.billing_config = BillingConfigInput {
            limit_value: Some(50.0),
            limit_unit: Some("USD".into()),
            reset_period: Some("monthly".into()),
            plan_limits: None,
        };
        add_provider(&store, &id, &create).unwrap();
        let before = store.get_provider(&id).unwrap().unwrap();

        // Name one field. Nothing else.
        update_provider(
            &store,
            &id,
            &ProviderPatch {
                name: Some("Renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let after = store.get_provider(&id).unwrap().unwrap();
        assert_eq!(after.name, "Renamed");
        assert_eq!(
            (
                after.base_url.as_str(),
                after.protocol,
                after.api_key.as_deref(),
                after.model_default.as_deref(),
                after.timeout_secs,
                after.retries,
                after.headers.as_deref(),
                after.period_limit,
                after.limit_unit.as_deref(),
                after.reset_period.as_deref(),
            ),
            (
                before.base_url.as_str(),
                before.protocol,
                before.api_key.as_deref(),
                before.model_default.as_deref(),
                before.timeout_secs,
                before.retries,
                before.headers.as_deref(),
                before.period_limit,
                before.limit_unit.as_deref(),
                before.reset_period.as_deref(),
            ),
            "an edit that names one field must not move any other"
        );
    }

    /// §6.1: **a replayed add mints one provider.**
    ///
    /// The id is the caller's, so an id that is already in the table *is* this
    /// operation's result — nothing else in the request could identify it (the
    /// name is a label). A second delivery answers with the provider that exists
    /// and writes nothing: no second row, no second binding, and the agents list
    /// it reports is its own, not the retry's.
    ///
    /// This fails against an insert that does not look first — the shape this
    /// had when the app wrote the row directly — because nothing in the old code
    /// ever had to ask whether the id was already there.
    #[test]
    fn a_replayed_add_mints_one_provider() {
        let store = Store::open_in_memory().unwrap();
        let id = kiwano_api::ids::mint_provider_id("DeepSeek");

        let first =
            add_provider(&store, &id, &input("DeepSeek", "https://api.deepseek.com")).unwrap();
        let again =
            add_provider(&store, &id, &input("DeepSeek", "https://api.deepseek.com")).unwrap();

        assert_eq!(first.id, again.id);
        assert_eq!(store.list_providers().unwrap().len(), 1, "one row, not two");
        assert!(
            store.get_provider(&id).unwrap().is_some(),
            "and it is the same one"
        );
    }

    /// The refusals carry the kind the status is picked from, and the sentences
    /// the UI shows — `billing_to_db`'s total vocabulary included, which is why
    /// the rule lives in `api::views` rather than being re-stated here.
    #[test]
    fn the_refusals_are_kinds_and_keep_their_wording() {
        let store = Store::open_in_memory().unwrap();
        let id = kiwano_api::ids::mint_provider_id("X");

        let unknown = add_provider(&store, &id, &{
            let mut i = input("X", "https://x.example");
            i.billing = "per-token".into();
            i
        })
        .unwrap_err();
        assert_eq!(unknown.kind(), kiwano_api::error::ApiErrorKind::Invalid);
        assert_eq!(
            unknown.message(),
            "unknown billing \"per-token\" (expected plan|payg|unl)"
        );
        assert!(
            store.get_provider(&id).unwrap().is_none(),
            "a refusal writes nothing"
        );

        // A billing the catalog *knows*, and still refused on this row: it
        // charges two ways, and a local provider holds one.
        let both = add_provider(&store, &id, &{
            let mut i = input("X", "https://x.example");
            i.billing = "both".into();
            i
        })
        .unwrap_err();
        assert_eq!(both.kind(), kiwano_api::error::ApiErrorKind::Invalid);
        assert_eq!(
            both.message(),
            "billing \"both\" must be resolved to plan or payg before saving"
        );
    }

    /// An edit is idempotent by nature — the body *is* the resulting state — and
    /// the two halves that are easy to get wrong are asserted here: a field the
    /// form always sends is authoritative, and a field it may omit keeps what is
    /// stored.
    #[test]
    fn an_edit_replaces_what_the_form_sent_and_keeps_what_it_did_not() {
        let store = Store::open_in_memory().unwrap();
        let id = kiwano_api::ids::mint_provider_id("Original");
        let mut create = input("Original", "https://one.example");
        create.api_key = "sk-original".into();
        create.model_default = "m-one".into();
        add_provider(&store, &id, &create).unwrap();

        // The edit renames it and points it elsewhere, and says nothing about
        // the key — which must survive, because an empty key is how a caller
        // says "keep the stored one".
        let edit = ProviderPatch {
            name: Some("Renamed".into()),
            endpoint: Some("https://two.example".into()),
            api_key: Some(String::new()),
            model_default: Some(String::new()),
            ..Default::default()
        };
        update_provider(&store, &id, &edit).unwrap();

        let row = store.get_provider(&id).unwrap().unwrap();
        assert_eq!(row.name, "Renamed");
        assert_eq!(row.base_url, "https://two.example");
        assert_eq!(
            row.api_key.as_deref(),
            Some("sk-original"),
            "a blank key keeps the stored one"
        );
        assert_eq!(
            row.model_default, None,
            "but an empty model default clears it — the form always sends that field"
        );

        // Editing something that is not there is a named refusal.
        let missing = update_provider(&store, "ghost", &edit).unwrap_err();
        assert_eq!(missing.kind(), kiwano_api::error::ApiErrorKind::NotFound);
        assert_eq!(missing.message(), "provider `ghost` not found");
    }

    /// The row's logo is derived from the name that was **stored**, so the
    /// create response and every later read agree.
    ///
    /// They did not: the add response derived the glyph and colour from the raw
    /// input while the row kept the trimmed name, so a provider added as
    /// `"  Alpha  "` came back with one avatar and showed a different one from
    /// the next refresh on.
    #[test]
    fn the_logo_matches_the_name_that_was_stored() {
        let store = Store::open_in_memory().unwrap();
        let id = kiwano_api::ids::mint_provider_id("Alpha");
        let created =
            add_provider(&store, &id, &input("  Alpha  ", "https://alpha.example")).unwrap();

        assert_eq!(created.name, "Alpha", "the row keeps the trimmed name");
        assert_eq!(created.logo_char, "A");
        assert_eq!(
            created.logo_color,
            kiwano_api::logo::palette_color("Alpha"),
            "the colour is the one the stored name picks"
        );
        // And the read path agrees, which is the property that was broken.
        let stored = store.get_provider(&id).unwrap().unwrap();
        assert_eq!(stored.name, created.name);
        assert_eq!(kiwano_api::logo::logo_char(&stored.name), created.logo_char);
    }

    /// "Save & Enable": the agents the form named get this provider as their
    /// primary, and any queue they already had is re-indexed behind it — the
    /// behaviour `bind_as_primary` exists to pin.
    #[test]
    fn saving_with_agents_binds_it_as_their_primary() {
        let store = Store::open_in_memory().unwrap();
        // An existing provider already bound to `claude` — the new one should
        // take the primary slot and push it down, not replace the binding.
        store
            .insert_provider(&Provider {
                id: "p-old".into(),
                name: "Old".into(),
                catalog_id: None,
                protocol: Protocol::OpenAI,
                openai_wire: crate::store::OpenAiWire::Both,
                base_url: "http://127.0.0.1:1".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk-old".into()),
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
            })
            .unwrap();
        bind_as_primary(&store, "claude", "p-old").unwrap();

        let id = kiwano_api::ids::mint_provider_id("New");
        let mut i = input("New", "https://new.example");
        i.agents = Some(vec!["claude".into()]);
        add_provider(&store, &id, &i).unwrap();

        assert_eq!(
            store.primary_provider_id("claude").unwrap().as_deref(),
            Some(id.as_str()),
            "the new provider is the primary"
        );
        let order: Vec<(String, i64)> = store
            .bindings_for_agent("claude")
            .unwrap()
            .into_iter()
            .map(|b| (b.provider_id, b.priority))
            .collect();
        assert_eq!(
            order,
            vec![(id, 0), ("p-old".to_string(), 1)],
            "and the queue closed up behind it"
        );
    }
}
