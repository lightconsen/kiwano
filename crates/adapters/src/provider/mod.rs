// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/provider.rs
// Copied on 2026-09-07. Modified for Kiwano (inlined the `crate::settings::
// CustomEndpoint` struct, and removed `resolve_usage_credentials` which
// depended on out-of-scope modules `app_config`/`pi_config`/`codex_config`).

// SSOT mode: provider copy files are no longer written

//! The provider model ported from cc-switch: the provider entry and its
//! manager, the metadata that hangs off both, and the two shapes another app's
//! provider list stores (universal and OpenCode).
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`provider::Provider`): the facade below re-exports it,
//! because `crate::grok_config` names `provider::Provider` and
//! `crate::opencode_config` names `provider::OpenCodeProviderConfig`, and
//! neither is edited.
//!
//! Field order is the wire format: these structs are the cc-switch on-disk
//! schema (`~/.cc-switch/config.json`) and `serde_json` is built with
//! `preserve_order`, so a renamed or reordered field is a silent data-format
//! break.
//!
//! The dependency order runs leaf to hub. `custom_endpoint`, `auth`,
//! `claude_desktop`, `codex_reasoning`, `local_proxy` and `usage` read no other
//! module here, so any of them may be imported without an ordering worry.
//! `meta` composes those six into the `ProviderMeta` hub; `record` and
//! `manager` are the provider entry and the id-keyed map of them, and
//! `universal` and `opencode` are the shapes stored for another app. Nothing
//! imports back up.

pub mod auth;
pub mod claude_desktop;
pub mod codex_reasoning;
pub mod custom_endpoint;
pub mod local_proxy;
pub mod manager;
pub mod meta;
pub mod opencode;
pub mod record;
pub mod universal;
pub mod usage;

// ── the public surface, re-exported so every `provider::x` path still resolves ──

pub use auth::{AuthBinding, AuthBindingSource};
pub use claude_desktop::{ClaudeDesktopMode, ClaudeDesktopModelRoute};
pub use codex_reasoning::CodexChatReasoningConfig;
pub use custom_endpoint::CustomEndpoint;
pub use local_proxy::LocalProxyRequestOverrides;
pub use manager::ProviderManager;
pub use meta::{parse_custom_user_agent, ProviderMeta};
pub use opencode::{
    OpenCodeModel, OpenCodeModelLimit, OpenCodeProviderConfig, OpenCodeProviderOptions,
};
pub use record::Provider;
pub use universal::{
    ClaudeModelConfig, CodexModelConfig, GeminiModelConfig, UniversalProvider,
    UniversalProviderApps, UniversalProviderModels,
};
pub use usage::{UsageData, UsageResult, UsageScript};
