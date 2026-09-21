//! `stream_options.include_usage` injection.
//!
//! OpenAI-compatible upstreams return no usage in the SSE stream unless
//! `include_usage` is declared, and without it streaming token/cost/cache
//! accounting is silently lost. A leaf: it edits an already-converted request
//! body in place.

use serde_json::{json, Value};

/// Inject `stream_options.include_usage` into OpenAI Chat Completions
/// streaming requests.
///
/// OpenAI-compatible upstreams do not return usage in the SSE stream by
/// default; `include_usage` must be declared explicitly for the final usage
/// chunk to be emitted. Without this injection, token/cost/cache accounting
/// for streaming requests is silently lost (input/output/cache all 0). Any
/// other `stream_options` fields the client passed through are preserved —
/// only `include_usage` is added; non-streaming requests are untouched.
///
/// Upstream, this was shared with a Codex Responses→Chat path
/// (`transform_codex_chat.rs`). That converter was not ported — nothing here
/// speaks the Responses API (see the module comment on what is converted) — so
/// this is now the only caller.
pub fn inject_openai_stream_include_usage(result: &mut Value) {
    let is_stream = result
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_stream {
        return;
    }
    match result.get_mut("stream_options") {
        Some(Value::Object(opts)) => {
            opts.insert("include_usage".to_string(), json!(true));
        }
        _ => {
            result["stream_options"] = json!({ "include_usage": true });
        }
    }
}
