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
import { useT, type KeyPath, type Messages } from "../../i18n";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import logoUrl from "@/assets/kiwano-logo.svg";
import { iconForEndpoint } from "@/components/icons/infer";
import { agentMeta } from "../../lib/agents";
import { SEGMENT_ICON } from "./agents";
import type { AgentRef, AgentRoute, Provider, StrategyKind } from "../../api/types";

// Layout constants, in wire-layer units. 1000 units is the full width, so a
// value here is also its percent (160 units = 16%).
const AGENT_EDGE = 160;
const HUB_X = 500;
const PROV_X = 780;
const ROW_H = 56;

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

export function RouteGraph({
  agent,
  route,
  byId,
  blocked,
}: {
  agent: AgentRef;
  route: AgentRoute;
  byId: Map<string, Provider>;
  /** Why the gateway refuses to route to a provider right now, by id. */
  blocked: Record<string, string>;
}) {
  const t = useT();
  const meta = agentMeta(agent);
  const bindings = route.bindings.filter((b) => byId.has(b.provider_id));
  const height = Math.max(176, bindings.length * ROW_H + 40);
  const firstY = (height - (bindings.length - 1) * ROW_H) / 2;
  const hubY = height / 2;
  const caption = [t(STRATEGY_LABEL[route.strategy]), quotaText(route.config)]
    .filter(Boolean)
    .join(" · ");

  return (
    <div
      className="relative mx-4 my-3 overflow-x-auto rounded-lg border border-line bg-surface"
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

        {/* Left: who is asking. */}
        <div
          className="absolute flex items-center gap-2 rounded-lg border border-line-strong bg-surface2 px-2.5 py-1.5"
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
          style={{ left: "50%", top: hubY, transform: "translate(-50%, -50%)" }}
        >
          <img src={logoUrl} alt="" className="h-8 w-8 rounded-lg" />
          <span className="whitespace-nowrap text-[10px] text-mut">{caption}</span>
        </div>

        {/* Right: the candidates, in priority order. */}
        {bindings.map((b, i) => {
          const p = byId.get(b.provider_id)!;
          const serving = p.serving_agents.includes(agent);
          const refused = blocked[p.id];
          const badge = badgeFor(route, b);
          return (
            <div
              key={b.provider_id}
              className="absolute flex items-center gap-2 rounded-lg px-2 py-1"
              style={{ left: "78%", right: "2%", top: firstY + i * ROW_H, transform: "translateY(-50%)" }}
              title={refused ?? undefined}
            >
              <span className="w-3 shrink-0 text-right font-mono text-[10px] text-mut">{i + 1}</span>
              {iconForEndpoint(p.endpoint) ? (
                <ProviderLogo icon={iconForEndpoint(p.endpoint)} name={p.name} size={20} />
              ) : (
                <ProviderLogo char={p.logo_char} color={p.logo_color} name={p.name} size={20} />
              )}
              <span className={`min-w-0 truncate text-[11.5px] ${refused ? "text-mut" : ""}`}>
                {p.name}
              </span>
              {badge && <span className="shrink-0 font-mono text-[10px] text-mut">{badge}</span>}
              {/* The live half: which candidate is actually carrying this
                  agent's traffic, and which ones the gateway is refusing. */}
              {refused ? (
                <span className="shrink-0 text-[10px]" style={{ color: "var(--red)" }}>
                  {t("providers.blocked")}
                </span>
              ) : serving ? (
                <span
                  className="h-1.5 w-1.5 shrink-0 rounded-full"
                  style={{ background: "var(--kiwi)" }}
                  title={t("providers.inUse")}
                />
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
