// How full a request's context window was: the prompt's size against the
// model's published window.
//
// Both halves need care.
//
// **The prompt.** The gateway stores three input-side buckets and does not
// record how the vendor spelled them (`cache_inclusive` is decided per request
// by the code path and never persisted). An OpenAI-shaped upstream reports
// `prompt_tokens` with the cache buckets folded in, so the prompt *is*
// `input_tokens`; an Anthropic-shaped one reports fresh input alone, so the
// prompt is the sum. The numbers themselves say which reading is possible: a
// folded count can never be smaller than a bucket inside it, while a
// fresh-only row routinely is (a 70k cache read against 800 new tokens). That
// test, not the protocol, is what this reads — a provider's protocol is not on
// the row either.
//
// **The window.** It comes from the Hub's price document, per `(provider,
// model)` row, and a bit under a sixth of the published table states none. A
// missing window is "not published", never zero: the caller shows the prompt
// without a percentage rather than dividing by a guessed denominator.
import type { ModelPrice, RequestLogEntry } from "../api/types";

/** The request's whole input side, in the vendor's own terms. */
export function promptTokens(r: Pick<RequestLogEntry, "input_tokens" | "cache_read_tokens" | "cache_creation_tokens">): number {
  const cache = r.cache_read_tokens + r.cache_creation_tokens;
  // `>=`: the folded reading, which is also the answer when nothing was cached.
  return r.input_tokens >= cache ? r.input_tokens : r.input_tokens + cache;
}

/** The context window the mirror publishes for one model, or `null`.
 *
 * Matching is exact (case- and whitespace-insensitive), the same tolerance the
 * shelf's per-model list uses: the gateway's normalized and prefix matching is
 * deliberately not reproduced here, so a miss is a miss rather than a row from
 * a neighbour. Among the rows that do match, this provider's own first — the
 * same order the gateway prices in — and then the lowest `provider_id`, which
 * is what a model the provider does not itself price is billed at. */
export function windowFor(
  prices: ModelPrice[] | null,
  model: string | null,
  catalogId?: string | null,
): number | null {
  if (!prices || !model) return null;
  const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();
  const rows = prices.filter((p) => same(p.model_id, model));
  if (rows.length === 0) return null;
  const own = catalogId ? rows.find((p) => same(p.provider_id, catalogId)) : undefined;
  const pick =
    own ?? [...rows].sort((a, b) => a.provider_id.localeCompare(b.provider_id))[0];
  return pick.context ?? null;
}

export interface ContextUsage {
  /** The request's whole input side. */
  prompt: number;
  /** The model's published window, or `null` when it publishes none. */
  window: number | null;
  /** Prompt over window, rounded — `null` whenever the window is. */
  pct: number | null;
}

/** The cell's whole answer, or `null` when there is nothing honest to say: a
 *  row whose usage was never reported (the zeros mean "unknown"), or one with
 *  no input-side tokens at all. */
export function contextUsage(
  r: Pick<
    RequestLogEntry,
    "model" | "input_tokens" | "cache_read_tokens" | "cache_creation_tokens" | "usage_missing"
  >,
  prices: ModelPrice[] | null,
  catalogId?: string | null,
): ContextUsage | null {
  if (r.usage_missing) return null;
  const prompt = promptTokens(r);
  if (prompt <= 0) return null;
  const window = windowFor(prices, r.model, catalogId);
  return { prompt, window, pct: window ? Math.round((prompt / window) * 100) : null };
}
