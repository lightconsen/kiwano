//! Which `model_provider` ids Codex owns. `CODEX_RESERVED_MODEL_PROVIDER_IDS`
//! is the built-in catalog, `active_codex_model_provider_id` reads the id a
//! config text selects, and `is_custom_codex_model_provider_id` answers whether
//! our transforms may write into that provider's table.
//!
//! A leaf: it reads no config text, and every other module here imports it.

use toml_edit::DocumentMut;

/// Reserved built-in provider IDs from OpenAI Codex's config/model-provider
/// catalog. Keep in sync with Codex `RESERVED_MODEL_PROVIDER_IDS` (0.149:
/// exactly these five; 0.148 is the same minus `amazon-bedrock-runtime`).
/// `oss` / `ollama-chat` are NOT reserved on 0.148/0.149 — both load as
/// ordinary custom tables — so listing them here would strand their bearer
/// token in the ignored top level. Mirror: providerConfigUtils.ts.
// Ported from cc-switch: src-tauri/src/codex_config.rs::CODEX_RESERVED_MODEL_PROVIDER_IDS
const CODEX_RESERVED_MODEL_PROVIDER_IDS: &[&str] = &[
    "amazon-bedrock",
    "amazon-bedrock-runtime",
    "openai",
    "ollama",
    "lmstudio",
];

pub(crate) fn active_codex_model_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::is_custom_codex_model_provider_id
pub(crate) fn is_custom_codex_model_provider_id(id: &str) -> bool {
    // Exact match, mirroring upstream: both the built-in provider lookup and
    // validate_reserved_model_provider_ids are case-sensitive, so `OpenAI`
    // etc. are legitimate custom ids whose tables must receive the token.
    // Keep in sync with the frontend list in src/utils/providerConfigUtils.ts.
    let id = id.trim();
    !id.is_empty() && !CODEX_RESERVED_MODEL_PROVIDER_IDS.contains(&id)
}
