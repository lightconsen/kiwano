// The demo's traffic: what the gateway would be doing if this page were the app.
//
// The site's demo is a browser build of the real screens over the dev fixtures
// (`src/api/dev`), which are static — a screenshot that happens to be made of
// DOM. This module is the missing half: it walks the same fixture objects the
// screens read, moves their counters the way a request would, and tells the app
// the numbers changed. Nothing here knows about a screen; it only edits the
// numbers, and the screens re-read them the way they re-read the real ones.

import type { Provider } from "../api/types";
import { providers } from "../api/dev/providers";
import { footerStats } from "../api/dev/api/app";
import { publishUsageTick } from "./bridge";

/** One simulated request's worth of tokens, in the shape the fixtures hold. */
function simulateRequest(): void {
  // Only rows that could be serving: disabled and unbound ones stay still, the
  // same way they would in the app. The predicate narrows `usage` to non-null,
  // which is what every line below reads.
  type Usage = NonNullable<Provider["usage"]>;
  type ServingRow = Provider & { usage: Usage & { cost: number } };
  const live = providers.filter(
    (p): p is ServingRow => Boolean(p.enabled && p.usage && p.health.state !== "off"),
  );
  if (live.length === 0) return;
  const row = live[Math.floor(Math.random() * live.length)];

  const input = 4_000 + Math.floor(Math.random() * 30_000);
  const cacheRead = Math.floor(input * 0.6);
  const output = 400 + Math.floor(Math.random() * 2_400);

  row.usage.requests += 1;
  row.usage.input_tokens += input;
  row.usage.cache_read_tokens += cacheRead;
  row.usage.output_tokens += output;
  // Cost moves in the row's own currency at a plausible per-million rate; the
  // figure is sample data either way, and the point is that it moves.
  row.usage.cost = Number((row.usage.cost + (input + output) / 1_000_000 * 2.4).toFixed(2));
  if (row.usage.quota) {
    row.usage.quota.used = Number(
      Math.min(row.usage.quota.limit, row.usage.quota.used + (row.usage.quota.limit / 900)).toFixed(2),
    );
  }
  // Latency drifts a little, so the Status column is not a frozen number.
  if (row.health.state === "ok" && row.health.latency_ms !== null) {
    const drift = Math.round((Math.random() - 0.5) * 90);
    row.health.latency_ms = Math.max(90, row.health.latency_ms + drift);
  }

  footerStats.today_requests += 1;
  footerStats.today_tokens += input + output;
}

/** A row's health flips now and then — the state the Status column shows. */
function simulateOutage(): void {
  const row = providers.find(
    (p) => p.enabled && p.usage && (p.health.state === "ok" || p.health.state === "error"),
  );
  if (!row) return;
  if (row.health.state === "ok") {
    row.health.state = "error";
    row.health.latency_ms = null;
    row.health.error = "sample data: the upstream stopped answering";
  } else {
    row.health.state = "ok";
    row.health.latency_ms = 180 + Math.floor(Math.random() * 900);
    delete row.health.error;
  }
}

let timer: number | undefined;

/**
 * Start the traffic. Requests land every `REQUEST_MS`, with a spread so the
 * screen does not tick like a metronome, and an outage sweep every ~20 s.
 */
export function startSimulation(): void {
  stopSimulation();
  const REQUEST_MS = 1_400;
  timer = window.setInterval(() => {
    // A frame nobody can see costs nothing to skip — an embedded demo left in a
    // background tab should be idle, not busy.
    if (document.hidden) return;
    // One to three requests per beat: enough that the counters and rings move
    // while a visitor is looking at them.
    const bursts = 1 + Math.floor(Math.random() * 3);
    for (let i = 0; i < bursts; i += 1) simulateRequest();
    publishUsageTick();
  }, REQUEST_MS);
  window.setInterval(() => {
    if (!document.hidden) simulateOutage();
  }, 20_000);
}

export function stopSimulation(): void {
  if (timer !== undefined) window.clearInterval(timer);
  timer = undefined;
}
