// The context-window reading: a request's prompt against its model's window.
//
// The two halves that go wrong are pinned here — which spelling of the input
// side a row is in, and what a table without the model means. Both are answers
// the page shows, so a helper that guesses is a number on screen that nobody
// published.
import { describe, expect, it } from "vitest";

import type { ModelPrice, RequestLogEntry } from "../api/types";
import { contextUsage, promptTokens, windowFor } from "./contextWindow";

function row(over: Partial<RequestLogEntry> = {}): RequestLogEntry {
  return {
    id: 1,
    ts: "2026-11-06T00:00:00Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "p-1",
    model: "deepseek-v4.1-flash",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: null,
    is_streaming: true,
    input_tokens: 800,
    output_tokens: 120,
    cache_read_tokens: 70_000,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 900,
    first_token_ms: null,
    request_headers: null,
    response_headers: null,
    request_size: 0,
    response_size: 0,
    truncated: false,
    request_notes: null,
    ...over,
  };
}

function price(over: Partial<ModelPrice> = {}): ModelPrice {
  return {
    provider_id: "commandcode",
    model_id: "deepseek-v4.1-flash",
    display_name: "DeepSeek V4.1 Flash",
    input: "0.15",
    output: "0.6",
    cache_read: "0.015",
    cache_creation: "0",
    currency: "USD",
    ...over,
  };
}

describe("a request's prompt size", () => {
  it("adds the cache buckets to a fresh-only input", () => {
    // Anthropic-shaped: `input_tokens` excludes what the prefix cache served.
    expect(promptTokens({ input_tokens: 800, cache_read_tokens: 70_000, cache_creation_tokens: 0 }))
      .toBe(70_800);
    expect(promptTokens({ input_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 2_095 }))
      .toBe(2_095);
  });

  it("leaves a folded input alone", () => {
    // OpenAI-shaped: the prompt already contains the cached part, which is why
    // it can never be the smaller of the two.
    expect(promptTokens({ input_tokens: 72_000, cache_read_tokens: 70_000, cache_creation_tokens: 0 }))
      .toBe(72_000);
    expect(promptTokens({ input_tokens: 1_000, cache_read_tokens: 0, cache_creation_tokens: 0 }))
      .toBe(1_000);
  });
});

describe("the window a model publishes", () => {
  it("prefers this provider's own row, and falls back to the lowest id", () => {
    const prices = [
      price({ provider_id: "aaa-relay", context: 64_000 }),
      price({ provider_id: "commandcode", context: 1_000_000 }),
    ];
    expect(windowFor(prices, "deepseek-v4.1-flash", "commandcode")).toBe(1_000_000);
    // A provider the Hub prices nowhere else is billed at the first row by id,
    // and the window shown has to be the same row's.
    expect(windowFor(prices, "deepseek-v4.1-flash", "self-hosted")).toBe(64_000);
    expect(windowFor(prices, "deepseek-v4.1-flash", null)).toBe(64_000);
  });

  it("says nothing for a model or a row that publishes no window", () => {
    expect(windowFor([price({ context: 200_000 })], "unknown-model", "commandcode")).toBeNull();
    expect(windowFor([price({ context: undefined })], "deepseek-v4.1-flash", "commandcode")).toBeNull();
    // No mirror at all — the seeder has not run yet.
    expect(windowFor(null, "deepseek-v4.1-flash", "commandcode")).toBeNull();
    expect(windowFor([], null, "commandcode")).toBeNull();
  });
});

describe("the cell's answer", () => {
  it("reads the prompt against the window", () => {
    const got = contextUsage(row(), [price({ context: 200_000 })], "commandcode");
    expect(got).toEqual({ prompt: 70_800, window: 200_000, pct: 35 });
  });

  it("keeps the prompt when no window is published", () => {
    // The half that is known still gets shown: "how big was this request" is a
    // fact, and the percentage is the part that needs a denominator.
    const got = contextUsage(row(), [price({ context: undefined })], "commandcode");
    expect(got).toEqual({ prompt: 70_800, window: null, pct: null });
  });

  it("draws nothing for a row with no usable input side", () => {
    // A zeroed row means the upstream reported no usage — not an empty prompt.
    expect(contextUsage(row({ usage_missing: true, input_tokens: 0, cache_read_tokens: 0 }), [], null))
      .toBeNull();
    expect(contextUsage(row({ input_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 }), [], null))
      .toBeNull();
  });
});
