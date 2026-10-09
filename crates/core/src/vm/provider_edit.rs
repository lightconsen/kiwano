//! Provider write paths: the inputs the UI sends, their normalization, and the
//! mutations that each end in an admin `/reload`.

use crate::detect::ShellVars;
use crate::vm::providers::{build_provider_vms, ProviderVm};
use crate::vm::{mint_provider_id, Aux};
use kiwano_api::providers::ProviderPatch;
use kiwanod::api::providers as daemon;
use kiwanod::api::providers_add as daemon_add;
use kiwanod::store::Store;

use std::path::Path;

/// Add a provider — served by the daemon (`kiwanod::api::providers_add::add_provider`).
///
/// This wrapper mints the id, which is what makes the signature every caller
/// knows stay put: **this call is a fresh intent**. The idempotent path is the
/// API's, where the caller sends the id it already minted (`migrate.local.md`
/// §6.1).
pub fn add_provider(store: &Store, input: &NewProviderInput) -> Result<ProviderVm, String> {
    let id = mint_provider_id(&input.name);
    daemon_add::add_provider(store, &id, input).map_err(|e| e.to_string())
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
///
/// Served by the daemon (`kiwanod::api::providers::set_provider_enabled`); this
/// wrapper keeps the signature every caller knows.
// The add's helpers moved to the daemon with it (`migrate.local.md` §10.13) —
// the same move-and-re-export the types made earlier, so the paths these were
// called from keep resolving. `update_provider` still uses them directly.
pub use kiwanod::api::limits::{
    known_limit_currencies, normalize_declared_prices, normalize_limit_unit,
};
pub use kiwanod::api::providers_add::{
    absolute_endpoint, advanced_columns, input_endpoints, plan_limits_json,
};

// The form's request types moved to `kiwano-api` (`migrate.local.md` §10.13):
// the daemon serves the command they belong to. Re-exported so the `vm::` paths
// that name them still resolve.
pub use kiwano_api::providers::{
    AdvancedInput, BillingConfigInput, NewEndpointInput, NewProviderInput, PlanLimitsInput,
};
pub use kiwanod::api::views::{
    advanced_vm, billing_to_db, billing_to_ui, display_endpoint, endpoint_note, provider_currency,
    vm_endpoints,
};

pub fn set_provider_enabled(store: &Store, id: &str, enabled: bool) -> Result<(), String> {
    daemon::set_provider_enabled(store, id, enabled).map_err(|e| e.to_string())
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
    daemon_add::bind_as_primary(store, agent, provider_id).map_err(|e| e.to_string())
}

/// Update provider: rewrite the providers row + rebind agents (the new set
/// becomes primary; removed ones are unbound). An empty api_key means keep
/// the existing key. Returns the refreshed VM (re-aggregated so badges and
/// notes stay consistent), read against `home` like [`build_provider_vms`].
/// Update a provider — the **write** is the daemon's
/// (`kiwanod::api::providers_add::update_provider`), and the **view** is this
/// side's. That split is `migrate.local.md` §5's fifth constraint: the
/// aggregation layer stays on the client, because it is the half that reads
/// *this machine* (which agents actually route here).
pub fn update_provider(
    store: &Store,
    aux: &Aux,
    home: &Path,
    id: &str,
    patch: &ProviderPatch,
    vars: &ShellVars,
) -> Result<ProviderVm, String> {
    kiwanod::api::providers_add::update_provider(store, id, patch).map_err(|e| e.to_string())?;
    let vms = build_provider_vms(store, aux, home, vars)?;
    vms.into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| "provider vanished after update".to_string())
}

/// Delete a provider. If it is some Agent's primary, the next-best candidate
/// in that Agent's list is promoted automatically.
///
/// The body lives in `kiwanod::api::providers` — the daemon serves this
/// (`migrate.local.md` §7 batch 1), and the promotion above is its work now.
pub fn delete_provider(store: &Store, id: &str) -> Result<bool, String> {
    daemon::delete_provider(store, id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::catalog::stored_key_for;
    use crate::vm::providers::build_provider_vms;
    use crate::vm::test_support::{
        catalog_input, catalog_store, full_patch, linkless_aux, live_home, no_vars, provider,
        store, stored_catalog_id,
    };
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Binding, StrategyType};

    /// Added from the CLI or by hand, the endpoint still names the entry:
    /// that is what makes the provider's own prices reachable.
    #[test]
    fn add_provider_links_the_unique_catalog_entry() {
        let s = catalog_store();

        // A bare host where the entry names a deeper path.
        let vm = add_provider(&s, &catalog_input("DeepSeek", "https://api.deepseek.com")).unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id).as_deref(), Some("deepseek"));

        // A path the entry advertises among two — still one entry, one link.
        let vm = add_provider(&s, &catalog_input("KFC", "https://api.kimi.com/coding")).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding")
        );

        // The other direction: the local URL carries the deeper path.
        let vm = add_provider(&s, &catalog_input("Bare", "https://api.bare.example/v1")).unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id).as_deref(), Some("bare-host"));
    }

    /// The default model the form collects survives the trip to storage and back.
    /// It used to be dropped on the floor — collected, sent, ignored — which is
    /// why reopening a provider showed an empty box however carefully it had been
    /// filled in.
    #[test]
    fn add_provider_stores_the_default_model_and_hands_it_back() {
        let s = catalog_store();
        let aux = linkless_aux();

        let mut input = catalog_input("DeepSeek", "https://api.deepseek.com");
        input.model_default = "deepseek-v4-pro".into();
        let vm = add_provider(&s, &input).unwrap();
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
        let blank_vm = add_provider(&s, &blank).unwrap();
        assert_eq!(blank_vm.model_default, None);
        assert_eq!(
            s.get_provider(&blank_vm.id).unwrap().unwrap().model_default,
            None
        );

        // An edit is authoritative: clearing the box clears the column, the way
        // emptying the endpoint list rewrites it.
        let mut edit = full_patch(catalog_input("DeepSeek", "https://api.deepseek.com"));
        edit.model_default = Some(String::new());
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
        let s = catalog_store();
        let aux = linkless_aux();

        // Two providers serving claude. The second one added is primary.
        let mut first = catalog_input("A", "https://a.example.com/v1");
        first.agents = Some(vec!["claude".into()]);
        let a = add_provider(&s, &first).unwrap();
        let mut second = catalog_input("B", "https://b.example.com/v1");
        second.agents = Some(vec!["claude".into()]);
        let b = add_provider(&s, &second).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(b.id.as_str())
        );

        // The agent is set up as a failover queue behind that primary.
        s.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();

        // An edit that says nothing about agents: nothing moves.
        let home = live_home(&["claude"]);
        let mut edit = full_patch(catalog_input("A", "https://a.example.com/v1"));
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
        let s = catalog_store();
        let mut input = catalog_input("Kimi", "https://api.moonshot.cn");
        input.api_key = "sk-secret".into();
        let vm = add_provider(&s, &input).unwrap();

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
        let s = catalog_store();

        let mut input = catalog_input("DeepSeek", "https://api.deepseek.com");
        input.catalog_id = Some("kimi-for-coding".into());
        let vm = add_provider(&s, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding"),
            "the caller's answer, not the endpoint's"
        );

        // Even when it names nothing: an id is a statement, not a hint to be
        // corrected into something the endpoint suggests.
        let mut input = catalog_input("Unlisted", "https://api.deepseek.com");
        input.catalog_id = Some("not-a-real-entry".into());
        let vm = add_provider(&s, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("not-a-real-entry")
        );
    }

    /// Two entries on one host: the endpoint cannot say which is meant, and a
    /// guess here is a wrong price with nothing to show for it.
    #[test]
    fn add_provider_does_not_guess_between_ambiguous_entries() {
        let s = catalog_store();
        let vm = add_provider(&s, &catalog_input("Ambiguous", "https://api.example.com")).unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id), None);
    }

    /// A genuinely custom provider — self-hosted, an aggregator the Hub does
    /// not list — has no entry to link to, and stays that way.
    #[test]
    fn add_provider_leaves_a_custom_endpoint_unlinked() {
        let s = catalog_store();
        for endpoint in [
            "https://my-own.example.com",
            "https://api.deepseek.com.evil.com",
            "https://api.deepseek.com:8443",
        ] {
            let vm = add_provider(&s, &catalog_input("Custom", endpoint)).unwrap();
            assert_eq!(stored_catalog_id(&s, &vm.id), None, "{endpoint}");
        }
    }

    /// The provider's extra per-protocol endpoints are part of the match, the
    /// same set the shelf's `added` derives from.
    #[test]
    fn add_provider_links_from_an_extra_endpoint() {
        let s = catalog_store();
        let mut input = catalog_input("KFC", "https://my-own.example.com");
        input.endpoints = vec![NewEndpointInput {
            protocol: "anthropic".into(),
            endpoint: "https://api.kimi.com/coding".into(),
        }];
        let vm = add_provider(&s, &input).unwrap();
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
        let vm = add_provider(&s, &input).unwrap();
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
        add_provider(&s, &input).unwrap();

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
            &full_patch(input),
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
