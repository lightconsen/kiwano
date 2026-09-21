//! The `gateway_settings` blobs: the log capture, compat shim and stream
//! timeout configurations the GUI writes and the gateway reads.
//!
//! Each type sits beside the key that holds it and the load/save pair that
//! reads and writes it, because all three are one contract: a blob the GUI
//! may write while the gateway is running, where a missing or corrupt value
//! reads back as the default rather than as an error.

use crate::error::Result;
use crate::store::Store;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Gateway-visible log capture configuration (`gateway_settings` JSON blob).
/// The GUI writes it; the gateway reads it at startup and on /reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogConfig {
    /// Master switch: capture data-plane requests at all. There is no second
    /// switch for bodies — capture means the whole request, bodies included,
    /// and the CSV export is where a body can be left out of a file.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Rows older than this many days are pruned. `None` keeps every row:
    /// capture is the point of the feature, and a retention nobody chose is a
    /// silent cap on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retain_days: Option<u32>,
    /// Per-body capture cap in bytes; larger bodies are stored truncated.
    /// `None` stores every byte that arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_body_bytes: Option<usize>,
}

fn default_true() -> bool {
    true
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            enabled: true,
            // Both limits are the user's to impose, not ours to guess: the
            // reason to capture a request is to be able to look at it later,
            // and a body cut at four megabytes or a row deleted after thirty
            // days is that look failing. A stored configuration still wins —
            // this is what an install without one gets.
            retain_days: None,
            max_body_bytes: None,
        }
    }
}

/// `gateway_settings` key holding the serialized `LogConfig`.
pub const LOG_CONFIG_KEY: &str = "request_logs";

/// The compat shim's configuration (`gateway_settings` JSON blob). One switch:
/// the rules themselves are built in and deliberately not configurable
/// one-by-one — each is narrow enough that the honest choices are "on" and
/// "off", and a matrix of toggles nobody asked for is just places to be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatShimConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for CompatShimConfig {
    fn default() -> Self {
        CompatShimConfig { enabled: true }
    }
}

/// `gateway_settings` key holding the serialized `CompatShimConfig`.
pub const COMPAT_SHIM_CONFIG_KEY: &str = "compat_shim";

/// How long an upstream stream may go without producing anything
/// (`gateway_settings` JSON blob, [`STREAM_TIMEOUTS_KEY`]).
///
/// `provider.timeout_secs` bounds only the wait for response *headers*. A
/// provider that returns its headers and then goes quiet had no limit at all: the
/// client sat on the open stream until its own 300s read timeout, which is a hang
/// reported as a timeout rather than as the failure it is.
///
/// Both are 0-disabled, because a cold local model legitimately takes a long time
/// to its first token — and a limit nobody can raise would be worse than none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamTimeouts {
    /// How long the upstream may take to send the stream's first byte, counted
    /// from the response's headers.
    #[serde(default = "default_first_byte_secs")]
    pub first_byte_secs: u64,
    /// How long a stream may stay silent between bytes before it is abandoned.
    #[serde(default = "default_idle_secs")]
    pub idle_secs: u64,
}

fn default_first_byte_secs() -> u64 {
    120
}

fn default_idle_secs() -> u64 {
    120
}

impl Default for StreamTimeouts {
    fn default() -> Self {
        StreamTimeouts {
            first_byte_secs: default_first_byte_secs(),
            idle_secs: default_idle_secs(),
        }
    }
}

impl StreamTimeouts {
    /// The deadline for the wait that has not been satisfied yet: the first byte,
    /// or the next byte.
    pub fn deadline(&self, first_byte_seen: bool) -> Option<Duration> {
        let secs = if first_byte_seen {
            self.idle_secs
        } else {
            self.first_byte_secs
        };
        (secs > 0).then(|| Duration::from_secs(secs))
    }

    /// True while neither limit is armed, so the stream can skip the timer.
    pub fn disabled(&self) -> bool {
        self.first_byte_secs == 0 && self.idle_secs == 0
    }
}

/// `gateway_settings` key holding the serialized `StreamTimeouts`.
pub const STREAM_TIMEOUTS_KEY: &str = "streaming";

impl Store {
    /// Load the log capture config (defaults when the key is absent/corrupt).
    pub fn load_log_config(&self) -> Result<LogConfig> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM gateway_settings WHERE key = ?1",
                params![LOG_CONFIG_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match value {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => LogConfig::default(),
        })
    }

    /// Persist the log capture config (also used by the GUI via its own
    /// connection — same table, so keep the key/format in sync).
    pub fn save_log_config(&self, cfg: &LogConfig) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let json = serde_json::to_string(cfg)?;
        conn.execute(
            "INSERT INTO gateway_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![LOG_CONFIG_KEY, json],
        )?;
        Ok(())
    }

    /// Load the compat shim config (defaults when the key is absent/corrupt).
    pub fn load_compat_shim_config(&self) -> Result<CompatShimConfig> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM gateway_settings WHERE key = ?1",
                params![COMPAT_SHIM_CONFIG_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match value {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => CompatShimConfig::default(),
        })
    }

    /// Persist the compat shim config (same contract as the log config).
    pub fn save_compat_shim_config(&self, cfg: &CompatShimConfig) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let json = serde_json::to_string(cfg)?;
        conn.execute(
            "INSERT INTO gateway_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![COMPAT_SHIM_CONFIG_KEY, json],
        )?;
        Ok(())
    }

    /// Streaming timeouts, same contract as the log config: the GUI writes, the
    /// gateway reads at startup and on `/reload`.
    pub fn load_stream_timeouts(&self) -> Result<StreamTimeouts> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM gateway_settings WHERE key = ?1",
                params![STREAM_TIMEOUTS_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match value {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => StreamTimeouts::default(),
        })
    }

    pub fn save_stream_timeouts(&self, cfg: &StreamTimeouts) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let json = serde_json::to_string(cfg)?;
        conn.execute(
            "INSERT INTO gateway_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![STREAM_TIMEOUTS_KEY, json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::temp_store;

    #[test]
    fn log_config_tolerates_a_stored_blob_from_before_the_switch_was_removed() {
        // A `capture_bodies` key left in `gateway_settings` by an older build
        // is an unknown field now: ignored, not a parse failure that would
        // reset the whole config to defaults.
        let (_dir, store) = temp_store();
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO gateway_settings (key, value) VALUES (?1, ?2)",
            params![
                LOG_CONFIG_KEY,
                r#"{"enabled":false,"capture_bodies":true,"retain_days":7,"max_body_bytes":1024}"#
            ],
        )
        .unwrap();
        drop(conn);
        assert_eq!(
            store.load_log_config().unwrap(),
            LogConfig {
                enabled: false,
                retain_days: Some(7),
                max_body_bytes: Some(1024),
            }
        );
    }

    #[test]
    fn log_config_roundtrip_with_defaults() {
        let (_dir, store) = temp_store();
        assert_eq!(store.load_log_config().unwrap(), LogConfig::default());

        store
            .save_log_config(&LogConfig {
                enabled: false,
                retain_days: Some(7),
                max_body_bytes: Some(1024),
            })
            .unwrap();
        assert_eq!(
            store.load_log_config().unwrap(),
            LogConfig {
                enabled: false,
                retain_days: Some(7),
                max_body_bytes: Some(1024)
            }
        );

        // Corrupt JSON falls back to defaults instead of breaking the gateway.
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE gateway_settings SET value = 'not-json' WHERE key = ?1",
            params![LOG_CONFIG_KEY],
        )
        .unwrap();
        drop(conn);
        assert_eq!(store.load_log_config().unwrap(), LogConfig::default());
    }
}
