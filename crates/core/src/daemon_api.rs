//! The client of the daemon's resource API — one function per command.
//!
//! `migrate.local.md` §7 batch 1 moves the *state* commands here: the app and
//! the CLI stop opening the store and ask the process that owns it. The shape
//! of each function is deliberately the same as the `vm::` function it
//! replaces (same arguments, same return type, same error string where there is
//! one), so a caller changes one line and no call site has to be reshaped
//! around the transport.
//!
//! Two things every one of these needs are in `sidecar`: the authorized GET
//! ([`crate::sidecar::admin_get_json`]) and the endpoint the app and the CLI
//! already resolve the same way.

use kiwano_adapters::model_pricing::ModelPriceEntry;
use kiwano_api::agents::CustomAgentVm;
use kiwano_api::client_keys::{
    ClientKeyCreatedVm, ClientKeyLimitVm, ClientKeyPolicyInput, ClientKeyVm, NewClientKeyInput,
};
use kiwano_api::dashboard::{CurrencyMetaVm, DashboardVm, FooterStatsVm, UsageAlertVm};
use kiwano_api::keys::ApiKeyVm;
use kiwano_api::providers::{
    NewProviderInput, ProviderPatch, ProviderRefVm, ProviderVm, SyncReportVm,
};
use kiwano_api::routes::{AgentLimitVm, AgentRouteVm};
use kiwano_api::settings::SettingsVm;
use kiwanod::api::catalog::CatalogListVm;
use kiwanod::api::import_history::HistoryImportReport;
use kiwanod::api::logs::RequestLogListVm;
use kiwanod::api::share::ImportReport;
use kiwanod::plan_quota::PlanQuotaReport;
use kiwanod::store::{RequestLogDetail, RequestLogEntry, RequestLogFilter};

use crate::sidecar::{self, AdminEndpoint};

/// A client of one daemon: where it is, and what it authenticates with.
///
/// Both halves in one value because they are answered together — and because
/// the second is the open question of `migrate.local.md` §13.5. Today a client
/// reads the token from the database it shares with its daemon
/// ([`DaemonApi::connect`]); a remote one will have to be told, which is
/// [`DaemonApi::with_token`]. Every resource call takes a `DaemonApi`, so that
/// question has one place to be answered in rather than thirty.
pub struct DaemonApi {
    endpoint: AdminEndpoint,
    token: Option<String>,
}

impl DaemonApi {
    /// The client for a daemon on this machine: same database, therefore the
    /// same token.
    pub fn connect() -> Self {
        // A named remote daemon wins over the local plane — the same resolution
        // the app and the CLI share (`sidecar::admin_endpoint_resolved`). A
        // malformed address is reported rather than fallen back from, so a typo
        // cannot silently reach a different daemon.
        let endpoint = match sidecar::admin_endpoint_resolved() {
            Ok(endpoint) => endpoint,
            Err(e) => {
                tracing::error!("{e}");
                // The local plane, which will fail to answer and surface as an
                // unreachable daemon — but the message above says why.
                sidecar::admin_endpoint()
            }
        };
        Self::with_token(endpoint, sidecar::admin_token())
    }

    pub fn with_token(endpoint: AdminEndpoint, token: Option<String>) -> Self {
        Self { endpoint, token }
    }

    /// A provider's key pool, masked — `vm::list_api_keys`, served by the daemon.
    pub fn list_api_keys(&self, provider_id: &str) -> Result<Vec<ApiKeyVm>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{provider_id}/keys"),
        )
    }

    /// Add a rotation key — `vm::add_api_key`. Safe to retry: the far end keys
    /// on the value, so a second delivery answers with the row already there
    /// rather than a second one (`migrate.local.md` §6.1).
    pub fn add_api_key(
        &self,
        provider_id: &str,
        api_key: &str,
        label: Option<&str>,
    ) -> Result<ApiKeyVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            api_key: &'a str,
            label: Option<&'a str>,
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{provider_id}/keys"),
            &Body { api_key, label },
        )
    }

    /// Remove a rotation key — `vm::delete_api_key`. `false` when there was no
    /// such row, which is what a retry finds and is not an error.
    pub fn delete_api_key(&self, id: i64) -> Result<bool, String> {
        let deleted: serde_json::Value = sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/keys/{id}"),
        )?;
        Ok(deleted
            .get("deleted")
            .and_then(|d| d.as_bool())
            .unwrap_or(false))
    }

    // ── Agent routes and bindings ──

    /// Every agent's route, its ordered candidates and its ceilings —
    /// `vm::build_agent_routes`.
    pub fn list_agent_routes(&self) -> Result<Vec<AgentRouteVm>, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/agent-routes")
    }

    /// Set an agent's strategy, and the JSON payload some of them carry —
    /// `vm::set_agent_strategy`.
    pub fn set_agent_strategy(
        &self,
        agent: &str,
        strategy: &str,
        config: Option<&str>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            strategy: &'a str,
            config: Option<&'a str>,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/strategy"),
            &Body { strategy, config },
        ))
    }

    /// Replace an agent's whole ceiling set — `vm::set_agent_limits`. A `PUT`:
    /// the array *is* the resulting set, and an empty one clears it.
    pub fn set_agent_limits(&self, agent: &str, limits: &[AgentLimitVm]) -> Result<(), String> {
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/limits"),
            &limits,
        ))
    }

    /// Every client key, masked, with its windows, its allowlists and what it
    /// spent over `days` (None = all time).
    pub fn list_client_keys(&self, days: Option<i64>) -> Result<Vec<ClientKeyVm>, String> {
        let path = match days {
            Some(days) => format!("/api/client-keys?days={days}"),
            None => "/api/client-keys".to_string(),
        };
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), &path)
    }

    /// One client key by its handle (`ck-…`).
    pub fn get_client_key(&self, id: &str, days: Option<i64>) -> Result<ClientKeyVm, String> {
        let path = match days {
            Some(days) => format!("/api/client-keys/{id}?days={days}"),
            None => format!("/api/client-keys/{id}"),
        };
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), &path)
    }

    /// Mint a key. The response is the one place the secret appears.
    pub fn add_client_key(&self, input: &NewClientKeyInput) -> Result<ClientKeyCreatedVm, String> {
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/client-keys",
            input,
        )
    }

    /// Drop a key, and with it its windows. Idempotent at the store; a missing
    /// handle answers not-found, which is what tells a caller a typo from a
    /// success.
    pub fn delete_client_key(&self, id: &str) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/client-keys/{id}"),
        ))
    }

    /// A new secret for the same key: same handle, same spend history.
    pub fn rotate_client_key(&self, id: &str) -> Result<ClientKeyCreatedVm, String> {
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/client-keys/{id}/rotate"),
            &serde_json::json!({}),
        )
    }

    /// Replace a key's whole window set — a `PUT`, like the agent ceilings: an
    /// empty array clears it.
    pub fn set_client_key_limits(
        &self,
        id: &str,
        limits: &[ClientKeyLimitVm],
    ) -> Result<(), String> {
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/client-keys/{id}/limits"),
            &limits,
        ))
    }

    /// Replace a key's allowlists (and its label, when one is sent).
    pub fn set_client_key_policy(
        &self,
        id: &str,
        policy: &ClientKeyPolicyInput,
    ) -> Result<(), String> {
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/client-keys/{id}/policy"),
            policy,
        ))
    }

    /// Import a batch of an agent's own history — `api::import_history`.
    ///
    /// One request carries one chunk (`history::chunks`); the caller loops. The
    /// report it returns is per chunk, and the caller sums them.
    pub fn import_history(
        &self,
        batch: &kiwano_api::history::HistoryBatch,
    ) -> Result<HistoryImportReport, String> {
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/import/history",
            batch,
        )
    }

    /// The imported sessions, newest first.
    pub fn list_sessions(
        &self,
        agent: Option<&str>,
        project: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<kiwanod::store::sessions::SessionRow>, String> {
        // Absent filters are omitted rather than sent empty: the far end reads an
        // empty value as a filter that matches nothing.
        let mut query: Vec<String> = Vec::new();
        for (key, value) in [("agent", agent), ("project", project), ("since", since)] {
            if let Some(value) = value {
                query.push(format!("{key}={}", encode_query(value)));
            }
        }
        let path = if query.is_empty() {
            "/api/history/sessions".to_string()
        } else {
            format!("/api/history/sessions?{}", query.join("&"))
        };
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), &path)
    }

    /// When this machine's history was last imported, if it ever has.
    pub fn history_scanned_at(&self) -> Result<Option<String>, String> {
        let v: serde_json::Value =
            sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/history/scan")?;
        Ok(v.get("scanned_at")
            .and_then(|s| s.as_str())
            .map(str::to_string))
    }

    /// Rewrite the candidate order — `vm::reorder_agent_bindings`.
    pub fn reorder_agent_bindings(
        &self,
        agent: &str,
        provider_ids: &[String],
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            provider_ids: &'a [String],
        }
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings"),
            &Body { provider_ids },
        ))
    }

    /// Patch one binding's weight and time window — `vm::update_agent_binding`.
    /// A `PATCH`: a field left out keeps its value, which is what the screen's
    /// half-filled forms rely on.
    pub fn update_agent_binding(
        &self,
        agent: &str,
        provider_id: &str,
        weight: Option<i64>,
        win_start: Option<&str>,
        win_end: Option<&str>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            weight: Option<i64>,
            win_start: Option<&'a str>,
            win_end: Option<&'a str>,
        }
        wrote(sidecar::admin_patch_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings/{provider_id}"),
            &Body {
                weight,
                win_start,
                win_end,
            },
        ))
    }

    /// Bind a provider to an agent — `vm::add_agent_binding`. A no-op when it
    /// is already bound, so a retry cannot reorder the queue (`§6.1`).
    pub fn add_agent_binding(&self, agent: &str, provider_id: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            provider_id: &'a str,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings"),
            &Body { provider_id },
        ))
    }

    /// Unbind one candidate — `vm::remove_agent_binding`. Unlike the key pool,
    /// removing what is not bound is an error; the daemon decides that, and the
    /// message travels back unchanged.
    pub fn remove_agent_binding(&self, agent: &str, provider_id: &str) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings/{provider_id}"),
        ))
    }

    /// Copy one agent's route onto another — `vm::apply_agent_route`.
    pub fn apply_agent_route(&self, target: &str, source: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            source: &'a str,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{target}/route/apply"),
            &Body { source },
        ))
    }
    // ── User-defined agents ──

    /// Every user-defined agent, with its placeholder key —
    /// `kiwanod::api::agents::custom_agents_with_keys`.
    ///
    /// The key is not a secret being handed out: it is the one the client minted
    /// and wrote into the agent's own config, and it is what lets the settings
    /// screen show it back.
    pub fn custom_agents(&self) -> Result<Vec<CustomAgentVm>, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/custom-agents")
    }

    /// Define a user-defined agent — `vm::add_custom_agent`.
    ///
    /// The **id is the caller's** (`kiwano_api::ids::mint_agent_id`): nothing
    /// else in the request can identify the operation, since the agent's id
    /// carries a random suffix, and an id the daemon has already seen is
    /// answered with the agent that exists (`migrate.local.md` §6.1). Passing it
    /// in is what lets a caller that retries hold the same id across attempts.
    pub fn add_custom_agent(
        &self,
        id: &str,
        label: &str,
        note: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<CustomAgentVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            id: &'a str,
            label: &'a str,
            note: Option<&'a str>,
            protocol: Option<&'a str>,
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/custom-agents",
            &Body {
                id,
                label,
                note,
                protocol,
            },
        )
    }

    /// Rename one, re-note it, re-protocol it — `vm::update_custom_agent`. A
    /// `PUT`: the body is the resulting state, and the id is in the path because
    /// it is the one field that does not change.
    pub fn update_custom_agent(
        &self,
        id: &str,
        label: &str,
        note: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<CustomAgentVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            label: &'a str,
            note: Option<&'a str>,
            protocol: Option<&'a str>,
        }
        sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/custom-agents/{id}"),
            &Body {
                label,
                note,
                protocol,
            },
        )
    }

    /// Delete an agent with its route and its key — `vm::remove_custom_agent`.
    /// Its usage history stays.
    pub fn remove_custom_agent(&self, id: &str) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/custom-agents/{id}"),
        ))
    }
    // ── Provider writes ──

    /// Switch a provider on or off — `vm::set_provider_enabled`.
    pub fn set_provider_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body {
            enabled: bool,
        }
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}/enabled"),
            &Body { enabled },
        ))
    }

    /// Delete a provider — `vm::delete_provider`. `false` when there was no such
    /// row, which is what a retry finds and is not an error.
    pub fn delete_provider(&self, id: &str) -> Result<bool, String> {
        let deleted: serde_json::Value = sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}"),
        )?;
        Ok(deleted
            .get("deleted")
            .and_then(|d| d.as_bool())
            .unwrap_or(false))
    }
    // ── Request logs and the credential banner ──

    /// One page of the audit trail — `vm::list_request_logs`.
    pub fn list_request_logs(
        &self,
        page: i64,
        page_size: i64,
        filter: RequestLogFilter<'_>,
    ) -> Result<RequestLogListVm, String> {
        let mut query = format!("page={page}&page_size={page_size}");
        // Absent filters are *omitted*, not sent empty: the far end reads an
        // empty value as a filter that matches nothing, which is a different
        // request from "no filter".
        for (key, value) in [
            ("agent", filter.agent),
            ("provider_id", filter.provider_id),
            ("status", filter.status),
            ("from", filter.from),
            ("to", filter.to),
        ] {
            if let Some(value) = value {
                query.push_str(&format!("&{key}={}", encode_query(value)));
            }
        }
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/logs?{query}"),
        )
    }

    /// One entry in full — `vm::get_request_log`. `None` when there is no such
    /// row, which is what a cleared trail answers.
    pub fn get_request_log(&self, id: i64) -> Result<Option<RequestLogDetail>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/logs/{id}"),
        )
    }

    /// Empty the audit trail — `vm::clear_request_logs`. Idempotent.
    pub fn clear_request_logs(&self) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/logs",
        ))
    }

    /// The newest unacknowledged credential finding — `vm::check_credential_finding`.
    pub fn check_credential_finding(&self) -> Result<Option<RequestLogEntry>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/credential-finding",
        )
    }

    /// Acknowledge a finding — `vm::ack_credential_finding`.
    pub fn ack_credential_finding(&self, id: i64) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body {
            id: i64,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/credential-finding/ack",
            &Body { id },
        ))
    }
    /// The price mirror — `vm::list_model_prices`.
    pub fn list_model_prices(&self) -> Result<Vec<ModelPriceEntry>, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/model-prices")
    }
    /// The Hub shelf — `vm::load_catalog`. Served by the daemon, which owns the
    /// cache it is read from and the endpoint matching that marks an entry as
    /// already added.
    pub fn list_catalog(&self) -> Result<CatalogListVm, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/catalog")
    }
    /// Add a provider — `vm::add_provider`. The **id is the caller's**
    /// (`kiwano_api::ids::mint_provider_id`): an id the daemon has already seen
    /// is answered with the provider that exists (`§6.1`). This is the same
    /// shape as `add_custom_agent`, and for the same reason: a client that
    /// retries must hold the same id across attempts.
    pub fn add_provider(&self, id: &str, input: &NewProviderInput) -> Result<ProviderVm, String> {
        // Flattened rather than wrapped, so the body is the form's own shape
        // plus the id — which is what the endpoint deserializes.
        let mut body = serde_json::to_value(input).map_err(|e| format!("cannot encode: {e}"))?;
        body.as_object_mut()
            .expect("the input is an object")
            .insert("id".into(), id.into());
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/providers",
            &body,
        )
    }
    /// Sync the Hub — the catalog and the price table, cached and applied.
    /// The daemon fetches, and re-reads its own route table when either wrote
    /// (`migrate.local.md` §10.14). A skip is still an answer: "checked, still
    /// current" is what the footer badge shows.
    pub fn sync_hub(&self) -> Result<SyncReportVm, String> {
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/sync-hub",
            &serde_json::Value::Null,
        )
    }
    /// One provider's plan usage — `kiwanod::plan_quota::get_plan_quota_report`.
    /// The daemon reads the vendor's own endpoint, which is the same reader the
    /// data plane enforces the ceiling with, so the display and the block
    /// cannot disagree. `force` skips the 5-minute cache.
    pub fn get_plan_quota(
        &self,
        provider_id: &str,
        force: bool,
    ) -> Result<PlanQuotaReport, String> {
        let query = if force { "?force=true" } else { "" };
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/plan-quota/{provider_id}{query}"),
        )
    }

    /// The network half of a Test button — one of the three probes.
    ///
    /// **Synchronous on purpose.** The transport underneath is a blocking
    /// socket, so an `async fn` here would block a runtime worker while looking
    /// like it yields; the caller is the one that knows whether it is on a
    /// thread that may block (the app's commands are `(async)`, which is
    /// Tauri's "not the main thread").
    ///
    /// `action` is `latency` | `endpoint` | `models`. A blank `api_key` is
    /// filled in with the stored credential — for that provider's own endpoints
    /// only, on the daemon's side (`migrate.local.md` §10.15). The response is
    /// the probe's own shape: `{latency_ms}`, the `ProbeReport`, or the model
    /// list, so the caller deserializes what it asked for.
    pub fn probe(
        &self,
        protocol: &str,
        endpoint: &str,
        api_key: Option<&str>,
        provider_id: Option<&str>,
        action: &str,
    ) -> Result<serde_json::Value, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            protocol: &'a str,
            endpoint: &'a str,
            api_key: Option<&'a str>,
            provider_id: Option<&'a str>,
            action: &'a str,
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/probe",
            &Body {
                protocol,
                endpoint,
                api_key,
                provider_id,
                action,
            },
        )
    }

    /// One prompt round trip against a stored provider — the Apps screen's Test
    /// button. The verdict is written to the health table, which the daemon owns.
    pub fn test_provider_latency(
        &self,
        id: &str,
    ) -> Result<kiwano_api::agents::PromptLatencyVm, String> {
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}/test-latency"),
            &serde_json::Value::Null,
        )
    }
    /// The settings blob — `vm::ui_settings`. The takeovers and the custom
    /// agents are **not** in it: the client layers those on from this machine's
    /// agents and their config files (`build_settings_with_home`), which is the
    /// one place that layering happens.
    pub fn get_settings(&self) -> Result<SettingsVm, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/settings")
    }

    /// Apply a patch — `vm::update_settings`. The daemon mirrors the
    /// gateway-facing knobs itself and re-reads them on its own reload, so this
    /// sends no `/reload`; the OS login item is the client's own side effect
    /// (`sync_autostart` in the app's command).
    pub fn update_settings(&self, patch: &serde_json::Value) -> Result<SettingsVm, String> {
        sidecar::admin_patch_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/settings",
            patch,
        )
    }
    // ── Config share and the log export ──

    /// The whole configuration as JSON — `vm::export_config`. The **caller**
    /// writes it to a file: the path came from the user's save dialog, which is
    /// the client's business (`migrate.local.md` §10.17).
    pub fn export_config(&self, include_keys: bool) -> Result<String, String> {
        let v: serde_json::Value = sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/config/export?include_keys={include_keys}"),
        )?;
        Ok(v["config"].as_str().unwrap_or_default().to_string())
    }

    /// Apply a shared document — `vm::import_config`. The text, not a path: the
    /// caller read the file, and the daemon validates what it says.
    pub fn import_config(&self, json: &str) -> Result<ImportReport, String> {
        // The text as the body, not a JSON string containing it: the daemon
        // parses and validates the document, and it cannot do that with a
        // quoted copy (`migrate.local.md` §10.39).
        sidecar::admin_post_text(
            &self.endpoint,
            self.token.as_deref(),
            "/api/config/import",
            json,
        )
    }

    /// The export's rows, for a caller that computes over them instead of
    /// writing them out — `kiwano insights` and the cache-shaping experiment.
    ///
    /// `bodies` says whether each row carries its request and response bodies.
    /// The cap is the export's, and `truncated` travels with the answer so a
    /// report can say its window was longer than the rows it saw rather than
    /// imply it saw all of them.
    pub fn export_rows(
        &self,
        filter: RequestLogFilter<'_>,
        limit: i64,
        bodies: bool,
    ) -> Result<(Vec<kiwanod::store::RequestLogExportRow>, bool), String> {
        let mut query = format!("limit={limit}&bodies={bodies}");
        for (key, value) in [
            ("agent", filter.agent),
            ("provider_id", filter.provider_id),
            ("status", filter.status),
            ("from", filter.from),
            ("to", filter.to),
        ] {
            if let Some(value) = value {
                query.push_str(&format!("&{key}={}", encode_query(value)));
            }
        }
        let page: serde_json::Value = sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/logs/export-rows?{query}"),
        )?;
        let rows = serde_json::from_value(page["rows"].clone())
            .map_err(|e| format!("gateway answered rows that cannot be read: {e}"))?;
        Ok((rows, page["truncated"].as_bool().unwrap_or(false)))
    }

    /// The filtered log slice as CSV **text** — the content half of the export.
    /// The caller writes the file.
    pub fn export_request_logs(
        &self,
        filter: RequestLogFilter<'_>,
    ) -> Result<(String, usize, bool), String> {
        let mut query = String::new();
        for (key, value) in [
            ("agent", filter.agent),
            ("provider_id", filter.provider_id),
            ("status", filter.status),
            ("from", filter.from),
            ("to", filter.to),
        ] {
            if let Some(value) = value {
                if !query.is_empty() {
                    query.push('&');
                }
                query.push_str(&format!("{key}={}", encode_query(value)));
            }
        }
        let page: serde_json::Value = sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/logs/export?{query}"),
        )?;
        Ok((
            page["csv"].as_str().unwrap_or_default().to_string(),
            page["rows_written"].as_u64().unwrap_or(0) as usize,
            page["truncated"].as_bool().unwrap_or(false),
        ))
    }
    /// Apply an edit — `vm::update_provider`'s **write** half. The view is the
    /// caller's (`migrate.local.md` §5's fifth constraint): the daemon updates
    /// the row and the bindings, and the caller assembles what it shows.
    ///
    /// A patch, so the caller says only what changed and needs no read of the
    /// row first. `PATCH` rather than `PUT` because that is what the body now
    /// means (`migrate.local.md` §10.38).
    pub fn update_provider(&self, id: &str, patch: &ProviderPatch) -> Result<(), String> {
        wrote(sidecar::admin_patch_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}"),
            patch,
        ))
    }

    /// Make a provider the first candidate for an agent — `vm::bind_as_primary`.
    ///
    /// One call, not "bind then reorder": the promotion reindexes every other
    /// binding, and a client doing it in two steps could be interrupted between
    /// them and leave two candidates claiming priority 1.
    pub fn bind_as_primary(&self, agent: &str, provider_id: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            provider_id: &'a str,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/primary"),
            &Body { provider_id },
        ))
    }

    /// Infer each provider's catalog entry from the synced shelf. Returns how
    /// many were linked.
    pub fn link_providers(&self) -> Result<usize, String> {
        let v: serde_json::Value = sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/catalog/link",
            &serde_json::json!({}),
        )?;
        Ok(v["linked"].as_u64().unwrap_or(0) as usize)
    }

    /// The currencies a limit may be written in — `vm::known_limit_currencies`.
    ///
    /// Asked rather than derived: the rule is "a currency this machine has no
    /// rate for is refused", and only the side holding the rate table can say
    /// which those are.
    pub fn limit_currencies(&self) -> Result<Vec<String>, String> {
        let v: serde_json::Value = sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/limit-currencies",
        )?;
        Ok(v["currencies"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A provider named by id: its name, and how it is billed.
    ///
    /// The narrow read a client without a form needs before it can state the
    /// rules that are phrased in terms of the stored mode — `providers edit`'s
    /// `--plan-limit-*` among them (`migrate.local.md` §10.38).
    pub fn provider_ref(&self, id: &str) -> Result<ProviderRefVm, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}"),
        )
    }
    /// Apply cc-switch rows the caller extracted — the **store half** of that
    /// import. The files stay on the client (`migrate.local.md` §10.19).
    pub fn import_cc_switch(
        &self,
        raws: &[kiwanod::api::import::RawProvider],
        skips: &[String],
    ) -> Result<kiwanod::api::import::ImportReportVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            raws: &'a [kiwanod::api::import::RawProvider],
            skips: &'a [String],
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/import/cc-switch",
            &Body { raws, skips },
        )
    }
    /// What a client needs before it acts on an agent's takeover: the key
    /// already registered, and where restore would point the agent if its backup
    /// turns out to be unusable.
    ///
    /// The rebuild half is why this is asked rather than read
    /// (`migrate.local.md` §10.43): it carries the provider's credential, which
    /// lives in the daemon's row.
    pub fn takeover_state(
        &self,
        agent: &str,
    ) -> Result<kiwanod::api::takeover::TakeoverStateVm, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/takeover/{agent}/state"),
        )
    }

    /// Take back one registration — what a takeover whose file half failed has
    /// to undo. Narrower than [`Self::takeover_teardown`]: the rest of the state
    /// is left as it was found.
    pub fn takeover_unregister(&self, agent: &str) -> Result<bool, String> {
        let v: serde_json::Value = sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/takeover/{agent}/key"),
        )?;
        Ok(v["removed"].as_bool().unwrap_or(false))
    }

    /// Undo the store half of a takeover — registration, bindings, strategy.
    /// One call, because they are one decision.
    pub fn takeover_teardown(&self, agent: &str) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/takeover/{agent}/state"),
        ))
    }

    /// The store half of starting a takeover — `vm::takeover::phase_state`.
    ///
    /// The **key is the caller's**, which is what makes a replay safe without an
    /// operation table here (`migrate.local.md` §8): registering the key it
    /// already minted is one row. The credential is what the caller read out of
    /// the agent's own config — those files are the client's, and §5's first
    /// constraint forbids paths in this interface.
    pub fn takeover_register(
        &self,
        agent: &str,
        key: &str,
        creds: Option<&kiwanod::api::takeover::CurrentCreds>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            key: &'a str,
            creds: Option<&'a kiwanod::api::takeover::CurrentCreds>,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/takeover/{agent}/state"),
            &Body { key, creds },
        ))
    }
    /// Where an agent on **this** machine should be pointed to reach this
    /// daemon's data plane.
    ///
    /// Composed from the two halves each side actually knows: the client knows
    /// the host — it dialled it — and the daemon reports the port, which may not
    /// be the conventional one (`migrate.local.md` §10.30). A local client gets
    /// `http://127.0.0.1:<port>`, which is what a takeover wrote before this
    /// existed.
    pub fn gateway_base(&self) -> Result<String, String> {
        let status: serde_json::Value =
            sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/status")?;
        let port = status["data_port"]
            .as_u64()
            .unwrap_or(kiwanod::server::DEFAULT_DATA_PORT as u64);
        // The daemon says which interface its data plane is on. A client that
        // used its own idea of the host instead would write an address nothing
        // listens on whenever the two planes are not on the same one — which is
        // the shape of the bug §10.30 shipped with, where the client composed
        // the far host and the listener was on loopback. A daemon too old to
        // report it falls back to the host this client dialled.
        let host = status["data_host"]
            .as_str()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.endpoint.host());
        Ok(format!("http://{host}:{port}"))
    }
    /// The Apps list, assembled by the daemon — `vm::build_provider_vms`.
    ///
    /// `live` is the one thing the client supplies: which agents actually route
    /// through the gateway, read out of their own config files, which are this
    /// machine's (`migrate.local.md` §5 #2). Everything else is the daemon's
    /// database, which is the whole reason this endpoint exists.
    pub fn provider_view(&self, live: &[String]) -> Result<Vec<ProviderVm>, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            live: &'a [String],
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/providers/view",
            &Body { live },
        )
    }
    // ── The Dashboard, the footer and the currency picker ──

    /// The Dashboard for one window — `vm::build_dashboard`.
    pub fn dashboard(
        &self,
        window: &str,
        provider_id: Option<&str>,
        agent: Option<&str>,
    ) -> Result<DashboardVm, String> {
        let mut query = format!("window={}", encode_query(window));
        for (key, value) in [("provider_id", provider_id), ("agent", agent)] {
            if let Some(value) = value {
                query.push_str(&format!("&{key}={}", encode_query(value)));
            }
        }
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/dashboard?{query}"),
        )
    }

    /// A usage window, whole — totals, spend per currency, per-provider
    /// breakdown, provider names. One answer, because it is one aggregate over
    /// one set of rows.
    pub fn usage_report(
        &self,
        days: i64,
        agent: Option<&str>,
    ) -> Result<kiwanod::api::dashboard::UsageReportVm, String> {
        let mut query = format!("days={days}");
        if let Some(agent) = agent {
            query.push_str(&format!("&agent={}", encode_query(agent)));
        }
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/usage?{query}"),
        )
    }

    /// Today's totals — `vm::build_footer_stats`. `version` is the **client's**
    /// own: the footer names the app the user is looking at.
    pub fn footer_stats(&self, version: &str) -> Result<FooterStatsVm, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/footer?version={}", encode_query(version)),
        )
    }

    /// The currencies a limit may be written in — `pricing::currency_meta`.
    pub fn currency_meta(&self) -> Result<CurrencyMetaVm, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/currency")
    }
    /// The current usage alerts — `vm::check_usage_alerts`. `mark` decides
    /// whether this poll may consume the dedup: the app's notification path
    /// sends true, a read-only caller sends false.
    pub fn usage_alerts(&self, mark: bool) -> Result<Vec<UsageAlertVm>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/usage-alerts?mark={mark}"),
        )
    }
}

/// A value as a query-string component: everything outside the unreserved set is
/// percent-encoded.
///
/// Not decoration — the time filters are timestamps.
/// `2026-10-01T00:00:00+08:00` sent raw arrives with its `+` read as a space, and
/// the far end then compares against a string no row has. Encoding the whole
/// value is the only version of this that cannot be wrong for a character nobody
/// thought of.
fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A write whose answer carries nothing a caller needs: `admin_send` already
/// turned a refusal into the `String` beside it, and the `{"ok":true}` body is
/// the convention, not information.
fn wrote(result: Result<serde_json::Value, String>) -> Result<(), String> {
    result.map(|_| ())
}

// Unix-only, like the stub in `sidecar`: the far end of these tests is a
// `UnixListener`, and a Windows named pipe cannot be created from `std`. What
// is checked here is the client's half of the transport — framing, the token
// header, the status check, the error envelope — and `kiwanod`'s own
// `admin_routes_answer_over_the_ipc_endpoint` covers the same ground on both
// platforms.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A daemon's side of the socket, canned: whatever the test wants answered,
    /// plus the request it was asked with. The client's own half — the framing,
    /// the token header, the status check, the error envelope — is what these
    /// tests are for, and none of it needs a running gateway.
    fn stub(
        response: &'static str,
    ) -> (
        tempfile::TempDir,
        AdminEndpoint,
        std::thread::JoinHandle<String>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("admin.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let endpoint = AdminEndpoint::beside_db(&dir.path().join("k.db"));
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read until the client stops writing, not once: headers and body
            // can arrive as separate segments, and a single `read` is how the
            // body would go missing from an assertion about the body.
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(250)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (dir, endpoint, handle)
    }

    #[test]
    fn a_key_pool_comes_back_parsed() {
        let body = r#"[{"id":3,"masked":"sk-liv…mnop","label":"backup","enabled":true,"created_at":"2026-01-01T00:00:00Z"}]"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let response: &'static str = Box::leak(response.into_boxed_str());
        let (_dir, endpoint, handle) = stub(response);

        let keys = DaemonApi::with_token(endpoint, Some("tok-123".into()))
            .list_api_keys("p-ant")
            .unwrap();

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, 3);
        assert_eq!(keys[0].masked, "sk-liv…mnop");
        assert_eq!(keys[0].label.as_deref(), Some("backup"));
        // The request is part of the contract too: the path names the resource
        // and its owner, and the token travels in its own header.
        let request = handle.join().unwrap();
        assert!(
            request.starts_with("GET /api/providers/p-ant/keys HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains("x-kiwano-admin-token: tok-123"),
            "{request}"
        );
    }

    /// The write half: the body is JSON, the content type says so, and a
    /// deletion uses the verb that names the row rather than a body.
    /// The verbs the route commands use, on the wire.
    ///
    /// Three of these are new to the client (`PUT`, `PATCH`, and a `POST` whose
    /// body carries an optional field), and a verb is not decoration: `PUT`
    /// claims the body is the resulting state, `PATCH` claims an absent field
    /// means "leave it alone". A transport that quietly sent POST for both would
    /// still work against this daemon and would be wrong the moment a second
    /// implementation read it. So they are asserted here, at the seam where a
    /// mistake would be invisible.
    /// The create carries the **id the caller minted** — the whole of §6.1's
    /// idempotency for a command whose id is random. A client that let the
    /// daemon mint it would have no way to identify a retry, so this asserts the
    /// field is on the wire rather than that the call compiles.
    #[test]
    fn creating_an_agent_sends_the_id_the_caller_minted() {
        let body = r#"{"id":"long-tasks-3f9a1c","label":"Long Tasks","note":null,"placeholder_key":"kw-ag-long-tasks-3f9a1c-ab12","protocol":null}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let created = api
            .add_custom_agent("long-tasks-3f9a1c", "Long Tasks", None, None)
            .unwrap();
        assert_eq!(created.id, "long-tasks-3f9a1c");

        let request = handle.join().unwrap();
        assert!(
            request.starts_with("POST /api/custom-agents HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(r#"{"id":"long-tasks-3f9a1c","label":"Long Tasks","#),
            "{request}"
        );

        // And a rename is a PUT whose body is the resulting state: the id is in
        // the path, because it is the one field that does not change.
        let (_d2, e2, h2) = stub(response);
        DaemonApi::with_token(e2, Some("tok-1".into()))
            .update_custom_agent(
                "long-tasks-3f9a1c",
                "Nightly",
                Some("moved"),
                Some("gemini"),
            )
            .unwrap();
        let request = h2.join().unwrap();
        assert!(
            request.starts_with("PUT /api/custom-agents/long-tasks-3f9a1c HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(r#"{"label":"Nightly","note":"moved","protocol":"gemini"}"#),
            "{request}"
        );
    }

    /// The list's query string, filters and all — and the one character that
    /// makes this test worth writing: a time filter with a `+` in its offset.
    ///
    /// `2026-10-01T00:00:00+08:00` sent raw arrives as `2026-10-01T00:00:00
    /// 08:00` (a space), and the far end then filters against a string no row
    /// has — a silently empty page rather than an error. An absent filter is
    /// omitted rather than sent empty, for the same class of reason: the daemon
    /// reads an empty value as "match nothing".
    #[test]
    fn the_log_query_string_omits_absent_filters_and_encodes_the_rest() {
        let body = r#"{"rows":[],"total":0}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let page = api
            .list_request_logs(
                2,
                50,
                RequestLogFilter {
                    agent: Some("claude"),
                    provider_id: None,
                    status: Some("error"),
                    from: Some("2026-10-01T00:00:00+08:00"),
                    to: None,
                },
            )
            .unwrap();
        assert_eq!(page.total, 0);

        let request = handle.join().unwrap();
        let line = request.lines().next().unwrap();
        assert!(
            line.starts_with("GET /api/logs?page=2&page_size=50&agent=claude&status=error&from="),
            "{line}"
        );
        assert!(
            line.contains("from=2026-10-01T00%3A00%3A00%2B08%3A00"),
            "the offset's `+` must be encoded: {line}"
        );
        assert!(
            !line.contains("provider_id") && !line.contains("to="),
            "an absent filter is omitted, not sent empty: {line}"
        );
    }

    /// The add carries the **id the caller minted**, flattened into the form's
    /// own shape — not a wrapper that would ask the daemon to deserialize a
    /// different structure from the one the form sends.
    ///
    /// This is the whole of §6.1's idempotency for this command, and it is the
    /// one thing a compiler cannot check: a client that wrapped the input in
    /// `{"input": …}` instead would still compile, and the endpoint would fail
    /// every add with a deserialization error instead of writing nothing on a
    /// retry.
    #[test]
    fn adding_a_provider_sends_the_id_flattened_into_the_form() {
        let body = r##"{"id":"deepseek-a1b2c3","name":"DeepSeek","logo_char":"D","logo_color":"#4D6BFE","logo_border":false,"currency":"USD","endpoint":"https://api.deepseek.com","protocol":"openai","endpoint_note":"OpenAI-compatible","billing":"payg","enabled":true,"agents":[],"serving_agents":[],"fallback_agents":[],"is_current":false,"health":{"state":"idle","latency_ms":null,"note":null,"source":null,"checked_at":null,"error":null},"usage":null,"endpoints":[],"advanced":null,"plan_query":null,"plan_limits":null,"prices":null,"catalog_id":null,"model_default":null,"limit_unit":null,"plan_price":null,"status_badge":null,"agents_note":null}"##;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let input = NewProviderInput {
            name: "DeepSeek".into(),
            api_key: "sk-test".into(),
            endpoint: "https://api.deepseek.com".into(),
            protocol: "openai".into(),
            openai_wire: Default::default(),
            model_default: String::new(),
            billing: "payg".into(),
            billing_config: kiwano_api::providers::BillingConfigInput {
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
        };
        let created = api.add_provider("deepseek-a1b2c3", &input).unwrap();
        assert_eq!(created.id, "deepseek-a1b2c3");

        let request = handle.join().unwrap();
        assert!(
            request.starts_with("POST /api/providers HTTP/1.1"),
            "{request}"
        );
        let sent = request.split("\r\n\r\n").nth(1).unwrap();
        assert!(
            sent.contains(r#""id":"deepseek-a1b2c3""#),
            "the minted id travels in the body: {sent}"
        );
        assert!(
            sent.contains(r#""name":"DeepSeek""#),
            "and the form's fields are at the top level, not nested: {sent}"
        );
        assert!(
            !sent.contains(r#""input":"#),
            "there is no wrapper object: {sent}"
        );
    }

    /// Every request carries the API generation (`migrate.local.md` §11.2).
    ///
    /// Asserted on the wire because nothing else can see it: a client that
    /// stopped sending the header would be treated as generation 0, which
    /// *works* today — the daemon still accepts it — and would stop working the
    /// day the floor moves. That is exactly the failure a version policy exists
    /// to make loud, and it would be silent if the header were not pinned here.
    #[test]
    fn every_request_carries_the_api_generation() {
        let body = r#"{"rows":[],"total":0}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        DaemonApi::with_token(endpoint, Some("tok-1".into()))
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();

        let request = handle.join().unwrap();
        assert!(
            request.contains(&format!(
                "{}: {}",
                kiwano_api::version::HEADER,
                kiwano_api::version::CURRENT
            )),
            "the generation is missing from: {request}"
        );
    }

    #[test]
    fn the_route_writes_use_the_verbs_the_contract_names() {
        let ok: &'static str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\n\r\n{\"ok\":true}";

        // PUT: the ceiling set is the whole state.
        let (_d1, e1, h1) = stub(ok);
        DaemonApi::with_token(e1, Some("tok-1".into()))
            .set_agent_limits(
                "claude",
                &[kiwano_api::routes::AgentLimitVm {
                    period: "day".into(),
                    period_limit: 100.0,
                    limit_unit: None,
                }],
            )
            .unwrap();
        let request = h1.join().unwrap();
        assert!(
            request.starts_with("PUT /api/agents/claude/limits HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(r#"[{"period":"day","period_limit":100.0,"limit_unit":null}]"#),
            "{request}"
        );

        // PATCH: an omitted field means "leave it alone", so it must not be
        // spelled as a null the far end would write.
        let (_d2, e2, h2) = stub(ok);
        DaemonApi::with_token(e2, Some("tok-1".into()))
            .update_agent_binding("claude", "p-ant", Some(7), None, None)
            .unwrap();
        let request = h2.join().unwrap();
        assert!(
            request.starts_with("PATCH /api/agents/claude/bindings/p-ant HTTP/1.1"),
            "{request}"
        );
        assert!(request.contains(r#""weight":7"#), "{request}");

        // DELETE: the path names the binding, so there is no body to send.
        let (_d3, e3, h3) = stub(ok);
        DaemonApi::with_token(e3, Some("tok-1".into()))
            .remove_agent_binding("claude", "p-ant")
            .unwrap();
        let request = h3.join().unwrap();
        assert!(
            request.starts_with("DELETE /api/agents/claude/bindings/p-ant HTTP/1.1"),
            "{request}"
        );
        assert!(!request.contains("Content-Type"), "{request}");

        // And a refusal keeps the daemon's own sentence, not a status code
        // retyped here — the app shows that string.
        let refusal: &'static str = "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 79\r\n\r\n{\"ok\":false,\"error\":\"provider ghost is not bound to claude\",\"kind\":\"not_found\"}";
        let (_d4, e4, _h4) = stub(refusal);
        let err = DaemonApi::with_token(e4, Some("tok-1".into()))
            .remove_agent_binding("claude", "ghost")
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err, "provider ghost is not bound to claude");
    }

    #[test]
    fn writes_send_json_and_the_right_verb() {
        let body = r#"{"id":7,"masked":"…abcd","label":null,"enabled":true,"created_at":"2026-01-01T00:00:00Z"}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let added = api
            .add_api_key("p-ant", "sk-rotated", Some("backup"))
            .unwrap();
        assert_eq!(added.id, 7);
        let request = handle.join().unwrap();
        assert!(
            request.starts_with("POST /api/providers/p-ant/keys HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains("Content-Type: application/json"),
            "{request}"
        );
        assert!(
            request.contains(r#"{"api_key":"sk-rotated","label":"backup"}"#),
            "{request}"
        );

        let deleted_body = r#"{"deleted":true}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{deleted_body}",
                deleted_body.len()
            )
            .into_boxed_str(),
        );
        let (_dir2, endpoint2, handle2) = stub(response);
        let ok = DaemonApi::with_token(endpoint2, Some("tok-1".into()))
            .delete_api_key(7)
            .unwrap();
        assert!(ok);
        let request2 = handle2.join().unwrap();
        assert!(
            request2.starts_with("DELETE /api/keys/7 HTTP/1.1"),
            "{request2}"
        );
        // No body on a deletion: the path is the whole request.
        assert!(!request2.contains("Content-Type"), "{request2}");
    }

    /// A refusal arrives as the daemon's own sentence, not as a status code
    /// retyped here: the message is what a user can act on.
    #[test]
    fn a_refusal_keeps_the_daemons_own_message() {
        let body = r#"{"ok":false,"error":"no such provider: p-missing"}"#;
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let response: &'static str = Box::leak(response.into_boxed_str());
        let (_dir, endpoint, _handle) = stub(response);

        let err = DaemonApi::with_token(endpoint, None)
            .list_api_keys("p-missing")
            .unwrap_err();
        assert_eq!(err, "no such provider: p-missing");
    }
}
