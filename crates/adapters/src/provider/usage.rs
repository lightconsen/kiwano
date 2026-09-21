//! Usage queries: the script config that drives them, and the results they
//! hand back.

use serde::{Deserialize, Serialize};

/// Usage query script config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageScript {
    pub enabled: bool,
    pub language: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// API key dedicated to usage queries (used by the generic template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    /// Base URL dedicated to usage queries (used by the generic and NewAPI templates)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    /// Access token (for endpoints that require login; used by the NewAPI template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessToken")]
    pub access_token: Option<String>,
    /// User ID (for endpoints that need a user identifier; used by the NewAPI template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    /// Template type (lets the backend decide validation rules)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "templateType")]
    pub template_type: Option<String>,
    /// Auto-query interval (minutes; 0 disables auto queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "autoQueryInterval")]
    pub auto_query_interval: Option<u64>,
    /// Coding Plan provider identifier (e.g. "kimi", "zhipu", "minimax")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "codingPlanProvider")]
    pub coding_plan_provider: Option<String>,
    /// Volcano Ark control-plane OpenAPI AccessKey ID (signs usage queries; separate credential from the inference key)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessKeyId")]
    pub access_key_id: Option<String>,
    /// Volcano Ark control-plane OpenAPI SecretAccessKey (same as above)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "secretAccessKey")]
    pub secret_access_key: Option<String>,
    /// Zhipu Team Plan organization ID (usage query header bigmodel-organization)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamOrganizationId")]
    pub team_organization_id: Option<String>,
    /// Zhipu Team Plan project ID (usage query header bigmodel-project)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamProjectId")]
    pub team_project_id: Option<String>,
}

/// Usage data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "planName")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isValid")]
    pub is_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "invalidMessage")]
    pub invalid_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// Usage query result (supports multiple plans)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<UsageData>>, // supports returning multiple plans
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
