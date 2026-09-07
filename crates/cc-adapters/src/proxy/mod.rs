//! Protocol conversion sublayer extracted from cc-switch's proxy
//! (https://github.com/farion1231/cc-switch, MIT License).
//!
//! Carries the Anthropic ↔ OpenAI request/response/SSE conversion core:
//!
//! - [`providers::transform`]: `anthropic_to_openai` / `openai_to_anthropic`
//! - [`providers::streaming`]: OpenAI SSE → Anthropic event stream
//! - [`sse`]: low-level SSE block/UTF-8 chunk utilities
//! - [`model_mapper`]: env-driven model mapping on outbound requests
//! - [`error_mapper`]: `ProxyError` → HTTP status mapping
//! - [`cache_injector`]: prompt-cache breakpoint injection
//! - [`json_canonical`]: stable JSON serialization helpers
//! - [`tool_media`]: tool-result image media rectification
//! - [`error`]: trimmed `ProxyError` (same variant names as upstream)
//! - [`types`]: `OptimizerConfig`

pub mod cache_injector;
pub mod error;
pub mod error_mapper;
pub mod json_canonical;
pub mod model_mapper;
pub mod providers;
pub mod sse;
pub mod tool_media;
pub mod types;

pub use error::ProxyError;
