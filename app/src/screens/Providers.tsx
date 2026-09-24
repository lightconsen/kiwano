// Home screen: Apps (local provider list, design/index.html #s-providers)
//
// The container: it owns the screen's state, the reads and the effects behind
// it, and composes the pieces that live in `screens/Providers/`.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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
import StrategyPanel from "../components/StrategyPanel";
import { NO_CLI_AGENTS, SEGMENTS } from "./Providers/agents";
import { AccessDialog } from "./Providers/AccessDialog";
import { AgentSettingsDialog } from "./Providers/AgentSettingsDialog";
import { NewAgentDialog } from "./Providers/NewAgentDialog";
import { DeclareAgentDirDialog, dirOf, type Redetect } from "./Providers/DeclareAgentDirDialog";
import { AgentTabTitle } from "./Providers/AgentTabTitle";
import { HeaderActions } from "./Providers/HeaderActions";
import { ProviderTable } from "./Providers/ProviderTable";
import { SegmentStrip } from "./Providers/SegmentStrip";

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
  /** Phase 1 detection result; null = probe not answered yet → the strip shows
      only taken-over and user-defined agents until it does */
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

  // Adopt a deep-linked segment arriving while mounted (App re-parses the
  // hash). Only a *change of* initialAgent adopts — the initial value rides
  // in through the useState initializer above, so reacting to `seg` here
  // would fight the visibility clip below (an initialAgent the strip cannot
  // show — say the probe found nothing — would bounce all↔agent forever).
  const prevInitial = useRef(initialAgent);
  useEffect(() => {
    if (initialAgent && initialAgent !== prevInitial.current) {
      prevInitial.current = initialAgent;
      setSeg(initialAgent);
    }
  }, [initialAgent]);

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

  /** The header's ⟳. All three of the things it does are awaited together, so
   * the spinner ends when the last one lands rather than when the first one
   * returns, and the re-probe is a click's rather than a re-read's: it spawns a
   * login shell plus one subprocess per agent. */
  const onRefresh = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      // All three, awaited together, so the spinner ends when the last one
      // lands rather than when the first one returns.
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

  // Only agents the probe found installed get a segment, plus the user's own
  // (which have no installation to detect). `agentDetect === null` is the
  // probe not having answered — no evidence of installation, and no reason to
  // show an agent that is probably not on the machine — so until it answers,
  // the strip is the user's custom agents only. A taken-over agent whose
  // binary has vanished reads the same way: no requests can be flowing, so
  // detection, not the takeover state, gates the tab.
  const visibleSegments = useMemo(
    () =>
      segments.filter(
        (s) =>
          s.id === "all" ||
          customAgents.some((a) => a.id === s.id) ||
          (agentDetect?.find((d) => d.agent === s.id)?.installed ?? false),
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

  return (
    <section className="flex min-h-full flex-col">
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <SegmentStrip
          visibleSegments={visibleSegments}
          seg={seg}
          agentVersions={agentVersions}
          pickSeg={pickSeg}
          declarable={declarable}
          setNewAgent={setNewAgent}
          setDeclaring={setDeclaring}
        />
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
        <HeaderActions
          refreshing={refreshing}
          refreshed={refreshed}
          onRefresh={onRefresh}
          onAdd={onAdd}
        />
      </div>

      <AgentTabTitle
        seg={seg}
        custom={custom}
        takenOver={takenOver}
        configPaths={configPaths}
        setAccessOpen={setAccessOpen}
        setAgentSettingsOpen={setAgentSettingsOpen}
      />

      <ProviderTable
        seg={seg}
        custom={custom}
        takenOver={takenOver}
        agentDetect={agentDetect}
        enabling={enabling}
        setAgentTakenOver={setAgentTakenOver}
        takeoverError={takeoverError}
        onAdd={onAdd}
        route={route}
        byId={byId}
        planQuotas={planQuotas}
        blocked={blocked}
        onMoveBinding={onMoveBinding}
        refetch={refetch}
        filtered={filtered}
        routes={routes}
        onEdit={onEdit}
        onDelete={onDelete}
        providers={providers}
      />

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
