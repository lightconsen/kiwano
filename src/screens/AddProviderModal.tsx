// Add/edit provider modal (design/index.html #modal, cc-switch AddProviderDialog pattern)
// Base controls use shadcn/ui (Dialog/Input/Label/Select/Button); the segmented pills are kept as design language
import { useEffect, useState } from "react";
import { Check, ChevronDown, Eye, Gauge, Infinity as InfinityIcon, Plus, RefreshCw, Store, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../api/client";
import {
  AGENTS,
  PLAN_QUERY_TEMPLATES,
  type AgentId,
  type ApiKeyEntry,
  type Billing,
  type CatalogEntry,
  type ProbeReport,
  type Protocol,
  type Provider,
} from "../api/types";
import { AgentChip } from "@/components/bits";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { hubAssetUrl, useHubUrl } from "../lib/hub";

const BILL_OPTIONS: { id: Billing; label: string }[] = [
  { id: "plan", label: "Plan" },
  { id: "payg", label: "Pay as you go" },
  { id: "unl", label: "Unlimited" },
];

const PROTOCOL_OPTIONS: { id: Protocol; label: string }[] = [
  { id: "openai", label: "OpenAI-compatible" },
  { id: "anthropic", label: "Anthropic" },
  { id: "gemini", label: "Gemini API" },
];

// Inline probe readout: ok shows the measured latency, every other verdict
// shows a short word (full detail lives in the span's title). Green/kiwi =
// usable (ok, or route exists but key invalid), red = broken/unreachable.
const PROBE_TEXT: Record<ProbeReport["verdict"], string> = {
  ok: "OK",
  auth: "Auth",
  unsupported: "404",
  error: "Error",
  unreachable: "Down",
};

function probeColor(verdict: ProbeReport["verdict"]): string {
  return verdict === "ok" || verdict === "auth" ? "var(--kiwi)" : "oklch(0.62 0.2 25)";
}

export default function AddProviderModal({
  open,
  preset,
  edit,
  onClose,
  onSaved,
}: {
  open: boolean;
  preset: CatalogEntry | null;
  edit: Provider | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const hubUrl = useHubUrl();
  const [mode, setMode] = useState<"shelf" | "custom">("shelf");
  // Catalog entry chosen in the "From Models" mode (preset pre-seeds it when
  // the modal opens from the Models page; otherwise picked in-modal)
  const [shelf, setShelf] = useState<CatalogEntry | null>(null);
  const [catalog, setCatalog] = useState<CatalogEntry[] | null>(null);
  const [query, setQuery] = useState("");
  const [name, setName] = useState("");
  const [protocol, setProtocol] = useState<Protocol>("openai");
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [endpoint, setEndpoint] = useState("");
  // Additional per-protocol endpoints (one provider serves agents speaking
  // other protocols natively, e.g. Qianfan openai + anthropic)
  const [altEndpoints, setAltEndpoints] = useState<{ protocol: Protocol; endpoint: string }[]>([]);
  const [altProbes, setAltProbes] = useState<Record<number, ProbeReport | null>>({});
  const [altTesting, setAltTesting] = useState<Record<number, boolean>>({});
  const [model, setModel] = useState("");
  // Controlled open state: the model select must never pop open on its own
  // when the modal is prefilled from a catalog entry
  const [modelOpen, setModelOpen] = useState(false);
  // Live model list fetched from the endpoint (Fetch button): overrides the
  // catalog options when present, and upgrades the free-text input in
  // custom/edit mode to a dropdown
  const [fetchedModels, setFetchedModels] = useState<string[] | null>(null);
  const [fetching, setFetching] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [billing, setBilling] = useState<Billing>("payg");
  // Payg spending-limit number (plan providers use the percent pair below)
  const [limitValue, setLimitValue] = useState("");
  // Pay-as-you-go spending-limit currency
  const [paygCurrency, setPaygCurrency] = useState("CNY");
  // ISO codes offered by the currency selects (bundled price table)
  const [currencies, setCurrencies] = useState<string[]>([]);
  const [agents, setAgents] = useState<AgentId[]>([]);
  // Multi-select dropdown for agent binding (rows = logo + name)
  const [agentsOpen, setAgentsOpen] = useState(false);
  const [testing, setTesting] = useState(false);
  // Protocol-aware probe of the primary endpoint (uses the form's API key
  // when present — a 401 verdict means the key is wrong, not the route)
  const [probe, setProbe] = useState<ProbeReport | null>(null);
  const [saving, setSaving] = useState(false);
  // Rotating Key management (spec §4.1 P1 multi-key rotation; available in edit mode)
  const [pollKeys, setPollKeys] = useState<ApiKeyEntry[]>([]);
  const [newKey, setNewKey] = useState("");
  const [newKeyLabel, setNewKeyLabel] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  // Advanced forwarding settings (per provider): response-header timeout,
  // same-provider retries before failover, custom upstream headers
  const [advOpen, setAdvOpen] = useState(false);
  const [advTimeout, setAdvTimeout] = useState("");
  const [advRetries, setAdvRetries] = useState("");
  const [advHeaders, setAdvHeaders] = useState<{ name: string; value: string }[]>([]);
  // Plan-quota query config (edit mode): "" = not configured. Fields are the
  // selected template's extra credentials (org/project IDs, AK/SK, quota URL).
  const [pqOpen, setPqOpen] = useState(false);
  const [pqTemplate, setPqTemplate] = useState("");
  const [pqFields, setPqFields] = useState<Record<string, string>>({});
  // Plan-mode percent limits: per-window utilization ceilings over the
  // vendor's rolling 5-hour / weekly windows (blank = no limit on it).
  const [planFiveHour, setPlanFiveHour] = useState("");
  const [planWeekly, setPlanWeekly] = useState("");

  // Prefill the form from a catalog entry (Models-page preset or in-modal pick)
  const applyShelf = (e: CatalogEntry) => {
    setName(e.name);
    setProtocol(e.protocol); // catalog entries carry their protocol fingerprint
    setApiKey("");
    setEndpoint(e.endpoint);
    setAltEndpoints((e.endpoints ?? []).map((x) => ({ protocol: x.protocol, endpoint: x.endpoint })));
    setAltProbes({});
    setModel(e.models[0] ?? "");
    setBilling(e.billing);
    setLimitValue(e.billing === "payg" ? "50" : "");
    setPaygCurrency("CNY");
    setPlanFiveHour("");
    setPlanWeekly("");
    setAgents(e.id === "deepseek" ? ["claude", "codex"] : []);
    setFetchedModels(null);
    setFetchError(null);
    setPqTemplate("");
    setPqFields({});
    setPqOpen(false);
    resetAdvanced();
  };

  // Default model options: a fetched live list wins over the catalog's
  // (primary models ∪ each additional endpoint's models)
  const shelfModelOptions = shelf
    ? Array.from(new Set([...shelf.models, ...(shelf.endpoints ?? []).flatMap((x) => x.models ?? [])]))
    : [];
  const modelOptions = fetchedModels ?? shelfModelOptions;
  const showModelSelect = (fetchedModels?.length ?? 0) > 0 || (!edit && mode === "shelf" && !!shelf);

  // Protocols this provider serves: the primary + every additional endpoint's
  const supportedProtocols = new Set<Protocol>([protocol, ...altEndpoints.map((r) => r.protocol)]);

  const setHeader = (i: number, patch: Partial<{ name: string; value: string }>) =>
    setAdvHeaders((rows) => rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const removeHeader = (i: number) =>
    setAdvHeaders((rows) => rows.filter((_, j) => j !== i));

  const addHeader = () => setAdvHeaders((rows) => [...rows, { name: "", value: "" }]);

  const resetAdvanced = () => {
    setAdvOpen(false);
    setAdvTimeout("");
    setAdvRetries("");
    setAdvHeaders([]);
  };

  // Fetch the live model-name list from the primary endpoint. Requires the
  // API key (cloud providers reject anonymous /models calls) — a missing key
  // surfaces as an inline error instead of a doomed request.
  const fetchModels = async () => {
    if (fetching || !endpoint.trim()) return;
    if (!apiKey.trim()) {
      setFetchError("Enter the API key first — providers reject anonymous model lists");
      return;
    }
    setFetching(true);
    setFetchError(null);
    try {
      const list = await api.listModels(protocol, endpoint.trim(), apiKey.trim());
      if (list.length === 0) {
        setFetchError("The endpoint returned no models");
      } else {
        setFetchedModels(list);
      }
    } catch (e) {
      setFetchError(String(e));
    } finally {
      setFetching(false);
    }
  };

  const testAlt = async (i: number) => {
    const row = altEndpoints[i];
    if (!row?.endpoint.trim()) return;
    setAltTesting((m) => ({ ...m, [i]: true }));
    try {
      const report = await api.testEndpoint(row.protocol, row.endpoint.trim(), apiKey.trim() || undefined);
      setAltProbes((m) => ({ ...m, [i]: report }));
    } catch (e) {
      // Rejected invoke → visible verdict instead of a silent no-op
      setAltProbes((m) => ({
        ...m,
        [i]: { verdict: "error", status: null, latency_ms: 0, detail: String(e) },
      }));
    } finally {
      setAltTesting((m) => ({ ...m, [i]: false }));
    }
  };

  useEffect(() => {
    if (!open) return;
    setShowKey(false);
    setProbe(null);
    setAgentsOpen(false);
    setModelOpen(false);
    setFetchedModels(null);
    setFetchError(null);
    setNewKey("");
    setNewKeyLabel("");
    setQuery("");
    if (edit) {
      setMode("custom");
      setShelf(null);
      setName(edit.name);
      setProtocol(edit.protocol);
      setApiKey(""); // leave empty = keep the existing key
      setEndpoint(edit.endpoint);
      setAltEndpoints((edit.endpoints ?? []).map((x) => ({ protocol: x.protocol, endpoint: x.endpoint })));
      setAltProbes({});
      setModel("");
      setBilling(edit.billing);
      const q = edit.usage?.quota;
      // Payg spending limit (plan providers now carry percent limits instead)
      setLimitValue(edit.billing === "payg" && q ? String(q.limit) : "");
      setPaygCurrency(edit.limit_unit && edit.limit_unit.length === 3 ? edit.limit_unit : "CNY");
      setPlanFiveHour(edit.plan_limits?.five_hour != null ? String(edit.plan_limits.five_hour) : "");
      setPlanWeekly(edit.plan_limits?.weekly != null ? String(edit.plan_limits.weekly) : "");
      setAgents([...edit.agents]);
      const pq = edit.plan_query;
      setPqTemplate(pq?.template ?? "");
      setPqFields(pq?.fields ? { ...pq.fields } : {});
      setPqOpen(!!pq);
      setAdvOpen(!!edit.advanced);
      setAdvTimeout(edit.advanced?.timeout_secs != null ? String(edit.advanced.timeout_secs) : "");
      setAdvRetries(edit.advanced?.retries != null ? String(edit.advanced.retries) : "");
      setAdvHeaders(
        Object.entries(edit.advanced?.headers ?? {}).map(([hName, hValue]) => ({ name: hName, value: hValue })),
      );
      api.listApiKeys(edit.id).then(setPollKeys).catch(() => setPollKeys([]));
      return;
    }
    // Default to the "From Models" catalog picker; a Models-page preset
    // pre-selects its entry directly
    setMode("shelf");
    setShelf(preset);
    resetAdvanced();
    if (preset) {
      applyShelf(preset);
    } else {
      setName("");
      setProtocol("openai");
      setApiKey("");
      setEndpoint("");
      setAltEndpoints([]);
      setModel("");
      setBilling("payg");
      setLimitValue("");
      setPaygCurrency("CNY");
      setPqTemplate("");
      setPqFields({});
      setPqOpen(false);
      setPlanFiveHour("");
      setPlanWeekly("");
      setAgents([]);
    }
  }, [open, preset, edit]);

  // Currency codes for the limit-unit / plan-query selects (fetched once)
  useEffect(() => {
    if (open && currencies.length === 0) {
      api.getCurrencyMeta().then((m) => setCurrencies(m.currencies)).catch(() => {});
    }
  }, [open, currencies.length]);

  // Catalog loads lazily, the first time the in-modal picker is shown
  useEffect(() => {
    if (open && !edit && mode === "shelf" && !shelf && catalog === null) {
      api.listCatalog().then((c) => setCatalog(c.entries));
    }
  }, [open, edit, mode, shelf, catalog]);

  // Close the agents dropdown on Escape (outside clicks are handled by the
  // transparent overlay rendered behind the open panel)
  useEffect(() => {
    if (!agentsOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAgentsOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [agentsOpen]);

  const canSave = name.trim() !== "" && endpoint.trim() !== "" && !saving;

  // Billing is intrinsic to the provider: locked to the catalog entry when
  // adding from Models, and to the stored value when editing. Vendors that
  // support several modes get one catalog entry per mode. Custom providers
  // (no catalog entry behind the form) pick freely at creation.
  // A Hub catalog row may carry a billing tag this build predates: keep it
  // visible instead of silently rewriting it to payg, and leave the picker
  // open — the backend rejects an unknown tag on save, so locking the form
  // would dead-end the entry.
  const billingKnown = BILL_OPTIONS.some((b) => b.id === billing);
  const billingLocked = (!!edit || (mode === "shelf" && !!shelf)) && billingKnown;

  // Non-empty credential fields of the selected template (empty rows dropped)
  const pqFieldInputs = (): Record<string, string> => {
    const def = PLAN_QUERY_TEMPLATES.find((t) => t.id === pqTemplate);
    const out: Record<string, string> = {};
    for (const f of def?.fields ?? []) {
      const v = (pqFields[f.key] ?? "").trim();
      if (v !== "") out[f.key] = v;
    }
    return out;
  };

  const maskKey = (k: string) => (k.length > 12 ? `${k.slice(0, 6)}…${k.slice(-4)}` : k);

  const addPollKey = async () => {
    if (!edit || !newKey.trim() || keyBusy) return;
    setKeyBusy(true);
    try {
      const row = await api.addApiKey(edit.id, newKey.trim(), newKeyLabel.trim() || undefined);
      setPollKeys((ks) => [...ks, row]);
      setNewKey("");
      setNewKeyLabel("");
    } finally {
      setKeyBusy(false);
    }
  };

  const removePollKey = async (id: number) => {
    if (keyBusy) return;
    setKeyBusy(true);
    try {
      await api.deleteApiKey(id);
      setPollKeys((ks) => ks.filter((k) => k.id !== id));
    } finally {
      setKeyBusy(false);
    }
  };

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    try {
      // Drop empty rows, rows duplicating the primary protocol, and duplicate
      // protocols — the store PK is (provider_id, protocol)
      const used = new Set<Protocol>([protocol]);
      const altInputs: { protocol: Protocol; endpoint: string }[] = [];
      for (const r of altEndpoints) {
        const ep = r.endpoint.trim();
        if (ep === "" || used.has(r.protocol)) continue;
        used.add(r.protocol);
        altInputs.push({ protocol: r.protocol, endpoint: ep });
      }
      // Authoritative advanced snapshot: prefilled from edit.advanced, so
      // re-sending it round-trips untouched values; nulls clear fields.
      const headerMap: Record<string, string> = {};
      for (const r of advHeaders) {
        if (r.name.trim() !== "" && r.value.trim() !== "") headerMap[r.name.trim()] = r.value.trim();
      }
      const input = {
        name: name.trim(),
        api_key: apiKey,
        endpoint: endpoint.trim(),
        protocol,
        model_default: model,
        billing,
        billing_config:
          billing === "plan"
            ? {
                // Percent limits only; the backend clears the legacy
                // number+unit+reset columns for plan rows.
                plan_limits: {
                  five_hour: planFiveHour.trim() ? Number(planFiveHour) : undefined,
                  weekly: planWeekly.trim() ? Number(planWeekly) : undefined,
                },
              }
            : {
                limit_value: limitValue ? Number(limitValue) : undefined,
                limit_unit: billing === "payg" ? paygCurrency : undefined,
              },
        agents,
        endpoints: altInputs,
        advanced: {
          timeout_secs: advTimeout ? Number(advTimeout) : null,
          retries: advRetries ? Number(advRetries) : null,
          headers: Object.keys(headerMap).length > 0 ? headerMap : undefined,
        },
        // Edit only: "" = clear the config, a template = authoritative snapshot.
        plan_query: edit ? (pqTemplate ? { template: pqTemplate, fields: pqFieldInputs() } : null) : undefined,
      };
      if (edit) {
        await api.updateProvider(edit.id, input);
      } else {
        await api.addProvider(input);
      }
      onSaved();
      onClose();
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      {/* overflow-x-hidden: WebKit (WKWebView) computes flex/grid min-content
          wider than Blink, letting some inner row force a horizontal scrollbar
          on the modal; nothing here legitimately scrolls horizontally, so clip. */}
      <DialogContent className="max-h-[min(600px,100dvh)] w-[calc(100%-2rem)] max-w-[480px] gap-0 overflow-x-hidden overflow-y-auto rounded-xl p-0 sm:max-w-[480px]">
        <DialogHeader className="flex h-11 flex-row items-center justify-between border-b border-line px-4">
          <DialogTitle className="text-[13px] font-semibold">
            {edit ? "Edit provider" : "Add provider"}
          </DialogTitle>
        </DialogHeader>

        <div className="px-4 py-3.5">
          {/* Mode switch (hidden in edit mode: no models involved) */}
          {!edit && (
            <div className="flex overflow-hidden rounded-md border border-line text-[11.5px]">
              <div
                className="btn flex h-8 min-w-0 flex-1 cursor-pointer items-center justify-center gap-1 font-medium"
                style={mode === "shelf" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
                onClick={() => setMode("shelf")}
              >
                <Store className="h-3 w-3" />
                From Models{shelf ? `: ${shelf.name}` : ""}
              </div>
              <div
                className="btn flex h-8 min-w-0 flex-1 cursor-pointer items-center justify-center"
                style={mode === "custom" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
                onClick={() => setMode("custom")}
              >
                Custom
              </div>
            </div>
          )}

          {/* In-modal catalog picker (shelf mode without a chosen entry) */}
          {!edit && mode === "shelf" && !shelf && (
            <div className="mt-3">
              <Input
                className="h-8 bg-bg text-[12px] dark:bg-bg"
                placeholder="Search the catalog…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
              {/* rows carry logo + name only (endpoint URLs were dropped: they
                  ellipsized awkwardly in the narrow list); overflow-x-hidden
                  stays as WKWebView hardening */}
              <div className="mt-1.5 max-h-40 overflow-x-hidden overflow-y-auto rounded-md border border-line">
                {(catalog ?? [])
                  .filter(
                    (e) =>
                      !e.added &&
                      e.name.toLowerCase().includes(query.trim().toLowerCase()),
                  )
                  .map((e) => (
                    <button
                      key={e.id}
                      className="btn flex w-full items-center gap-2 border-b border-line px-2.5 py-1.5 text-left last:border-b-0 hover:bg-surface2"
                      onClick={() => {
                        setShelf(e);
                        applyShelf(e);
                      }}
                    >
                      <ProviderLogo
                        logo={e.logo && hubUrl ? hubAssetUrl(hubUrl, e.logo) : undefined}
                        icon={e.icon}
                        name={e.name}
                        color={e.logo_color}
                        size={16}
                      />
                      <span className="min-w-0 truncate text-[12px] font-medium">{e.name}</span>
                    </button>
                  ))}
                {catalog && catalog.filter((e) => !e.added && e.name.toLowerCase().includes(query.trim().toLowerCase())).length === 0 && (
                  <div className="px-2.5 py-3 text-center text-[11px] text-mut">No matching providers</div>
                )}
                {!catalog && (
                  <div className="px-2.5 py-3 text-center text-[11px] text-mut">Loading…</div>
                )}
              </div>
            </div>
          )}

          {/* Chosen catalog entry (with a way back to the picker) */}
          {!edit && mode === "shelf" && shelf && (
            <div className="mt-3 flex items-center gap-2 rounded-md border border-line px-2.5 py-1.5">
              <ProviderLogo
                logo={shelf.logo && hubUrl ? hubAssetUrl(hubUrl, shelf.logo) : undefined}
                icon={shelf.icon}
                name={shelf.name}
                color={shelf.logo_color}
                size={16}
              />
              <span className="min-w-0 truncate text-[12px] font-medium">{shelf.name}</span>
              <button
                className="ml-auto flex-none text-[11px] font-medium"
                style={{ color: "var(--kiwi)" }}
                onClick={() => setShelf(null)}
              >
                Change
              </button>
            </div>
          )}

          <div className="mt-3 space-y-3">
            <div>
              <Label className="text-[11px] font-medium text-mut">Name</Label>
              <Input
                className="mt-1 h-8 bg-bg text-[12px] dark:bg-bg"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">Protocol</Label>
              {/* Read-only coverage: every supported protocol renders as a lit
                  badge (primary + each additional endpoint), the rest dimmed.
                  The authoritative selection is the per-endpoint rows below. */}
              <div className="mt-1 flex gap-1.5">
                {PROTOCOL_OPTIONS.map((p) => (
                  <span
                    key={p.id}
                    className="flex h-7 cursor-default items-center rounded-md border px-2 text-[11.5px]"
                    style={supportedProtocols.has(p.id)
                      ? { borderColor: "var(--kiwi-dim)", background: "var(--kiwi-soft)", color: "var(--kiwi)" }
                      : { borderColor: "var(--line)", color: "var(--mut)" }}
                  >
                    {p.label}
                  </span>
                ))}
              </div>
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">
                API Key{" "}
                <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                  {edit ? "Leave blank to keep the current key" : "Stored in the local keychain only"}
                </span>
              </Label>
              <div className="relative mt-1">
                <Input
                  type={showKey ? "text" : "password"}
                  className="h-8 bg-bg pr-8 font-mono text-[12px] dark:bg-bg"
                  value={apiKey}
                  placeholder={edit ? "••••••••" : ""}
                  onChange={(e) => setApiKey(e.target.value)}
                />
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="absolute top-1/2 right-1 -translate-y-1/2 text-mut"
                  onClick={() => setShowKey(!showKey)}
                  aria-label="Show / hide key"
                >
                  <Eye className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>

            {/* Endpoint URLs, one row per protocol: protocol badge + URL +
                Test. The first row is the primary endpoint, the rest serve
                agents speaking another protocol natively (same API key pool).
                The list comes from the catalog (shelf) or the stored provider
                (edit) — read-only, no add/remove. */}
            <div>
              <Label className="text-[11px] font-medium text-mut">
                Endpoint URL{" "}
                <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                  {!edit && mode === "shelf" && shelf
                    ? "from the catalog · one per protocol · shares the API key"
                    : "one per protocol · shares the API key"}
                </span>
              </Label>
              <div className="mt-1 space-y-1.5">
                {/* Primary endpoint row */}
                <div className="flex items-center gap-1.5">
                  <span className="flex h-8 w-[108px] flex-none items-center overflow-hidden rounded-md border border-line bg-surface2 px-2 text-[11.5px] text-mut">
                    <span className="truncate">
                      {PROTOCOL_OPTIONS.find((p) => p.id === protocol)?.label ?? protocol}
                    </span>
                  </span>
                  <Input
                    className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                    value={endpoint}
                    readOnly={!edit && mode === "shelf" && !!shelf}
                    onChange={(e) => {
                      setEndpoint(e.target.value);
                      setProbe(null);
                      // The fetched list belongs to the old endpoint
                      setFetchedModels(null);
                      setFetchError(null);
                    }}
                  />
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 flex-none gap-1 px-2.5 text-[11px]"
                    disabled={testing}
                    onClick={async () => {
                      setTesting(true);
                      try {
                        setProbe(await api.testEndpoint(protocol, endpoint.trim(), apiKey.trim() || undefined));
                      } catch (e) {
                        // A rejected invoke (e.g. command missing in a stale app
                        // binary) must still land as a visible verdict, not vanish
                        setProbe({ verdict: "error", status: null, latency_ms: 0, detail: String(e) });
                      } finally {
                        setTesting(false);
                      }
                    }}
                  >
                    <Gauge className="h-3 w-3" />
                    Test{" "}
                    {probe && (
                      <span className="font-mono" style={{ color: probeColor(probe.verdict) }} title={probe.detail}>
                        {probe.verdict === "ok" ? `${probe.latency_ms}ms` : PROBE_TEXT[probe.verdict]}
                      </span>
                    )}
                  </Button>
                </div>
                {altEndpoints.map((r, i) => (
                  <div key={i} className="flex items-center gap-1.5">
                    <span className="flex h-8 w-[108px] flex-none items-center overflow-hidden rounded-md border border-line bg-surface2 px-2 text-[11.5px] text-mut">
                      <span className="truncate">
                        {PROTOCOL_OPTIONS.find((p) => p.id === r.protocol)?.label ?? r.protocol}
                      </span>
                    </span>
                    <span className="min-w-0 flex-1">
                      <Input
                        className="h-8 w-full bg-bg font-mono text-[12px] dark:bg-bg"
                        value={r.endpoint}
                        readOnly
                      />
                    </span>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8 flex-none gap-1 px-2.5 text-[11px]"
                      disabled={!r.endpoint.trim() || altTesting[i]}
                      onClick={() => testAlt(i)}
                    >
                      <Gauge className="h-3 w-3" />
                      Test{" "}
                      {altProbes[i] && (
                        <span
                          className="font-mono"
                          style={{ color: probeColor(altProbes[i]!.verdict) }}
                          title={altProbes[i]!.detail}
                        >
                          {altProbes[i]!.verdict === "ok"
                            ? `${altProbes[i]!.latency_ms}ms`
                            : PROBE_TEXT[altProbes[i]!.verdict]}
                        </span>
                      )}
                    </Button>
                  </div>
                ))}
              </div>
            </div>

            <div>
              <div className="flex items-center justify-between">
                <Label className="text-[11px] font-medium text-mut">Default model</Label>
                {/* Pulls the live model list from the primary endpoint; needs
                    the API key, so a click without one shows an inline error */}
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 gap-1 px-2 text-[10.5px] text-mut"
                  disabled={fetching || !endpoint.trim()}
                  onClick={fetchModels}
                >
                  <RefreshCw className={`h-3 w-3${fetching ? " animate-spin" : ""}`} />
                  {fetching ? "Fetching…" : "Fetch"}
                </Button>
              </div>
              {showModelSelect ? (
                <Select value={model} open={modelOpen} onOpenChange={setModelOpen} onValueChange={(v) => setModel(v ?? "")}>
                  <SelectTrigger className="mt-1 w-full bg-bg text-[12px] dark:bg-bg">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {modelOptions.map((m) => (
                      <SelectItem key={m} value={m}>
                        {m}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <Input
                  className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder="model-id"
                />
              )}
              {fetchError && (
                <p className="mt-1 text-[10.5px]" style={{ color: "oklch(0.62 0.2 25)" }} title={fetchError}>
                  {fetchError}
                </p>
              )}
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">
                Billing
                {billingLocked && (
                  <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                    {edit ? "fixed for this provider" : "from the catalog"}
                  </span>
                )}
              </Label>
              {billingLocked ? (
                <div className="mt-1 flex h-8 items-center rounded-md border border-line bg-surface2 px-2.5 text-[11.5px] text-ink">
                  {BILL_OPTIONS.find((b) => b.id === billing)?.label ?? billing}
                </div>
              ) : (
                <div className="mt-1 grid grid-cols-3 gap-1.5">
                  {BILL_OPTIONS.map((b) => {
                    const active = billing === b.id;
                    return (
                      <button
                        key={b.id}
                        className="btn h-8 cursor-pointer rounded-md border text-[11.5px]"
                        style={active
                          ? { borderColor: "var(--kiwi-dim)", background: "var(--kiwi-soft)", color: "var(--kiwi)" }
                          : { borderColor: "var(--line)", color: "var(--mut)" }}
                        onClick={() => setBilling(b.id)}
                      >
                        {b.label}
                      </button>
                    );
                  })}
                </div>
              )}
              {!billingKnown && (
                <p className="mt-1 text-[10.5px]" style={{ color: "var(--amber)" }}>
                  Unrecognized billing tag “{billing}” · pick a mode
                </p>
              )}
            </div>

            {billing === "plan" && (
              <div>
                <Label className="text-[11px] font-medium text-mut">
                  Usage limits
                  <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                    optional · % of each plan window
                  </span>
                </Label>
                <div className="mt-1 grid grid-cols-2 gap-2">
                  <div>
                    <Input
                      className="h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                      value={planFiveHour}
                      onChange={(e) => setPlanFiveHour(e.target.value)}
                      placeholder="e.g. 20 · blank = no limit"
                      inputMode="decimal"
                    />
                    <p className="mt-1 text-[10px] text-mut">5-hour window</p>
                  </div>
                  <div>
                    <Input
                      className="h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                      value={planWeekly}
                      onChange={(e) => setPlanWeekly(e.target.value)}
                      placeholder="e.g. 60 · blank = no limit"
                      inputMode="decimal"
                    />
                    <p className="mt-1 text-[10px] text-mut">Weekly window</p>
                  </div>
                </div>
                <p className="mt-1.5 text-[10.5px] text-mut">
                  Kiwano stops routing to this provider once its live plan-quota
                  utilization for a window reaches the percent, and resumes when
                  usage drops back under. Takes effect when a plan query is configured.
                </p>
              </div>
            )}

            {billing === "payg" && (
              <div>
                <Label className="text-[11px] font-medium text-mut">
                  Spending limit <span className="text-[10px]" style={{ color: "var(--kiwi)" }}>optional · leave blank to show the usage trend</span>
                </Label>
                <div className="mt-1 flex gap-1.5">
                  <Input
                    className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                    value={limitValue}
                    onChange={(e) => setLimitValue(e.target.value)}
                    placeholder="50"
                  />
                  <Select value={paygCurrency} onValueChange={(v) => setPaygCurrency(v ?? "CNY")}>
                    <SelectTrigger className="w-[84px] bg-bg text-[12px] dark:bg-bg">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(currencies.length ? currencies : [paygCurrency]).map((c) => (
                        <SelectItem key={c} value={c}>
                          {c}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            )}

            {billing === "unl" && (
              <div className="flex h-8 items-center gap-1.5 text-[11.5px] text-mut">
                <InfinityIcon className="h-3.5 w-3.5" />
                No quota config · usage info hidden in lists
              </div>
            )}

            <div className="relative">
              <Label
                className="cursor-pointer select-none text-[11px] font-medium text-mut"
                onClick={() => setAgentsOpen((o) => !o)}
              >
                Bind to agents after saving
              </Label>
              <button
                type="button"
                className="mt-1 flex min-h-8 w-full cursor-pointer flex-wrap items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-left text-[12px] dark:bg-bg"
                onClick={() => setAgentsOpen((o) => !o)}
              >
                {agents.length === 0 ? (
                  <span className="text-mut">Select agents…</span>
                ) : (
                  <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
                    {AGENTS.filter((a) => agents.includes(a.id)).map((a) => (
                      <span
                        key={a.id}
                        className="flex items-center gap-1 rounded bg-surface2 px-1 py-0.5 text-[10.5px]"
                      >
                        <AgentChip meta={a} size={12} />
                        {a.label}
                      </span>
                    ))}
                  </span>
                )}
                <ChevronDown
                  className={`ml-auto h-3.5 w-3.5 flex-none text-mut transition-transform ${agentsOpen ? "rotate-180" : ""}`}
                />
              </button>
              {agentsOpen && (
                <>
                  {/* Click-catcher behind the panel: any press elsewhere in the
                      modal lands here and closes the dropdown */}
                  <div className="fixed inset-0 z-40" onClick={() => setAgentsOpen(false)} />
                  <div className="absolute z-50 mt-1 max-h-[210px] w-full overflow-y-auto rounded-md border border-line bg-bg py-1 shadow-lg">
                  {AGENTS.map((a) => {
                    const active = agents.includes(a.id);
                    return (
                      <button
                        key={a.id}
                        type="button"
                        className="flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-left text-[12px] hover:bg-surface2"
                        onClick={() =>
                          setAgents(active ? agents.filter((x) => x !== a.id) : [...agents, a.id])
                        }
                      >
                        <AgentChip meta={a} size={16} />
                        <span className="min-w-0 flex-1 truncate">{a.label}</span>
                        {active && <Check className="h-3.5 w-3.5 flex-none" style={{ color: "var(--kiwi)" }} />}
                      </button>
                    );
                  })}
                  </div>
                </>
              )}
            </div>

            {/* Rotating keys (edit mode: multiple keys rotate automatically, spec §4.1 P1) */}
            {edit && (
              <div>
                <Label className="text-[11px] font-medium text-mut">
                  Rotating keys{" "}
                  <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                    rotate with the primary key · avoids rate limits
                  </span>
                </Label>
                <div className="mt-1 space-y-1">
                  {pollKeys.map((k) => (
                    <div
                      key={k.id}
                      className="flex h-8 items-center justify-between rounded-md border border-line px-2.5"
                    >
                      <span className="flex items-center gap-2">
                        <span className="font-mono text-[11.5px]">{maskKey(k.api_key)}</span>
                        {k.label && <span className="text-[10.5px] text-mut">{k.label}</span>}
                      </span>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        className="text-mut"
                        aria-label="Delete key"
                        onClick={() => removePollKey(k.id)}
                      >
                        <XIcon />
                      </Button>
                    </div>
                  ))}
                  <div className="flex gap-1.5">
                    <Input
                      className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                      value={newKey}
                      onChange={(e) => setNewKey(e.target.value)}
                      placeholder="sk-… add key"
                    />
                    <Input
                      className="h-8 w-[84px] bg-bg text-[11.5px] dark:bg-bg"
                      value={newKeyLabel}
                      onChange={(e) => setNewKeyLabel(e.target.value)}
                      placeholder="Label"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8 gap-1 px-2.5 text-[11px]"
                      disabled={!newKey.trim() || keyBusy}
                      onClick={addPollKey}
                    >
                      <Plus className="h-3 w-3" />
                      Add
                    </Button>
                  </div>
                </div>
              </div>
            )}

            {/* Plan quota query (edit mode): queries the provider's own plan
                usage endpoint for quota chips. Extra credentials render per
                template; templates without them reuse the primary API key. */}
            {edit && (
              <div>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 w-full justify-between text-[11.5px] text-mut"
                  onClick={() => setPqOpen((o) => !o)}
                >
                  <span className="flex items-center gap-1.5">
                    <Gauge className="h-3 w-3" />
                    Plan quota query
                    {pqTemplate && (
                      <span className="text-[10px]" style={{ color: "var(--kiwi)" }}>
                        {PLAN_QUERY_TEMPLATES.find((t) => t.id === pqTemplate)?.label ?? pqTemplate}
                      </span>
                    )}
                  </span>
                  <ChevronDown className={`h-3.5 w-3.5 transition-transform ${pqOpen ? "rotate-180" : ""}`} />
                </Button>
                {pqOpen && (
                  <div className="mt-2 space-y-2.5">
                    <div>
                      <Label className="text-[10.5px] font-medium text-mut">Template</Label>
                      {/* "none" sentinel: Radix SelectItem rejects empty values */}
                      <Select
                        value={pqTemplate === "" ? "none" : pqTemplate}
                        onValueChange={(v) => setPqTemplate(v === "none" || v == null ? "" : v)}
                      >
                        <SelectTrigger className="mt-1 w-full bg-bg text-[12px] dark:bg-bg">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="none">None</SelectItem>
                          {PLAN_QUERY_TEMPLATES.map((t) => (
                            <SelectItem key={t.id} value={t.id}>
                              {t.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    {(PLAN_QUERY_TEMPLATES.find((t) => t.id === pqTemplate)?.fields ?? []).map((f) => (
                      <div key={f.key}>
                        <Label className="text-[10.5px] font-medium text-mut">{f.label}</Label>
                        <Input
                          className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                          value={pqFields[f.key] ?? ""}
                          onChange={(e) => setPqFields((m) => ({ ...m, [f.key]: e.target.value }))}
                        />
                      </div>
                    ))}
                    <p className="text-[10.5px] text-mut">
                      Queries the plan usage with the provider's API key (5-min cache);
                      quota chips refresh on the Providers page.
                    </p>
                  </div>
                )}
              </div>
            )}

            {/* Advanced forwarding settings (per provider, gateway defaults
                when blank): timeout = time to response headers, never aborts
                an in-flight stream; retries cover failures before any bytes
                reach the client; headers merge over injected credentials */}
            <div>
              <Button
                variant="outline"
                size="sm"
                className="h-8 w-full justify-between text-[11.5px] text-mut"
                onClick={() => setAdvOpen((o) => !o)}
              >
                <span className="flex items-center gap-1.5">
                  <Gauge className="h-3 w-3" />
                  Advanced (timeout / retries / headers)
                </span>
                <ChevronDown
                  className={`h-3.5 w-3.5 transition-transform ${advOpen ? "rotate-180" : ""}`}
                />
              </Button>
              {advOpen && (
                <div className="mt-2 space-y-2.5">
                  <div className="grid grid-cols-2 gap-1.5">
                    <div>
                      <Label className="text-[10.5px] font-medium text-mut">Timeout (s)</Label>
                      <Input
                        type="number"
                        min={1}
                        max={3600}
                        className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                        value={advTimeout}
                        onChange={(e) => setAdvTimeout(e.target.value)}
                        placeholder="10"
                      />
                    </div>
                    <div>
                      <Label className="text-[10.5px] font-medium text-mut">Retries</Label>
                      <Input
                        type="number"
                        min={0}
                        max={5}
                        className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                        value={advRetries}
                        onChange={(e) => setAdvRetries(e.target.value)}
                        placeholder="0"
                      />
                    </div>
                  </div>
                  <p className="text-[10.5px] text-mut">
                    Blank = gateway default · timeout caps time to response headers, never an
                    in-flight stream · retries apply to this provider before failover
                  </p>
                  <div>
                    <Label className="text-[10.5px] font-medium text-mut">
                      Custom headers{" "}
                      <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                        merged last · can override the API key header
                      </span>
                    </Label>
                    <div className="mt-1 space-y-1.5">
                      {advHeaders.map((r, i) => (
                        <div key={i} className="flex items-center gap-1.5">
                          <Input
                            className="h-8 w-[38%] min-w-0 flex-none bg-bg font-mono text-[12px] dark:bg-bg"
                            value={r.name}
                            onChange={(e) => setHeader(i, { name: e.target.value })}
                            placeholder="Header-Name"
                          />
                          <Input
                            className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                            value={r.value}
                            onChange={(e) => setHeader(i, { value: e.target.value })}
                            placeholder="value"
                          />
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            className="flex-none text-mut"
                            aria-label="Remove header"
                            onClick={() => removeHeader(i)}
                          >
                            <XIcon />
                          </Button>
                        </div>
                      ))}
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-7 w-full gap-1 text-[11px] text-mut"
                        onClick={addHeader}
                      >
                        <Plus className="h-3 w-3" />
                        Add header
                      </Button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>

        <DialogFooter className="mx-0 mb-0 flex-row justify-end gap-2 rounded-b-xl border-t border-line bg-transparent px-4 py-3">
          <Button variant="outline" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" className="font-semibold" disabled={!canSave} onClick={save}>
            {saving ? "Saving…" : edit ? "Save" : "Save & enable"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
