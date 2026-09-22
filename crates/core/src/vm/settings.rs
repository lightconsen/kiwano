//! The settings screen: the `app_settings` row as a `SettingsVm`, the defaults
//! that fill in what an older row never stored, and the timezone offset the
//! rest of the module reads through `tz_offset`.

use crate::detect::ShellVars;
use crate::vm::agents::{agent_protocols, CustomAgentVm, ADDITIVE_AGENTS, AGENTS};
use crate::vm::routes::display_path;
use crate::vm::{e2s, Aux};
use kiwanod::store::Store;
use serde::{Deserialize, Serialize};

/// The stored UTC offset (minutes east of UTC) the frontend keeps current.
pub(crate) fn tz_offset(aux: &Aux) -> i64 {
    ui_settings(aux).tz_offset_minutes
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TakeoverVm {
    pub agent: String,
    pub label: String,
    pub placeholder_key: Option<String>,
    pub enabled: bool,
    /// The files a takeover rewrites for this agent, in the form a reader would
    /// write them (`~/.claude/settings.json`). Read out of the same function the
    /// takeover itself writes through, so what the screen shows is the file that
    /// would actually change — not a second list to keep in step. Empty for an
    /// agent whose paths cannot be resolved on this platform (claude-desktop
    /// outside macOS).
    #[serde(default)]
    pub config_paths: Vec<String>,
    /// Additive-mode agent (config keeps multiple providers; takeover writes a
    /// gateway entry and selects it) rather than exclusive-switch mode.
    #[serde(default)]
    pub additive: bool,
    /// The protocols this agent's clients speak ([`AGENT_PROTOCOLS`]). On this
    /// row because it is *the* per-agent row the screen already reads — `additive`
    /// above is a fact about the agent rather than about takeover state, and this
    /// is the same kind of thing. A **label**: nothing routes or validates by it.
    #[serde(default)]
    pub protocols: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SettingsVm {
    pub language: String,
    pub theme: String,
    pub autostart: bool,
    pub close_to_tray: bool,
    pub gateway_listen: String,
    pub takeovers: Vec<TakeoverVm>,
    /// Agents the user defined (migration v16). Separate from `takeovers`
    /// because the two answer different questions: a takeover says "we rewrote
    /// this agent's config", and a custom agent has no config to rewrite.
    #[serde(default)]
    pub custom_agents: Vec<CustomAgentVm>,
    pub auto_failover: bool,
    pub request_logs: bool,
    /// Compat shim on the gateway's passthrough path (mirrored into
    /// gateway_settings for the sidecar). Default on: each rule is narrow
    /// enough that the honest choices are on and off, and off reproduces the
    /// 400s the shim exists to absorb.
    #[serde(default = "default_true")]
    pub compat_shim: bool,
    /// Outbound credential detection on the gateway's data plane (mirrored into
    /// gateway_settings for the sidecar). `"alert"` or `"off"`; default
    /// `"alert"`, because the pass reports and never blocks — an install that
    /// ignores this switch loses nothing by it.
    #[serde(default = "default_dlp_mode")]
    pub dlp_mode: String,
    /// Request-log retention in days (mirrored into gateway_settings for the
    /// sidecar); 0 keeps every row, which is the default — capture is the
    /// point of the feature, and a retention nobody chose is a silent cap on
    /// it. Same spelling as the stream timeouts below.
    #[serde(default)]
    pub log_retention_days: u32,
    /// Per-body capture cap in bytes (mirrored into gateway_settings); 0
    /// stores every byte, which is the default. The cap exists to bound two
    /// things at once — the row on disk and the buffer a streaming response is
    /// held in until it ends — so an install that sets none should know it is
    /// buying completeness with memory.
    #[serde(default)]
    pub log_max_body_bytes: u32,
    /// How long an upstream stream may take to its first byte before the
    /// gateway abandons it (mirrored into gateway_settings; 0 disables).
    #[serde(default = "default_stream_first_byte_secs")]
    pub stream_first_byte_secs: u32,
    /// How long a stream may stay silent between bytes before the gateway
    /// abandons it (0 disables).
    #[serde(default = "default_stream_idle_secs")]
    pub stream_idle_secs: u32,
    /// Cost-alert toggle (spec §4.1 P1): system notification when usage hits the per-period limit
    #[serde(default = "default_true")]
    pub cost_alert: bool,
    pub hub_logged_in: bool,
    #[serde(default = "default_hub_url")]
    pub hub_url: String,
    /// Preferred display currency (ISO code). Only the Dashboard converts into
    /// it, via the Hub's published exchange rates — rolling per-currency
    /// buckets into one number is what it is for. Everywhere else an amount
    /// keeps the currency it was priced in.
    #[serde(default = "default_preferred_currency")]
    pub preferred_currency: String,
    /// Auto-check for app updates at startup (opt-out; silent, notification only).
    #[serde(default = "default_true")]
    pub auto_check_update: bool,
    /// Version the user closed in the update banner. Remembered so one release
    /// does not re-announce itself on every launch — a newer one will show.
    #[serde(default)]
    pub dismissed_update: Option<String>,
    /// Minutes east of UTC (UTC+8 → 480). Every day boundary in the UI — the
    /// dashboard's "today", its daily chart buckets, the footer, and usage-alert
    /// reset periods — is the user's day, not UTC's. The frontend keeps it
    /// current; 0 (UTC) is the fallback for a settings blob written before this
    /// existed.
    #[serde(default)]
    pub tz_offset_minutes: i64,
    /// The Models list's sort choice, remembered across sessions: `"<key>:<dir>"`
    /// with key in name|protocol|tag|price and dir 1|-1. None means the default
    /// order (providers you already have, then tag rank, then name). A plain
    /// string rather than a struct on purpose — a value written by a build that
    /// knows a sort key this one does not simply falls back to the default.
    #[serde(default)]
    pub shelf_sort: Option<String>,
    /// Which Models view the user last chose: `"model"` for the one grouped by
    /// model, anything else (None) for the provider table. Remembered for the
    /// same reason as `shelf_sort`, only more so — the shelf is remounted after
    /// every mutation, so a view held in component state would snap back the
    /// moment the user added a provider while browsing models.
    #[serde(default)]
    pub shelf_view: Option<String>,
    // ── Features panel (docs/request-logs-applications.md): every flag is an
    // opt-in, default off. None of them reach the gateway's forward path, so
    // none are mirrored into gateway_settings. ──
    /// Alert when the month's current spend slope projects past a provider's
    /// currency limit, before the limit is actually hit.
    #[serde(default)]
    pub feat_cost_forecast: bool,
    /// Alert on error-rate spikes, latency outliers and traffic bursts,
    /// measured against the same hour's trailing-7-day baseline.
    #[serde(default)]
    pub feat_anomaly_alerts: bool,
    /// Alert when an agent hits its own per-period budget (agent_limits) —
    /// the enforcement is the gateway's, this is the missing notification.
    #[serde(default)]
    pub feat_agent_limit_alerts: bool,
    /// Let agents query their own aggregate stats over MCP (`kiwano mcp`).
    /// Aggregates only; bodies never leave the machine.
    #[serde(default)]
    pub feat_mcp_self_query: bool,
    /// Allow `kiwano rules apply` to append insights-derived rules to the
    /// agent's CLAUDE.md/AGENTS.md (backed up, reversible).
    #[serde(default)]
    pub feat_rule_injection: bool,
    /// Add tuning suggestions (retry budgets, route health) to the insights
    /// report. Advice only — nothing is reconfigured.
    #[serde(default)]
    pub feat_tuning_advice: bool,
    /// Enable the cache-shaping offline experiment (`kiwano cache-experiment`).
    #[serde(default)]
    pub feat_cache_experiment: bool,
}

pub fn default_preferred_currency() -> String {
    "CNY".into()
}

pub fn default_hub_url() -> String {
    crate::sync::DEFAULT_HUB_URL.into()
}

impl Default for SettingsVm {
    fn default() -> Self {
        Self {
            // Not a locale: the UI resolves this one from the OS. Defaulting to
            // a concrete language would decide the reader's language for them
            // on first run, and an existing install carries whatever it stored.
            language: "system".into(),
            theme: "dark".into(),
            autostart: true,
            close_to_tray: true,
            gateway_listen: "127.0.0.1:8317".into(),
            takeovers: Vec::new(),
            custom_agents: Vec::new(),
            auto_failover: true,
            request_logs: true,
            compat_shim: true,
            dlp_mode: default_dlp_mode(),
            log_retention_days: 0,
            log_max_body_bytes: 0,
            stream_first_byte_secs: default_stream_first_byte_secs(),
            stream_idle_secs: default_stream_idle_secs(),
            cost_alert: true,
            hub_logged_in: false,
            hub_url: default_hub_url(),
            preferred_currency: default_preferred_currency(),
            auto_check_update: true,
            dismissed_update: None,
            tz_offset_minutes: 0,
            shelf_sort: None,
            shelf_view: None,
            feat_cost_forecast: false,
            feat_anomaly_alerts: false,
            feat_agent_limit_alerts: false,
            feat_mcp_self_query: false,
            feat_rule_injection: false,
            feat_tuning_advice: false,
            feat_cache_experiment: false,
        }
    }
}

pub fn default_true() -> bool {
    true
}

/// The DLP mode an install that never chose one gets — the reporting mode, for
/// the reason on `SettingsVm::dlp_mode`.
pub fn default_dlp_mode() -> String {
    "alert".to_string()
}

/// Mirrors `kiwanod::store::StreamTimeouts::default`, which is what the gateway
/// falls back to when the settings row is absent. Kept in step by the round-trip
/// test below rather than by hoping.
pub fn default_stream_first_byte_secs() -> u32 {
    120
}

pub fn default_stream_idle_secs() -> u32 {
    120
}

// ── Settings ──

/// The Hub endpoint older builds shipped as the default. That host no longer
/// resolves, and `#[serde(default)]` cannot repair it: the value is already
/// stored, so the default never applies again. Anyone who ran a build from
/// before the domain change would keep failing to sync forever, and silently —
/// a failed sync only logs, and the shelf stays empty.
const LEGACY_HUB_URL: &str = "https://hub.kiwano.app/catalog.json";

/// Read UI settings (used by Rust-side logic like tray/autostart; the
/// store-free part of `build_settings`).
pub fn ui_settings(aux: &Aux) -> SettingsVm {
    let mut s: SettingsVm = aux
        .load_settings_json()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    // Heal the one value known to be a bygone default. An endpoint the user
    // chose — even a broken one — is left exactly as it is.
    if s.hub_url == LEGACY_HUB_URL {
        s.hub_url = default_hub_url();
    }
    s
}

pub fn build_settings(store: &Store, aux: &Aux, vars: &ShellVars) -> Result<SettingsVm, String> {
    build_settings_with_home(store, aux, &kiwano_adapters::config::get_home_dir(), vars)
}

/// [`build_settings`] against an explicit home directory.
///
/// Takeover state is read out of the agent's live config files, so the caller
/// has to say which tree to read. Production passes `$HOME`; tests pass a temp
/// root, which is what keeps "is claude taken over" from depending on whether
/// the developer running the suite happens to have taken claude over.
pub fn build_settings_with_home(
    store: &Store,
    aux: &Aux,
    home: &std::path::Path,
    vars: &ShellVars,
) -> Result<SettingsVm, String> {
    let mut s: SettingsVm = ui_settings(aux);
    // A takeover *is* its rewrite: the agent's own config carries our
    // placeholder key, which is also how its requests get attributed. Neither
    // of the other two possible claims counts here — not the `placeholder_keys`
    // table (a row can outlive its rewrite), and not the backup (it is the
    // escape hatch, and it outlives the rewrite by design). Counting the backup
    // answered "is this agent ours?" with "we could undo a takeover", which is
    // how an agent reads as taken over while its traffic goes straight to its
    // own provider — and its provider reads as in use.
    s.takeovers = AGENTS
        .iter()
        .map(|(agent, label)| {
            let key = crate::takeover::live_placeholder_key(agent, home, vars);
            TakeoverVm {
                agent: agent.to_string(),
                label: label.to_string(),
                // The key is read out of the file, never out of the table, so
                // what the UI offers to copy is what the agent actually sends.
                enabled: key.is_some(),
                placeholder_key: key,
                additive: ADDITIVE_AGENTS.contains(agent),
                protocols: agent_protocols(agent)
                    .iter()
                    .map(|p| (*p).to_string())
                    .collect(),
                config_paths: crate::takeover::takeover_paths(agent, home, vars)
                    .unwrap_or_default()
                    .iter()
                    .map(|p| display_path(p, home))
                    .collect(),
            }
        })
        .collect();
    // The user's own agents, with the key their traffic is attributed by. The
    // key comes out of the table here — where a built-in's comes out of its
    // config file — because for these there is no file to read: the row *is*
    // the agent, and it is deleted with it, so it cannot outlive anything.
    let keys = store.list_placeholder_keys().map_err(e2s)?;
    s.custom_agents = store
        .list_custom_agents()
        .map_err(e2s)?
        .into_iter()
        .map(|a| CustomAgentVm {
            placeholder_key: keys.iter().find(|k| k.agent == a.id).map(|k| k.key.clone()),
            id: a.id,
            label: a.label,
            note: a.note,
            protocol: a.protocol,
        })
        .collect();
    Ok(s)
}

pub fn update_settings(
    store: &Store,
    aux: &Aux,
    patch: &serde_json::Value,
    vars: &ShellVars,
) -> Result<SettingsVm, String> {
    let mut merged = aux
        .load_settings_json()
        .unwrap_or_else(|| serde_json::to_value(SettingsVm::default()).expect("default settings"));
    if let (Some(obj), Some(p)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            // takeovers are managed through set_agent_takeover, not this patch
            if k == "takeovers" {
                continue;
            }
            // preferred currency: normalize + validate the ISO code
            if k == "preferred_currency" {
                let Some(code) = v.as_str() else { continue };
                let code = code.trim().to_ascii_uppercase();
                if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                    return Err(format!("invalid currency code: {code}"));
                }
                obj.insert(k.clone(), serde_json::Value::String(code));
                continue;
            }
            obj.insert(k.clone(), v.clone());
        }
    }
    aux.save_settings_json(&merged).map_err(e2s)?;
    // Request-log capture config lives in the shared gateway_settings table:
    // the sidecar reads it at startup and on /reload, so keep both copies in
    // sync whenever the UI patches one of these keys.
    if patch.get("request_logs").is_some()
        || patch.get("log_retention_days").is_some()
        || patch.get("log_max_body_bytes").is_some()
    {
        let mut cfg = store.load_log_config().unwrap_or_default();
        if let Some(v) = patch.get("request_logs").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        // Zero is how the patch says "keep every row": the field is a count of
        // days, and the only count that means *no* retention is none of them.
        // Inside the config the two states stay distinct — `None` is a
        // decision, zero would be a bug — but a JSON patch has one number to
        // say it with, and `0` is the number every other time limit here uses.
        if let Some(v) = patch.get("log_retention_days").and_then(|v| v.as_u64()) {
            if let Ok(days) = u32::try_from(v) {
                cfg.retain_days = (days > 0).then_some(days);
            }
        }
        if let Some(v) = patch.get("log_max_body_bytes").and_then(|v| v.as_u64()) {
            if let Ok(bytes) = usize::try_from(v) {
                cfg.max_body_bytes = (bytes > 0).then_some(bytes);
            }
        }
        store.save_log_config(&cfg).map_err(e2s)?;
    }
    // The compat shim's switch moves the same way: one gateway_settings key
    // the sidecar reads at startup and on /reload.
    if patch.get("compat_shim").is_some() {
        let mut cfg = store.load_compat_shim_config().unwrap_or_default();
        if let Some(v) = patch.get("compat_shim").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        store.save_compat_shim_config(&cfg).map_err(e2s)?;
    }
    // The DLP mode moves the same way: one gateway_settings key the sidecar
    // reads at startup and on /reload. A mode rather than a flag, so the third
    // answer (block) does not need a new storage shape — and a value this build
    // does not recognize leaves the gateway on the mode it already had rather
    // than guessing at one.
    if patch.get("dlp_mode").is_some() {
        let mut cfg = store.load_dlp_config().unwrap_or_default();
        if let Some(mode) = patch
            .get("dlp_mode")
            .and_then(|v| v.as_str())
            .and_then(kiwanod::store::DlpMode::parse_str)
        {
            cfg.mode = mode;
        }
        store.save_dlp_config(&cfg).map_err(e2s)?;
    }
    // Same contract for the streaming timeouts: the sidecar reads them at
    // startup and on /reload, so the two copies move together.
    if patch.get("stream_first_byte_secs").is_some() || patch.get("stream_idle_secs").is_some() {
        let mut cfg = store.load_stream_timeouts().unwrap_or_default();
        if let Some(v) = patch.get("stream_first_byte_secs").and_then(|v| v.as_u64()) {
            cfg.first_byte_secs = v;
        }
        if let Some(v) = patch.get("stream_idle_secs").and_then(|v| v.as_u64()) {
            cfg.idle_secs = v;
        }
        store.save_stream_timeouts(&cfg).map_err(e2s)?;
    }
    build_settings(store, aux, vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::routes::display_path;
    use crate::vm::takeover::set_agent_takeover;
    use crate::vm::test_support::{no_vars, store};
    use crate::vm::Aux;
    use std::path::Path;

    #[test]
    fn legacy_takeover_backup_still_loads() {
        let aux = Aux::open_in_memory().unwrap();
        // Builds before the `existed` field stored a bare [path, content] array.
        let legacy =
            serde_json::json!([["/tmp/kiwano-test/settings.json", "{\"a\":1}"]]).to_string();
        aux.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO takeover_backups (agent, files, backed_up_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["claude", legacy, "2026-01-01T00:00:00Z"],
            )
            .unwrap();

        let (_, files) = aux
            .load_takeover_backup("claude")
            .expect("legacy backup loads");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/tmp/kiwano-test/settings.json");
        assert_eq!(files[0].content, "{\"a\":1}");
        // Read as "existed", so disabling still writes the content back instead
        // of deleting a file it cannot prove we created.
        assert!(files[0].existed);
    }

    #[test]
    fn legacy_hub_url_is_healed_on_load() {
        let aux = Aux::open_in_memory().unwrap();
        let mut v = serde_json::to_value(SettingsVm::default()).unwrap();

        // A settings blob written by a build from before the domain change.
        v["hub_url"] = serde_json::json!(LEGACY_HUB_URL);
        aux.save_settings_json(&v).unwrap();
        assert_eq!(ui_settings(&aux).hub_url, default_hub_url());

        // An endpoint the user picked is never rewritten, broken or not.
        v["hub_url"] = serde_json::json!("https://hub.example.com/catalog.json");
        aux.save_settings_json(&v).unwrap();
        assert_eq!(
            ui_settings(&aux).hub_url,
            "https://hub.example.com/catalog.json"
        );
    }

    // The app owns the streaming timeouts' UI and the gateway owns their
    // enforcement, and the two halves meet in `gateway_settings`: a patch has to
    // reach the sidecar's copy, not just this side's JSON, or the settings row
    // would look applied and change nothing. The defaults are asserted against
    // the gateway's own so the two cannot drift apart in silence.
    #[test]
    fn streaming_timeouts_round_trip_into_the_gateways_copy() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();

        assert_eq!(
            kiwanod::store::StreamTimeouts {
                first_byte_secs: u64::from(default_stream_first_byte_secs()),
                idle_secs: u64::from(default_stream_idle_secs()),
            },
            kiwanod::store::StreamTimeouts::default(),
            "the app's defaults and the gateway's must be the same numbers"
        );

        let vm = update_settings(
            &s,
            &aux,
            &serde_json::json!({ "stream_idle_secs": 45 }),
            &no_vars(),
        )
        .unwrap();
        assert_eq!(vm.stream_idle_secs, 45);

        let cfg = s.load_stream_timeouts().unwrap();
        assert_eq!(cfg.idle_secs, 45, "the sidecar's copy moved");
        assert_eq!(
            cfg.first_byte_secs,
            u64::from(default_stream_first_byte_secs()),
            "patching one leaves the other alone"
        );

        // And the screen reads back what it wrote.
        let reread = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert_eq!(reread.stream_idle_secs, 45);
    }

    /// The DLP mode is the same contract as the switches above: the UI's copy
    /// and the sidecar's blob have to move together, or the switch does nothing.
    #[test]
    fn the_dlp_mode_reaches_the_gateways_own_settings() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        assert_eq!(
            s.load_dlp_config().unwrap().mode,
            kiwanod::store::DlpMode::Alert,
            "an install that never chose a mode gets the reporting one"
        );

        let vm = update_settings(
            &s,
            &aux,
            &serde_json::json!({ "dlp_mode": "off" }),
            &no_vars(),
        )
        .unwrap();
        assert_eq!(vm.dlp_mode, "off");
        assert_eq!(
            s.load_dlp_config().unwrap().mode,
            kiwanod::store::DlpMode::Off,
            "the sidecar's copy moved"
        );
        assert_eq!(ui_settings(&aux).dlp_mode, "off", "and so did the UI's");
    }

    // The files a takeover would rewrite, as the agent's settings dialog lists
    // them. Read out of `takeover_paths` — the function the takeover itself writes
    // through — so the list cannot drift from what a takeover actually touches,
    // and printed with `~` because that is how the same paths read in the docs.
    #[test]
    fn the_takeover_vm_carries_the_files_a_takeover_rewrites() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        let vm = build_settings_with_home(&s, &aux, home, &no_vars()).unwrap();
        let paths = |agent: &str| {
            vm.takeovers
                .iter()
                .find(|t| t.agent == agent)
                .unwrap_or_else(|| panic!("{agent} is in the registry"))
                .config_paths
                .clone()
        };

        // Spelled with the platform's separator, because that is what the screen
        // should show and what the row's copy button should hand over. A literal
        // `~/.claude/settings.json` here is an assertion only Unix passes — which
        // is how this test first went red, on Windows, against a `display_path`
        // that glued `~/` to a Windows path.
        let sep = std::path::MAIN_SEPARATOR;
        let at_home = |tail: &str| format!("~{sep}{}", tail.replace('/', &sep.to_string()));
        assert_eq!(paths("claude"), vec![at_home(".claude/settings.json")]);
        // Two files: the config Codex reads and the auth it keeps beside it.
        assert_eq!(
            paths("codex"),
            vec![at_home(".codex/config.toml"), at_home(".codex/auth.json")]
        );
        assert_eq!(paths("grokbuild"), vec![at_home(".grok/config.toml")]);

        // Every built-in resolves to at least one file — except the one whose
        // takeover is macOS-only. `claude-desktop` comes back empty elsewhere by
        // design, and the row renders nothing for it rather than a path that does
        // not exist.
        for t in vm.takeovers.iter().filter(|t| t.agent != "claude-desktop") {
            assert!(!t.config_paths.is_empty(), "{} has no files", t.agent);
        }
        let desktop = vm
            .takeovers
            .iter()
            .find(|t| t.agent == "claude-desktop")
            .expect("claude-desktop is in the registry");
        assert_eq!(
            desktop.config_paths.is_empty(),
            cfg!(not(target_os = "macos")),
            "claude-desktop's files are macOS-only"
        );

        // A path outside the home tree keeps its absolute form rather than being
        // rewritten into a `~/…` that names nothing. Built the way the real ones
        // are — a component at a time — because a tail written as one literal with
        // forward slashes keeps them on Windows: `Path` separates on either, so the
        // raw text is what the caller wrote.
        assert_eq!(
            display_path(&home.join(".hermes").join("config.yaml"), home),
            at_home(".hermes/config.yaml")
        );
        assert_eq!(
            display_path(Path::new("/opt/hermes/config.yaml"), home),
            "/opt/hermes/config.yaml"
        );
    }

    #[test]
    fn settings_roundtrip_and_merge() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        // Every build reads this temp home, never the developer's real one:
        // takeover state comes from the live files, so `$HOME` would otherwise
        // decide what these assertions see.
        let tmp = tempfile::tempdir().unwrap();
        let v0 = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert_eq!(v0.language, "system");
        assert!(v0.takeovers.iter().all(|t| !t.enabled));

        let patch =
            serde_json::json!({ "language": "en", "telemetry": true, "takeovers": "ignored" });
        // The patch still carries `telemetry`, a key older blobs hold and
        // nothing reads any more: it must be ignored, not choke the merge.
        let v1 = update_settings(&s, &aux, &patch, &no_vars()).unwrap();
        assert_eq!(v1.language, "en");
        // The stored blob now holds a key the struct no longer declares. It has
        // to parse anyway: a failed parse falls back to every default at once,
        // which would silently reset the reader's whole settings page.
        let reread = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert_eq!(reread.language, "en", "an old key is ignored, not fatal");
        // takeovers untouched by patch (read against the temp home, like every
        // other assertion here — `update_settings` builds its answer against
        // the process home, which this test must not depend on)
        let after_patch = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert!(after_patch.takeovers.iter().all(|t| !t.enabled));

        // no ~/.claude/settings.json → rejected and no key left behind
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap_err();
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != "claude"));

        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        let v2 = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v2.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(claude.enabled);
        assert!(claude
            .placeholder_key
            .as_deref()
            .unwrap_or_default()
            .starts_with("kw-ag-claude-"));
        // config actually rewritten + restored via the escape hatch
        let env: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(env["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");
        let v3 = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert!(
            !v3.takeovers
                .iter()
                .find(|t| t.agent == "claude")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn takeover_state_comes_from_the_live_file_not_the_key_table() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();

        // The registration disappears (a rolled-back key row) while the config
        // stays rewritten: the reader must still report the takeover, because
        // that is the state the user has to be able to undo.
        for k in s.list_placeholder_keys().unwrap() {
            if k.agent == "claude" {
                s.delete_placeholder_key(&k.key).unwrap();
            }
        }
        let v = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(
            claude.enabled,
            "the live config still points at the gateway"
        );
        // …and the key is read out of the file rather than out of the table.
        assert!(claude
            .placeholder_key
            .as_deref()
            .unwrap_or_default()
            .starts_with("kw-ag-claude-"));

        // The reverse: a key row with no rewritten file and no backup is not a
        // takeover.
        std::fs::write(&settings, "{}").unwrap();
        aux.delete_takeover_backup("claude").unwrap();
        s.upsert_placeholder_key("kw-ag-claude-orphan", "claude")
            .unwrap();
        let v = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(
            !claude.enabled,
            "a key with no rewritten config (and no backup) is not a takeover"
        );
        assert!(claude.placeholder_key.is_none());
    }
}
