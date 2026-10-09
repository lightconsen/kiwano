//! Config sharing (spec §4.1 P1): one-click export/import of a config scheme.
//!
//! File format kiwano-config v1:
//! `providers` (gateway Provider rows) + `routes` (per-Agent strategy and
//! candidate order). Credentials are **omitted by default**: `api_key` is
//! only written when the caller opts in (`export_config(store, true)`), for
//! the local-backup / cross-device migration case where the file never leaves
//! the machine. An export without keys still imports cleanly — matching is by
//! `(name, base_url)`, and a key is only backfilled into a local row that has
//! none.
//!
//! Import semantics (merge, not overwrite): match existing providers by
//! `(name, base_url)` — on a hit the local row is kept and the key is only
//! backfilled when the local row has no key and the scheme carries one; on a
//! miss a new provider is created (id regenerated to avoid clashing with
//! local ids). Bindings are remapped from exported id → final id and
//! upserted in order (priority = order); strategy rows are upserted
//! directly; bindings of untouched agents are left alone. The caller is
//! responsible for triggering admin /reload.

use kiwanod::store::Store;

pub const FORMAT_VERSION: u32 = 1;

/// Export all providers and per-Agent route schemes as shareable JSON.
///
/// `include_keys` is the explicit opt-in for credentials. It is `false` on the
/// registered `export_config` IPC command — that surface is dormant (no
/// frontend screen calls it) and must not leak when it is finally wired up.
/// Pass `true` only for a local backup that never leaves the machine.
/// Write an export to `path`, owner-only on unix.
///
/// The file holds plaintext credentials when `include_keys` is set, and
/// `std::fs::write` creates it with whatever the umask allows — which on a
/// shared machine can be world-readable. The database this is a backup of is
/// already 0600 (see `crate::store`'s `harden_permissions`); the export should
/// not be the thing that undoes it.
///
/// Returns the number of providers written.
/// Write an export to `path`, owner-only on unix.
///
/// The file holds plaintext credentials when `include_keys` was set, and
/// `std::fs::write` creates it with whatever the umask allows — which on a
/// shared machine can be world-readable. The database this is a backup of is
/// already 0600 (see `crate::store`'s `harden_permissions`); the export should
/// not be the thing that undoes it.
///
/// The document comes from the daemon; what is here is the writing, and the
/// permissions it lands with.
pub fn write_config_file(path: &str, json: &str) -> Result<(), String> {
    std::fs::write(path, json).map_err(|e| format!("cannot write {path}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// The shareable JSON — built by the daemon (`kiwanod::api::share::export_config`):
/// the providers and routes are its rows, and it is the side that knows what a
/// complete document is. The **file** is the client's, which is why
/// `export_config_to_file` below stays.
pub fn export_config(store: &Store, include_keys: bool) -> Result<String, String> {
    kiwanod::api::share::export_config(store, include_keys).map_err(|e| e.to_string())
}

/// Import a scheme — served by the daemon. The client reads the file and hands
/// over its text; everything that touches the store happens on the far side.
pub fn import_config(store: &Store, json: &str) -> Result<ImportReport, String> {
    kiwanod::api::share::import_config(store, json).map_err(|e| e.to_string())
}

// The report travels with the function that produces it.
pub use kiwanod::api::share::ImportReport;

/// How many providers a document carries — the count the export command
/// reports. Read out of the document rather than the store: the two agree, and
/// the document is the thing that was actually written.
pub fn provider_count(json: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("providers")?.as_array().map(Vec::len))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwanod::store::{Billing, Protocol};
    use kiwanod::store::{Binding, Provider, StrategyType};

    fn provider(id: &str, name: &str, base_url: &str, api_key: Option<&str>) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: Protocol::OpenAI,
            base_url: base_url.into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: api_key.map(Into::into),
            model_default: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: now_stamp(),
            updated_at: now_stamp(),
        }
    }

    fn now_stamp() -> String {
        kiwanod::store::now_rfc3339()
    }

    #[test]
    fn export_roundtrip_and_merge_by_identity() {
        let src = Store::open_in_memory().unwrap();
        src.insert_provider(&provider(
            "p1",
            "Alpha",
            "https://a.example.com",
            Some("sk-a"),
        ))
        .unwrap();
        src.insert_provider(&provider(
            "p2",
            "Beta",
            "https://b.example.com",
            Some("sk-b"),
        ))
        .unwrap();
        src.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (pid, pr) in [("p1", 0), ("p2", 1)] {
            src.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: pid.into(),
                priority: pr,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let json = export_config(&src, true).unwrap();
        assert!(json.contains("kiwano_config"));

        // Import into an empty DB → everything is created fresh + route remapping takes effect
        let dst = Store::open_in_memory().unwrap();
        let report = import_config(&dst, &json).unwrap();
        assert_eq!(report.providers_added, 2);
        assert_eq!(report.routes_applied, 1);
        assert_eq!(dst.list_providers().unwrap().len(), 2);
        assert!(dst.primary_provider_id("claude").unwrap().is_some());
        let bs = dst.bindings_for_agent("claude").unwrap();
        assert_eq!(bs.len(), 2);
        assert_eq!(
            dst.get_strategy("claude").unwrap().unwrap().kind,
            StrategyType::Failover
        );

        // Import with a local same-name/same-endpoint provider (no key) → keep local + backfill key
        let dst2 = Store::open_in_memory().unwrap();
        dst2.insert_provider(&provider("local-1", "Alpha", "https://a.example.com", None))
            .unwrap();
        let report2 = import_config(&dst2, &json).unwrap();
        assert_eq!(report2.providers_kept, 1);
        assert_eq!(report2.providers_added, 1);
        let local = dst2.get_provider("local-1").unwrap().unwrap();
        assert_eq!(local.api_key.as_deref(), Some("sk-a"));
        // claude primary maps to the local id
        assert_eq!(
            dst2.primary_provider_id("claude").unwrap().as_deref(),
            Some("local-1")
        );
    }

    #[test]
    fn import_rejects_garbage_and_unknown_versions() {
        let s = Store::open_in_memory().unwrap();
        assert!(import_config(&s, "not json").is_err());
        assert!(import_config(&s, r#"{"kiwano_config": 99}"#).is_err());
    }

    /// A store with two keyed providers and one bound agent.
    fn seeded_store() -> Store {
        let src = Store::open_in_memory().unwrap();
        src.insert_provider(&provider(
            "p1",
            "Alpha",
            "https://a.example.com",
            Some("sk-a"),
        ))
        .unwrap();
        src.insert_provider(&provider(
            "p2",
            "Beta",
            "https://b.example.com",
            Some("sk-b"),
        ))
        .unwrap();
        src.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (pid, pr) in [("p1", 0), ("p2", 1)] {
            src.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: pid.into(),
                priority: pr,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        src
    }

    #[test]
    fn export_omits_credentials_unless_opted_in() {
        let src = seeded_store();

        // Default: no key material anywhere in the file.
        let json = export_config(&src, false).unwrap();
        assert!(!json.contains("sk-a"), "{json}");
        assert!(!json.contains("sk-b"), "{json}");
        // Provider identity and routes still travel.
        assert!(json.contains("Alpha"));
        assert!(json.contains("https://b.example.com"));
        assert!(json.contains("claude"));

        // Opt-in (local backup): keys are present and round-trip.
        let json = export_config(&src, true).unwrap();
        assert!(json.contains("sk-a"));
        assert!(json.contains("sk-b"));

        let dst = Store::open_in_memory().unwrap();
        import_config(&dst, &json).unwrap();
        let by_name = |name: &str| {
            dst.list_providers()
                .unwrap()
                .into_iter()
                .find(|p| p.name == name)
                .unwrap()
                .api_key
        };
        assert_eq!(by_name("Alpha").as_deref(), Some("sk-a"));
        assert_eq!(by_name("Beta").as_deref(), Some("sk-b"));
    }

    #[test]
    fn keyless_export_still_merges() {
        let json = export_config(&seeded_store(), false).unwrap();

        // Fresh DB: rows are created, routes remap, keys are simply absent.
        let dst = Store::open_in_memory().unwrap();
        let report = import_config(&dst, &json).unwrap();
        assert_eq!(report.providers_added, 2);
        assert_eq!(report.routes_applied, 1);
        assert_eq!(dst.bindings_for_agent("claude").unwrap().len(), 2);
        assert!(dst.list_providers().unwrap().iter().all(|p| p
            .api_key
            .as_deref()
            .unwrap_or("")
            .is_empty()));

        // Existing local row *with* a key: the import must not clear it.
        let dst2 = Store::open_in_memory().unwrap();
        dst2.insert_provider(&provider(
            "local-1",
            "Alpha",
            "https://a.example.com",
            Some("sk-local"),
        ))
        .unwrap();
        let report2 = import_config(&dst2, &json).unwrap();
        assert_eq!(report2.providers_kept, 1);
        assert_eq!(report2.providers_added, 1);
        assert_eq!(
            dst2.get_provider("local-1")
                .unwrap()
                .unwrap()
                .api_key
                .as_deref(),
            Some("sk-local")
        );
        assert_eq!(
            dst2.primary_provider_id("claude").unwrap().as_deref(),
            Some("local-1")
        );
    }
}
