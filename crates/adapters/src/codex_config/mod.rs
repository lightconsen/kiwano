// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/codex_config.rs
// Copied on 2026-09-07. Modified for Kiwano (Tier C function-level port).
//
// Ported: the config path acquisition, the atomic live-config write, TOML
// validation, auth/base-url extraction, and the live-write gates — the planner
// `plan_codex_live_write` with its `preflight_codex_live_write` wrapper, the
// pre-write repairs (`migrate_stale_reserved_provider_tables`,
// `backfill_codex_custom_provider_names`, `normalize_codex_legacy_openai_reroute`),
// the two auth-safety gates and the provider-table conflict rejection.
//
// Deliberately NOT ported, and not an oversight: the ~6k lines of managed-OAuth
// logic and the settings-override hook. `restore_preserving_newer_same_account_auth`
// exists to protect a CLI-rotated OAuth refresh token; Kiwano hosts no OAuth and
// keeps no login generation to preserve (an API-key `auth.json` has none), so
// there is nothing for it to protect. `align_codex_requires_openai_auth_with_login_preservation`
// and the unified-session-bucket injection are login-UX and settings-hook
// features with no Kiwano caller.

//! Codex (`~/.codex`) config write core ported from cc-switch.
//!
//! Each ported function is marked with a `// Ported from cc-switch:` comment
//! naming its upstream origin. Functions not marked are Kiwano scaffolding.
//!
//! The gates run on *text*, never on disk: a caller builds a plan, compares or
//! discards it, and only writes what the plan approved. `crate::takeover` uses
//! this module for every Codex transform it performs, so the user's `~/.codex`
//! only ever receives text these gates have judged.
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`codex_config::plan_codex_live_write`): the facade below
//! re-exports it, because `crates/core/src/takeover/{rewrite,strip,state,disable}.rs`
//! and `crates/core/src/vm/takeover.rs` name it that way and none of them is
//! edited.
//!
//! `paths` and `provider_id` are leaves: a path resolves from the home
//! directory and an id predicate reads no config text, so any other module may
//! import either without an ordering worry. `extract` reads a config text back
//! and `gates` judges one; `repairs` rewrites the shapes 0.149 refuses to load
//! and reads `gates`; `plan` composes the two into the live-write plan, and
//! `takeover` is the Kiwano scaffolding that points a planned write at the
//! local gateway.
//!
//! `provider_id` holds the reserved-id list beside the two predicates over it
//! because that is this file's hard floor: every other module here reads
//! `active_codex_model_provider_id`, and `CODEX_RESERVED_MODEL_PROVIDER_IDS`
//! has no reader but the custom-id predicate next to it.

pub mod extract;
pub mod gates;
pub mod paths;
pub mod plan;
pub mod provider_id;
pub mod repairs;
pub mod takeover;

// ── the public surface, re-exported so every `codex_config::x` path still resolves ──

pub use extract::{
    extract_codex_api_key, extract_codex_auth_api_key, extract_codex_base_url,
    extract_codex_experimental_bearer_token,
};
pub use paths::{
    get_codex_auth_path, get_codex_config_dir, get_codex_config_path, get_codex_provider_paths,
    read_and_validate_codex_config_text, read_codex_config_text, validate_config_toml,
    write_codex_live_atomic, write_codex_live_config_atomic,
};
pub use plan::{
    plan_codex_live_write, preflight_codex_live_write, prepare_codex_provider_live_config,
    CodexLiveWritePlan,
};
pub use takeover::{
    codex_config_placeholder_key, is_loopback_gateway_url, plan_codex_takeover_live_write,
    rebuild_codex_live_from_provider, remove_codex_gateway_route, GATEWAY_PLACEHOLDER_PREFIX,
};

pub(crate) use provider_id::is_custom_codex_model_provider_id;

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) fn valid_config_text() -> &'static str {
        r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
wire_api = "responses"
"#
    }

    /// A healthy pre-0.149-shaped third-party config: one active custom
    /// provider table, a Responses wire API, and a name.
    pub(crate) fn healthy_third_party_config() -> String {
        r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
wire_api = "responses"
"#
        .to_string()
    }

    pub(crate) fn provider_table(text: &str, id: &str) -> toml::Value {
        let mut doc: toml::Value = toml::from_str(text).expect("plan output parses");
        doc.get_mut("model_providers")
            .and_then(|p| p.get_mut(id))
            .map(|table| table.clone())
            .unwrap_or_else(|| panic!("[model_providers.{id}] missing from plan output"))
    }
}
