//! The GUI's own KV store (`app_settings`), the one `app_settings` blob the
//! gateway reads a field out of, and the Hub's cached exchange rates.
//!
//! `app_settings` is written by the GUI alone, except for the plan-quota
//! cache, which both processes keep current — the gateway enforces the
//! ceilings it feeds, and two caches could disagree about a provider's
//! utilization.

use crate::error::Result;
use crate::store::Store;
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `app_settings` key prefix for a directory a user declared for a built-in
/// agent (`detect_dir:<agent id>`). One row per agent; see
/// [`Store::manual_agent_dirs`].
const MANUAL_AGENT_DIR_PREFIX: &str = "detect_dir:";

/// The daemon's own copy of the user's UTC offset, in minutes east of UTC.
///
/// Split out of the app's `ui` blob because the daemon is what needs it
/// (`migrate.local.md` §9.2.1): quota period boundaries are the user's days.
const TZ_OFFSET_KEY: &str = "tz_offset_minutes";

impl Store {
    // ── The GUI's KV (`app_settings`) ──
    //
    // Both processes open this one file, and `app_settings` used to be read and
    // written by the GUI alone. The plan-quota cache now lives there under both:
    // the gateway enforces the ceilings it feeds, so if each side fetched and
    // cached separately they could disagree about a provider's utilization —
    // and a provider would be blocked by one process and served by the other.

    /// One `app_settings` value. Absent or unreadable reads as absent.
    pub fn app_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    /// This install's identity, minting one if the database has none.
    ///
    /// Idempotent by design: every start reads the same row back, so the value
    /// means "this database", not "this process". That is the whole point — a
    /// second value is how two databases are told apart.
    pub fn ensure_install_id(&self) -> Result<String> {
        const KEY: &str = "gateway.install_id";
        if let Some(existing) = self.gateway_setting(KEY) {
            return Ok(existing);
        }
        let minted = uuid::Uuid::new_v4().to_string();
        self.set_gateway_setting(KEY, &minted)?;
        Ok(minted)
    }

    /// One of the gateway's own keys. `gateway_settings` is the daemon's KV —
    /// the log capture, the compat shim and the stream timeouts live there as
    /// JSON blobs — and this is the plain (key, value) pair for a value that is
    /// not a blob.
    pub fn gateway_setting(&self, key: &str) -> Option<String> {
        self.conn
            .lock()
            .expect("store mutex poisoned")
            .query_row(
                "SELECT value FROM gateway_settings WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_gateway_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO gateway_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn set_app_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    /// Every `app_settings` row whose key starts with `prefix`, as
    /// (key, value). For the one-time sweeps that have to find keys they did
    /// not write themselves.
    pub fn app_settings_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare("SELECT key, value FROM app_settings WHERE key LIKE ?1")?;
        let rows = stmt
            .query_map(params![format!("{prefix}%")], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_app_setting(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        Ok(conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])? > 0)
    }

    /// The install directory a user declared for a built-in agent the detector
    /// could not find on its own, keyed by agent id.
    ///
    /// One row per agent rather than a JSON blob: the value is a path, the key
    /// names its owner, and a row that cannot be read can only ever lose
    /// itself. Nothing here fails — a declaration is a hint, and a store that
    /// cannot read one behaves exactly like a store that never had it.
    pub fn manual_agent_dirs(&self) -> std::collections::BTreeMap<String, PathBuf> {
        let Ok(rows) = self.app_settings_with_prefix(MANUAL_AGENT_DIR_PREFIX) else {
            return Default::default();
        };
        rows.into_iter()
            .filter_map(|(key, value)| {
                let agent = key.strip_prefix(MANUAL_AGENT_DIR_PREFIX)?;
                let dir = value.trim();
                (!agent.is_empty() && !dir.is_empty())
                    .then(|| (agent.to_string(), PathBuf::from(dir)))
            })
            .collect()
    }

    /// Record an agent's declared directory, replacing whatever was there.
    pub fn set_manual_agent_dir(&self, agent: &str, dir: &Path) -> Result<()> {
        self.set_app_setting(
            &format!("{MANUAL_AGENT_DIR_PREFIX}{agent}"),
            &dir.to_string_lossy(),
        )
    }

    /// Forget it. `true` when there was one to forget.
    pub fn clear_manual_agent_dir(&self, agent: &str) -> Result<bool> {
        self.delete_app_setting(&format!("{MANUAL_AGENT_DIR_PREFIX}{agent}"))
    }

    /// The user's UTC offset in minutes east, from the `ui` blob the GUI keeps
    /// current. Reset periods follow the user's clock — a monthly limit rolls
    /// over at their midnight, not UTC's — so the enforcement side needs it.
    /// Absent means UTC, which is what an older settings blob yields.
    pub fn ui_tz_offset_minutes(&self) -> i64 {
        // **Its own key first, the `ui` blob second** (`migrate.local.md`
        // §9.2.1): the offset is moving out of the app's blob because the daemon
        // is the side that needs it — every quota period boundary is measured in
        // the user's day, not UTC's — and a daemon reading a blob the app owns
        // is the arrangement being retired.
        //
        // The blob stays as the fallback for as long as an app writes only that:
        // §9.2.2's warning is exactly this field, because the app writes the blob
        // *whole*, so a daemon that read only its own key would stop seeing a
        // timezone the user had just changed — silently, as a period resetting
        // at the wrong hour.
        if let Some(own) = self
            .gateway_setting(TZ_OFFSET_KEY)
            .and_then(|v| v.trim().parse::<i64>().ok())
        {
            return own;
        }
        self.app_setting("ui")
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.get("tz_offset_minutes").and_then(|t| t.as_i64()))
            .unwrap_or(0)
    }

    /// Keep the daemon's own copy of the UTC offset in step with the blob the
    /// app writes. Called wherever the offset can change (`migrate.local.md`
    /// §9.2.2's hazard, and the only reason this key exists at all).
    pub fn set_ui_tz_offset_minutes(&self, minutes: i64) -> Result<()> {
        self.set_gateway_setting(TZ_OFFSET_KEY, &minutes.to_string())
    }

    /// The Hub's exchange rates, as `models.json` publishes them
    /// (`rates[currency]` = units per 1 USD; USD pivots).
    ///
    /// Read from the cached document rather than from the price mirror, which
    /// stores the rows but not the rates above them. This is the same row the
    /// GUI seeds prices from, so the two never disagree about what a dollar is
    /// worth.
    ///
    /// Empty when nothing has been synced, or when the cached payload cannot be
    /// read. `convert_amount` then passes each amount through unchanged, which
    /// is the same answer the GUI gives on an install that has never synced —
    /// and one that has no priced usage to convert anyway.
    /// The whole `ui` settings blob, as JSON, or `None` when it has not been
    /// written yet. The daemon's settings paths read and patch it whole — the
    /// same row the app's `Aux::load_settings_json` returned.
    pub fn settings_json(&self) -> Option<serde_json::Value> {
        self.app_setting("ui")
            .and_then(|raw| serde_json::from_str(&raw).ok())
    }

    /// Write the whole `ui` settings blob.
    pub fn save_settings_json(&self, value: &serde_json::Value) -> Result<()> {
        self.set_app_setting("ui", &value.to_string())?;
        Ok(())
    }

    /// The user's `hub_url` from the `ui` settings blob, or the default when
    /// the blob has not been written yet. This is the same `app_settings` KV the
    /// daemon already reads its own settings from, so the sync — when it moves —
    /// resolves the Hub's address from the same row the app does.
    pub fn ui_hub_url(&self) -> Option<String> {
        self.app_setting("ui")
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.get("hub_url")?.as_str().map(str::to_string))
    }

    pub fn hub_exchange_rates(&self) -> HashMap<String, f64> {
        let payload: Option<String> = {
            let conn = self.conn.lock().expect("store mutex poisoned");
            conn.query_row(
                "SELECT payload FROM hub_models_cache WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
        };
        payload
            .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
            .and_then(|doc| {
                Some(
                    doc.get("exchange_rates")?
                        .as_object()?
                        .iter()
                        .filter_map(|(code, per_usd)| Some((code.clone(), per_usd.as_f64()?)))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::temp_store;

    #[test]
    fn manual_agent_dirs_round_trip_and_lose_only_their_own_bad_rows() {
        let (_dir, store) = temp_store();
        assert!(store.manual_agent_dirs().is_empty());

        store
            .set_manual_agent_dir("gemini", Path::new("/opt/custom/bin"))
            .unwrap();
        store
            .set_manual_agent_dir("qwen", Path::new("/srv/qwen/bin"))
            .unwrap();
        let dirs = store.manual_agent_dirs();
        assert_eq!(dirs.get("gemini"), Some(&PathBuf::from("/opt/custom/bin")));
        assert_eq!(dirs.get("qwen"), Some(&PathBuf::from("/srv/qwen/bin")));

        // Replacing one leaves its neighbour where it was.
        store
            .set_manual_agent_dir("gemini", Path::new("/elsewhere"))
            .unwrap();
        let dirs = store.manual_agent_dirs();
        assert_eq!(dirs.get("gemini"), Some(&PathBuf::from("/elsewhere")));
        assert_eq!(dirs.get("qwen"), Some(&PathBuf::from("/srv/qwen/bin")));

        // A row with nothing usable in it drops out; the others stand.
        store
            .set_app_setting(&format!("{MANUAL_AGENT_DIR_PREFIX}broken"), "   ")
            .unwrap();
        let dirs = store.manual_agent_dirs();
        assert!(!dirs.contains_key("broken"));
        assert!(dirs.contains_key("qwen"));

        // Clearing reports whether there was one, and is idempotent after.
        assert!(store.clear_manual_agent_dir("gemini").unwrap());
        assert!(!store.clear_manual_agent_dir("gemini").unwrap());
        assert!(!store.manual_agent_dirs().contains_key("gemini"));
    }
}
