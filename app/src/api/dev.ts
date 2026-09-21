// Browser-dev data source — numbers match the design/index.html prototype verbatim.
// During integration src/api/client.ts switches to the Tauri invoke implementation and this file is retired.

import type { AgentDirHit, AgentId, AgentLimit, AgentRoute, ApiKeyEntry, AppSettings, CatalogList, ConfigShareReport, CurrencyMeta, CustomAgent, DashboardData, DashboardWindow, FooterStats, GatewayStatus, HubSyncReport, ImportReport, KiwanoApi, ModelPrice, NewProviderInput, PlanQuotaReport, ProbeReport, PromptLatency, Protocol, Provider, RequestLogDetail, RequestLogExport, RequestLogFilter, RequestLogList, StrategyKind, UpdateInfo, UpdateProgress, UsageAlert } from "./types";
import { AGENTS } from "./types";
import { DEV_DECLARED_VERSION, NOT_INSTALLED_AGENTS, binaryFor, declaredDirs } from "./dev/agent_dirs";
import { catalog, storedKeyFor } from "./dev/catalog";
import { dashboards } from "./dev/dashboard";
import { delay } from "./dev/delay";
import { CATALOG_TOTAL, endpointKey, endpointNote } from "./dev/endpoints";
import { DevApiKeyRow, devApiKeys, toApiKeyEntry } from "./dev/keys";
import { matchingLogs, requestLogs } from "./dev/logs";
import { MATRIX } from "./dev/matrix";
import { providers } from "./dev/providers";
import { agentRoutes, agentsOf, applyServingFlags, bind, quotaFallbackNow, servingNow, standbyFlags, strategyOf } from "./dev/routes";
import { settings } from "./dev/settings";

let idSeq = 100;

/** Dev-only: flips after the first syncHub() so the conditional path is visible. */
let devHubSynced = false;

/** Dev-only dedup for the sample feature alerts (see checkUsageAlerts). */
const devAlerted = new Set<string>();

export const devApi: KiwanoApi = {
  async getGatewayStatus(): Promise<GatewayStatus> {
    await delay();
    // The rows the matrix marks as over a limit. The gateway is what decides
    // this, and both the dimmed Usage cell and the Status column come from here
    // — a mock that always answers `[]` leaves the state unreachable in `pnpm
    // dev`, which is the only place it can be looked at.
    return {
      running: true,
      port: 8317,
      blocked: MATRIX.filter((m) => m.blocked).map((m) => ({ id: m.id, reason: m.blocked! })),
    };
  },

  async listProviders(filter: AgentId | "all" = "all"): Promise<Provider[]> {
    await delay();
    const serving = servingNow();
    const fallback = quotaFallbackNow();
    const { backups } = standbyFlags();
    return providers
      .filter((p) => filter === "all" || p.agents.includes(filter))
      .map((p) => {
        const agents = agentsOf(p.id);
        return {
          ...p,
          agents: agents as Provider["agents"],
          serving_agents: agents.filter((a) => serving.has(`${a}/${p.id}`)) as Provider["serving_agents"],
          is_current: agents.some((a) => serving.has(`${a}/${p.id}`)),
          // First in line behind an over-threshold primary — a separate claim
          // from "serving", so it gets its own slot on the wire.
          ...(agents.some((a) => fallback.has(`${a}/${p.id}`))
            ? { fallback_agents: agents.filter((a) => fallback.has(`${a}/${p.id}`)) }
            : {}),
          agents_note: backups.has(p.id)
            ? "Failover queue"
            : agents.length
              ? `${agents.length} agent(s)`
              : undefined,
        };
      });
  },

  async addProvider(input: NewProviderInput): Promise<Provider> {
    await delay();
    const endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    const p: Provider = {
      id: input.name.toLowerCase().replace(/\s+/g, "-") + "-" + ++idSeq,
      name: input.name,
      // Which catalog entry it came from: `vm::add_provider` stores it, and the
      // edit dialog reaches the entry through it for the endpoints, the currency
      // and the quota query the stored row may be missing.
      catalog_id: input.catalog_id ?? null,
      // What the entry bills in, or what the user declared — the fixture has no
      // catalog to ask, so a hand-added provider takes the demo's own currency.
      currency: input.prices?.currency ?? "CNY",
      logo_char: input.name.charAt(0).toUpperCase(),
      logo_color: "#555555",
      endpoint: input.endpoint.replace(/^https?:\/\//, ""),
      endpoint_note: endpointNote(input.protocol, endpoints),
      protocol: input.protocol,
      endpoints,
      billing: input.billing,
      limit_unit: input.billing_config.limit_unit,
      enabled: true,
      agents: input.agents ?? [],
      // Optimistic, mirrors vm::add_provider; listProviders recomputes
      serving_agents: [],
      is_current: (input.agents ?? []).length > 0,
      agents_note: (input.agents ?? []).length
        ? `${input.agents!.length} agent(s)`
        : undefined,
      health: { state: "idle", latency_ms: null },
      usage: null,
      advanced: input.advanced,
      model_default: input.model_default.trim() || null,
      plan_query: input.plan_query ?? null,
      plan_limits: input.billing_config.plan_limits ?? null,
      // What the user declared this provider charges; absent = none declared,
      // which is how a provider priced by the Hub's table reads.
      prices: input.prices ?? null,
    };
    providers.unshift(p);
    return p;
  },

  async updateProvider(id: string, input: NewProviderInput): Promise<Provider> {
    await delay();
    const t = providers.find((p) => p.id === id);
    if (!t) throw new Error(`provider ${id} not found`);
    t.name = input.name.trim();
    t.model_default = input.model_default.trim() || null;
    t.endpoint = input.endpoint.replace(/^https?:\/\//, "");
    t.protocol = input.protocol;
    t.endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    t.endpoint_note = endpointNote(input.protocol, t.endpoints);
    t.billing = input.billing;
    // Absent `agents` keeps the existing bindings (mirrors vm::update_provider):
    // which providers serve an agent is edited on the Apps screen, not here.
    if (input.agents !== undefined) t.agents = [...input.agents];
    t.serving_agents = t.agents.filter((a) => servingNow().has(`${a}/${t.id}`));
    t.is_current = t.serving_agents.length > 0;
    const fb = t.agents.filter((a) => quotaFallbackNow().has(`${a}/${t.id}`));
    if (fb.length > 0) (t as { fallback_agents?: Provider["fallback_agents"] }).fallback_agents = fb;
    else delete (t as { fallback_agents?: Provider["fallback_agents"] }).fallback_agents;
    t.agents_note = t.agents.length ? `${t.agents.length} agent(s)` : undefined;
    // Absent `advanced` keeps existing values (mirrors vm::update_provider).
    if (input.advanced !== undefined) t.advanced = input.advanced;
    if (input.plan_query !== undefined) t.plan_query = input.plan_query;
    t.limit_unit = input.billing_config.limit_unit;
    t.plan_limits = input.billing_config.plan_limits ?? null;
    // Absent `prices` keeps what is stored (mirrors vm::update_provider): the
    // form omits the field whenever its Prices section is not on screen.
    if (input.prices !== undefined) {
      t.prices = input.prices.models.length ? input.prices : null;
    }
    return t;
  },

  async deleteProvider(id: string): Promise<void> {
    await delay();
    const i = providers.findIndex((p) => p.id === id);
    if (i >= 0) providers.splice(i, 1);
    // vm::delete_provider in the mock's smaller world: the provider's bindings
    // go with it, and every agent whose head it was promotes the next
    // candidate, renumbering what is left. Without this the route kept a
    // pointer to a row that is gone — which `pnpm dev` renders as a table with
    // a row missing rather than as the promotion the app actually performs.
    for (const r of agentRoutes) {
      if (!r.bindings.some((b) => b.provider_id === id)) continue;
      const wasHead = r.bindings[0].provider_id === id;
      r.bindings = r.bindings.filter((b) => b.provider_id !== id);
      if (wasHead) r.bindings = r.bindings.map((b, i) => ({ ...b, priority: i }));
    }
  },

  async setProviderEnabled(id: string, enabled: boolean): Promise<void> {
    await delay();
    // vm::set_provider_enabled: the row's own flag, no route touched. What
    // changes is whether it may serve — which `servingNow` reads, the same way
    // the gateway's route table reads `providers.enabled`.
    const target = providers.find((p) => p.id === id);
    if (!target) throw new Error(`provider not found: ${id}`);
    target.enabled = enabled;
    applyServingFlags();
  },

  async testProviderLatency(id: string): Promise<PromptLatency> {
    // A real round trip cannot happen in a browser mock, so this answers the
    // shape the app reads: a model, a latency, and a failure when the provider
    // is one the fixture says is broken.
    await delay(600);
    const p = providers.find((x) => x.id === id);
    if (!p) throw new Error(`provider not found: ${id}`);
    const model = p.model_default ?? "ping-model";
    const checked_at = new Date().toISOString();
    // …and it records the verdict, because that is what the backend does
    // (`vm::test_provider_latency` writes it as the provider's health) and it is
    // what makes the Status cell catch up after a click. A mock that only moved
    // the number on the button would send `pnpm dev` looking at a cell that
    // stays stale — the disagreement this file's header warns about.
    if (p.enabled === false) {
      // Answered and refused: a 401 is the vendor talking, so reachability is
      // not what failed — the key is.
      p.health = {
        state: "error",
        latency_ms: 180,
        source: "test",
        checked_at,
        error: "invalid API key",
      };
      return { provider_id: id, model, latency_ms: 180, status: 401, error: "invalid API key" };
    }
    const base = 220 + (id.length % 7) * 130;
    p.health = { state: "ok", latency_ms: base, source: "test", checked_at };
    return { provider_id: id, model, latency_ms: base, status: 200, error: null };
  },

  async testLatency(endpoint: string): Promise<number> {
    await delay(400);
    const table: Record<string, number> = {
      "api.deepseek.com": 312,
      "api.moonshot.cn": 287,
      "open.bigmodel.cn": 305,
      "localhost:11434": 25,
    };
    return table[endpoint.replace(/^https?:\/\//, "")] ?? 260;
  },

  // Dev probe: pretend the protocol's models route answered (ok), unless the
  // endpoint is empty/decorated with "invalid" — exercises the chip styling
  async testEndpoint(
    _protocol: Protocol,
    endpoint: string,
    apiKey?: string,
    providerId?: string,
  ): Promise<ProbeReport> {
    await delay(500);
    const latency_ms = 200 + Math.floor(Math.random() * 300);
    if (!endpoint.trim() || endpoint.includes("invalid")) {
      return { verdict: "unreachable", status: null, latency_ms, detail: "connection failed (dev)" };
    }
    // Mirrors the backend: a blank key falls back to the stored one, but only
    // for an endpoint that provider already answers on.
    if (!apiKey?.trim() && !storedKeyFor(providerId, endpoint)) {
      return { verdict: "auth", status: 401, latency_ms, detail: "no key for this endpoint (dev)" };
    }
    return { verdict: "ok", status: 200, latency_ms, detail: "12 models listed (dev)" };
  },

  // Dev model list: resolves through the catalog entry matching the endpoint
  // (primary or per-protocol), falling back to a canned OpenAI-style list
  async listModels(
    _protocol: Protocol,
    endpoint: string,
    apiKey: string,
    providerId?: string,
  ): Promise<string[]> {
    await delay(600);
    if (!apiKey.trim() && !storedKeyFor(providerId, endpoint)) {
      throw new Error("no stored key covers this endpoint — enter one to fetch its models");
    }
    // Normalized, like the real matching: the form holds `api.moonshot.cn` where
    // the entry publishes `https://api.moonshot.cn`, and comparing the raw
    // strings missed every entry — so Fetch answered with the generic list even
    // for a provider the catalog knows.
    const ep = endpointKey(endpoint);
    const hit = catalog.find(
      (e) =>
        endpointKey(e.endpoint) === ep ||
        (e.endpoints ?? []).some((x) => endpointKey(x.endpoint) === ep),
    );
    if (hit) {
      return Array.from(new Set([...hit.models, ...(hit.endpoints ?? []).flatMap((x) => x.models ?? [])]));
    }
    return ["gpt-5.2", "gpt-5.2-mini", "o4-mini", "text-embedding-3-large"];
  },

  async listModelPrices(): Promise<ModelPrice[]> {
    await delay();
    // The mock's mirror: one row per catalog `price_ref`, plus the second model
    // DeepSeek prices by time of day — the case the dialog exists to show, and
    // one the catalog cannot (it names a single representative model).
    const rows: ModelPrice[] = catalog
      .filter((e) => e.price_ref)
      .map((e) => ({
        provider_id: e.id,
        model_id: e.price_ref!.model_id,
        display_name: e.price_ref!.display_name,
        input: e.price_ref!.input,
        output: e.price_ref!.output,
        cache_read: "0.03",
        cache_creation: "0",
        currency: e.price_ref!.currency,
        ...(e.price_ref!.off_peak ? { off_peak: e.price_ref!.off_peak } : {}),
        ...(e.price_ref!.peak_hours ? { peak_hours: e.price_ref!.peak_hours } : {}),
        ...(e.price_ref!.long_context ? { long_context: e.price_ref!.long_context } : {}),
      }));
    rows.push({
      provider_id: "deepseek",
      model_id: "deepseek-reasoner (R1)",
      display_name: "DeepSeek Reasoner (R1)",
      input: "0.55",
      output: "2.19",
      cache_read: "0.14",
      cache_creation: "0",
      currency: "USD",
      off_peak: { in: "0.28", out: "1.10", cache_read: "0.07", cache_creation: "0" },
      peak_hours: {
        tz_offset: 480,
        windows: [
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "09:00", end: "12:00" },
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "14:00", end: "18:00" },
        ],
      },
    });
    return rows;
  },

  async openUrl(url: string): Promise<void> {
    // Browser dev: no OS opener. Opening a tab is the closest thing, and the
    // real command is fenced to http(s) anyway.
    window.open(url, "_blank", "noopener");
  },

  async listCatalog(): Promise<CatalogList> {
    await delay();
    // Mirror the backend: `added` derives from the provider list at read
    // time, matching the primary endpoint OR any additional per-protocol
    // endpoint on either side (merged catalog entries stay one row)
    const keys = new Set(
      providers.flatMap((p) => [
        endpointKey(p.endpoint),
        ...(p.endpoints ?? []).map((e) => endpointKey(e.endpoint)),
      ]),
    );
    return {
      total: CATALOG_TOTAL,
      entries: catalog.map((e) => ({
        ...e,
        added:
          keys.has(endpointKey(e.endpoint)) ||
          (e.endpoints ?? []).some((x) => keys.has(endpointKey(x.endpoint))),
      })),
    };
  },

  async getDashboard(window: DashboardWindow, providerId?: string, agentId?: string): Promise<DashboardData> {
    await delay();
    const base = dashboards[window];
    // Filter options mirror the backend: who has traffic in the window,
    // independent of the active filter. Ids resolve through the provider
    // list; by_provider labels are short display names, so unmatched ones
    // fall back to the label itself as the id.
    const filter_providers = base.by_provider.map((p) => ({ id: p.id, label: p.name }));
    const filter_agents = base.by_agent.map((a) => ({ id: a.agent, label: a.label }));
    const data: DashboardData = { ...base, filter_providers, filter_agents };
    // The mock narrows the breakdowns only (headline stats stay fixture-wide).
    // By id, which is what the screen filters on: matching on the display name
    // was a lookup that quietly stopped working the moment the two lists spelled
    // a vendor differently.
    if (providerId) {
      data.by_provider = base.by_provider
        .filter((p) => p.id === providerId)
        .map((p) => ({ ...p, pct: 100 }));
    }
    if (agentId) {
      data.by_agent = base.by_agent.filter((a) => a.agent === agentId);
    }
    return data;
  },

  async getSettings(): Promise<AppSettings> {
    await delay();
    // Fresh copy every call: the caller mutates nothing, but React needs a new
    // reference to re-render after setTakeover mutated the fixture in place.
    return structuredClone(settings);
  },

  async updateSettings(patch: Partial<AppSettings>): Promise<AppSettings> {
    await delay();
    Object.assign(settings, patch);
    return settings;
  },

  async updateCustomAgent(
    id: string,
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent> {
    await delay();
    const list = settings.custom_agents;
    const index = list.findIndex((a) => a.id === id);
    if (index < 0) throw new Error(`no such custom agent: ${id}`);
    // The label, the note and the protocol move; the id is what routes and keys
    // point at. All three are replaced rather than patched — `null` here means
    // "not said", which is a value the field can hold — so a caller sends the
    // ones it is not changing back unchanged, the same line the backend draws.
    const updated: CustomAgent = {
      ...list[index],
      label: label.trim(),
      note: note?.trim() ? note.trim() : null,
      protocol: protocol ?? null,
    };
    settings.custom_agents = [
      ...list.slice(0, index),
      updated,
      ...list.slice(index + 1),
    ];
    return structuredClone(updated);
  },

  async addCustomAgent(
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent> {
    await delay();
    const created: CustomAgent = {
      // The same shape vm::add_custom_agent derives, so a copied id reads the
      // same in either mode.
      id: `${label.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "custom"}-${Math.random().toString(16).slice(2, 8)}`,
      label: label.trim(),
      note: note?.trim() ? note.trim() : null,
      protocol: protocol ?? null,
      placeholder_key: "",
    };
    created.placeholder_key = `kw-ag-${created.id}-${Math.random().toString(16).slice(2, 6)}`;
    settings.custom_agents = [...settings.custom_agents, created];
    // A route with the same default as the backend's: single, no candidates yet.
    agentRoutes.push({
      agent: created.id,
      strategy: "single",
      config: null,
      bindings: [],
      limits: [],
    });
    return structuredClone(created);
  },

  async removeCustomAgent(id: string): Promise<void> {
    await delay();
    settings.custom_agents = settings.custom_agents.filter((a) => a.id !== id);
    // The route and the key go with it; usage rows are history and stay (the
    // mock has none for these, but the dashboard fixtures are keyed by agent
    // name, so a removed agent's traffic would survive here too).
    const i = agentRoutes.findIndex((r) => r.agent === id);
    if (i >= 0) agentRoutes.splice(i, 1);
  },

  async setTakeover(agent: AgentId, enabled: boolean): Promise<void> {
    await delay();
    const t = settings.takeovers.find((x) => x.agent === agent);
    if (!t) return;
    if (enabled) {
      const suffix = Math.random().toString(16).slice(2, 6);
      t.placeholder_key = `kw-ag-${agent}-${suffix}`;
      t.enabled = true;
    } else {
      // The route goes with the takeover (vm::set_agent_takeover): the agent
      // has its own config back, so it is nobody's candidate — and the
      // providers it named stay in the list, unbound.
      const i = agentRoutes.findIndex((r) => r.agent === agent);
      if (i >= 0) agentRoutes.splice(i, 1);
      t.placeholder_key = null;
      t.enabled = false;
    }
  },

  async importCcSwitch(): Promise<ImportReport> {
    await delay(600);
    return {
      imported: 2,
      skipped: 1,
      detail: [
        "source: ~/.cc-switch/config.json (3 rows)",
        "import ccs-claude-default: DeepSeek (anthropic)",
        "import ccs-codex-default: Groq (openai)",
        "skip claude/claude-official: missing endpoint or key",
      ],
    };
  },

  async syncHub(): Promise<HubSyncReport> {
    await delay(500);
    // Mirror the real conditional sync: the first call downloads, later ones
    // report "unchanged" (manifest sha matched) so both UI branches are
    // reachable in dev.
    const unchanged = devHubSynced;
    devHubSynced = true;
    return {
      fetched: 42,
      synced_at: new Date().toISOString(),
      hub_url: settings.hub_url,
      unchanged,
      pricing_version: 1,
      pricing_unchanged: unchanged,
    };
  },

  async checkUsageAlerts(): Promise<UsageAlert[]> {
    await delay();
    // The two feature flags the fixture turns on each get their sample alert,
    // so the notification path is exercisable in `pnpm dev` — but firing them
    // on every 60s poll would make the dev session unusable, so they answer
    // once per page load, the way the backend's per-period dedup reads.
    const out: UsageAlert[] = [];
    if (settings.feat_cost_forecast && !devAlerted.has("forecast")) {
      devAlerted.add("forecast");
      out.push({
        provider_id: "deepseek",
        provider_name: "DeepSeek",
        used: 31.2,
        limit: 50,
        unit: "CNY",
        kind: "cost_forecast",
        message: "DeepSeek: on pace for 62 CNY this period — past the 50 limit",
      });
    }
    if (settings.feat_anomaly_alerts && !devAlerted.has("anomaly")) {
      devAlerted.add("anomaly");
      out.push({
        provider_id: "",
        provider_name: "",
        used: 0,
        limit: 0,
        unit: "",
        kind: "anomaly",
        message: "Error spike: 19 of the last hour's 23 requests failed (83% — 7-day baseline 2%)",
      });
    }
    return out;
  },

  async getPlanQuota(providerId: string, force?: boolean): Promise<PlanQuotaReport> {
    await delay(600);
    const p = providers.find((x) => x.id === providerId);
    const template = p?.plan_query?.template ?? "kimi";
    if (!p?.plan_query) {
      return {
        provider_id: providerId,
        template,
        success: false,
        error: "This provider has no plan query configured",
        note: null,
        tiers: [],
        queried_at: Date.now(),
        cached: false,
      };
    }
    return {
      provider_id: providerId,
      template,
      success: true,
      error: null,
      note: "Standard plan",
      tiers: [
        { name: "five_hour", utilization: 42.5, resets_at: "2026-09-10T18:00:00Z", used: null, limit: null, unit: null },
        { name: "monthly", utilization: 18.3, resets_at: "2026-09-30", used: 295, limit: 460, unit: "requests" },
      ],
      queried_at: Date.now(),
      cached: !force && Math.random() < 0.5,
    };
  },

  async getCurrencyMeta(): Promise<CurrencyMeta> {
    await delay();
    return {
      currencies: ["USD", "CNY"],
      exchange_rates: { USD: 1.0, CNY: 7.1 },
      preferred: settings.preferred_currency,
    };
  },

  async listApiKeys(providerId: string): Promise<ApiKeyEntry[]> {
    await delay();
    return devApiKeys.filter((k) => k.provider_id === providerId).map(toApiKeyEntry);
  },

  async addApiKey(providerId: string, apiKey: string, label?: string): Promise<ApiKeyEntry> {
    await delay();
    const row: DevApiKeyRow = {
      id: ++idSeq,
      provider_id: providerId,
      key: apiKey.trim(),
      label: label?.trim() || undefined,
      enabled: true,
      created_at: new Date().toISOString(),
    };
    devApiKeys.push(row);
    return toApiKeyEntry(row);
  },

  async deleteApiKey(id: number): Promise<void> {
    await delay();
    const i = devApiKeys.findIndex((k) => k.id === id);
    if (i >= 0) devApiKeys.splice(i, 1);
  },

  async getAgentRoutes(): Promise<AgentRoute[]> {
    await delay();
    // Mirror the backend: only agents with at least one binding get a route
    return agentRoutes
      .filter((r) => r.bindings.length > 0)
      .map((r) => ({ ...r, bindings: r.bindings.map((b) => ({ ...b })) }));
  },

  async setAgentLimits(agent: AgentId, limits: AgentLimit[]): Promise<void> {
    await delay();
    // A window of zero is the absence of that window rather than a ceiling of
    // nothing, mirroring vm::set_agent_limits — a fixture that stored it would
    // show a limit the gateway would not enforce.
    strategyOf(agent).limits = limits.filter((l) => l.period && l.period_limit > 0);
  },

  async updateAgentStrategy(agent: AgentId, strategy: StrategyKind, config?: string | null): Promise<void> {
    await delay();
    strategyOf(agent).strategy = strategy;
    strategyOf(agent).config = config ?? null;
    // Entering roundrobin: reseed the weights as an even split of 100
    // (remainder to the head of the queue), mirroring vm::set_agent_strategy
    if (strategy === "roundrobin") {
      const bs = strategyOf(agent).bindings;
      bs.forEach((b, i) => {
        b.weight = Math.max(1, Math.floor(100 / bs.length) + (i < 100 % bs.length ? 1 : 0));
      });
    }
  },

  async reorderAgentBindings(agent: AgentId, providerIds: string[]): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    route.bindings = providerIds
      .map((pid, i) => {
        const b = route.bindings.find((x) => x.provider_id === pid)!;
        return { ...b, priority: i };
      })
      .sort((a, b) => a.priority - b.priority);
  },

  async updateAgentBinding(
    agent: AgentId,
    providerId: string,
    patch: { weight?: number; win_start?: string | null; win_end?: string | null },
  ): Promise<void> {
    await delay();
    const b = strategyOf(agent).bindings.find((x) => x.provider_id === providerId);
    if (!b) throw new Error(`provider ${providerId} is not bound to ${agent}`);
    if (patch.weight != null) b.weight = Math.max(1, patch.weight);
    // Both bounds are set/cleared together: a half window would never match.
    if (patch.win_start !== undefined || patch.win_end !== undefined) {
      const s = patch.win_start || null;
      const e = patch.win_end || null;
      if (s && e) {
        b.win_start = s;
        b.win_end = e;
      } else {
        b.win_start = null;
        b.win_end = null;
      }
    }
  },

  async addAgentBinding(agent: AgentId, providerId: string): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    if (route.bindings.some((b) => b.provider_id === providerId)) return;
    route.bindings.push(bind(providerId, route.bindings.length));
    // Mirror the backend: a provider's agents derive from its bindings
    const p = providers.find((x) => x.id === providerId)!;
    if (!p.agents.includes(agent)) p.agents.push(agent);
  },

  async removeAgentBinding(agent: AgentId, providerId: string): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    const i = route.bindings.findIndex((b) => b.provider_id === providerId);
    if (i < 0) throw new Error(`provider ${providerId} is not bound to ${agent}`);
    route.bindings.splice(i, 1);
    const p = providers.find((x) => x.id === providerId)!;
    p.agents = p.agents.filter((a) => a !== agent);
  },

  async applyAgentRoute(target: AgentId, source: AgentId): Promise<void> {
    await delay();
    if (target === source) throw new Error("cannot copy an agent's route onto itself");
    const src = agentRoutes.find((r) => r.agent === source);
    if (!src || src.bindings.length === 0) throw new Error(`${source} has no route to copy`);
    const existing = agentRoutes.find((r) => r.agent === target);
    const before = new Set(existing?.bindings.map((b) => b.provider_id) ?? []);
    const copy: AgentRoute = {
      agent: target,
      strategy: src.strategy,
      config: src.config,
      bindings: src.bindings.map((b) => ({ ...b })),
      // The ceilings are the agent's own, not part of the route being copied:
      // inheriting someone else's budget is not what "copy this route" means.
      limits: existing?.limits ?? [],
    };
    const i = agentRoutes.findIndex((r) => r.agent === target);
    if (i >= 0) agentRoutes[i] = copy;
    else agentRoutes.push(copy);
    // Mirror the backend: a provider's agents derive from its bindings
    for (const pid of before) {
      if (copy.bindings.some((b) => b.provider_id === pid)) continue;
      const p = providers.find((x) => x.id === pid);
      if (p) p.agents = p.agents.filter((a) => a !== target);
    }
    for (const b of copy.bindings) {
      const p = providers.find((x) => x.id === b.provider_id);
      if (p && !p.agents.includes(target)) p.agents.push(target);
    }
  },

  async exportConfig(_path: string): Promise<number> {
    await delay(300);
    return providers.length;
  },

  async importConfig(_path: string): Promise<ConfigShareReport> {
    await delay(400);
    return { providers_added: 1, providers_kept: 2, routes_applied: 1 };
  },

  async getFooterStats(): Promise<FooterStats> {
    await delay();
    return {
      today_requests: 1284,
      today_tokens: 1_900_000,
      hub_synced: true,
      version: "v0.1.2",
    };
  },

  async checkAppUpdate(): Promise<UpdateInfo | null> {
    await delay();
    return null;
  },

  async getPendingUpdate(): Promise<UpdateInfo | null> {
    await delay();
    return null;
  },

  async downloadAndInstallAppUpdate(): Promise<void> {
    await delay();
  },

  async onUpdateProgress(_cb: (p: UpdateProgress) => void): Promise<() => void> {
    return () => {};
  },

  async detectAgents() {
    await delay(120);
    // Dev fixture: most agents installed, a few not — the missing ones are
    // what the "point Kiwano at it" menu exists for, and they match the
    // versions fixture below. The registry's own ids: `a.id` is an `AgentRef`
    // since agents can also be user-defined, and only built-ins are detectable.
    return AGENTS.map((a) => {
      const id = a.id as AgentId;
      const declared = declaredDirs[id];
      if (declared) {
        return {
          agent: id,
          installed: true,
          path: `${declared}/${binaryFor(id)}`,
          manual: true,
        };
      }
      const installed = !NOT_INSTALLED_AGENTS.includes(id);
      return {
        agent: id,
        installed,
        path: installed ? `/usr/local/bin/${binaryFor(id)}` : null,
        manual: false,
      };
    });
  },

  async agentSearchDirs(_agent: AgentId): Promise<string[]> {
    await delay();
    // The real list is the walk's, which is per-platform and per-tool. The
    // fixture answers with the shared prefixes, which is the shape of it.
    return [
      "~/.local/bin",
      "~/.npm-global/bin",
      "~/n/bin",
      "~/.volta/bin",
      "~/.local/share/mise/shims",
      "~/Library/pnpm",
      "/opt/homebrew/bin",
      "/usr/local/bin",
      "/usr/bin",
      "/bin",
      "/usr/sbin",
      "/sbin",
      // A real one, and the reason the list has to truncate: PATH entries are
      // as long as the platform feels like making them.
      "/var/run/com.apple.security.cryptexd/codex.system/bootstrap/usr/local/bin",
    ];
  },

  async verifyAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit> {
    await delay();
    // The real backend runs the executable it finds and reports what it printed.
    // The fixture has no filesystem to look in, so every absolute directory
    // "works" — which leaves the failure branch reachable only by typing a
    // relative path, and the rest of it to the real app.
    const trimmed = dir.trim();
    if (!trimmed.startsWith("/")) throw new Error(`no \`${binaryFor(agent)}\` in ${trimmed}`);
    return { path: `${trimmed}/${binaryFor(agent)}`, version: DEV_DECLARED_VERSION };
  },

  async setAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit> {
    await delay();
    const trimmed = dir.trim();
    if (!trimmed.startsWith("/")) throw new Error(`no \`${binaryFor(agent)}\` in ${trimmed}`);
    declaredDirs[agent] = trimmed;
    return { path: `${trimmed}/${binaryFor(agent)}`, version: DEV_DECLARED_VERSION };
  },

  async clearAgentDir(agent: AgentId): Promise<void> {
    await delay();
    delete declaredDirs[agent];
  },

  async probeAgentVersions() {
    await delay(600);
    // A declared agent is one the probe now finds, so it belongs here too —
    // otherwise a directory the user just pointed at would show a tab with no
    // version, which is not what the real backend does.
    const declared = Object.keys(declaredDirs).map((agent) => ({
      agent: agent as AgentId,
      version: DEV_DECLARED_VERSION,
    }));
    return [
      { agent: "claude", version: "2.1.83 (Claude Code)" },
      { agent: "codex", version: "0.42.0" },
      { agent: "grokbuild", version: "0.9.4" },
      { agent: "claude-desktop", version: null },
      { agent: "opencode", version: "1.0.120" },
      { agent: "openclaw", version: "0.23.1" },
      { agent: "hermes", version: "0.8.2" },
      { agent: "pi", version: "0.5.12" },
      { agent: "cline", version: "3.0.62" },
      ...declared,
    ];
  },

  async listRequestLogs(page: number, pageSize: number, filter?: RequestLogFilter): Promise<RequestLogList> {
    await delay();
    const rows = matchingLogs(filter);
    const start = (page - 1) * pageSize;
    return { rows: rows.slice(start, start + pageSize).map((r) => ({ ...r })), total: rows.length };
  },

  async getRequestLog(id: number): Promise<RequestLogDetail | null> {
    await delay();
    return requestLogs.find((r) => r.id === id) ?? null;
  },

  // No filesystem in the browser, so this reports what it would have written
  // rather than pretending to write it — the toolbar's feedback then matches
  // the desktop app's without a second code path here. `_includeBodies` has
  // nothing to do without a file to shape, so it only has to be accepted.
  async exportRequestLogs(
    _path: string,
    filter?: RequestLogFilter,
    _includeBodies?: boolean,
  ): Promise<RequestLogExport> {
    await delay();
    return { rows_written: matchingLogs(filter).length, truncated: false };
  },

  async openLogFolder(): Promise<void> {
    // The browser mock has no filesystem to reveal.
    await delay();
  },

  async clearRequestLogs(): Promise<void> {
    await delay();
    requestLogs.length = 0;
  },
};
