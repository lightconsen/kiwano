// The Dashboard's fixtures: one authored table per window, and the builders
// that derive every tile from it. Mirrors `vm::build_dashboard`.

import type { AgentId, DashboardData, DashboardWindow } from "../types";
import { AGENTS } from "../types";
// The same formatter the screens print with: a fixture that formats its own
// tokens is a fixture that can disagree with the page about how they read.
import { fmtTokens } from "../../lib/format";

/** The Dashboard's fixtures, computed from one table per window.
 *
 * In the backend every tile is a view of the `usage` rows over the same window
 * (`vm::build_dashboard`): the trend buckets them by day, the two breakdowns
 * group them by provider and by agent, and the headline counts the request log —
 * those rows plus the requests that never reached a provider. Hand-written tiles
 * drift apart from all of that (the numbers here used to disagree about their
 * own total, by provider and by agent and against the chart), and a mock that
 * disagrees with itself is a `pnpm dev` session reporting that a screen is fine
 * when it is not. So the day totals below are authored and everything else is
 * derived.
 *
 * What is *not* derived: the per-provider totals the Apps screen shows. Those
 * are a different window's fixture (`providers[].usage`) and the two are not
 * linked, so a provider's own card can still disagree with this page.
 */

/** One window's day totals: the trend's own numbers, and the source of the rest. */
type WindowDays = { date: string; requests: number; tokens: number }[];

/** How a window's requests and cost split across providers.
 *
 * `offPeakRatio` is what the same request costs at the row's other rates: 1 for
 * a vendor with no schedule, and below 1 for DeepSeek, whose rows carry one —
 * which is what puts a non-zero peak premium on this page.
 */
const PROVIDER_MIX = [
  { id: "deepseek", name: "DeepSeek", color: "#4D6BFE", share: 0.585, costPerRequest: 0.036, offPeakRatio: 0.892 },
  { id: "kimi", name: "Kimi", color: "#555555", share: 0.215, costPerRequest: 0.037, offPeakRatio: 1 },
  { id: "glm", name: "GLM", color: "#3859FF", share: 0.15, costPerRequest: 0.035, offPeakRatio: 1 },
  { id: "ollama", name: "Ollama", color: "#1c1c1e", share: 0.05, costPerRequest: 0, offPeakRatio: 1 },
];

/** Which agents drive the traffic, across every provider. Claude Code heads the
    list because it heads the real one: it is what most of this traffic is. */
const AGENT_MIX: { agent: AgentId; share: number }[] = [
  { agent: "claude", share: 0.77 },
  { agent: "codex", share: 0.2 },
  { agent: "opencode", share: 0.03 },
];

/** Split `total` by `shares` so the parts add back up to the whole. Largest
    remainder, because the point of this file is that the parts *do* add up. */
function split(total: number, shares: number[]): number[] {
  const exact = shares.map((s) => total * s);
  const out = exact.map(Math.floor);
  let rest = total - out.reduce((n, v) => n + v, 0);
  const byRemainder = exact
    .map((v, i) => ({ i, frac: v - Math.floor(v) }))
    .sort((a, b) => b.frac - a.frac);
  for (const { i } of byRemainder) {
    if (rest <= 0) break;
    out[i] += 1;
    rest -= 1;
  }
  return out;
}

const round2 = (n: number) => Math.round(n * 100) / 100;

function buildWindow(
  window: DashboardWindow,
  days: WindowDays,
  opts: { failures: number; deltaPct: number; latency: number; latencyDelta: number },
): DashboardData {
  const requests = days.reduce((n, d) => n + d.requests, 0);
  const tokens = days.reduce((n, d) => n + d.tokens, 0);
  // The headline is the request log: the rows that reached a provider plus the
  // ones that did not (a refused key, an unreachable upstream).
  const headlineRequests = requests + opts.failures;

  const perProvider = split(requests, PROVIDER_MIX.map((p) => p.share));
  const by_provider = PROVIDER_MIX.map((p, i) => {
    const cost = round2(perProvider[i] * p.costPerRequest);
    return {
      id: p.id,
      name: p.name,
      color: p.color,
      requests: perProvider[i],
      // The share is of the *headline*, which is the backend's divisor — so the
      // slices need not reach 100% (traffic with no provider is still traffic).
      pct: Math.round((perProvider[i] * 100) / Math.max(1, headlineRequests)),
      cost,
      cost_off_peak: round2(cost * p.offPeakRatio),
    };
  }).filter((p) => p.requests > 0);

  const cost = round2(by_provider.reduce((n, p) => n + p.cost, 0));
  const costOffPeak = round2(by_provider.reduce((n, p) => n + p.cost_off_peak, 0));
  // Costs are split in cents: the agent rows are the same money as the provider
  // rows, seen from the other side, and they have to add up to it.
  const splitCents = (amount: number) =>
    split(Math.round(amount * 100), AGENT_MIX.map((a) => a.share)).map((c) => c / 100);
  const perAgent = split(requests, AGENT_MIX.map((a) => a.share));
  const perAgentTokens = split(tokens, AGENT_MIX.map((a) => a.share));
  const perAgentCost = splitCents(cost);
  const perAgentOffPeak = splitCents(costOffPeak);
  const by_agent = AGENT_MIX.map((a, i) => ({
    agent: a.agent,
    // The registry's label where it has one; a user-defined agent's traffic is
    // not in this fixture, so the fallback is the id, as in the app.
    label: AGENTS.find((m) => m.id === a.agent)?.label ?? a.agent,
    requests: perAgent[i],
    tokens: fmtTokens(perAgentTokens[i]),
    cost: perAgentCost[i],
    cost_off_peak: perAgentOffPeak[i],
  })).filter((a) => a.requests > 0);

  return {
    window,
    requests: headlineRequests,
    requests_delta_pct: opts.deltaPct,
    // input + output is the trend's own token total, which is how the two are
    // read together on this page.
    input_tokens: Math.round(tokens * 0.85),
    output_tokens: tokens - Math.round(tokens * 0.85),
    cache_read_tokens: Math.round(tokens * 0.85 * 0.57),
    cost,
    cost_off_peak: costOffPeak,
    latency_ms: opts.latency,
    latency_delta_pct: opts.latencyDelta,
    trend: days,
    by_provider,
    by_agent,
    // Derived per request by `getDashboard`, like the backend's.
    filter_providers: [],
    filter_agents: [],
  };
}

export const dashboards: Record<DashboardWindow, DashboardData> = {
  today: buildWindow(
    "today",
    [
      { date: "02:00", requests: 18, tokens: 120_000 },
      { date: "05:00", requests: 24, tokens: 150_000 },
      { date: "08:00", requests: 56, tokens: 360_000 },
      { date: "11:00", requests: 62, tokens: 400_000 },
      { date: "14:00", requests: 48, tokens: 310_000 },
      { date: "17:00", requests: 44, tokens: 260_000 },
      { date: "20:00", requests: 32, tokens: 200_000 },
    ],
    { failures: 7, deltaPct: 4, latency: 1100, latencyDelta: 3 },
  ),
  "7d": buildWindow(
    "7d",
    [
      { date: "09-01", requests: 118, tokens: 900_000 },
      { date: "09-02", requests: 132, tokens: 1_000_000 },
      { date: "09-03", requests: 190, tokens: 1_300_000 },
      { date: "09-04", requests: 176, tokens: 1_200_000 },
      { date: "09-05", requests: 228, tokens: 1_500_000 },
      { date: "09-06", requests: 186, tokens: 1_300_000 },
      { date: "09-07", requests: 254, tokens: 1_600_000 },
    ],
    { failures: 12, deltaPct: 12, latency: 1200, latencyDelta: 9 },
  ),
  "30d": buildWindow(
    "30d",
    [
      { date: "08-09", requests: 196, tokens: 1660000 },
      { date: "08-10", requests: 210, tokens: 1140000 },
      { date: "08-11", requests: 211, tokens: 1460000 },
      { date: "08-12", requests: 186, tokens: 1490000 },
      { date: "08-13", requests: 203, tokens: 1560000 },
      { date: "08-14", requests: 179, tokens: 1450000 },
      { date: "08-15", requests: 207, tokens: 910000 },
      { date: "08-16", requests: 150, tokens: 1070000 },
      { date: "08-17", requests: 202, tokens: 1270000 },
      { date: "08-18", requests: 234, tokens: 960000 },
      { date: "08-19", requests: 241, tokens: 1180000 },
      { date: "08-20", requests: 183, tokens: 920000 },
      { date: "08-21", requests: 180, tokens: 1050000 },
      { date: "08-22", requests: 231, tokens: 1430000 },
      { date: "08-23", requests: 178, tokens: 1670000 },
      { date: "08-24", requests: 151, tokens: 1440000 },
      { date: "08-25", requests: 187, tokens: 1280000 },
      { date: "08-26", requests: 188, tokens: 1170000 },
      { date: "08-27", requests: 192, tokens: 1670000 },
      { date: "08-28", requests: 235, tokens: 1660000 },
      { date: "08-29", requests: 168, tokens: 1210000 },
      { date: "08-30", requests: 245, tokens: 1360000 },
      { date: "08-31", requests: 227, tokens: 1040000 },
      { date: "09-01", requests: 189, tokens: 1390000 },
      { date: "09-02", requests: 152, tokens: 1050000 },
      { date: "09-03", requests: 178, tokens: 1530000 },
      { date: "09-04", requests: 227, tokens: 960000 },
      { date: "09-05", requests: 182, tokens: 1340000 },
      { date: "09-06", requests: 152, tokens: 1490000 },
      { date: "09-07", requests: 197, tokens: 1190000 },
    ],
    { failures: 40, deltaPct: 23, latency: 1300, latencyDelta: 5 },
  ),
  // One bar per week, which is what the backend does once the span outgrows a
  // bar per day: the demo's history is older than that limit on purpose.
  all: buildWindow(
    "all",
    [
      { date: "06-30", requests: 940, tokens: 6_900_000 },
      { date: "07-07", requests: 1120, tokens: 8_100_000 },
      { date: "07-14", requests: 980, tokens: 7_400_000 },
      { date: "07-21", requests: 1250, tokens: 9_300_000 },
      { date: "07-28", requests: 1180, tokens: 8_800_000 },
      { date: "08-04", requests: 1310, tokens: 9_700_000 },
      { date: "08-11", requests: 1240, tokens: 9_200_000 },
      { date: "08-18", requests: 1390, tokens: 10_400_000 },
      { date: "08-25", requests: 1280, tokens: 9_500_000 },
      { date: "09-01", requests: 1150, tokens: 8_600_000 },
    ],
    { failures: 96, deltaPct: 31, latency: 1250, latencyDelta: 7 },
  ),
};
