//! adapters — extraction layer for code ported from cc-switch
//! (https://github.com/farion1231/cc-switch, MIT License).
//!
//! This crate hosts the Tier A modules extracted from cc-switch (with the
//! file-header attribution required by tech.md §1.7) so Kiwano can reuse
//! their provider/config management semantics:
//!
//! - [`error`]: shared `AppError` type
//! - [`config`]: home-dir resolution + atomic file writes
//! - [`provider`]: provider model, manager and metadata
//! - [`model_capabilities`]: image-input capability resolution
//! - [`model_pricing`]: bundled per-million-token price table + cost
//!   calculation (matching semantics ported from cc-switch usage stats)
//! - [`gemini_config`]: Gemini env/settings file management
//! - [`claude_desktop_config`]: Claude Desktop gateway profile + deploymentMode
//!   writes (macOS Claude-3p configLibrary subset)
//! - [`grok_config`]: Grok TOML live-config management
//! - [`opencode_config`]: OpenCode opencode.json management
//! - [`gateway_takeover`]: content→content gateway-entry upserts for the
//!   additive-mode agents (opencode/openclaw/hermes/pi) used by takeover
//! - [`codex_config`]: Codex config write core (Tier C, function-level port)
//! - [`proxy`]: protocol conversion sublayer (Tier B) — Anthropic ↔ OpenAI
//!   request/response/SSE conversion extracted from cc-switch's proxy
//!
//! Global settings-override hooks from cc-switch are replaced here by
//! explicit parameter injection or local stubs; nothing else was restructured.

pub mod claude_desktop_config;
pub mod codex_config;
pub mod config;
pub mod error;
pub mod gateway_takeover;
pub mod gemini_config;
pub mod grok_config;
pub mod model_capabilities;
pub mod model_pricing;
pub mod opencode_config;
pub mod provider;
pub mod proxy;
