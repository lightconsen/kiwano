//! The custom-endpoint entry `ProviderMeta` keeps, deduplicated by URL.

use serde::{Deserialize, Serialize};

/// Custom endpoint entry (stored deduplicated by URL).
///
/// Kiwano: inlined from cc-switch `crate::settings::CustomEndpoint` so this
/// crate does not depend on the settings module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEndpoint {
    pub url: String,
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
}
