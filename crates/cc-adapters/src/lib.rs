//! cc-adapters — extraction layer for code ported from cc-switch
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
//! - [`gemini_config`]: Gemini env/settings file management
//! - [`grok_config`]: Grok TOML live-config management
//! - [`opencode_config`]: OpenCode opencode.json management
//!
//! Global settings-override hooks from cc-switch are replaced here by
//! explicit parameter injection or local stubs; nothing else was restructured.

pub mod config;
pub mod error;
pub mod gemini_config;
pub mod grok_config;
pub mod model_capabilities;
pub mod opencode_config;
pub mod provider;
