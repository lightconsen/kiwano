// The agent's route as a picture: agent → gateway → candidates.
//
// The same three-column composition the site's strategy diagrams use, drawn
// from the agent's *live* route instead of a fixed example — so it answers the
// question the table answers serially ("which provider, in what order, doing
// what") in one glance, and carries the strategy's own vocabulary on each
// candidate (rank, weight, time window).
//
// Read-only on purpose. Everything that changes the route — reorder, remove,
// weight, the strategy itself — stays in the table (or the panel below it),
// because a picture is a poor place to hit a small target, and two sets of
// controls over one route is how they start disagreeing.
//
// No measuring: the candidate rows are laid out by the same constants the wires
// are drawn with, so the picture cannot drift from its own layout when the
// window resizes. The wire layer is an SVG whose viewBox is 1000 units wide and
// `H` tall (`preserveAspectRatio="none"`), so x is proportional and y is
// exact — the DOM nodes below position their x in percent of the same units.
import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Pin } from "lucide-react";

import { useT, type KeyPath, type Messages } from "../../i18n";
import { api } from "../../api/client";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { BillTag } from "@/components/bits";
import logoUrl from "@/assets/kiwano-logo.svg";
import { ProviderMark } from "@/components/icons/ProviderMark";
import { agentMeta } from "../../lib/agents";
import { fmtMoney, fmtTokens } from "../../lib/format";
import { SEGMENT_ICON } from "./agents";
import { HealthSummary, TestLatencyButton } from "./rows";
import { planUtilization, providerCurrency, ringColor } from "./usage";
import type { AgentRef, AgentRoute, PlanQuotaReport, Provider, StrategyKind } from "../../api/types";

// Layout constants, in wire-layer units. 1000 units is the full width, so a
// value here is also its percent (160 units = 16%).
const AGENT_EDGE = 160;
const HUB_X = 500;
const PROV_X = 700;
const PROV_LEFT = "70%";
const ROW_H = 62;

/** The strategy's name, from the panel's own vocabulary. */
const STRATEGY_LABEL: Record<StrategyKind, KeyPath<Messages>> = {
  single: "strategy.single",
  failover: "strategy.failover",
  roundrobin: "strategy.roundrobin",
  timewindow: "strategy.timewindow",
  quota: "strategy.quota",
  "least-busy": "strategy.leastBusy",
};

/** The quota strategy's own setting, read the way the panel writes it. */
function quotaText(config: string | null): string | null {
  if (!config) return null;
  try {
    const c = JSON.parse(config) as { limit?: number; unit?: string };
    if (!c.limit) return null;
    return `${c.limit} ${c.unit === "tokens" ? "tok" : "req"}`;
  } catch {
    return null;
  }
}

/** What this candidate's own strategy says about it, beside its name.
    Ranking is the whole vocabulary of `single` and `failover`, and it is
    already on the row — a badge repeating the number would be noise. */
function badgeFor(route: AgentRoute, binding: AgentRoute["bindings"][number]): string | null {
  switch (route.strategy) {
    case "roundrobin":
      return `×${binding.weight}`;
    case "timewindow":
      return binding.win_start && binding.win_end
        ? `${binding.win_start}–${binding.win_end}`
        : null;
    default:
      return null;
  }
}

/** The usage/quota column, in the one line this picture has for it.
 *
 * The same four facts the table's usage cell carries, minus its second line:
 * the billing tag (plan / payg / unl), the percentage the ring would draw, what
 * that percentage is *of* (a group of quota, a plan's live tiers, or the
 * spend), and the window's request count. Everything is `null`-able: a provider
 * with nothing recorded yet shows nothing, which is the table's empty cell too.
 */
function usageParts(
  p: Provider,
  plan: PlanQuotaReport | undefined,
): {
  tag: Provider["billing"];
  pct: number | null;
  color?: string;
  /** The quota amount, the live tier percentages, or the spend. */
  amount: string | null;
  requests: number | null;
} {
  const u = p.usage;
  const requests = u?.requests ?? null;
  if (u?.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    const amount =
      u.quota.unit === "requests" || u.quota.unit === "wan_tokens"
        ? `${u.quota.used}/${u.quota.limit}`
        : `${fmtMoney(u.quota.used, u.quota.unit)}/${fmtMoney(u.quota.limit, u.quota.unit)}`;
    return { tag: p.billing, pct, amount, requests, color: ringColor(p.billing, pct) };
  }
  if (p.billing === "plan" && p.plan_limits) {
    const pct = planUtilization(p, plan);
    // The live tiers, when the endpoint answered: "43%/18%". Absent until it
    // does, and the ring alone is still the right answer then.
    const amount = plan?.success
      ? plan.tiers.map((tier) => `${Math.round(tier.utilization)}%`).join("/")
      : null;
    return {
      tag: p.billing,
      pct,
      amount,
      requests,
      color: pct != null ? ringColor(p.billing, pct) : undefined,
    };
  }
  if (u) {
    // No ceiling, so the line says what the window cost — in the provider's own
    // currency, never converted — or, for a provider nothing prices, the tokens
    // it carried.
    const amount =
      u.cost != null
        ? fmtMoney(u.cost, u.cost_currency ?? providerCurrency(p) ?? "USD")
        : `${fmtTokens(u.input_tokens + u.output_tokens)} tok`;
    return { tag: p.billing, pct: null, amount, requests };
  }
  return { tag: p.billing, pct: null, amount: null, requests };
}

/** Share of input-side tokens served from cache — `CacheCell`'s arithmetic,
    without its column. `null` when the window holds no input-side tokens: a 0%
    there would read as "the cache never hits" where the truth is "nothing yet". */
function cachePct(u: Provider["usage"]): number | null {
  if (!u) return null;
  const denom = u.input_tokens + u.cache_read_tokens + u.cache_creation_tokens;
  if (denom === 0) return null;
  return Math.round((u.cache_read_tokens / denom) * 100);
}

/** How long a wire stays lit after the provider behind it recorded a request. */
const FLASH_MS = 900;

/** The pulse's travel time — the request's own path, agent → gateway → provider.
    A little under [`FLASH_MS`], so the light has arrived at the provider before
    the wire dims back to its resting glow. */
const PULSE_MS = 800;

/** The numbers whose movement means "this provider just carried a request". */
function usageMark(u: Provider["usage"] | undefined): string {
  if (!u) return "";
  return `${u.requests}|${u.input_tokens}|${u.output_tokens}|${u.cache_read_tokens}|${u.cache_creation_tokens}`;
}

/**
 * Which candidates just took a request.
 *
 * Judged on the providers' own counters rather than on the gateway's
 * `usage-changed` tick: that event says only "something happened somewhere",
 * and a graph for one agent must not flash because another agent's traffic
 * moved. The hook compares each provider's numbers against the last render and
 * lights the ones that changed — the same idea as the usage cell's tint, at the
 * other end of the wire.
 *
 * `seq` counts the moves, not the providers: it keys the travelling pulse, so a
 * request arriving during another's pulse restarts the light at the agent
 * instead of inheriting the frame the previous one is standing on.
 *
 * The first read never flashes: those numbers did not move, they arrived.
 */
function useUsageFlash(marks: Record<string, string>): { lit: Set<string>; seq: number } {
  const [flash, setFlash] = useState<{ lit: Set<string>; seq: number }>({
    lit: new Set(),
    seq: 0,
  });
  const previous = useRef<Record<string, string> | null>(null);
  const key = JSON.stringify(marks);
  useEffect(() => {
    const before = previous.current;
    previous.current = marks;
    if (!before) return;
    const moved = Object.keys(marks).filter(
      (id) => before[id] !== undefined && before[id] !== marks[id],
    );
    if (moved.length === 0) return;
    setFlash((f) => ({ lit: new Set(moved), seq: f.seq + 1 }));
    const timer = window.setTimeout(() => setFlash((f) => ({ ...f, lit: new Set() })), FLASH_MS);
    return () => window.clearTimeout(timer);
    // Re-run on the numbers, not on identity: `marks` is rebuilt every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return flash;
}

export function RouteGraph({
  agent,
  route,
  byId,
  blocked,
  planQuotas,
  onChanged,
}: {
  agent: AgentRef;
  route: AgentRoute;
  byId: Map<string, Provider>;
  /** Why the gateway refuses to route to a provider right now, by id. */
  blocked: Record<string, string>;
  /** Live plan-quota reports, by provider id — what the usage ring reads for a
      plan provider, the same read the table's usage column makes. */
  planQuotas: Record<string, PlanQuotaReport>;
  /** Something about the route changed (a reorder) — the screen re-reads. */
  onChanged: () => void;
}) {
  const t = useT();
  const meta = agentMeta(agent);
  // Each candidate keeps its *real* index in the stored route: a binding whose
  // provider row is gone is skipped on screen but still exists, and a reorder
  // computed from the visible ones would silently drop it.
  const candidates = route.bindings
    .map((b, index) => ({ b, index }))
    .filter(({ b }) => byId.has(b.provider_id));
  const bindings = candidates.map(({ b }) => b);
  const height = Math.max(176, bindings.length * ROW_H + 40);
  const firstY = (height - (bindings.length - 1) * ROW_H) / 2;
  const hubY = height / 2;
  /** Move the candidate at `from` to `to`, then let the screen re-read. The
      list is the *stored* one, including bindings this picture cannot draw. */
  const moveTo = (from: number, to: number) => {
    const ids = route.bindings.map((b) => b.provider_id);
    if (to < 0 || to >= ids.length || to === from) return;
    const [moved] = ids.splice(from, 1);
    ids.splice(to, 0, moved);
    void api.reorderAgentBindings(route.agent, ids).then(onChanged);
  };

  const flash = useUsageFlash(
    Object.fromEntries(candidates.map(({ b }) => [b.provider_id, usageMark(byId.get(b.provider_id)?.usage)])),
  );

  const caption = [t(STRATEGY_LABEL[route.strategy]), quotaText(route.config)]
    .filter(Boolean)
    .join(" · ");

  /** The agent's own wire, which every candidate's leg attaches to. */
  const trunk = `M ${AGENT_EDGE} ${hubY} L ${HUB_X - 22} ${hubY}`;

  return (
    <div
      className="relative mx-10 my-4 overflow-x-auto"
      style={{ height }}
      role="img"
      aria-label={t("providers.graphAria", {
        agent: meta.label,
        strategy: t(STRATEGY_LABEL[route.strategy]),
        n: bindings.length,
      })}
    >
      <div className="relative min-w-[560px]" style={{ height }}>
        {/* Wires behind the chips. `preserveAspectRatio="none"` is what lets x
            be proportional while y stays exact — the two axes answer different
            questions here. */}
        <svg
          className="absolute inset-0 h-full w-full"
          viewBox={`0 0 1000 ${height}`}
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          {/* The trunk stays a quiet neutral wire, whichever candidate is
              serving: it is the one stretch of the route that carries *every*
              request, so painting it kiwi would put a permanent bright line
              where the travelling light is meant to be the thing you notice.
              Dark grey is the backdrop the pulse reads against. */}
          <path
            d={trunk}
            fill="none"
            stroke="var(--line)"
            strokeWidth="1.5"
            vectorEffect="non-scaling-stroke"
          />
          {bindings.map((b, i) => {
            const p = byId.get(b.provider_id)!;
            const y = firstY + i * ROW_H;
            const serving = p.serving_agents.includes(agent);
            const refused = blocked[p.id];
            const path = `M ${HUB_X + 22} ${hubY} C ${HUB_X + 120} ${hubY}, ${PROV_X - 120} ${y}, ${PROV_X} ${y}`;
            // The request's whole way, not just the candidate's leg of it: the
            // pulse crosses the agent's wire and the gateway before it reaches
            // the provider, which is what the request itself did.
            const journey = `M ${AGENT_EDGE} ${hubY} L ${HUB_X} ${hubY} C ${HUB_X + 120} ${hubY}, ${PROV_X - 120} ${y}, ${PROV_X} ${y}`;
            // A wider copy underneath that exists only while a request is
            // landing: the glow that answers "this one just took traffic". A
            // second path rather than an SVG filter, because this canvas is
            // stretched horizontally and a blur would stretch with it.
            const lit = flash.lit.has(p.id);
            return (
              <g key={b.provider_id}>
                <path
                  d={path}
                  fill="none"
                  stroke={refused ? "transparent" : "var(--kiwi)"}
                  strokeWidth={lit ? 7 : 5}
                  strokeLinecap="round"
                  strokeOpacity={refused ? 0 : lit ? 0.6 : 0}
                  style={{ transition: `stroke-opacity ${FLASH_MS}ms ease-out` }}
                  vectorEffect="non-scaling-stroke"
                />
                {/* Resting wire, in the same neutral as the trunk: the route is
                    dark grey until a request lights it, and the light is then
                    the only green on the canvas. Serving and standby differ by
                    opacity alone — a candidate carrying traffic is drawn at
                    full strength, the ones behind it faint. */}
                <path
                  d={path}
                  fill="none"
                  stroke={refused ? "var(--red)" : "var(--line)"}
                  strokeWidth="1.5"
                  strokeDasharray={refused ? "4 3" : undefined}
                  opacity={serving || refused ? 1 : 0.55}
                  vectorEffect="non-scaling-stroke"
                />
                {lit && (
                  // The light travelling the wire. `pathLength` normalises the
                  // dash to a fraction of any route, so it reads the same whether
                  // the candidate is the first row or the fourth; `seq` keys it,
                  // so a second request restarts the travel from the agent.
                  // The movement itself is the `route-pulse` keyframe in
                  // index.css — SMIL would show only its frozen end frame.
                  <path
                    key={flash.seq}
                    d={journey}
                    fill="none"
                    stroke="var(--kiwi)"
                    strokeWidth="2.5"
                    strokeLinecap="round"
                    strokeDasharray="10 90"
                    pathLength={100}
                    vectorEffect="non-scaling-stroke"
                    style={{ animation: `route-pulse ${PULSE_MS}ms linear forwards` }}
                  />
                )}
              </g>
            );
          })}
        </svg>

        {/* Left: who is asking. No rounded rectangle behind it: the agent reads
            as a label on the wire, the way the candidates do on the other side,
            so nothing in this picture is a box. */}
        <div
          className="absolute flex flex-col items-center gap-1"
          // Anchored by its right edge, so the block grows leftward from the
          // wire rather than into it — and hung from the logo's centre, the
          // same half-icon offset the hub uses, so the wire meets the mark and
          // the name sits under it.
          style={{ right: "85%", top: hubY, transform: "translateY(-16px)" }}
        >
          <ProviderLogo
            icon={SEGMENT_ICON[agent]}
            char={meta.chip_char}
            color={meta.chip_color}
            name={meta.label}
            size={32}
          />
          <span className="max-w-[130px] truncate text-[11px] font-medium">{meta.label}</span>
        </div>

        {/* Middle: the gateway, with what it is doing on this route. */}
        <div
          className="absolute flex flex-col items-center gap-1"
          // Half the icon's height (h-8 = 32px), not half the block: the wires
          // meet the mark, and the two lines of text hang below it.
          style={{ left: `${HUB_X / 10}%`, top: hubY, transform: "translate(-50%, -16px)" }}
        >
          <img src={logoUrl} alt="" className="h-8 w-8 rounded-lg" />
          {/* Named, not just pictured: the mark alone is ambiguous on a screen
              where every candidate is also a logo. */}
          <span className="whitespace-nowrap text-[11px] font-semibold leading-none">Kiwano</span>
          <span className="whitespace-nowrap text-[10px] text-mut">{caption}</span>
        </div>

        {/* Right: the candidates, in priority order. Each row carries what the
            table's four data columns carry, in the same left-to-right order:
            rank (Priority), the usage ring and its amount, the cache share,
            and the status verdict. */}
        {candidates.map(({ b, index }, i) => {
          const p = byId.get(b.provider_id)!;
          const refused = blocked[p.id];
          const badge = badgeFor(route, b);
          const usage = usageParts(p, planQuotas[p.id]);
          const cache = cachePct(p.usage);
          return (
            <div
              key={b.provider_id}
              className="group absolute flex flex-col gap-0.5"
              style={{ left: PROV_LEFT, right: "1%", top: firstY + i * ROW_H, transform: "translateY(-50%)" }}
            >
              {/* Who it is, and where it sits in the queue. */}
              <div className="flex items-center gap-1.5" title={refused ?? undefined}>
                <span
                  className="w-3 shrink-0 text-right font-mono text-[10px] text-mut"
                  title={t("providers.colPriority")}
                >
                  {index + 1}
                </span>
                <ProviderMark provider={p} size={16} />
                <span className={`min-w-0 flex-1 truncate text-[11.5px] ${refused ? "text-mut" : ""}`}>
                  {p.name}
                </span>
                {badge && <span className="shrink-0 font-mono text-[10px] text-mut">{badge}</span>}
                {/* Priority: the same three moves the table's column offers,
                    revealed on hover the way its buttons are — invisible, not
                    absent, so the name does not shift when they appear. */}
                <span className="flex w-[80px] shrink-0 items-center justify-end gap-0.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
                  {route.strategy !== "roundrobin" && (
                    <button
                      type="button"
                      className={`rounded p-0.5 text-mut hover:text-ink${index === 0 ? " invisible" : ""}`}
                      aria-label={t("providers.makePrimary")}
                      title={t("providers.makePrimaryTitle")}
                      onClick={() => moveTo(index, 0)}
                    >
                      <Pin className="h-3 w-3" />
                    </button>
                  )}
                  <TestLatencyButton provider={p} onTested={onChanged} compact />
                  {route.bindings.length > 1 && (
                    <>
                      <button
                        type="button"
                        className="rounded p-0.5 text-mut hover:text-ink disabled:opacity-30"
                        aria-label={t("providers.moveUp")}
                        disabled={index === 0}
                        onClick={() => moveTo(index, index - 1)}
                      >
                        <ArrowUp className="h-3 w-3" />
                      </button>
                      <button
                        type="button"
                        className="rounded p-0.5 text-mut hover:text-ink disabled:opacity-30"
                        aria-label={t("providers.moveDown")}
                        disabled={index === route.bindings.length - 1}
                        onClick={() => moveTo(index, index + 1)}
                      >
                        <ArrowDown className="h-3 w-3" />
                      </button>
                    </>
                  )}
                </span>
              </div>
              {/* What the table's four columns say, together on the line(s)
                  under the name: billing, usage, requests, cache, status.
                  `flex-wrap`, not `truncate`: the column is a fixed share of the
                  window, and ellipsising the amount — the one figure the line
                  exists for — would hide it exactly where the window is small.
                  A wrapped second line still fits inside ROW_H. */}
              <div className="flex flex-wrap items-center gap-x-1.5 gap-y-0 pl-[26px] font-mono text-[10.5px] text-mut">
                {usage.pct != null && (
                  <span className="shrink-0" style={{ color: usage.color }} title={t("providers.colUsage")}>
                    {usage.pct}%
                  </span>
                )}
                {/* Billing tag and what it is billing, as one unit: a wrap that
                    put "PAYG" on one line and its amount on the next would read
                    as two separate facts. */}
                <span className="flex shrink-0 items-center gap-1.5">
                  <BillTag billing={usage.tag} />
                  {usage.amount && <span>{usage.amount}</span>}
                </span>
                {/* The window's request count: the table shows it only on rows
                    without a quota, and it is the number a reader checking
                    "is this provider carrying traffic at all" wants on every
                    row. */}
                {usage.requests != null && (
                  <span className="shrink-0">
                    {usage.requests.toLocaleString()} {t("providers.unitReq")}
                  </span>
                )}
                <span
                  className="shrink-0"
                  title={
                    cache == null
                      ? t("providers.cacheNoData")
                      : t("providers.cacheHitTitle", {
                          pct: String(cache),
                          read: fmtTokens(p.usage?.cache_read_tokens ?? 0),
                          denom: fmtTokens(
                            (p.usage?.input_tokens ?? 0) +
                              (p.usage?.cache_read_tokens ?? 0) +
                              (p.usage?.cache_creation_tokens ?? 0),
                          ),
                        })
                  }
                >
                  {cache == null ? "–" : t("providers.graphCache", { pct: cache })}
                </span>
                {/* `ml-auto` so the verdict sits at the row's right edge — the
                    status column's own place in the table — whether it fits on
                    the first line or ends up on the second. */}
                <span className="ml-auto flex shrink-0 items-center">
                  {refused ? (
                    <span style={{ color: "var(--red)" }}>{t("providers.blocked")}</span>
                  ) : (
                    <HealthSummary p={p} />
                  )}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
