// The row cells the All tab's provider row and the agent tabs' binding rows
// share, and the All tab's row itself.
import { useEffect, useState } from "react";

import { Gauge, Pause, Play, RefreshCw, SquarePen, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { iconForEndpoint } from "@/components/icons/infer";
import { api } from "../../api/client";
import { useT, type Translate } from "../../i18n";
import { AgentChip, Dot, Logo } from "../../components/bits";
import { agentMeta } from "../../lib/agents";
import { fmtLatency } from "../../lib/format";
import { type AgentRoute, type PlanQuotaReport, type Provider } from "../../api/types";
import { CacheCell, UsageCell } from "./usage";

/** What deleting this provider does to the routes it is in — the sentence the
    row shows while the delete is armed.
 *
 * Every fact it needs is already in the screen: the provider names its agents,
 * and `routes` holds each agent's candidate order, so the promotion ("Kimi
 * becomes primary for Codex") and the empty case ("Codex is left with no
 * provider") are read off the same data the agent tabs render. Two things it
 * deliberately does not say: anything at all when the provider is in no route,
 * and a promotion under roundrobin — there every candidate takes turns, so the
 * order is a priority rather than a primary.
 */
function deleteConsequence(p: Provider, routes: AgentRoute[] | null, t: Translate): string {
  const parts: string[] = [];
  if (p.agents.length === 0) return "";
  const names = p.agents.map((a) => agentMeta(a).label).join(", ");
  parts.push(t("providers.delRemovedFrom", { agents: names }));
  for (const agent of p.agents) {
    const route = routes?.find((r) => r.agent === agent);
    const bindings = route?.bindings ?? [];
    if (bindings.length === 0 || bindings[0].provider_id !== p.id) continue;
    const label = agentMeta(agent).label;
    if (bindings.length === 1) {
      parts.push(t("providers.delEmpties", { agent: label }));
    } else if (route?.strategy !== "roundrobin") {
      parts.push(
        t("providers.delPromotes", { provider: bindings[1].provider_name, agent: label }),
      );
    }
  }
  return parts.join(" · ");
}

/** The row's latency test: one prompt, one number.
 *
 * The number lands on the button — it is a measurement the reader just asked for,
 * a real completion, so a slow model reads slow — but the click is also what
 * makes the Status cell current. The backend records the verdict as this
 * provider's health (`vm::test_provider_latency`), and `onTested` re-reads the
 * row, so the cell shows what the click just found. That is the repair path for a
 * stale or wrong verdict: a cell reading "No answer" while the endpoint plainly
 * answers is exactly what this button can settle.
 */
export function TestLatencyButton({
  provider,
  onTested,
}: {
  provider: Provider;
  onTested: () => void;
}) {
  const t = useT();
  const [state, setState] = useState<
    | { kind: "idle" }
    | { kind: "busy" }
    | { kind: "done"; ms: number; model: string }
    | { kind: "failed"; why: string }
  >({ kind: "idle" });

  const test = async () => {
    setState({ kind: "busy" });
    try {
      const r = await api.testProviderLatency(provider.id);
      if (r.error) setState({ kind: "failed", why: r.error });
      else setState({ kind: "done", ms: r.latency_ms, model: r.model });
    } catch (e) {
      setState({ kind: "failed", why: e instanceof Error ? e.message : String(e) });
    } finally {
      // Either way a verdict was recorded — a failure is a verdict too — so the
      // row is re-read rather than left showing what it showed before.
      onTested();
    }
  };

  const label =
    state.kind === "done"
      ? `${state.ms}ms`
      : state.kind === "failed"
        ? t("providers.testLatencyFailed")
        : t("providers.testLatency");
  // The model in the tooltip is the one the *ping* used, not the row's
  // remembered default: with no default set, the backend falls back to the
  // model the catalog prices, and that is what the number is about.
  const title =
    state.kind === "failed"
      ? state.why
      : state.kind === "done"
        ? `${t("providers.testLatencyTitle")} · ${state.model}`
        : t("providers.testLatencyTitle");

  return (
    <Button
      variant="ghost"
      size="sm"
      className="h-7 gap-1 whitespace-nowrap border border-line px-1.5 text-[10.5px]"
      style={state.kind === "failed" ? { color: "var(--red)" } : undefined}
      aria-label={t("providers.testLatency")}
      title={title}
      disabled={state.kind === "busy"}
      onClick={test}
    >
      {state.kind === "busy" ? (
        <RefreshCw className="h-3.5 w-3.5 animate-spin" />
      ) : (
        <Gauge className="h-3.5 w-3.5" />
      )}
      {state.kind === "idle" ? null : <span>{label}</span>}
    </Button>
  );
}
/** First grid cell: brand mark + name + endpoint subtitle. Shared by the All-tab
    management rows and the agent-tab strategy rows. The All tab badges the
    collapsed is_current; agent tabs pass inUse to badge membership in that
    agent's serving set only. A quota-over backup reads as isFallback — first in
    line, not in use — and gets its own chip, which is a different claim than
    "In use" and must not share its mark. */
export function IdentityCell({ p, inUse, isFallback }: { p: Provider; inUse?: boolean; isFallback?: boolean }) {
  const t = useT();
  // Brand mark inferred from the endpoint host; unknown hosts keep the letter avatar
  const brandIcon = iconForEndpoint(p.endpoint);
  // Per agent the two claims are disjoint, but the All tab collapses them, and
  // a provider can serve one agent while standing first-in-line for another:
  // "In use" is the stronger claim, so it wins the one badge slot.
  const showInUse = inUse ?? p.is_current;
  const showFallback = isFallback === true && !showInUse;
  return (
    <div className="flex min-w-0 w-[31%] items-center gap-2.5">
      {brandIcon ? (
        <ProviderLogo icon={brandIcon} name={p.name} size={32} />
      ) : (
        <Logo char={p.logo_char} color={p.logo_color} border={p.logo_border} />
      )}
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="text-[13px] font-semibold">{p.name}</span>
          {showFallback ? (
            <span
              className="rounded px-1.5 py-px text-[10px] font-medium text-mut"
              style={{ background: "var(--surface2)" }}
              title={t("providers.quotaFallbackTitle")}
            >
              {t("providers.quotaFallback")}
            </span>
          ) : showInUse && (
            <span className="rounded px-1.5 py-px text-[10px] font-medium" style={{ background: "var(--kiwi)", color: "oklch(0.18 0.03 132)" }}>
              {t("providers.inUse")}
            </span>
          )}
          {p.status_badge && (
            <span className="rounded px-1.5 py-px text-[10px] font-medium text-mut" style={{ background: "var(--surface2)" }}>
              {p.status_badge}
            </span>
          )}
        </div>
        {/* Endpoint only. The protocol is a property of how Kiwano talks to
            the provider, not of where it is — it belongs in the edit dialog,
            not in a subtitle every row repeats. */}
        <div className="mt-0.5 truncate font-mono text-[11px] text-mut">{p.endpoint}</div>
      </div>
    </div>
  );
}

/** Status cell: whether Kiwano will use this provider, and anything known that
    says otherwise.
 *
 * Three things can be said here, in this order. **Parked** (the backend's note)
 * and **refused** (a limit, passed in as `blocked`) are facts about Kiwano's own
 * decision. Otherwise the cell shows a **measurement**, and says which one it is,
 * because the two are not the same claim: the provider's own requests through the
 * gateway, with the user's key, or — when it has had no traffic in the last day —
 * the gateway's unsigned GET to its endpoint, which proves something answers
 * there and nothing about authorization. Both are tooltipped with what they are,
 * and the probe with when it ran.
 *
 * Nothing at all is still a legitimate answer: enabled, no traffic yet, no
 * verdict on file. That is the state of a provider added a minute ago. */
export function HealthCell({ p, blocked }: { p: Provider; blocked?: string }) {
  const t = useT();
  // The provider is up and reachable — that is not what changed. What changed
  // is whether Kiwano will use it, which is this column's question.
  if (blocked) {
    return (
      <div className="w-[12%]" title={t("providers.notRoutingTitle", { reason: blocked })}>
        <span className="flex items-center gap-1.5 text-[11.5px]" style={{ color: "var(--red)" }}>
          <Dot state="error" />
          {t("providers.blocked")}
        </span>
        <div className="mt-0.5 truncate text-[10.5px] text-mut">{blocked}</div>
      </div>
    );
  }
  const h = p.health;
  // Parked: out of every route, which outranks any reading of the endpoint.
  if (h.note) {
    return (
      <div className="w-[12%]">
        <span className="flex items-center gap-1.5 text-[11.5px] text-mut">
          <Dot state={h.state} />
          {h.note}
        </span>
      </div>
    );
  }
  // Measured, and not working. Two ways to get here, and the reason is what
  // tells them apart: nothing answered at all, or something answered and
  // refused the key — a 401 is the vendor talking, not the network being quiet,
  // and reading it as silence sends the reader looking in the wrong place.
  if (h.state === "error") {
    const said = h.error;
    return (
      <div
        className="w-[12%]"
        title={
          said
            ? t("providers.refusedTitle", { error: said, time: fmtClock(h.checked_at) })
            : t("providers.unreachableTitle", { time: fmtClock(h.checked_at) })
        }
      >
        <span className="flex items-center gap-1.5 text-[11.5px] text-mut">
          <Dot state="error" />
          {said ? t("providers.refused") : t("providers.unreachable")}
        </span>
      </div>
    );
  }
  if (h.latency_ms == null) return <div className="w-[12%]" />;
  // Whose measurement it is, in the tooltip: your own traffic, the gateway's
  // unsigned GET, or the test you ran — each a different amount of proof, and
  // the last one the only one that exercises the key on demand.
  const title =
    h.source === "traffic"
      ? t("providers.latencyTraffic")
      : h.source === "test"
        ? t("providers.latencyTest", { time: fmtClock(h.checked_at) })
        : t("providers.latencyProbe", { time: fmtClock(h.checked_at) });
  return (
    <div className="w-[12%]" title={title}>
      <span className="flex items-center gap-1.5 text-[11.5px] text-mut">
        <Dot state="ok" />
        <span className="font-mono">{fmtLatency(h.latency_ms)}</span>
      </span>
    </div>
  );
}

/** "HH:MM" out of the RFC3339 instant a probe was taken at — the same reading the
    request log prints, cut to the clock: a tooltip has no room for a date, and a
    verdict older than a day is one the prober has already replaced. */
function fmtClock(ts?: string): string {
  return ts ? ts.slice(11, 16) : "";
}

/** The row's routing state, as the coloured bar on its left edge draws it.
 *
 * That edge is the one channel the eye runs down without reading, so the state
 * lives there and not only in a badge. `useT` is not needed: these are class
 * names, not labels — every state is spelled out in the row as well (the In-use
 * badge, the Status column's Disabled/Blocked), so colour is never the only
 * carrier.
 *
 * Three states only, and the rest of the list stays quiet. A provider that is
 * bound but idle — a failover standby, a queue member whose window is not now —
 * is the ordinary resting state of most rows, and marking it would make the bar
 * a column of grey with the three states that matter lost inside it. */
export function rowState(p: Provider, serving: boolean, blocked?: string): string {
  if (!p.enabled) return "paused";
  if (blocked) return "blocked";
  return serving ? "serving" : "";
}

export function ProviderRow({
  p,
  plan,
  blocked,
  routes,
  onEdit,
  onDelete,
  onChanged,
}: {
  p: Provider;
  plan?: PlanQuotaReport;
  blocked?: string;
  /** Every agent's route: what the delete confirmation reads to say what it does */
  routes: AgentRoute[] | null;
  onEdit: (p: Provider) => void;
  onDelete: (p: Provider) => void;
  onChanged: () => void;
}) {
  const t = useT();
  // Delete is a two-step confirm: the first click enters the confirm state; a second click within 3 seconds actually deletes
  const [confirmDel, setConfirmDel] = useState(false);
  useEffect(() => {
    if (!confirmDel) return;
    const t = setTimeout(() => setConfirmDel(false), 3000);
    return () => clearTimeout(t);
  }, [confirmDel]);
  // While the delete is armed the consequence gets a line of its own under the
  // row: which agents lose the provider, who takes over from it, and whether a
  // route is left with nothing. A line, not a tooltip — this is the moment the
  // reader decides — and not squeezed into the row, whose columns are fixed
  // fractions of its width and have no room for a sentence.
  const consequence = confirmDel ? deleteConsequence(p, routes, t) : "";
  const state = rowState(p, p.is_current, blocked);
  return (
    <>
    <div className={`row group flex h-[58px] items-center border-b border-line px-4${state ? ` ${state}` : ""}`}>
      <IdentityCell p={p} isFallback={p.fallback_agents && p.fallback_agents.length > 0} />

      <div className="flex w-[16%] items-center">
        {p.agents.length === 0 ? (
          <span className="text-[11px] text-mut">{t("providers.unbound")}</span>
        ) : (
          <>
            {/* Mini-logos, slightly overlapping (earlier agents on top); the
                opaque chip background keeps marks legible where they overlap */}
            {p.agents.map((a, i) => (
              <span
                key={a}
                className={`relative inline-flex rounded-[4px] bg-bg${i > 0 ? "-ml-0.5" : ""}`}
                style={{ zIndex: p.agents.length - i }}
              >
                <AgentChip meta={agentMeta(a)} size={14} />
              </span>
            ))}
            {p.agents_note && <span className="ml-1.5 text-[11px] text-mut">{p.agents_note}</span>}
          </>
        )}
      </div>

      <UsageCell p={p} plan={plan} blocked={blocked} />

      <CacheCell u={p.usage} />

      <HealthCell p={p} blocked={blocked} />

      {/* Actions stay out of the resting row: reveal on hover / keyboard focus,
          or while a delete confirmation is pending. Everything that acts on the
          *provider* is here — edit, delete, and the park below — which is where
          they add up to one thing: this row is the only view that speaks for the
          provider on its own, rather than for one agent's use of it. (The old
          `enable` was the exception that made the name ambiguous: it promoted the
          provider to primary for every agent it was bound to, which is the agent
          tabs' `makePrimary` now — see `providers use` in the CLI.) */}
      <div
        className={`flex min-w-[140px] flex-1 items-center justify-end gap-1.5 transition-opacity${
          confirmDel ? "" : " opacity-0 group-hover:opacity-100 focus-within:opacity-100"
        }`}
      >
        <TestLatencyButton provider={p} onTested={onChanged} />
        {/* A player's transport pair: start it, or stop it. The glyph is the
            action the click takes, so a working row offers ⏸ and a parked one
            offers ▶ — and the *state* is already on the row, in the Status cell,
            which is what frees the icons to be verbs. */}

        <Button
          variant="ghost"
          size="sm"
          className="h-7 w-7 shrink-0 rounded-md border border-line px-0 text-mut"
          aria-label={p.enabled ? t("providers.disable") : t("providers.enable")}
          title={p.enabled ? t("providers.disableTitle") : t("providers.enableTitle")}
          onClick={() => api.setProviderEnabled(p.id, !p.enabled).then(onChanged)}
        >
          {p.enabled ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
        </Button>
        {/* Square icon buttons, one per action: the row's actions are symbols a
            reader already knows, and a padded pill around a lone glyph reads as
            a label that failed to load. The delete grows into its confirmation
            word, which is the one state that has something to say. */}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 w-7 shrink-0 rounded-md border border-line px-0 text-mut"
          aria-label={t("common.edit")}
          title={t("providers.editProvider")}
          onClick={() => onEdit(p)}
        >
          <SquarePen className="h-3.5 w-3.5" />
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className={`h-7 shrink-0 rounded-md border border-line text-[10.5px] ${
            confirmDel ? "whitespace-nowrap px-1.5" : "w-7 px-0 text-mut"
          }`}
          style={confirmDel ? { color: "var(--red)", borderColor: "var(--red)" } : undefined}
          aria-label={t("common.delete")}
          title={confirmDel ? t("providers.clickAgain") : t("providers.deleteProvider")}
          onClick={() => (confirmDel ? onDelete(p) : setConfirmDel(true))}
        >
          {confirmDel ? t("providers.confirm") : <Trash2 className="h-3.5 w-3.5" />}
        </Button>
      </div>
    </div>
    {consequence && (
      <div
        className="flex items-center border-b border-line px-4 py-1.5 text-[10.5px]"
        style={{ background: "color-mix(in srgb, var(--red) 10%, transparent)", color: "var(--red)" }}
      >
        <span className="min-w-0 flex-1 truncate">{consequence}</span>
      </div>
    )}
    </>
  );
}
