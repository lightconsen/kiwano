//! Protocol conversion sublayer extracted from cc-switch's proxy
//! (https://github.com/farion1231/cc-switch, MIT License).
//!
//! Carries the Anthropic ↔ OpenAI request/response/SSE conversion core:
//!
//! - [`providers::transform`]: `anthropic_to_openai` / `openai_to_anthropic`
//! - [`providers::streaming`]: OpenAI SSE → Anthropic event stream
//! - [`sse`]: low-level SSE block/UTF-8 chunk utilities
//! - [`model_mapper`]: the `[1M]` local-capability marker strip
//! - [`error_mapper`]: `ProxyError` → HTTP status mapping
//! - [`json_canonical`]: stable JSON serialization helpers
//! - [`tool_media`]: tool-result image media rectification
//! - [`error`]: trimmed `ProxyError` (same variant names as upstream)

pub mod error;
pub mod error_mapper;
pub mod json_canonical;
pub mod model_mapper;
pub mod providers;
pub mod sse;
pub mod tool_media;

pub use error::ProxyError;
