//! The settings screen: the `app_settings` row as a `SettingsVm`, the defaults
//! that fill in what an older row never stored, and the timezone offset the
//! rest of the module reads through `tz_offset`.

use crate::detect::ShellVars;
use crate::vm::agents::{agent_protocols, CustomAgentVm, ADDITIVE_AGENTS, AGENTS};
use crate::vm::routes::display_path;
use crate::vm::{e2s, Aux};
use kiwanod::store::Store;

/// The stored UTC offset (minutes east of UTC) the frontend keeps current.
pub(crate) fn tz_offset(store: &Store) -> i64 {
    // The blob comes from the store — the same row `update_settings` patches —
    // so a day boundary the user just set is the one the dashboard uses.
    kiwanod::api::settings::ui_settings(store)
        .map(|s| s.tz_offset_minutes)
        .unwrap_or(0)
}

// The blob and its types moved to `kiwano-api` — the daemon serves them.
pub use kiwano_api::settings::{
    default_dlp_mode, default_hub_url, default_preferred_currency, default_stream_first_byte_secs,
    default_stream_idle_secs, default_true, SettingsVm, TakeoverVm,
};

/// The settings blob, read from the store — the row the daemon patches.
///
/// It used to read the blob out of the **aux** connection. Nothing writes that
/// blob any more, so the callers still asking for it (the tray's close
/// behaviour, `kiwano mcp`, two CLI features) were reading a value no one had
/// written; the aux-shaped function is gone rather than left as a trap
/// (`migrate.local.md` §10.25).
pub fn ui_settings(store: &Store) -> SettingsVm {
    kiwanod::api::settings::ui_settings(store).unwrap_or_default()
}

// ── Settings ──

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
    _aux: &Aux,
    home: &std::path::Path,
    vars: &ShellVars,
) -> Result<SettingsVm, String> {
    // The blob comes from the store — the same row the daemon patches — so a
    // layered answer agrees with the write it follows. The `aux` is still here
    // for the ui-settings *healing* an older build wrote, which only the client
    // knows about; its own blob is not.
    let mut s: SettingsVm =
        kiwanod::api::settings::ui_settings(store).map_err(|e| e.to_string())?;
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

/// Patch the settings — served by the daemon
/// (`kiwanod::api::settings::update_settings`). The takeovers and custom agents
/// are layered on by `build_settings` below, because they describe this machine.
pub fn update_settings(
    store: &Store,
    aux: &Aux,
    patch: &serde_json::Value,
    vars: &ShellVars,
) -> Result<SettingsVm, String> {
    kiwanod::api::settings::update_settings(store, patch).map_err(|e| e.to_string())?;
    // The blob the daemon wrote is the store's, so the layered answer is built
    // from it too — reading the patch back out of an `aux` the daemon never
    // wrote to would answer with the settings as they were before the patch.
    build_settings_with_home(store, aux, &kiwano_adapters::config::get_home_dir(), vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::routes::display_path;
    use crate::vm::takeover::set_agent_takeover;
    use crate::vm::takeover::StateHalf;
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
        let s = store();
        let mut v = serde_json::to_value(SettingsVm::default()).unwrap();

        // A settings blob written by a build from before the domain change.
        // Read through the **store**, which is where the daemon patches it and
        // therefore the only row any reader should look at.
        v["hub_url"] = serde_json::json!(kiwano_api::settings::LEGACY_HUB_URL);
        s.save_settings_json(&v).unwrap();
        assert_eq!(ui_settings(&s).hub_url, default_hub_url());

        // An endpoint the user picked is never rewritten, broken or not.
        v["hub_url"] = serde_json::json!("https://hub.example.com/catalog.json");
        s.save_settings_json(&v).unwrap();
        assert_eq!(
            ui_settings(&s).hub_url,
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
        assert_eq!(
            kiwanod::api::settings::ui_settings(&s).unwrap().dlp_mode,
            "off",
            "and so did the UI's"
        );
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

        // Every built-in resolves to at least one file — except the two whose
        // files exist only under conditions this machine may not meet.
        // `claude-desktop`'s takeover is macOS-only, and `hanaagent`'s provider
        // catalog is written by the app on its first run: until then there is
        // no file to point at, and the row renders nothing rather than a path
        // that does not exist (the takeover refuses, with the way out in the
        // message).
        for t in vm
            .takeovers
            .iter()
            .filter(|t| t.agent != "claude-desktop" && t.agent != "hanaagent")
        {
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
        set_agent_takeover(
            &s,
            &aux,
            "claude",
            true,
            8317,
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess,
        )
        .unwrap_err();
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != "claude"));

        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(
            &s,
            &aux,
            "claude",
            true,
            8317,
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess,
        )
        .unwrap();
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
        set_agent_takeover(
            &s,
            &aux,
            "claude",
            false,
            8317,
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess,
        )
        .unwrap();
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
        set_agent_takeover(
            &s,
            &aux,
            "claude",
            true,
            8317,
            tmp.path(),
            &no_vars(),
            StateHalf::InProcess,
        )
        .unwrap();

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
