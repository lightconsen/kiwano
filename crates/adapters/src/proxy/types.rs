// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/types.rs
// Copied on 2026-09-07. Modified for Kiwano (only the `OptimizerConfig` block
// was carried over — it is the sole `types.rs` item cache_injector.rs needs).

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// Request optimizer config
///
/// Stored in the settings table, key = "optimizer_config".
/// Only effective for Bedrock providers (CLAUDE_CODE_USE_BEDROCK = "1")
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerConfig {
    /// Master switch (off by default; users must opt in)
    #[serde(default)]
    pub enabled: bool,
    /// Thinking optimizer sub-switch (defaults on once the master switch is enabled)
    #[serde(default = "default_true")]
    pub thinking_optimizer: bool,
    /// Cache injection sub-switch (defaults on once the master switch is enabled)
    #[serde(default = "default_true")]
    pub cache_injection: bool,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            thinking_optimizer: true,
            cache_injection: true,
        }
    }
}
