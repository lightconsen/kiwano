//! View-model layer: maps the gateway `Store` rows to the frontend contract
//! in `src/api/types.ts` (field names must match exactly — serde default
//! snake_case). The UI is mock-free; this is the single source of mapping.
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`vm::build_provider_vms`): the facade below re-exports it,
//! because `crates/cli`, `app/src-tauri` and the sibling modules in this crate
//! all still name it that way.
//!
//! `fmt` and `time` are leaves — they read no store and no setting, so any other
//! module may import them without an ordering worry. `providers` owns the
//! provider read model and the billing vocabulary; `provider_edit` owns the write
//! paths and imports back from it, never the other way round.
//!
//! The auxiliary SQLite connection lives in `crate::auxiliary` and is re-exported
//! here as `vm::Aux`, so `crate::vm::Aux` paths keep resolving.

pub mod agents;
pub mod alerts;
pub mod catalog;
pub mod dashboard;
pub mod fmt;
pub mod keys;
pub mod limits;
pub mod logs;
pub mod provider_edit;
pub mod providers;
pub mod routes;
pub mod settings;
pub mod takeover;
pub mod time;

// ── Auxiliary connection (moved to `crate::auxiliary`, re-exported here) ──

pub use crate::auxiliary::Aux;

fn e2s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ── Mutations (called from commands; each ends with an admin /reload) ──

pub fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "provider".into()
    } else {
        trimmed.to_string()
    }
}

// ── the public surface, re-exported so every `vm::x` path still resolves ──

pub use agents::{
    add_custom_agent, agent_protocols, is_builtin_agent, remove_custom_agent,
    test_provider_latency, update_custom_agent, CustomAgentVm, PromptLatencyVm, ADDITIVE_AGENTS,
    AGENTS, AGENT_PROTOCOLS,
};
pub use alerts::{check_usage_alerts, UsageAlertVm};
pub use catalog::{
    default_catalog_currency, link_providers, load_catalog, stored_key_for, CatalogBilling,
    CatalogEndpointVm, CatalogEntryVm, CatalogListVm, CatalogPriceRefVm, SyncReportVm,
};
pub use dashboard::{
    build_dashboard, build_footer_stats, AgentDistVm, BlockedProviderVm, DashboardVm,
    FilterOptionVm, FooterStatsVm, GatewayStatusVm, ProviderDistVm, TrendVm,
};
pub use fmt::fmt_tokens;
pub use keys::{add_api_key, delete_api_key, list_api_keys, ApiKeyVm};
pub use limits::{
    import_declared_prices, known_limit_currencies, normalize_declared_prices,
    normalize_limit_unit, ProviderPriceInput, ProviderPricesInput,
};
pub use logs::{
    ack_credential_finding, check_credential_finding, clear_request_logs, export_request_logs_csv,
    get_request_log, list_request_logs, RequestLogDetailVm, RequestLogExportVm, RequestLogListVm,
};
pub use provider_edit::{
    add_provider, bind_as_primary, delete_provider, set_provider_enabled, update_provider,
    AdvancedInput, BillingConfigInput, NewEndpointInput, NewProviderInput, PlanLimitsInput,
};
pub use providers::{
    billing_to_ui, build_provider_vms, HealthVm, ProviderAdvancedVm, ProviderEndpointVm,
    ProviderVm, QuotaVm, UsageVm,
};
pub use routes::{
    add_agent_binding, apply_agent_route, build_agent_routes, remove_agent_binding,
    reorder_agent_bindings, set_agent_limits, set_agent_strategy, update_agent_binding,
    AgentLimitVm, AgentRouteVm, BindingVm,
};
pub use settings::{
    build_settings, build_settings_with_home, default_hub_url, default_preferred_currency,
    default_stream_first_byte_secs, default_stream_idle_secs, default_true, ui_settings,
    update_settings, SettingsVm, TakeoverVm,
};
pub use takeover::set_agent_takeover;
pub use time::{rfc3339, unix_now};

#[cfg(test)]
pub(crate) mod test_support {
    use crate::detect::ShellVars;
    use crate::vm::limits::{ProviderPriceInput, ProviderPricesInput};
    use crate::vm::provider_edit::{BillingConfigInput, NewProviderInput};
    use crate::vm::time::{rfc3339, unix_now};
    use crate::vm::Aux;
    use kiwanod::store::UsageRecord;
    use kiwanod::store::{Billing, Provider, Store};

    /// No shell environment: these tests root every agent file at the temp home
    /// they injected, and the variables have their own tests in `takeover`.
    pub(crate) fn no_vars() -> ShellVars {
        ShellVars::new()
    }

    pub(crate) fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    /// An `Aux` with nothing cached, so nothing can be inferred from a catalog
    /// — what the tests that are not about the link want.
    pub(crate) fn linkless_aux() -> Aux {
        Aux::open_in_memory().unwrap()
    }

    /// An agent config tree carrying the gateway's placeholder key for each
    /// named agent — the live evidence `build_provider_vms` reads before a
    /// binding counts as a route. Named agents get the key in the file their
    /// takeover writes; every other agent stays dormant, as it would be with no
    /// takeover ever enabled.
    pub(crate) fn live_home(agents: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for agent in agents {
            let rel = match *agent {
                "claude" => ".claude/settings.json",
                // auth.json rather than config.toml: the Codex config is read
                // through the parsed detector, the rest by a value scan.
                "codex" => ".codex/auth.json",
                "grokbuild" => ".grok/config.toml",
                "opencode" => ".config/opencode/opencode.json",
                "openclaw" => ".openclaw/openclaw.json",
                "hermes" => ".hermes/config.yaml",
                "pi" => ".pi/agent/settings.json",
                "mimo" => ".config/mimocode/mimocode.jsonc",
                "mcode" => ".minimax/config.yaml",
                other => panic!("no agent-config fixture for {other}"),
            };
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("kw-ag-{agent}-test")).unwrap();
        }
        dir
    }

    pub(crate) fn provider(id: &str, name: &str, billing: Billing) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: kiwanod::store::Protocol::OpenAI,
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some("sk-test".into()),
            model_default: None,
            billing,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: "2026-09-07T00:00:00Z".into(),
            updated_at: "2026-09-07T00:00:00Z".into(),
        }
    }

    // ── provider ↔ catalog link ──────────────────────────────────────────

    /// A catalog covering the shapes the link has to tell apart: an entry whose
    /// endpoint names a deeper path than a user types, an entry advertising two
    /// paths, a bare host, and two entries sharing one host — which must stay
    /// unlinked, because the endpoint cannot say which of them is meant.
    pub(crate) const LINK_CATALOG: &str = r#"{"total":5,"entries":[
        {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com/anthropic"}]},
        {"id":"kimi-for-coding","name":"Kimi For Coding","tag":"official","rating":4,
         "billing":"plan","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.kimi.com/coding/v1"},
                      {"protocol":"anthropic","endpoint":"https://api.kimi.com/coding"}]},
        {"id":"bare-host","name":"Bare Host","tag":"third","rating":3,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.bare.example"}]},
        {"id":"host-one","name":"Host One","tag":"third","rating":3,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.example.com/one"}]},
        {"id":"host-two","name":"Host Two","tag":"third","rating":3,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.example.com/two"}]}
    ]}"#;

    pub(crate) fn catalog_aux() -> Aux {
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_cache(LINK_CATALOG, "2026-09-07T00:00:00Z")
            .unwrap();
        aux
    }

    /// The link as stored. The inference writes the row, so this is where the
    /// answer can be read — `ProviderVm` deliberately does not carry it.
    pub(crate) fn stored_catalog_id(s: &Store, id: &str) -> Option<String> {
        s.get_provider(id)
            .unwrap()
            .expect("provider row")
            .catalog_id
    }

    /// The smallest input `add_provider` takes, so a test can vary the endpoint
    /// and leave everything else alone.
    pub(crate) fn catalog_input(name: &str, endpoint: &str) -> NewProviderInput {
        NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
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
        }
    }

    pub(crate) fn prices(currency: &str, models: Vec<ProviderPriceInput>) -> ProviderPricesInput {
        ProviderPricesInput {
            currency: currency.into(),
            models,
        }
    }

    /// `requests` rows starting at `first_secs`, as the gateway would have
    /// written them: a usage row and its request_logs twin.
    pub(crate) fn seed_usage_rows(s: &Store, first_secs: i64, requests: i64, tokens: i64) {
        for i in 0..requests {
            let ts = rfc3339(first_secs + i);
            s.record_usage(&kiwanod::store::UsageRecord {
                ts: ts.clone(),
                agent: "claude".into(),
                provider_id: "demo-alpha".into(),
                model: Some("demo-model".into()),
                input_tokens: tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(100),
                status: "ok".into(),
                cost: Some(0.5),
                cost_currency: Some("USD".into()),
                cost_off_peak: None,
            })
            .unwrap();
            // The headline count reads request_logs, not usage: seed both, as
            // the gateway does for a forwarded request.
            s.insert_request_log(&kiwanod::store::RequestLogNew {
                ts,
                method: "POST".into(),
                path: "/v1/messages".into(),
                query: None,
                agent: Some("claude".into()),
                attribution: Some("key".into()),
                provider_id: Some("demo-alpha".into()),
                model: Some("demo-model".into()),
                status_code: 200,
                error_kind: None,
                error_message: None,
                session_id: None,
                is_streaming: false,
                input_tokens: tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
                usage_missing: false,
                latency_ms: Some(100),
                first_token_ms: None,
                request_headers: None,
                response_headers: None,
                request_body: None,
                response_body: None,
                request_size: 0,
                response_size: 0,
                truncated: false,
                cost: Some(0.5),
                cost_currency: Some("USD".into()),
                cost_off_peak: None,
                request_notes: None,
            })
            .unwrap();
        }
    }

    pub(crate) fn usage_row(provider_id: &str) -> UsageRecord {
        UsageRecord {
            ts: rfc3339(unix_now()),
            agent: "claude".into(),
            provider_id: provider_id.into(),
            model: None,
            input_tokens: 1_000,
            output_tokens: 100,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: None,
            status: "ok".into(),
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
        }
    }

    /// A store and an aux on **one file**, which is what production has: the
    /// GUI opens both on `kiwano.db`, so a rate cached by a sync is the rate the
    /// gateway reads back when it measures a limit.
    pub(crate) fn store_and_aux_on_one_file() -> (tempfile::TempDir, Store, Aux) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();
        (dir, store, aux)
    }
}
