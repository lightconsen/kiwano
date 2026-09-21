// Home screen: Apps (local provider list, design/index.html #s-providers)
//
// The container: it owns the screen's state, the reads and the effects behind
// it, and composes the pieces that live in `screens/Providers/`.
import { useCallback, useEffect, useMemo, useState } from "react";

import { Check, Plus, RefreshCw, Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Menu,
  MenuContent,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  MenuTrigger,
} from "@/components/ui/menu";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { api } from "../api/client";
import { LoadFailed, errText } from "../components/LoadFailed";
import { useReload } from "../lib/reload";
import { agentMeta, isBuiltinAgent, rememberCustomAgents } from "../lib/agents";
import { useT } from "../i18n";
import {
  AGENTS,
  type AgentDetect,
  type AgentId,
  type AgentRef,
  type AgentRoute,
  type CustomAgent,
  type PlanQuotaReport,
  type Protocol,
  type Provider,
} from "../api/types";
import { AGENT_ICON } from "../components/bits";
import StrategyPanel, { CopyRouteRow } from "../components/StrategyPanel";
import {
  AgentOnboarding,
  NO_CLI_AGENTS,
  SEGMENTS,
  SEGMENT_ICON,
  segmentLabel,
} from "./Providers/agents";
import { AccessDialog } from "./Providers/AccessDialog";
import { AgentSettingsDialog } from "./Providers/AgentSettingsDialog";
import { NewAgentDialog } from "./Providers/NewAgentDialog";
import { DeclareAgentDirDialog, dirOf, type Redetect } from "./Providers/DeclareAgentDirDialog";
import { ProviderRow } from "./Providers/rows";
import { AddBindingRow, BindingRow, BindProviderSelect } from "./Providers/BindingRow";

export default function Providers({
  onAdd,
  onEdit,
  agentDetect = null,
  agentVersions = {},
  onRedetect,
  initialAgent = null,
}: {
  onAdd: () => void;
  onEdit: (p: Provider) => void;
  /** Phase 1 detection result; null = probe unavailable → show every agent */
  agentDetect?: AgentDetect[] | null;
  /** Phase 2 versions by agent id (arrive async, tooltip only) */
  agentVersions?: Partial<Record<AgentRef, string>>;
  /** Re-run agent detection (App owns the result). The refresh button's third job */
  onRedetect?: Redetect;
  /** Deep-linked agent segment (#providers/<agent>, e.g. from Settings takeover rows) */
  initialAgent?: AgentRef | null;
}) {
  const t = useT();
  const [providers, setProviders] = useState<Provider[] | null>(null);
  const [routes, setRoutes] = useState<AgentRoute[] | null>(null);
  const [seg, setSeg] = useState<AgentRef | "all">(initialAgent ?? "all");
  const [takenOver, setTakenOver] = useState<Set<AgentRef> | null>(null);
  const [customAgents, setCustomAgents] = useState<CustomAgent[]>([]);
  const [newAgent, setNewAgent] = useState(false);
  // The built-in agent whose install directory is being declared, if any.
  const [declaring, setDeclaring] = useState<{
    id: AgentId;
    label: string;
    icon?: string;
    dir?: string;
    declared?: boolean;
  } | null>(null);
  // The credentials dialog, opened from the icon beside a user-defined agent's
  // name (there is no card on the tab itself).
  const [accessOpen, setAccessOpen] = useState(false);
  // The built-in agent's settings dialog (its takeover, and the files behind it).
  const [agentSettingsOpen, setAgentSettingsOpen] = useState(false);
  // The files behind each built-in agent, for the row above the table. Its own
  // state because the takeover *set* below throws the rest of that read away.
  const [configPathsByAgent, setConfigPathsByAgent] = useState<Record<string, string[]>>({});
  // The protocols each built-in agent's clients speak. The same read, and the
  // same kind of fact: what the agent *is*, carried on the takeover row that
  // exists per agent rather than in a table of its own.
  const [protocolsByAgent, setProtocolsByAgent] = useState<Record<string, Protocol[]>>({});
  // The rest of what the takeover row carries per agent: the key a client is
  // pointed at with, and whether the rewrite keeps the agent's other providers.
  // Both belong in the agent's own settings dialog — the same read, one more
  // fact about the agent rather than about the app.
  const [takeoverInfo, setTakeoverInfo] = useState<
    Record<string, { key: string | null; additive: boolean }>
  >({});
  // What a client is pointed at: the gateway's own listen address from settings.
  const [listen, setListen] = useState("127.0.0.1:8317");
  // Every currency the Hub prices, for a ceiling's unit picker — read only when
  // the agent's own providers do not name one.
  const [knownCurrencies, setKnownCurrencies] = useState<string[]>([]);
  const [enabling, setEnabling] = useState(false);
  /** Why the last takeover attempt was refused — see `AgentOnboarding`. */
  const [takeoverError, setTakeoverError] = useState<string | null>(null);
  // Plan-quota reports per provider, auto-refreshed on load
  const [planQuotas, setPlanQuotas] = useState<Record<string, PlanQuotaReport>>({});
  // A provider read that failed. Kept apart from everything the rows show: it
  // is a property of the read, not of the data, and it outlives whichever
  // follow-up read clears it. Stale rows stay on screen under it (like
  // RequestLogs) — a failure after a successful load is not a reason to drop
  // what the reader has — and the initial-load failure is what the screen's
  // empty state switches on.
  const [loadErr, setLoadErr] = useState<string | null>(null);
  // The header's ⟳: in flight, and finished. Named for the whole of what it does
  // — the provider list, the routes and settings, a forced quota read, and the
  // agent re-probe — rather than for the half that used to own it.
  const [refreshing, setRefreshing] = useState(false);
  // Flashed when the last of those lands. The quota numbers are the only part of
  // this that can visibly move, and often they have not: without the check, a
  // refresh that found nothing new reads exactly like a click that did nothing.
  const [refreshed, setRefreshed] = useState(false);

  // Segment switch that keeps the #providers/<agent> deep link truthful
  const pickSeg = (id: AgentRef | "all") => {
    setSeg(id);
    window.location.hash = id === "all" ? "providers" : `providers/${id}`;
  };

  // Why the gateway is refusing to route, straight from the gateway. Polled on
  // its own re-evaluation interval; this screen is the one that has to show it.
  const [blocked, setBlocked] = useState<Record<string, string>>({});
  useEffect(() => {
    const load = () =>
      api
        .getGatewayStatus()
        .then((s) => setBlocked(Object.fromEntries(s.blocked.map((b) => [b.id, b.reason]))))
        .catch(() => {});
    load();
    const t = window.setInterval(load, 30_000);
    return () => window.clearInterval(t);
  }, []);

  // Adopt a deep-linked segment arriving while mounted (App re-parses the hash)
  useEffect(() => {
    if (initialAgent && initialAgent !== seg) setSeg(initialAgent);
  }, [initialAgent, seg]);

  // Returns what it started, so a caller with a spinner can await it — the app's
  // own reload does (`lib/reload.ts`). Nothing else awaits it: the mutations that
  // call this pass it as a plain "something changed" callback.
  //
  // Every branch absorbs its own failure — the promise never rejects, so a
  // mutation that calls `refetch()` fire-and-forget cannot end in an unhandled
  // rejection, and the button's `await` is always answered. What a branch does
  // with the failure is its own: providers keep their stale rows under `loadErr`
  // (or the initial-load empty state), routes fall back to the onboarding null,
  // and settings are a refresh of things read once at launch.
  const refetch = useCallback(() => {
    const providers = api
      .listProviders()
      .then((r) => {
        setProviders(r);
        setLoadErr(null);
      })
      .catch((e) => setLoadErr(errText(e)));
    // Per-agent strategy routes drive the agent-tab candidate rows
    const routes = api
      .getAgentRoutes()
      .then(setRoutes)
      .catch(() => setRoutes(null));
    // Takeover state drives the per-agent onboarding panel; the same read
    // carries the user's own agents, which drive their segments and resolver.
    const settings = api
      .getSettings()
      .then((s) => {
        setTakenOver(new Set(s.takeovers.filter((t) => t.enabled).map((t) => t.agent)));
        setConfigPathsByAgent(
          Object.fromEntries(s.takeovers.map((k) => [k.agent, k.config_paths ?? []])),
        );
        setProtocolsByAgent(
          Object.fromEntries(s.takeovers.map((k) => [k.agent, k.protocols ?? []])),
        );
        setTakeoverInfo(
          Object.fromEntries(
            s.takeovers.map((k) => [
              k.agent,
              { key: k.placeholder_key ?? null, additive: k.additive },
            ]),
          ),
        );
        setCustomAgents(s.custom_agents);
        rememberCustomAgents(s.custom_agents);
        if (s.gateway_listen) setListen(s.gateway_listen);
      })
      .catch(() => {});
    // The currencies a limit may be denominated in — the Hub's rate table, which
    // is also what makes a ceiling comparable with the costs behind it.
    const currencies = api
      .getCurrencyMeta()
      .then((m) => setKnownCurrencies(m.currencies))
      .catch(() => {});
    return Promise.all([providers, routes, settings, currencies]);
  }, []);
  // Not `useEffect(refetch, …)`: an effect may not return a promise, which is
  // what this now returns.
  useEffect(() => {
    void refetch();
  }, [refetch]);
  useReload(refetch);

  // Auto plan-quota refresh on load: one cached call per provider configured
  // with a plan query (the 5-min backend cache keeps this cheap)
  useEffect(() => {
    if (!providers) return;
    for (const p of providers) {
      if (!p.plan_query) continue;
      api
        .getPlanQuota(p.id)
        .then((r) => setPlanQuotas((m) => ({ ...m, [p.id]: r })))
        .catch(() => {});
    }
  }, [providers]);

  // Manual refresh: bypasses the backend's 5-minute quota cache
  // The forced quota read (bypasses the backend's 5-minute cache). It reports
  // what it did rather than owning the button's busy state: all three halves of
  // the refresh are awaited together, so one spinner covers the click.
  const refreshQuotas = async () => {
    if (!providers) return;
    const results = await Promise.all(
      providers
        .filter((p) => p.plan_query)
        .map((p) => api.getPlanQuota(p.id, true).then((r) => [p.id, r] as const).catch(() => null)),
    );
    const fresh: Record<string, PlanQuotaReport> = {};
    for (const r of results) {
      if (r) fresh[r[0]] = r[1];
    }
    setPlanQuotas((m) => ({ ...m, ...fresh }));
  };

  // `timer`, not `t`: `t` is the translator here.
  useEffect(() => {
    if (!refreshed) return;
    const timer = setTimeout(() => setRefreshed(false), 1600);
    return () => clearTimeout(timer);
  }, [refreshed]);

  // What the "+" menu offers besides a custom agent: the built-ins the
  // detector could not find, plus any already declared (so a declaration can be
  // corrected or dropped). Null detection means "no idea" — offering guesses
  // then would be noise, so the menu keeps its first item only.
  const declarable = useMemo(() => {
    if (!agentDetect) return [];
    return AGENTS.filter((a) => !NO_CLI_AGENTS.includes(a.id as AgentId))
      .map((a) => {
        const hit = agentDetect.find((d) => d.agent === a.id);
        const declared = hit?.manual ?? false;
        return {
          id: a.id as AgentId,
          label: a.label,
          icon: AGENT_ICON[a.id],
          declared,
          // A declaration comes back as the binary it resolved to, so the
          // dialog opens on the directory the user named last time.
          dir: declared && hit?.path ? dirOf(hit.path) : "",
          missing: !hit?.installed,
        };
      })
      .filter((a) => a.declared || a.missing);
  }, [agentDetect]);

  // The strip reads in three groups: the agents already routed through the
  // gateway, then the ones merely installed, then the user's own — which are
  // never "not installed", since there is nothing to install. Within a group
  // the registry order stands (the sort is stable), and "all" stays first.
  const segments = useMemo(() => {
    const [all, ...builtins] = SEGMENTS;
    const custom = customAgents.map((a) => ({
      id: a.id as AgentRef,
      label: a.label,
      icon: undefined,
    }));
    const group = (id: AgentRef | "all") => {
      if (takenOver?.has(id)) return 0;
      if (customAgents.some((a) => a.id === id)) return 2;
      return 1;
    };
    return [all, ...[...builtins, ...custom].sort((a, b) => group(a.id) - group(b.id))];
  }, [customAgents, takenOver]);

  // Only agents that phase 1 detected as installed get a segment; a failed
  // probe (null) keeps every agent visible — and a user-defined agent always
  // shows, since it has no installation to detect.
  const visibleSegments = useMemo(
    () =>
      segments.filter(
        (s) =>
          s.id === "all" ||
          customAgents.some((a) => a.id === s.id) ||
          !agentDetect ||
          agentDetect.find((d) => d.agent === s.id)?.installed,
      ),
    [segments, customAgents, agentDetect],
  );

  // Drop a hidden segment if the detection result changed under us.
  useEffect(() => {
    if (seg !== "all" && !visibleSegments.some((s) => s.id === seg)) {
      setSeg("all");
      window.location.hash = "providers";
    }
  }, [visibleSegments, seg]);

  const onDelete = async (p: Provider) => {
    await api.deleteProvider(p.id);
    refetch();
  };

  // Swap two candidates in the current agent's strategy queue (priority order)
  const onMoveBinding = async (idx: number, dir: -1 | 1) => {
    if (seg === "all") return;
    const route = routes?.find((r) => r.agent === seg);
    if (!route) return;
    const ids = route.bindings.map((b) => b.provider_id);
    const j = idx + dir;
    if (j < 0 || j >= ids.length) return;
    [ids[idx], ids[j]] = [ids[j], ids[idx]];
    await api.reorderAgentBindings(seg, ids);
    refetch();
  };

  // Creating one opens its tab: what the user wants next is to bind a provider
  // to it, and that is the tab's empty state.
  const onAgentCreated = (id: string) => {
    setNewAgent(false);
    refetch();
    pickSeg(id);
  };

  const onDeleteAgent = async (id: string) => {
    await api.removeCustomAgent(id);
    setSeg("all");
    window.location.hash = "providers";
    refetch();
  };

  /** Turn a built-in agent's takeover on or off.
   *
   * Only ever a built-in: a user-defined agent has no config to take over, so
   * neither caller is shown for one — and the guard is what keeps that true
   * rather than a comment claiming it.
   *
   * Enabling is what the Apps onboarding offers; disabling is the escape hatch
   * back to the agent's own configuration, offered by the agent's own settings
   * dialog. Either way the screen re-reads afterwards instead of assuming the
   * outcome; a refusal is left to the caller to show, which is why this reports
   * whether the call landed rather than only having run. */
  const setAgentTakenOver = async (agent: AgentRef, enabled: boolean): Promise<boolean> => {
    if (!isBuiltinAgent(agent)) return false;
    setEnabling(true);
    setTakeoverError(null);
    try {
      await api.setTakeover(agent, enabled);
      return true;
    } catch (e) {
      // Kept, not swallowed: a takeover that was refused leaves the agent
      // exactly as it was, so without this the user sees a spin and no reason.
      setTakeoverError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      setEnabling(false);
      refetch();
    }
  };

  /** Turning a takeover off, from the dialog it is offered in.
   *
   * The dialog closes only once the call landed. A refused restore leaves the
   * agent taken over, so closing on the way out would take the reason with it —
   * the dialog is the only place that can still show one. */
  const disableAgentTakeover = async (agent: AgentRef) => {
    if (await setAgentTakenOver(agent, false)) setAgentSettingsOpen(false);
  };

  if (!providers) {
    // A read that never succeeded gets the failure state, not an endless
    // "Loading…": the reader is waiting on something that is not coming, and
    // retrying is the only action that can change that.
    if (loadErr) {
      return <LoadFailed detail={loadErr} onRetry={() => refetch()} />;
    }
    return <div className="p-8 text-center text-[12px] text-mut">{t("common.loading")}</div>;
  }

  const filtered = providers.filter((p) => seg === "all" || p.agents.includes(seg));
  const agentsBound = new Set(providers.flatMap((p) => p.agents)).size;
  // The files a takeover would rewrite for the tab in hand, read out of the
  // settings read this screen already makes. Empty while that read is in flight
  // or for an id the registry does not know.
  const configPaths = seg === "all" ? [] : (configPathsByAgent[seg] ?? []);
  // What the same tab's agent speaks, from the same read. Only its own dialog
  // shows it: the tab itself is about the providers behind it.
  const agentProtocols = seg === "all" ? [] : (protocolsByAgent[seg] ?? []);
  // Inside an agent tab with bindings the list IS the strategy candidate queue
  const route = seg === "all" ? null : (routes?.find((r) => r.agent === seg) ?? null);
  // The currencies a ceiling on this agent may be written in: what the providers
  // it is bound to bill in. An agent whose providers say nothing — none bound
  // yet, or none of them linked to a catalog entry and none with declared
  // prices — falls back to every currency the Hub prices, so the picker still
  // offers money rather than nothing.
  const limitCurrencies = (agent: AgentRef): string[] => {
    const own = [
      ...new Set(
        providers
          .filter((p) => p.agents.includes(agent))
          // `Boolean` guards a gateway older than this screen: the app and the
          // daemon are two binaries, and a missing currency must not become an
          // empty option in the picker.
          .map((p) => p.currency)
          .filter(Boolean),
      ),
    ];
    return own.length > 0 ? own.sort() : knownCurrencies;
  };
  const byId = new Map(providers.map((p) => [p.id, p]));
  // The agent this tab belongs to, when the user defined it: routes and a key,
  // and no config file anywhere.
  const custom = seg === "all" ? undefined : customAgents.find((a) => a.id === seg);
  // Disabling a takeover keeps the stored route (re-enabling restores it), but
  // the agent config no longer points at the gateway — the route is dormant.
  // An agent tab for an agent that is not taken over shows the (re-)takeover
  // onboarding instead of the dormant binding rows.
  //
  // A user-defined agent has no config to take over, so it is never in that
  // state: its tab shows its route from the start.
  const notTakenOver = seg !== "all" && !custom && !(takenOver?.has(seg) ?? false);

  return (
    <section className="flex min-h-full flex-col">
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <div className="flex max-w-full overflow-x-auto rounded-lg border border-line text-[12px]">
          {visibleSegments.map((s, i) => {
            const label = segmentLabel(t, s.id);
            const ver = s.id === "all" ? undefined : agentVersions[s.id];
            return (
              <button
                key={s.id}
                title={s.id === "all" ? label : ver ? `${label} · v${ver}` : label}
                aria-label={label}
                className={`seg flex h-7 shrink-0 items-center justify-center px-2.5${i > 0 ? " border-l border-line" : ""}${seg === s.id ? " active" : ""}`}
                onClick={() => pickSeg(s.id)}
              >
                {s.icon ? (
                  <ProviderLogo icon={s.icon} name={label} size={15} />
                ) : s.id !== "all" ? (
                  // Any agent with no mark to port — a user-defined route, or a
                  // built-in whose logo has not been supplied — reads as its own
                  // letter, in the colour the resolver derived for it. The rule
                  // used to be "a built-in always has a mark", which stopped
                  // being true the moment one arrived without.
                  <ProviderLogo
                    char={agentMeta(s.id).chip_char}
                    color={agentMeta(s.id).chip_color}
                    name={label}
                    size={15}
                  />
                ) : (
                  <span className="text-[12px] text-mut">{label}</span>
                )}
              </button>
            );
          })}
          {/* Its own button rather than part of the strip: the strip is a
              switch between agents that exist, and this is how one comes to. */}
          <Menu>
            <MenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 w-7 shrink-0 px-0 text-mut hover:text-ink"
                  aria-label={t("providers.addAgentMenu")}
                  title={t("providers.addAgentMenuTitle")}
                />
              }
            >
              <Plus className="h-3.5 w-3.5" />
            </MenuTrigger>
            <MenuContent>
              <MenuItem onClick={() => setNewAgent(true)}>
                <Plus className="h-3.5 w-3.5 text-mut" />
                {t("providers.newAgent")}
              </MenuItem>
              {declarable.length > 0 && (
                <>
                  <MenuSeparator />
                  <MenuLabel>{t("providers.notDetected")}</MenuLabel>
                  {declarable.map((a) => (
                    <MenuItem key={a.id} onClick={() => setDeclaring(a)}>
                      {a.icon ? (
                        <span aria-hidden className="inline-flex shrink-0">
                          <ProviderLogo icon={a.icon} name={a.label} size={14} />
                        </span>
                      ) : (
                        <span className="h-3.5 w-3.5 shrink-0" />
                      )}
                      <span className="min-w-0 truncate">{a.label}</span>
                      {a.declared && (
                        <span className="ml-auto shrink-0 text-[10.5px] text-mut">
                          {t("providers.alreadyDeclared")}
                        </span>
                      )}
                    </MenuItem>
                  ))}
                </>
              )}
            </MenuContent>
          </Menu>
        </div>
        <span className="ml-1.5 text-[11.5px] text-mut">
          {t("providers.counts", { providers: providers.length, agents: agentsBound })}
        </span>
        {loadErr && (
          <span
            className="ml-1.5 text-[11px]"
            style={{ color: "var(--red)" }}
            title={loadErr}
            role="alert"
          >
            {t("common.loadFailed")}
          </span>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-7 w-7 px-0 text-mut"
          aria-label={t("common.refresh")}
          title={t("providers.refreshTitle")}
          disabled={refreshing}
          onClick={async () => {
            if (refreshing) return;
            setRefreshing(true);
            try {
              // All three, awaited together, so the spinner ends when the last
              // one lands rather than when the first one returns.
              await Promise.all([
                refetch(),
                refreshQuotas(),
                // Deliberately separate from `refetch`: that one is the generic
                // "something changed" callback handed to every child, and
                // re-probing spawns a login shell plus one subprocess per agent,
                // so only a click should pay for it.
                onRedetect?.(),
              ]);
              setRefreshed(true);
            } finally {
              setRefreshing(false);
            }
          }}
        >
          {refreshed ? (
            <Check className="h-3.5 w-3.5" style={{ color: "var(--kiwi)" }} />
          ) : (
            <RefreshCw className={`h-3.5 w-3.5${refreshing ? " animate-spin" : ""}`} />
          )}
        </Button>
        <Button
          size="sm"
          className="h-7 gap-1 px-2.5 text-[12px] font-semibold"
          onClick={onAdd}
        >
          <Plus className="h-3.5 w-3.5" />
          {t("providers.addProvider")}
        </Button>
      </div>

      {/* A user-defined agent's tab starts with the two values a client is
          configured with — the whole of what it is from the outside. */}
      {custom && (
        // The tab names the agent: the strip shows it as a letter avatar, so a
        // page with no name on it is one you have to identify from the highlight
        // over there. Everything *about* it — the credentials, and deleting it —
        // is behind the icon, which also keeps a destructive control out of the
        // row you click around in.
        <div className="mx-4 my-0.5 flex items-center gap-1.5">
          <ProviderLogo
            char={agentMeta(seg).chip_char}
            color={agentMeta(seg).chip_color}
            name={custom.label}
            size={18}
          />
          <span className="text-[13px] font-semibold">{custom.label}</span>
          {custom.note && (
            <span className="min-w-0 truncate text-[11px] text-mut">{custom.note}</span>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
            aria-label={t("providers.agentSettingsFor", { agent: custom.label })}
            title={t("providers.agentSettingsFor", { agent: custom.label })}
            onClick={() => setAccessOpen(true)}
          >
            <Settings2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      )}

      {/* A built-in agent's tab names itself the same way a user-defined one
          does — the strip shows icons, so a page with no name on it is one you
          have to identify from the highlight over there. What this one adds is
          the file behind it: the config a takeover rewrites, which is also what
          a user has to reach for by hand when something goes wrong. */}
      {!custom && seg !== "all" && (takenOver?.has(seg) ?? false) && (
        <div className="mx-4 my-0.5 flex items-center gap-1.5">
          <ProviderLogo
            icon={SEGMENT_ICON[seg]}
            char={agentMeta(seg).chip_char}
            color={agentMeta(seg).chip_color}
            name={agentMeta(seg).label}
            size={18}
          />
          <span className="text-[13px] font-semibold">{agentMeta(seg).label}</span>
          {configPaths.length > 0 && (
            <span className="min-w-0 truncate font-mono text-[11px] text-mut">
              {configPaths[0]}
              {/* The rest are listed in the dialog rather than elided here: codex
                  keeps two files and claude-desktop four, and a row that showed
                  only the first would read as if that were the whole of it. */}
              {configPaths.length > 1 ? ` +${configPaths.length - 1}` : ""}
            </span>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
            aria-label={t("providers.agentSettingsFor", { agent: agentMeta(seg).label })}
            title={t("providers.agentSettingsFor", { agent: agentMeta(seg).label })}
            onClick={() => setAgentSettingsOpen(true)}
          >
            <Settings2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      )}

      {notTakenOver ? (
        <AgentOnboarding
          agent={seg}
          installed={
            agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : true
          }
          takenOver={false}
          busy={enabling}
          onTakeover={() => void setAgentTakenOver(seg, true)}
          takeoverError={takeoverError}
          onAdd={onAdd}
        />
      ) : (
        <>
          <div className="flex h-7 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
            <span className="w-[31%]">{t("providers.colProvider")}</span>
            <span className="w-[16%]">
              {route ? t("providers.colRole") : t("providers.colBoundAgents")}
            </span>
            <span className="w-[26%]">{t("providers.colUsage")}</span>
            <span className="w-[8%]" title={t("providers.cacheColTitle")}>
              {t("providers.colCache")}
            </span>
            <span className="w-[12%]">{t("providers.colStatus")}</span>
            <span className="min-w-[140px] flex-1 text-right">
              {route ? t("providers.colPriority") : t("providers.colActions")}
            </span>
          </div>

          {route
            ? route.bindings.map((b, i) => {
                const p = byId.get(b.provider_id);
                // Skip a binding whose provider row vanished (deleted mid-session)
                return p ? (
                  <BindingRow
                    key={b.provider_id}
                    p={p}
                    route={route}
                    b={b}
                    idx={i}
                    plan={planQuotas[p.id]}
                    blocked={blocked[p.id]}
                    onMove={onMoveBinding}
                    onChanged={refetch}
                  />
                ) : null;
              })
            : filtered.map((p) => (
                <ProviderRow
                  key={p.id}
                  p={p}
                  plan={planQuotas[p.id]}
                  blocked={blocked[p.id]}
                  routes={routes}
                  onEdit={onEdit}
                  onDelete={onDelete}
                  onChanged={refetch}
                />
              ))}

          {/* Bind-one-more entry under the candidate queue of an agent tab */}
          {route && (
            <AddBindingRow
              agent={route.agent}
              providers={providers}
              boundIds={new Set(route.bindings.map((b) => b.provider_id))}
              onChanged={refetch}
            />
          )}

          {filtered.length === 0 && seg !== "all" && (
            <AgentOnboarding
              agent={seg}
              kind={custom ? "route" : "takeover"}
              installed={
                agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : true
              }
              takenOver={true}
              busy={enabling}
              onTakeover={() => void setAgentTakenOver(seg, true)}
              takeoverError={takeoverError}
              onAdd={onAdd}
              bindSlot={
                <BindProviderSelect
                  providers={providers}
                  boundIds={new Set(providers.filter((p) => p.agents.includes(seg)).map((p) => p.id))}
                  onPick={(pid) => api.addAgentBinding(seg, pid).then(refetch)}
                />
              }
              copySlot={
                <CopyRouteRow agent={seg} routes={routes ?? []} onChanged={refetch} />
              }
            />
          )}

          {filtered.length === 0 && seg === "all" && (
            <div className="px-4 py-8 text-center text-[12px] text-mut">
              {t("providers.none")}{" "}
              <button className="font-semibold" style={{ color: "var(--kiwi)" }} onClick={onAdd}>
                {t("providers.addProvider")}
              </button>
            </div>
          )}
        </>
      )}

      {/* Strategy config lives in the agent's own tab. For a built-in that means
          once it is taken over (before that there is no live route to configure);
          for a user-defined agent always — it *is* its route, and gating it on a
          takeover left the tab with no way to change the strategy at all.
          Routes come from here (single fetch): a route created while the panel is
          mounted (first bind / copy) must show up without a tab switch. */}
      {seg !== "all" && (custom || (takenOver?.has(seg) ?? false)) && (
        <StrategyPanel agent={seg} routes={routes} onChanged={refetch} />
      )}

      <NewAgentDialog
        open={newAgent}
        onClose={() => setNewAgent(false)}
        onSaved={onAgentCreated}
      />

      <DeclareAgentDirDialog
        // Live, not as it was when the menu was clicked: the whole point of the
        // dialog's own "check again" is that the answer can change under it.
        agent={
          declaring
            ? {
                ...declaring,
                installed: agentDetect?.find((d) => d.agent === declaring.id)?.installed,
                path: agentDetect?.find((d) => d.agent === declaring.id)?.path,
              }
            : null
        }
        onRedetect={onRedetect}
        onClose={() => setDeclaring(null)}
        onSaved={() => {
          setDeclaring(null);
          // Re-probe: the tab the declaration was about is what the user is
          // waiting for.
          void onRedetect?.();
        }}
        onCleared={() => {
          setDeclaring(null);
          void onRedetect?.();
        }}
      />

      {!custom && seg !== "all" && (takenOver?.has(seg) ?? false) && (
        <AgentSettingsDialog
          agent={seg}
          label={agentMeta(seg).label}
          paths={configPaths}
          protocols={agentProtocols}
          placeholderKey={takeoverInfo[seg]?.key ?? null}
          additive={takeoverInfo[seg]?.additive ?? false}
          limits={route?.limits ?? []}
          currencies={limitCurrencies(seg)}
          busy={enabling}
          disableError={takeoverError}
          onDisable={() => void disableAgentTakeover(seg)}
          onChanged={refetch}
          open={agentSettingsOpen}
          onClose={() => setAgentSettingsOpen(false)}
        />
      )}

      {custom && (
        <AccessDialog
          agent={custom.id}
          label={custom.label}
          keyName={custom.placeholder_key}
          protocol={custom.protocol}
          listen={listen}
          limits={route?.limits ?? []}
          currencies={limitCurrencies(custom.id)}
          open={accessOpen}
          onClose={() => setAccessOpen(false)}
          onDelete={() => onDeleteAgent(custom.id)}
          // The update replaces all three of the agent's own fields, so each
          // control below sends the two it is not changing back unchanged —
          // the note and the protocol here, the name and the note there.
          onRenamed={async (label) => {
            await api.updateCustomAgent(custom.id, label, custom.note, custom.protocol);
            refetch();
          }}
          onProtocolChanged={async (protocol) => {
            await api.updateCustomAgent(custom.id, custom.label, custom.note, protocol);
            refetch();
          }}
          onChanged={refetch}
        />
      )}

      {/* No closing note. It had grown into a strip of standing boilerplate —
          the privacy sentences, "switching applies instantly", how to read the
          table — repeated at the foot of every tab, where the rows and their own
          tooltips already say what each column means and what each control does. */}
    </section>
  );
}
