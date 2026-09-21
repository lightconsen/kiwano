// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/providers/transform.rs
// Copied on 2026-09-07. Modified for Kiwano (cross-crate entry points relaxed to pub; the TokenUsage-dependent dedup test rewritten to assert id passthrough directly).

//! Format conversion module
//!
//! One direction is converted, and it is worth naming plainly because the title
//! "Anthropic ↔ OpenAI" suggests more: an Anthropic **request** becomes an OpenAI
//! Chat Completions request (`anthropic_to_openai`), and an OpenAI Chat
//! **response** — JSON or SSE — becomes an Anthropic one (`openai_to_anthropic`
//! and the streaming converter behind [`create_anthropic_sse_stream`]).
//!
//! That is what a Claude Code→OpenAI-compatible provider needs, and it is the
//! only pair this gateway converts. Not converted, and deliberately not built:
//! the Responses API in either direction (`/v1/responses` reaches an
//! OpenAI-speaking provider natively, and nothing else), an OpenAI Chat request
//! arriving for an Anthropic provider, and `/v1/messages/count_tokens`. Each of
//! those is refused with a reason rather than approximated — `resolve_inbound` in
//! the gateway lists them — because a half-converted request is a wrong answer
//! with no way to tell.
//!
//! Reference: anthropic-proxy-rs
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`transform::anthropic_to_openai`): the facade below
//! re-exports it, because `crates/gateway/src/forward/{convert,shim}.rs` name
//! `anthropic_to_openai`, `openai_to_anthropic` and
//! `inject_openai_stream_include_usage` and are not edited.
//!
//! `billing`, `reasoning`, `schema`, `tool_choice` and `stream_options` are
//! leaves — each is a pure function over a `&str` or a JSON value, so any module
//! here may import one without an ordering worry. `message` turns one Anthropic
//! message into the OpenAI messages it becomes, `request` composes the whole
//! converted body out of those pieces, and `response` is the other direction: it
//! shares nothing with either.

pub mod billing;
pub mod message;
pub mod reasoning;
pub mod request;
pub mod response;
pub mod schema;
pub mod stream_options;
pub mod tool_choice;

// ── the public surface, re-exported so every `transform::x` path still resolves ──

pub use billing::strip_leading_anthropic_billing_header;
pub use reasoning::{is_openai_o_series, resolve_reasoning_effort, supports_reasoning_effort};
pub use request::{anthropic_to_openai, anthropic_to_openai_with_reasoning_content};
pub use response::openai_to_anthropic;
pub use schema::clean_schema;
pub use stream_options::inject_openai_stream_include_usage;
