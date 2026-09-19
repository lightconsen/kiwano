//! MCP stdio server (Features: agent self-query).
//!
//! Newline-delimited JSON-RPC 2.0 over stdin/stdout, hand-rolled — the
//! protocol surface an agent's MCP client actually uses is three methods
//! (`initialize`, `tools/list`, `tools/call`) plus notifications, and the
//! repo's standing rule is to not take a dependency a hundred lines replace
//! (the sidecar writes its own HTTP the same way).
//!
//! The tools answer with aggregates only. Bodies never leave the machine —
//! they are sampled for measurement exactly the way `kiwano insights` samples
//! them, and what crosses stdout is the report, not the material.

use std::io::BufRead;

use kiwano_core::vm;
use serde_json::{json, Value};

use crate::cmds::build_insights_report;
use crate::{CliError, Ctx};

/// The newest protocol revision this server speaks. Clients negotiate down
/// from their own; nothing here depends on a newer draft's features.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The tool catalogue, re-sent on every `tools/list` — static, small, and
/// cheaper to rebuild than to share.
fn tools() -> Value {
    json!([
        {
            "name": "get_usage_summary",
            "description": "Requests, tokens, cache hit rate and cost over the last N days, across every agent and provider.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": { "type": "integer", "description": "Window length in days (1-90, default 7)" }
                }
            }
        },
        {
            "name": "get_insights",
            "description": "The insights report: cache hit rate, context growth, retry storms and overhead findings, with evidence row ids.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": { "type": "integer", "description": "Window length in days (1-90, default 7)" },
                    "agent": { "type": "string", "description": "Restrict to one agent" }
                }
            }
        },
        {
            "name": "get_session_growth",
            "description": "The sessions whose context grew fastest in the window — the compact candidates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": { "type": "integer", "description": "Window length in days (1-90, default 7)" }
                }
            }
        }
    ])
}

/// The flag gate: the MCP server exists only while the user has asked for it.
/// The failure text is a CLI error rather than a protocol message because it
/// precedes the handshake — the client is not speaking MCP yet.
pub fn mcp(ctx: &mut Ctx) -> Result<(), CliError> {
    if !vm::ui_settings(ctx.aux()?).feat_mcp_self_query {
        return Err(CliError::usage(
            "agent self-query is off — enable it under Settings → Features",
        ));
    }
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| CliError::runtime(format!("reading stdin: {e}")))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            // A malformed line is not a request and has no id to answer to;
            // the protocol's answer to garbage is silence.
            Err(_) => continue,
        };
        match handle(ctx, &msg) {
            Ok(Some(resp)) => ctx.out.line(resp.to_string()),
            Ok(None) => {}
            // A store failure mid-session is reported per-request and the loop
            // continues — one bad query must not hang up on the agent.
            Err(e) => {
                if let Some(id) = msg.get("id").cloned() {
                    ctx.out.line(
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": e.message}})
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

/// One message in, at most one response out (notifications answer nothing).
fn handle(ctx: &mut Ctx, msg: &Value) -> Result<Option<Value>, CliError> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");

    if method == "tools/call" {
        let Some(id) = id else { return Ok(None) };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let (content, is_error) = call_tool(ctx, &params);
        return Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": content }],
                "isError": is_error
            }
        })));
    }
    Ok(meta_response(method, id))
}

/// The protocol half of the server — everything except `tools/call`, which is
/// the half that reads the store. Pure, so the handshake is testable without
/// a database.
fn meta_response(method: &str, id: Option<Value>) -> Option<Value> {
    let result = |v: Value| {
        id.clone()
            .map(|id| json!({"jsonrpc": "2.0", "id": id, "result": v}))
    };
    match method {
        "initialize" => result(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "kiwano", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => result(json!({})),
        "tools/list" => result(json!({ "tools": tools() })),
        // Notifications (notifications/initialized, cancelled, …) get no
        // answer; anything else with an id gets the protocol's own shrug.
        _ if method.starts_with("notifications/") => None,
        _ => id.map(|id| {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})
        }),
    }
}

/// `tools/call`, with the answer folded to (text, is_error): the payload is
/// always a JSON document carried as text, which is the MCP convention.
fn call_tool(ctx: &mut Ctx, params: &Value) -> (String, bool) {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let days = args
        .get("days")
        .and_then(Value::as_i64)
        .unwrap_or(7)
        .clamp(1, 90);
    let store = match ctx.store() {
        Ok(s) => s,
        Err(e) => return (e.message, true),
    };
    let tuning = ctx
        .aux()
        .map(|aux| vm::ui_settings(aux).feat_tuning_advice)
        .unwrap_or(false);

    let result: Result<Value, CliError> = match name {
        "get_usage_summary" => usage_summary(store, days),
        "get_insights" => {
            let agent = args.get("agent").and_then(Value::as_str).map(String::from);
            build_insights_report(store, days, agent, tuning).map(|r| json!(r))
        }
        "get_session_growth" => build_insights_report(store, days, None, tuning)
            .map(|r| json!({ "days": r.days, "top_sessions": r.top_sessions })),
        _ => return (format!("unknown tool: {name}"), true),
    };
    match result {
        Ok(v) => (v.to_string(), false),
        Err(e) => (e.message, true),
    }
}

/// Windowed totals plus the cache hit rate, the answer to "how am I doing".
fn usage_summary(store: &kiwanod::store::Store, days: i64) -> Result<Value, CliError> {
    let since = vm::rfc3339(vm::unix_now() - days * 86_400);
    let t = store
        .usage_totals(None, None, Some(&since))
        .map_err(|e| CliError::runtime(e.to_string()))?;
    let cost = store
        .usage_cost_by_currency(None, None, Some(&since))
        .map_err(|e| CliError::runtime(e.to_string()))?;
    let denom = t.input_tokens + t.cache_read_tokens + t.cache_creation_tokens;
    let cache_hit_pct = if denom > 0 {
        (t.cache_read_tokens as f64 * 1000.0 / denom as f64).round() / 10.0
    } else {
        0.0
    };
    Ok(json!({
        "days": days,
        "requests": t.requests,
        "input_tokens": t.input_tokens,
        "output_tokens": t.output_tokens,
        "cache_read_tokens": t.cache_read_tokens,
        "cache_creation_tokens": t.cache_creation_tokens,
        "cache_hit_pct": cache_hit_pct,
        "cost_by_currency": cost,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_answers_with_the_server_card() {
        let r = meta_response("initialize", Some(json!(1))).expect("a request gets a response");
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["serverInfo"]["name"], "kiwano");
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(r["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_get_no_answer() {
        assert!(meta_response("notifications/initialized", None).is_none());
        assert!(meta_response("notifications/cancelled", None).is_none());
    }

    #[test]
    fn unknown_methods_get_the_protocols_shrug() {
        let r = meta_response("resources/list", Some(json!("a"))).expect("it has an id");
        assert_eq!(r["error"]["code"], -32601);
        // …but only with an id — a stray notification-looking message is dropped.
        assert!(meta_response("resources/list", None).is_none());
    }

    #[test]
    fn tools_list_names_the_three_tools() {
        let r = meta_response("tools/list", Some(json!(2))).unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["get_usage_summary", "get_insights", "get_session_growth"]
        );
    }
}
