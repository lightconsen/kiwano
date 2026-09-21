//! Auth binding: where a provider's credential is read from, and the managed
//! account it is bound to.

use serde::{Deserialize, Serialize};

/// Auth binding source
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthBindingSource {
    /// Read credentials from the provider's own config (default)
    #[default]
    ProviderConfig,
    /// Use managed-account auth (e.g. GitHub Copilot OAuth)
    ManagedAccount,
}

/// Generic auth binding
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthBinding {
    /// Auth source
    #[serde(default)]
    pub source: AuthBindingSource,
    /// Managed auth provider identifier (e.g. github_copilot)
    #[serde(rename = "authProvider", skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    /// Managed account ID; empty means follow the auth provider's default account
    #[serde(rename = "accountId", skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}
