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
    AdvancedInput, HealthVm, NewProviderInput, PlanLimitsInput, ProviderVm,
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

pub fn input_endpoints(input: &NewProviderInput) -> Vec<crate::store::ProviderEndpoint> {
    input
        .endpoints
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
        protocol: crate::store::Protocol::parse_str(&input.protocol)
            .unwrap_or(crate::store::Protocol::OpenAI),
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
