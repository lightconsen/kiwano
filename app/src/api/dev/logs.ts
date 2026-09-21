// The request log fixtures, and the one predicate the list and the export
// share. Mirrors `crates/core/src/vm/logs.rs`.

import type { RequestLogDetail, RequestLogEntry, RequestLogFilter } from "../types";

// ── Request logs (data-plane audit trail; fixtures mirror gateway capture) ──

export const requestLogs: RequestLogDetail[] = [
  {
    id: 3,
    ts: "2026-09-09T21:04:11Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "deepseek",
    model: "claude-sonnet-4-5",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: "user_acct__session_9f3a",
    is_streaming: false,
    input_tokens: 2095,
    output_tokens: 503,
    cache_read_tokens: 0,
    cache_creation_tokens: 2095,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 1180,
    first_token_ms: null,
    request_headers: '{"content-type":"application/json","x-kw-session":"s-1"}',
    response_headers: '{"content-type":"application/json"}',
    request_size: 214,
    response_size: 331,
    truncated: false,
    // The shim touched this one: history replayed against DeepSeek carried
    // thinking blocks the upstream refuses (cc-switch discussion #3216).
    request_notes:
      "thinking: removed unsupported type \"adaptive\"\nassistant history: removed 2 thinking/redacted_thinking block(s)",
    request_body: '{"model":"claude-sonnet-4-5","stream":false,"messages":[{"role":"user","content":"Refactor the retry loop in forward.rs"}]}',
    response_body: '{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Done — see the diff."}],"usage":{"input_tokens":2095,"output_tokens":503}}',
  },
  {
    id: 2,
    ts: "2026-09-09T20:58:37Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "deepseek",
    model: "claude-sonnet-4-5",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: "user_acct__session_9f3a",
    is_streaming: true,
    input_tokens: 25,
    output_tokens: 171,
    cache_read_tokens: 11,
    cache_creation_tokens: 3,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 940,
    first_token_ms: 210,
    request_headers: '{"content-type":"application/json"}',
    response_headers: '{"content-type":"text/event-stream"}',
    request_size: 96,
    response_size: 1240,
    truncated: false,
    request_notes: null,
    request_body: '{"model":"claude-sonnet-4-5","stream":true,"messages":[{"role":"user","content":"hello"}]}',
    response_body: 'event: message_start\ndata: {"type":"message_start"}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}\n\nevent: message_stop\ndata: {"type":"message_stop"}\n\n',
  },
  {
    id: 1,
    ts: "2026-09-09T20:51:02Z",
    method: "POST",
    path: "/v1/chat/completions",
    query: null,
    agent: "codex",
    attribution: "key",
    provider_id: "deepseek",
    model: null,
    status_code: 502,
    error_kind: "protocol_mismatch",
    error_message: "kiwano-gateway: provider `deepseek` speaks openai; anthropic inbound conversion not implemented for this direction",
    session_id: null,
    is_streaming: false,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: null,
    first_token_ms: null,
    request_headers: '{"content-type":"application/json"}',
    response_headers: null,
    request_size: 74,
    response_size: 0,
    truncated: false,
    request_notes: null,
    request_body: '{"model":"gpt-5.2","messages":[]}',
    response_body: null,
  },
];

/** The status/agent/provider/date predicate shared by the mock's list and
 *  export, so the two cannot disagree — same reason the real store has one
 *  WHERE for both. Date bounds compare as strings, exactly as SQLite does on
 *  the RFC3339 column. */
export function matchingLogs(filter?: RequestLogFilter): RequestLogEntry[] {
  return requestLogs.filter(
    (r) =>
      (!filter?.agent || r.agent === filter.agent) &&
      (!filter?.provider_id || r.provider_id === filter.provider_id) &&
      (!filter?.status ||
        (filter.status === "error" && r.status_code >= 400) ||
        (filter.status === "ok" && r.status_code < 400)) &&
      (!filter?.from || r.ts >= filter.from) &&
      (!filter?.to || r.ts < filter.to),
  );
}
