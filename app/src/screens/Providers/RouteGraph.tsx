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
import { ArrowDown, ArrowUp, Pin } from "lucide-react";

import { useT, type KeyPath, type Messages } from "../../i18n";
import { api } from "../../api/client";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import logoUrl from "@/assets/kiwano-logo.svg";
import { iconForEndpoint } from "@/components/icons/infer";
import { agentMeta } from "../../lib/agents";
import { fmtMoney, fmtTokens } from "../../lib/format";
import { SEGMENT_ICON } from "./agents";
import { HealthSummary, TestLatencyButton } from "./rows";
import { planUtilization, ringColor } from "./usage";
import type { AgentRef, AgentRoute, PlanQuotaReport, Provider, StrategyKind } from "../../api/types";

// Layout constants, in wire-layer units. 1000 units is the full width, so a
// value here is also its percent (160 units = 16%).
const AGENT_EDGE = 160;
const HUB_X = 500;
const PROV_X = 720;
const PROV_LEFT = "72%";
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
 * The ring carries the percentage (the table's own ring, same thresholds), and
 * the text carries what the percentage is *of*: a group of quota, a plan's live
 * tiers, or the window's tokens. `null` text means the provider has nothing to
 * show yet, which is the table's empty cell too. */
function usageParts(
  p: Provider,
  plan: PlanQuotaReport | undefined,
): { pct: number | null; text: string | null; color?: string } {
  const u = p.usage;
  if (u?.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    const amount =
      u.quota.unit === "requests" || u.quota.unit === "wan_tokens"
        ? `${u.quota.used}/${u.quota.limit}`
        : `${fmtMoney(u.quota.used, u.quota.unit)}/${fmtMoney(u.quota.limit, u.quota.unit)}`;
    return { pct, text: amount, color: ringColor(p.billing, pct) };
  }
  if (p.billing === "plan" && p.plan_limits) {
    const pct = planUtilization(p, plan);
    // The live tiers, when the endpoint answered: "5h 43%". Absent until it
    // does, and the ring alone is still the right answer then.
    const text = plan?.success
      ? plan.tiers.map((tier) => `${Math.round(tier.utilization)}%`).join("/")
      : null;
    return { pct, text, color: pct != null ? ringColor(p.billing, pct) : undefined };
  }
  if (u) {
    return { pct: null, text: `${fmtTokens(u.input_tokens + u.output_tokens)} tok` };
  }
  return { pct: null, text: null };
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

  const caption = [t(STRATEGY_LABEL[route.strategy]), quotaText(route.config)]
    .filter(Boolean)
    .join(" · ");

  return (
    <div
      className="relative mx-4 my-3 overflow-x-auto"
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
          <path
            d={`M ${AGENT_EDGE} ${hubY} L ${HUB_X - 22} ${hubY}`}
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
            return (
              <path
                key={b.provider_id}
                d={`M ${HUB_X + 22} ${hubY} C ${HUB_X + 120} ${hubY}, ${PROV_X - 120} ${y}, ${PROV_X} ${y}`}
                fill="none"
                stroke={refused ? "var(--red)" : serving ? "var(--kiwi)" : "var(--line)"}
                strokeWidth={serving ? 2 : 1.5}
                strokeDasharray={refused ? "4 3" : undefined}
                opacity={serving || refused ? 1 : 0.55}
                vectorEffect="non-scaling-stroke"
              />
            );
          })}
        </svg>

        {/* Left: who is asking. No rounded rectangle behind it: the agent reads
            as a label on the wire, the way the candidates do on the other side,
            so nothing in this picture is a box. */}
        <div
          className="absolute flex items-center gap-2"
          style={{ left: "2%", top: hubY, transform: "translateY(-50%)", maxWidth: "14%" }}
        >
          <ProviderLogo
            icon={SEGMENT_ICON[agent]}
            char={meta.chip_char}
            color={meta.chip_color}
            name={meta.label}
            size={18}
          />
          <span className="truncate font-mono text-[11px]">{meta.label}</span>
        </div>

        {/* Middle: the gateway, with what it is doing on this route. */}
        <div
          className="absolute flex flex-col items-center gap-1"
          style={{ left: `${HUB_X / 10}%`, top: hubY, transform: "translate(-50%, -50%)" }}
        >
          <img src={logoUrl} alt="" className="h-8 w-8 rounded-lg" />
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
                {iconForEndpoint(p.endpoint) ? (
                  <ProviderLogo icon={iconForEndpoint(p.endpoint)} name={p.name} size={16} />
                ) : (
                  <ProviderLogo char={p.logo_char} color={p.logo_color} name={p.name} size={16} />
                )}
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
              {/* The table's other three columns, together on the line under
                  the name: usage, cache, status. */}
              <div className="flex items-center gap-1.5 pl-[26px] font-mono text-[10.5px] text-mut">
                {usage.pct != null && (
                  <span style={{ color: usage.color }} title={t("providers.colUsage")}>
                    {usage.pct}%
                  </span>
                )}
                {usage.text && <span className="truncate">{usage.text}</span>}
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
                <span className="flex shrink-0 items-center">
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
