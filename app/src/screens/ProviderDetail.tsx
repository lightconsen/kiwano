// One provider's own page (#providers/one/<id>).
//
// The All tab is an index: one row per provider, sized for comparing them. This
// is the subject — what this provider is, who it serves, how it is doing, and
// what it has carried lately. Read-only by design: every control that changes
// it (edit, park, delete) stays on the row it was already on, because a page
// reached by double-clicking is a place to look, and moving the destructive
// ones here would put them one mis-click away from a list that is mostly
// double-clicking.
//
// The fleet figures come from the same `listProviders` read the list uses, so
// the two can never disagree about a number — the detail is a wider view of a
// row, not a second opinion about it.
import { useEffect, useMemo, useState } from "react";
import { ChevronRight, Gauge, KeyRound, Pencil } from "lucide-react";

import { api } from "../api/client";
import { LoadFailed, errText } from "../components/LoadFailed";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { iconForEndpoint } from "@/components/icons/infer";
import { Dot } from "../components/bits";
import { agentMeta } from "../lib/agents";
import { fmtLatency, fmtMoney, fmtTokens } from "../lib/format";
import { useT } from "../i18n";
import type {
  AgentDetect,
  AgentRoute,
  AgentRef,
  ApiKeyEntry,
  Provider,
  RequestLogEntry,
} from "../api/types";

/** How many of this provider's most recent requests the page lists. */
const RECENT = 12;

/** What this provider's requests cost, in the currency it bills in — absent
    when the summary carries no figure (no priced traffic in the window). */
function cost(p: Provider): { currency: string; amount: number } | null {
  const amount = p.usage?.cost;
  const currency = p.usage?.cost_currency ?? p.currency;
  return amount == null || !currency ? null : { currency, amount };
}

export default function ProviderDetail({
  providerId,
  providers,
  routes,
  agentDetect,
  onEdit,
}: {
  providerId: string;
  /** The whole list, from the screen the page was opened from — one read, two
      renderings. `null` while it is still in flight. */
  providers: Provider[] | null;
  routes: AgentRoute[] | null;
  agentDetect: AgentDetect[] | null;
  onEdit: (p: Provider) => void;
}) {
  const t = useT();
  const [logs, setLogs] = useState<RequestLogEntry[] | null>(null);
  const [logErr, setLogErr] = useState<string | null>(null);
  const [keys, setKeys] = useState<ApiKeyEntry[] | null>(null);

  const provider = providers?.find((p) => p.id === providerId) ?? null;

  // This provider's most recent requests. Its own read rather than a slice of
  // anything already loaded: the page can be opened directly from a hash, with
  // no list behind it.
  useEffect(() => {
    let alive = true;
    setLogs(null);
    setLogErr(null);
    api
      .listRequestLogs(1, RECENT, { provider_id: providerId })
      .then((r) => alive && setLogs(r.rows))
      .catch((e) => alive && setLogErr(errText(e)));
    return () => {
      alive = false;
    };
  }, [providerId]);

  // The rotating-key pool. Best-effort: a provider with only its primary key
  // answers an empty list, and a failed read is not worth an error state on a
  // page whose subject is the provider, not its keys.
  useEffect(() => {
    let alive = true;
    api
      .listApiKeys(providerId)
      .then((k) => alive && setKeys(k))
      .catch(() => alive && setKeys([]));
    return () => {
      alive = false;
    };
  }, [providerId]);

  /** Where this provider sits in each agent that bound it. */
  const places = useMemo(() => {
    if (!routes) return [];
    return routes
      .map((r) => {
        const idx = r.bindings.findIndex((b) => b.provider_id === providerId);
        return idx < 0 ? null : { route: r, index: idx };
      })
      .filter((x): x is { route: AgentRoute; index: number } => x !== null)
      .sort((a, b) => a.route.agent.localeCompare(b.route.agent));
  }, [routes, providerId]);

  const back = () => {
    window.location.hash = "#providers";
  };

  if (!providers) {
    return <div className="p-8 text-center text-[12px] text-mut">{t("common.loading")}</div>;
  }
  if (!provider) {
    // A hash naming a provider that is not here: deleted, or a stale link. An
    // answer rather than an error, and the way back is a breadcrumb away.
    return (
      <section className="p-4">
        <nav aria-label={t("providers.detailBreadcrumb")} className="mb-3">
          <ol className="flex items-center gap-1.5 text-[11px] text-mut">
            <li>
              <button className="cursor-pointer hover:text-fg" onClick={back}>
                {t("app.nav.apps")}
              </button>
            </li>
          </ol>
        </nav>
        <div className="rounded-lg border border-line bg-surface px-3.5 py-6 text-center text-[12px] text-mut">
          {t("providers.detailGone")}
        </div>
      </section>
    );
  }

  const brand = iconForEndpoint(provider.endpoint);
  const u = provider.usage;
  const inOut = u ? u.input_tokens + u.output_tokens : 0;
  const cacheDenom = u ? u.input_tokens + u.cache_read_tokens + u.cache_creation_tokens : 0;
  const cachePct = cacheDenom > 0 ? Math.round(((u?.cache_read_tokens ?? 0) / cacheDenom) * 100) : null;

  return (
    <section className="p-4">
      <nav aria-label={t("providers.detailBreadcrumb")} className="mb-2">
        <ol className="flex items-center gap-1.5 text-[11px] text-mut">
          <li>
            <button className="cursor-pointer hover:text-fg" onClick={back}>
              {t("app.nav.apps")}
            </button>
          </li>
          <li aria-hidden="true">
            <ChevronRight className="h-3 w-3" />
          </li>
          <li aria-current="page" className="flex min-w-0 items-center">
            <span className="truncate">{provider.name}</span>
          </li>
        </ol>
      </nav>

      {/* Who it is. */}
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-center gap-3">
          {brand ? (
            <ProviderLogo icon={brand} name={provider.name} size={40} />
          ) : (
            <ProviderLogo char={provider.logo_char} color={provider.logo_color} name={provider.name} size={40} />
          )}
          <div className="min-w-0">
            <h2 className="flex items-center gap-2 text-[15px] font-semibold">
              {provider.name}
              {!provider.enabled && (
                <span className="rounded px-1.5 py-px text-[10px] font-medium text-mut" style={{ background: "var(--surface2)" }}>
                  {t("providers.detailParked")}
                </span>
              )}
              {provider.is_current && (
                <span
                  className="rounded px-1.5 py-px text-[10px] font-medium"
                  style={{ background: "var(--kiwi)", color: "oklch(0.18 0.03 132)" }}
                >
                  {t("providers.inUse")}
                </span>
              )}
            </h2>
            <div className="mt-0.5 truncate font-mono text-[11px] text-mut">{provider.endpoint}</div>
            <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-mut">
              <span className="font-mono">{provider.protocol}</span>
              <span>·</span>
              <span>{provider.endpoint_note}</span>
              {provider.model_default && (
                <>
                  <span>·</span>
                  <span className="font-mono">{provider.model_default}</span>
                </>
              )}
            </div>
          </div>
        </div>
        <button
          type="button"
          className="flex shrink-0 items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] text-mut hover:text-ink"
          onClick={() => onEdit(provider)}
        >
          <Pencil className="h-3 w-3" />
          {t("common.edit")}
        </button>
      </div>

      {/* The numbers, one card. */}
      <div className="mt-3 grid grid-cols-4 gap-2">
        <Stat label={t("providers.detailRequests")} value={u ? String(u.requests) : "—"} />
        <Stat
          label={t("providers.detailTokens")}
          value={u ? fmtTokens(inOut) : "—"}
          hint={u ? t("providers.usageInOut", { input: fmtTokens(u.input_tokens), output: fmtTokens(u.output_tokens) }) : undefined}
        />
        <Stat
          label={t("providers.detailCache")}
          value={cachePct == null ? "—" : `${cachePct}%`}
          hint={cachePct == null ? t("providers.cacheNoData") : undefined}
        />
        <Stat
          label={t("providers.colStatus")}
          value={provider.health.latency_ms != null ? fmtLatency(provider.health.latency_ms) : "—"}
          hint={provider.health.error ?? provider.health.note ?? undefined}
          dot={provider.health.state}
        />
      </div>

      {/* Cost, when the ledger knows one. */}
      {cost(provider) && (
        <div className="mt-2 flex items-center gap-3 text-[11.5px] text-mut">
          <span className="text-[10px] uppercase tracking-wide">{t("providers.detailCost")}</span>
          <span className="font-mono text-ink">
            {fmtMoney(cost(provider)!.amount, cost(provider)!.currency)}
          </span>
        </div>
      )}

      {/* Who it serves, and where it sits in each queue. */}
      <section className="mt-4">
        <h3 className="mb-1.5 text-[12.5px] font-semibold">{t("providers.detailAgents")}</h3>
        {places.length === 0 ? (
          <div className="rounded-lg border border-line bg-surface px-3 py-3 text-[11.5px] text-mut">
            {t("providers.detailNoAgents")}
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-line">
            {places.map(({ route, index }) => {
              const meta = agentMeta(route.agent as AgentRef);
              const serving = provider.serving_agents.includes(route.agent);
              const detect = agentDetect?.find((d) => d.agent === route.agent);
              return (
                <div
                  key={route.agent}
                  className="flex items-center gap-2 border-b border-line px-3 py-2 text-[11.5px] last:border-b-0"
                >
                  <span className="min-w-0 flex-1 truncate">{meta.label}</span>
                  {/* Its place in that agent's queue — the number the provider
                      itself has no way to know. */}
                  <span className="shrink-0 font-mono text-[10.5px] text-mut" title={t("providers.detailRankTitle")}>
                    #{index + 1}
                  </span>
                  <span className="w-[52px] shrink-0 text-right font-mono text-[10.5px] text-mut">
                    {route.strategy}
                  </span>
                  <span className="flex w-[76px] shrink-0 items-center justify-end gap-1 text-[10.5px]">
                    {serving ? (
                      <>
                        <Dot state="ok" />
                        <span className="text-mut">{t("providers.inUse")}</span>
                      </>
                    ) : (
                      <span className="text-mut">{t("providers.detailStandby")}</span>
                    )}
                  </span>
                  <span className="w-[64px] shrink-0 text-right text-[10.5px] text-mut">
                    {detect?.installed ? t("providers.detailInstalled") : ""}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </section>

      {/* The key pool and the transport settings — both facts about this
          provider that the list has no room for. */}
      <div className="mt-4 grid grid-cols-2 gap-2">
        <section className="rounded-lg border border-line bg-surface p-3">
          <h3 className="mb-1.5 flex items-center gap-1.5 text-[12.5px] font-semibold">
            <KeyRound className="h-3.5 w-3.5 text-mut" />
            {t("providers.detailKeys")}
          </h3>
          {/* The provider's own key is never sent to the UI — only the pool's
              masked entries are. Saying so beats an empty row the reader has
              to interpret. */}
          <Line label={t("providers.detailPrimaryKey")} value={t("providers.detailKeyHidden")} />
          {keys?.map((k) => (
            <Line
              key={k.id}
              label={k.label || `#${k.id}`}
              value={k.enabled ? k.masked : `${k.masked} · ${t("providers.detailDisabled")}`}
              mono
            />
          ))}
          {keys?.length === 0 && <div className="mt-1 text-[10.5px] text-mut">{t("providers.detailNoPool")}</div>}
        </section>

        <section className="rounded-lg border border-line bg-surface p-3">
          <h3 className="mb-1.5 flex items-center gap-1.5 text-[12.5px] font-semibold">
            <Gauge className="h-3.5 w-3.5 text-mut" />
            {t("providers.detailTransport")}
          </h3>
          <Line
            label={t("addProvider.proxyLabel")}
            value={provider.advanced?.proxy ?? t("providers.detailNoProxy")}
            mono
          />
          <Line
            label={t("addProvider.timeoutLabel")}
            value={provider.advanced?.timeout_secs != null ? `${provider.advanced.timeout_secs}s` : t("providers.detailDefault")}
          />
          <Line
            label={t("addProvider.retriesLabel")}
            value={provider.advanced?.retries != null ? String(provider.advanced.retries) : t("providers.detailDefault")}
          />
          <Line
            label={t("addProvider.customHeaders")}
            value={String(Object.keys(provider.advanced?.headers ?? {}).length)}
          />
        </section>
      </div>

      {/* What it carried, newest first. */}
      <section className="mt-4">
        <h3 className="mb-1.5 text-[12.5px] font-semibold">{t("providers.detailRecent")}</h3>
        {logErr ? (
          <LoadFailed detail={logErr} onRetry={() => setLogs(null)} />
        ) : !logs ? (
          <div className="py-3 text-center text-[11.5px] text-mut">{t("common.loading")}</div>
        ) : logs.length === 0 ? (
          <div className="rounded-lg border border-line bg-surface px-3 py-3 text-[11.5px] text-mut">
            {t("providers.detailNoTraffic")}
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-line">
            {logs.map((r) => (
              <button
                key={r.id}
                className="flex w-full cursor-pointer items-center gap-2 border-b border-line px-3 py-1.5 text-left font-mono text-[10.5px] last:border-b-0 hover:bg-surface2"
                onClick={() => {
                  window.location.hash = `#dashboard/log/${r.id}`;
                }}
              >
                <span className="shrink-0 text-mut">{r.ts.slice(11, 19)}</span>
                <span className="w-[68px] shrink-0 text-mut">{r.agent ?? "—"}</span>
                <span className="min-w-0 flex-1 truncate">{r.model ?? "—"}</span>
                <span
                  className="w-[38px] shrink-0 text-right"
                  style={{ color: r.status_code >= 400 ? "var(--red)" : undefined }}
                >
                  {r.status_code}
                </span>
                <span className="w-[54px] shrink-0 text-right text-mut">
                  {r.latency_ms != null ? fmtLatency(r.latency_ms) : "—"}
                </span>
                <span className="w-[70px] shrink-0 text-right text-mut">{fmtTokens(r.input_tokens + r.output_tokens)}</span>
              </button>
            ))}
          </div>
        )}
      </section>
    </section>
  );
}

function Stat({
  label,
  value,
  hint,
  dot,
}: {
  label: string;
  value: string;
  hint?: string;
  dot?: Provider["health"]["state"];
}) {
  return (
    <div className="rounded-lg border border-line bg-surface px-3 py-2" title={hint}>
      <div className="text-[10px] uppercase tracking-wide text-mut">{label}</div>
      <div className="mt-0.5 flex items-center gap-1.5 font-mono text-[13px]">
        {dot && <Dot state={dot} />}
        {value}
      </div>
    </div>
  );
}

function Line({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5 text-[11px]">
      <span className="shrink-0 text-mut">{label}</span>
      <span className={`min-w-0 truncate text-right${mono ? " font-mono text-[10.5px]" : ""}`} title={value}>
        {value}
      </span>
    </div>
  );
}
