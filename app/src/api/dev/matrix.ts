// The billing × limit × strategy matrix: one row per combination the Apps
// screen's Usage cell can render. `providers.ts` appends them to its table.

import type { Billing, PlanLimits, PlanQuery, ProviderHealth, UsageSummary } from "../types";

// ── The billing × limit × strategy matrix ───────────────────────────────────
//
// `UsageCellBody` branches on a provider's billing and on which limit it
// carries, and the rows above cover four corners of that: payg + a currency
// limit, plan + a quota, unlimited, and a parked one. This block is the rest of
// the table — one row per combination the cell can render, including the two
// shapes the gateway reports *refusing* to route (an over-spend and an
// over-ceiling). Each row is also bound to an agent whose strategy makes the
// columns around the cell vary: a failover standby, a weighted rotation, a
// night window, a quota ceiling.
//
// `pnpm dev` → Apps → All is the page they exist for. They are deliberately
// absent from the Dashboard fixtures, which are built from PROVIDER_MIX: a demo
// table whose totals move every time a cell gains a state is a worse demo, and
// those numbers are asserted against each other.
type MatrixSpec = {
  id: string;
  /** The combination, not a vendor — the Provider column is its own legend. */
  name: string;
  logo: string;
  color: string;
  billing: Billing;
  /** Payg: the unit its spending limit is counted in. */
  limit_unit?: string;
  plan_price?: string;
  plan_limits?: PlanLimits;
  plan_query?: PlanQuery;
  usage: UsageSummary | null;
  /** The gateway's own sentence, for a row it is refusing to route. */
  blocked?: string;
  /** What the Status column says. Absent = enabled with nothing measured yet,
      which is the blank cell (see `health` in the row builder below). */
  health?: ProviderHealth;
};

export const MATRIX: MatrixSpec[] = [
  {
    id: "m-payg-cny",
    name: "PAYG · CNY limit",
    logo: "¥",
    color: "#4D6BFE",
    billing: "payg",
    limit_unit: "CNY",
    usage: {
      requests: 796,
      input_tokens: 5_400_000,
      cache_read_tokens: 4_300_000,
      cache_creation_tokens: 600_000,
      output_tokens: 800_000,
      cost: 28.6,
      cost_currency: "CNY",
      latency_ms: 1100,
      quota: { used: 28.6, limit: 50, unit: "CNY", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-payg-req",
    name: "PAYG · request limit",
    logo: "R",
    color: "#7C3AED",
    billing: "payg",
    limit_unit: "requests",
    usage: {
      requests: 640,
      input_tokens: 2_100_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 420_000,
      cost: 42.5,
      cost_currency: "CNY",
      latency_ms: 860,
      quota: { used: 640, limit: 1000, unit: "requests", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-payg-wan",
    name: "PAYG · 10k-token limit",
    logo: "T",
    color: "#0EA5E9",
    billing: "payg",
    limit_unit: "wan_tokens",
    usage: {
      requests: 210,
      input_tokens: 900_000,
      cache_read_tokens: 120_000,
      cache_creation_tokens: 300_000,
      output_tokens: 300_000,
      cost: 18.2,
      cost_currency: "CNY",
      latency_ms: 1220,
      quota: { used: 120, limit: 500, unit: "wan_tokens", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-payg-trend",
    name: "PAYG · no limit",
    logo: "~",
    color: "#16A34A",
    billing: "payg",
    usage: {
      requests: 148,
      input_tokens: 610_000,
      cache_read_tokens: 40_000,
      cache_creation_tokens: 0,
      output_tokens: 190_000,
      cost: 12.4,
      cost_currency: "CNY",
      latency_ms: 940,
      quota: null,
      spark: [4, 7, 3, 9, 6, 11, 8],
    },
  },
  {
    id: "m-payg-unpriced",
    name: "PAYG · unpriced",
    logo: "?",
    color: "#6B7280",
    billing: "payg",
    usage: {
      requests: 12,
      input_tokens: 44_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 9_800,
      cost: null,
      cost_currency: null,
      latency_ms: 1310,
      quota: null,
      // One day of use: the series that has no span to draw, and the reason the
      // sparkline draws a single sample as a dot.
      spark: [3],
    },
  },
  {
    id: "m-payg-over",
    name: "PAYG · over its limit",
    logo: "!",
    color: "#B91C1C",
    billing: "payg",
    limit_unit: "CNY",
    // `LimitState::describe`'s Spend arm with no reset window, which is what a
    // provider limited only by the form carries (`reset_period` is the CLI's and
    // an import's to set).
    blocked: "52.40 of 50.00 CNY this period",
    usage: {
      requests: 1_204,
      input_tokens: 8_100_000,
      cache_read_tokens: 5_600_000,
      cache_creation_tokens: 900_000,
      output_tokens: 1_100_000,
      cost: 52.4,
      cost_currency: "CNY",
      latency_ms: 1180,
      quota: { used: 52.4, limit: 50, unit: "CNY", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-plan-quota",
    name: "PLAN · plan quota",
    logo: "P",
    color: "#D97757",
    billing: "plan",
    plan_price: "¥49/mo",
    plan_query: { template: "kimi" },
    usage: {
      requests: 295,
      input_tokens: 1_600_000,
      cache_read_tokens: 300_000,
      cache_creation_tokens: 100_000,
      output_tokens: 300_000,
      cost: 10.9,
      cost_currency: "CNY",
      latency_ms: 287,
      quota: { used: 295, limit: 460, unit: "requests", resets_at: "2026-09-30" },
      spark: null,
    },
  },
  {
    id: "m-plan-ceiling",
    name: "PLAN · 50% / 90% ceiling",
    logo: "%",
    color: "#DB2777",
    billing: "plan",
    plan_price: "¥20/mo",
    // The report's five-hour utilization is 42.5%, so this ceiling is at 85% —
    // the amber ring. No usage summary: the cell renders from the live report
    // and the ceiling alone.
    plan_limits: { five_hour: 50, weekly: 90 },
    plan_query: { template: "zhipu" },
    usage: null,
    // Nothing has run through it, so the only thing that could have measured it
    // is the row's own Test — which is the case that button exists for.
    health: { state: "ok", latency_ms: 218, source: "test", checked_at: "2026-09-15T11:38:00Z" },
  },
  {
    id: "m-plan-over",
    name: "PLAN · ceiling reached",
    logo: "!",
    color: "#991B1B",
    billing: "plan",
    plan_price: "¥20/mo",
    // Below the reported 42.5%: the ring is full and the gateway refuses.
    // `LimitState::describe`'s PlanWindow arm, verbatim.
    plan_limits: { five_hour: 40, weekly: 100 },
    plan_query: { template: "minimax" },
    blocked: "five_hour window at 43% of a 40% ceiling",
    usage: null,
  },
  {
    id: "m-plan-none",
    name: "PLAN · no quota endpoint",
    logo: "∅",
    color: "#A16207",
    billing: "plan",
    plan_price: "¥99/mo",
    // A plan that publishes no way to read its usage and carries no ceiling:
    // the cell has no plan branch to take.
    usage: {
      requests: 12,
      input_tokens: 88_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 21_000,
      cost: 3.2,
      cost_currency: "CNY",
      latency_ms: 700,
      quota: null,
      spark: null,
    },
  },
  {
    id: "m-key-refused",
    name: "PAYG · key refused",
    logo: "×",
    color: "#9F1239",
    billing: "payg",
    usage: null,
    // Reachable, and unusable: the vendor answered and said no. The cell has to
    // read as the key rather than as silence — a 401 is not "nobody is home".
    health: {
      state: "error",
      latency_ms: 60,
      source: "test",
      checked_at: "2026-09-15T11:38:00Z",
      error: "invalid API key",
    },
  },
  {
    id: "m-unl",
    name: "UNL · local",
    logo: "∞",
    color: "#1C1C1E",
    billing: "unl",
    usage: {
      requests: 31,
      input_tokens: 150_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 50_000,
      cost: null,
      cost_currency: null,
      latency_ms: null,
      quota: null,
      spark: null,
    },
  },
  {
    id: "m-unl-empty",
    name: "UNL · no history yet",
    logo: "·",
    color: "#3F3F46",
    billing: "unl",
    // A provider nothing has run through: the cell renders nothing at all, which
    // is a state too (the ring and the limit line need no totals, this one has
    // neither).
    usage: null,
    // Nothing has run through it, so the prober is the only thing that can say
    // anything about it — which is the case the loop exists for.
    health: { state: "ok", latency_ms: 18, source: "probe", checked_at: "2026-09-15T07:20:00Z" },
  },
];
