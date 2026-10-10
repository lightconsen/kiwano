// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/providers/transform.rs
// Copied on 2026-09-07. Modified for Kiwano (cross-crate entry points relaxed to pub; the TokenUsage-dependent dedup test rewritten to assert id passthrough directly).

//! Format conversion module
//!
//! Two pairs are converted now, and the title is worth reading carefully because
//! "Anthropic ↔ OpenAI" suggests a symmetry that is not there. A Claude Code
//! client on an OpenAI-compatible provider gets `anthropic_to_openai` (request)
//! and `openai_to_anthropic` (response, JSON or the SSE behind
//! [`create_anthropic_sse_stream`]). A Codex client — which speaks the Responses
//! API and nothing else — gets `responses_to_chat` (request),
//! `chat_to_responses` (response) and the SSE adapter
//! `responses_streaming::create_responses_sse_stream`.
//!
//! The Responses forward direction was built because it has real users: refusing
//! `/v1/responses` against a Chat-only provider meant refusing Codex outright.
//! The **reverse** is still refused, deliberately: an OpenAI Chat request
//! arriving for a Responses-only upstream would have to invent an `input` and a
//! stored-response story no Chat client asked for, so nothing was built for it.
//! Also not converted: an OpenAI Chat request arriving for an Anthropic provider,
//! and `/v1/messages/count_tokens`. Each of those is refused with a reason rather
//! than approximated — `resolve_inbound` in the gateway lists them — because a
//! half-converted request is a wrong answer with no way to tell.
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
//! shares nothing with either. `responses_request` and `responses_response` are
//! the Responses pair; `responses_response` also exposes the shared usage and
//! object builders the streaming adapter imports, so the chunked and whole
//! responses stay in step.

pub mod billing;
pub mod message;
pub mod reasoning;
pub mod request;
pub mod response;
pub mod responses_request;
pub mod responses_response;
pub mod schema;
pub mod stream_options;
pub mod tool_choice;

// ── the public surface, re-exported so every `transform::x` path still resolves ──

pub use billing::strip_leading_anthropic_billing_header;
pub use reasoning::{is_openai_o_series, resolve_reasoning_effort, supports_reasoning_effort};
pub use request::{anthropic_to_openai, anthropic_to_openai_with_reasoning_content};
pub use response::openai_to_anthropic;
pub use responses_request::responses_to_chat;
pub use responses_response::chat_to_responses;
pub use schema::clean_schema;
pub use stream_options::inject_openai_stream_include_usage;
