// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/{opencode_config,openclaw_config,hermes_config,pi_config}.rs
// Copied on 2026-09-08. Modified for Kiwano: only the provider-entry write +
// selection subsets were ported, reshaped as pure content→content transforms
// for the takeover pipeline (read → rewrite in memory → backup → atomic write,
// see src-tauri/src/takeover.rs). The DB-backed provider CRUD, CAS revision
// checks and 0600-permission plumbing were dropped because the takeover
// pipeline is the sole writer and serializes file access itself.

//! Gateway takeover transforms for the agents whose config this app writes an
//! entry into: opencode, openclaw, hermes, pi, workbuddy, codebuddy, kimi,
//! qwen, cline, mimo and mcode.
//!
//! All but cline are additive: their configs hold many providers and select
//! one, so "takeover" means upsert a gateway entry pointing at the local
//! gateway and select it — every pre-existing provider entry survives. (mimo's
//! entry is filed under its doc-mandated `custom` id rather than
//! `kiwano-gateway`; see that module.) The gateway forwards model names
//! verbatim, so selections keep the user's existing model id and only swap the
//! provider prefix.
//!
//! Cline is the exception this module's name does not cover: its selector names
//! a provider *id* rather than an entry of ours, so the takeover replaces the
//! slot that id names. See [`upsert_cline_gateway`] for why that is not a
//! preference.
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`gateway_takeover::upsert_qwen_gateway`): the facade below
//! re-exports it, because `crates/core/src/takeover/rewrite.rs` and
//! `crates/core/src/creds.rs` name it that way and neither is edited.
//!
//! `gateway` and `json` are leaves — the entry's constants and the JSONC text
//! helpers read no agent's schema, so any other module may import them without
//! an ordering worry. Each agent owns its own writers; `readers` owns the four
//! `CurrentProvider` readers, filed by consumer rather than by agent because
//! they serve the first-takeover *import* (`creds.rs`) and not the takeover.
//! Cline's reader stays with cline, as the single file already kept it: it
//! reshapes the endpoint it reads and reads through a selector rather than a
//! provider id. `model_list` holds what the two model-list agents (workbuddy,
//! codebuddy) share.

pub mod cline;
pub mod codebuddy;
pub mod gateway;
pub mod hermes;
pub mod json;
pub mod kimi;
pub mod mcode;
pub mod mimo;
pub mod model_list;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod qwen;
pub mod readers;
pub mod workbuddy;

// ── the public surface, re-exported so every `gateway_takeover::x` path still resolves ──

pub use cline::{read_cline_current, upsert_cline_gateway};
pub use codebuddy::upsert_codebuddy_models_gateway;
pub use gateway::GATEWAY_PROVIDER_ID;
pub use hermes::upsert_hermes_gateway;
pub use kimi::upsert_kimi_gateway;
pub use mcode::{read_mcode_current, upsert_mcode_gateway};
pub use mimo::{read_mimo_current, upsert_mimo_gateway};
pub use model_list::GATEWAY_VENDOR;
pub use openclaw::{upsert_openclaw_gateway, upsert_openclaw_models_json};
pub use opencode::upsert_opencode_gateway;
pub use pi::{select_pi_gateway, upsert_pi_models_gateway};
pub use qwen::upsert_qwen_gateway;
pub use readers::{
    read_hermes_current, read_openclaw_current, read_opencode_current, read_pi_current,
    CurrentProvider,
};
pub use workbuddy::upsert_workbuddy_gateway;
