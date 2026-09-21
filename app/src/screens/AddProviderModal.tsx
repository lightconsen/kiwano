// Add/edit provider modal (design/index.html #modal, cc-switch AddProviderDialog pattern)
// Base controls use shadcn/ui (Dialog/Input/Label/Select/Button); the segmented pills are kept as design language
//
// The container: it owns the form's state, the eight effects behind it and the
// save, and composes the pieces that live in `screens/AddProviderModal/`.
import { useEffect, useState } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api } from "../api/client";
import { useT } from "../i18n";
import {
  PLAN_QUERY_TEMPLATES,
  type AgentRef,
  type ApiKeyEntry,
  type Billing,
  type CatalogBilling,
  type CatalogEntry,
  type DeclaredPrice,
  type ProbeReport,
  type Protocol,
  type Provider,
} from "../api/types";
import { useHubUrl } from "../lib/hub";
import { AdvancedSection } from "./AddProviderModal/advanced";
import { AgentBinding } from "./AddProviderModal/binding";
import { BillingSection } from "./AddProviderModal/billing";
import { EndpointsSection } from "./AddProviderModal/endpoints";
import { Footer } from "./AddProviderModal/footer";
import { IdentitySection } from "./AddProviderModal/identity";
import { RotatingKeys } from "./AddProviderModal/keys";
import {
  invalidPct,
  PaygLimit,
  PlanCeilings,
  PlanQuotaNotice,
  UnlimitedNote,
} from "./AddProviderModal/limits";
import { CatalogPicker, EntryChip, ModeSwitch } from "./AddProviderModal/mode";
import { ModelSection } from "./AddProviderModal/model";
import { invalidRate, PricesSection, RATE_FIELDS } from "./AddProviderModal/prices";

export default function AddProviderModal({
  open,
  preset,
  edit,
  preferredCurrency = "USD",
  onClose,
  onSaved,
}: {
  open: boolean;
  preset: CatalogEntry | null;
  edit: Provider | null;
  /** The user's display currency, which seeds a new spending limit's. */
  preferredCurrency?: string;
  onClose: () => void;
  onSaved: () => void;
}) {
  const hubUrl = useHubUrl();
  const t = useT();
  // Billing labels: resolved here so a language change re-renders them along
  // with the rest of the form.
  const billOptions: { id: Billing; label: string }[] = [
    { id: "plan", label: t("addProvider.billingPlan") },
    { id: "payg", label: t("addProvider.billingPayg") },
    { id: "unl", label: t("addProvider.billingUnlimited") },
  ];
  const [mode, setMode] = useState<"shelf" | "custom">("shelf");
  // Catalog entry chosen in the "From Models" mode (preset pre-seeds it when
  // the modal opens from the Models page; otherwise picked in-modal)
  const [shelf, setShelf] = useState<CatalogEntry | null>(null);
  const [catalog, setCatalog] = useState<CatalogEntry[] | null>(null);
  /** The catalog entry the provider being edited came from, when it is still
      published. It is the authority on two things the stored row can have lost
      — see the effect that loads it. */
  const [editEntry, setEditEntry] = useState<CatalogEntry | null>(null);
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
  // `CatalogBilling`, not `Billing`: a shelf entry can say `both`, which the
  // user settles here (see `bothEntry`). Only the settled value is ever saved.
  const [billing, setBilling] = useState<CatalogBilling>("payg");
  // Payg spending-limit number (plan providers use the percent pair below)
  const [limitValue, setLimitValue] = useState("");
  // The spending limit is denominated in the currency its provider bills in —
  // declared by the catalog entry, never picked here: a limit the user cannot
  // reconcile with the prices it is compared against fires at the wrong time.
  // Entries published before the field existed fall back to USD; editing a
  // provider keeps the currency its limit was saved with, so re-labelling it
  // is a deliberate move rather than a side effect of opening the modal.
  // …and to the catalog entry, because the stored unit is only half the story:
  // `normalize_limit_unit` drops it when the limit is left blank, so a
  // pay-as-you-go provider with no spending limit has no currency on its row at
  // all and fell through to USD — the dialog contradicting the catalog it was
  // added from, and the provider's own prices with it.
  const savedCurrency =
    edit?.limit_unit && edit.limit_unit.length === 3 ? edit.limit_unit : null;
  // Held, not derived: a picker that the user can use has to. Seeded in the order
  // the answers are trustworthy — the row's own unit (set only while it has a
  // limit), the entry's currency (what its published prices are in), and the
  // user's display currency, which is all a provider with no entry has to go on.
  const [limitCurrency, setLimitCurrency] = useState(
    savedCurrency ?? editEntry?.currency ?? shelf?.currency ?? preferredCurrency,
  );
  // What the limit can be denominated in: the Hub's rate table, because that is
  // what makes the limit comparable to the costs it is measured against, plus
  // whatever the row already says — a currency the table has since dropped must
  // not vanish from the picker and silently re-denominate the limit.
  const [currencies, setCurrencies] = useState<string[]>([]);
  const [agents, setAgents] = useState<AgentRef[]>([]);
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
  const [pqTemplate, setPqTemplate] = useState("");
  const [pqFields, setPqFields] = useState<Record<string, string>>({});
  // Plan-mode percent limits: per-window utilization ceilings over the
  // vendor's rolling 5-hour / weekly windows (blank = no limit on it).
  const [planFiveHour, setPlanFiveHour] = useState("");
  const [planWeekly, setPlanWeekly] = useState("");
  // What this provider charges, per million tokens in the currency above — the
  // figures the gateway costs its requests with. Held as the form wrote them
  // (TEXT decimals, blank boxes included) so a half-typed row survives a
  // re-render; the blanks become zeros on the way out, which is the backend's
  // own reading of a rate nobody stated.
  const [prices, setPrices] = useState<DeclaredPrice[]>([]);

  /** Whether the form is bound to a catalog entry — Add-from-Models before the
      user switches to Custom.
   *
   * The entry declared the protocol, the endpoints and the currency its prices are
   * in, so those three are read-only while it is in play and the user's own to set
   * otherwise: a provider typed in by hand has nobody else to declare them, and an
   * existing row is the user's own already. The coverage badges above the endpoint
   * list are derived from its rows, so they follow whatever is picked. */
  const fromEntry = !edit && mode === "shelf" && !!shelf;

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
    // An entry prices what it publishes, so whatever the user had declared for
    // some other provider is not this provider's prices — and leaving them would
    // show them again if the form came back to Custom in the same session.
    setPrices([]);
    setLimitValue(e.billing === "payg" ? "50" : "");
    // Its prices are published in this, so the limit has to be too. An entry that
    // publishes none (the field is younger than some of them) leaves the user's own
    // currency, which is what the initial seed would have given it.
    setLimitCurrency(e.currency ?? preferredCurrency);
    setPlanFiveHour("");
    setPlanWeekly("");
    setAgents(e.id === "deepseek" ? ["claude", "codex"] : []);
    setFetchedModels(null);
    setFetchError(null);
    // The entry's own quota query, when it publishes one: the ceiling fields are
    // gated on it, so this is what opens them for the four providers whose
    // vendors can be asked how much of the plan is spent.
    setPqTemplate(e.plan_query?.template ?? "");
    setPqFields({});
    resetAdvanced();
  };

  // Default model options: a fetched live list wins over the catalog's
  // (primary models ∪ each additional endpoint's models)
  const shelfModelOptions = shelf
    ? Array.from(new Set([...shelf.models, ...(shelf.endpoints ?? []).flatMap((x) => x.models ?? [])]))
    : [];
  const modelOptions = fetchedModels ?? shelfModelOptions;
  const showModelSelect = (fetchedModels?.length ?? 0) > 0 || (!edit && mode === "shelf" && !!shelf);

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
    // Blank is only a blocker when there is nothing to fall back on: the edit
    // form never holds the stored key, and the backend uses it when the endpoint
    // is one this provider already answers on.
    if (!apiKey.trim() && !edit) {
      setFetchError(t("addProvider.errorNoKey"));
      return;
    }
    setFetching(true);
    setFetchError(null);
    try {
      const list = await api.listModels(protocol, endpoint.trim(), apiKey.trim(), edit?.id);
      if (list.length === 0) {
        setFetchError(t("addProvider.errorNoModels"));
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
      const report = await api.testEndpoint(
        row.protocol,
        row.endpoint.trim(),
        apiKey.trim() || undefined,
        edit?.id,
      );
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
      // What the add form collected and used to throw away: persisted since v14,
      // so an edit shows the model that was chosen rather than an empty box.
      setModel(edit.model_default ?? "");
      setBilling(edit.billing);
      const q = edit.usage?.quota;
      // Payg spending limit (plan providers now carry percent limits instead)
      setLimitValue(edit.billing === "payg" && q ? String(q.limit) : "");
      setLimitCurrency(savedCurrency ?? preferredCurrency);
      setPlanFiveHour(edit.plan_limits?.five_hour != null ? String(edit.plan_limits.five_hour) : "");
      setPlanWeekly(edit.plan_limits?.weekly != null ? String(edit.plan_limits.weekly) : "");
      // What the row declared, back into the boxes it came from. A null blob is
      // read as no rows rather than an error: it is what a provider priced by
      // the Hub carries, which is almost all of them.
      setPrices((edit.prices?.models ?? []).map((m) => ({ ...m })));
      setAgents([...edit.agents]);
      const pq = edit.plan_query;
      setPqTemplate(pq?.template ?? "");
      setPqFields(pq?.fields ? { ...pq.fields } : {});
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
    // The currency is *not* seeded in the initial state: this component stays
    // mounted while the dialog is closed, so that would read the user's currency
    // before the app has loaded settings and keep USD for the session. Seeded per
    // open instead — and again by `applyShelf` below, which states the entry's own.
    setLimitCurrency(preset?.currency ?? preferredCurrency);
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
      setPrices([]);
      setPqTemplate("");
      setPqFields({});
        setPlanFiveHour("");
      setPlanWeekly("");
      setAgents([]);
    }
  }, [open, preset, edit]);

  // The currencies the limit can be denominated in, read once per open. A failure
  // leaves the list empty and the picker showing only the currency in force, which
  // is what it did before there was a picker at all.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    api
      .getCurrencyMeta()
      .then((m) => {
        if (alive) setCurrencies(m.currencies);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [open]);

  // Catalog loads lazily, the first time the in-modal picker is shown
  useEffect(() => {
    if (open && !edit && mode === "shelf" && !shelf && catalog === null) {
      api.listCatalog().then((c) => setCatalog(c.entries));
    }
  }, [open, edit, mode, shelf, catalog]);

  /// The catalog entry an edited provider was added from, when the Hub still
  /// publishes it.
  ///
  /// Two things the stored row can be missing live here. **Endpoints**: a
  /// provider added before the shelf's endpoints were carried through answers on
  /// one protocol while its entry declares two, and nothing else can tell the
  /// dialog what the second one is. **Currency**: it is discarded whenever the
  /// spending limit is left blank (`normalize_limit_unit` drops the unit with
  /// the limit), which left a pay-as-you-go provider reading as USD in the
  /// dialog whatever the catalog says it bills in.
  useEffect(() => {
    if (!open || !edit?.catalog_id) {
      setEditEntry(null);
      return;
    }
    let alive = true;
    api
      .listCatalog()
      .then((c) => {
        if (alive) setEditEntry(c.entries.find((e) => e.id === edit.catalog_id) ?? null);
      })
      .catch(() => {
        if (alive) setEditEntry(null);
      });
    return () => {
      alive = false;
    };
  }, [open, edit]);

  // …and the quota query, on the same rule. A plan provider whose row has none
  // still has one if its entry publishes it: Kimi for Coding is exactly that —
  // added before the app carried the field through, so its row says nothing and
  // the dialog reported it as a vendor with no quota endpoint at all. Only
  // fills a blank; a template the row does carry is left alone.
  useEffect(() => {
    const template = editEntry?.plan_query?.template;
    if (template) setPqTemplate((current) => current || template);
  }, [editEntry]);

  // …and the currency with it: the row keeps one only while it has a limit, so a
  // pay-as-you-go provider whose limit is blank has none to read and its entry is
  // what says which currency its prices are in.
  useEffect(() => {
    if (editEntry?.currency) setLimitCurrency(editEntry.currency);
  }, [editEntry]);

  // Fill in the endpoints the stored row is missing, once the entry lands.
  // Union, not replace: a protocol the provider already answers on keeps the
  // endpoint it was saved with, and only protocols it lacks are added — so the
  // list reads the same whether the row predates the field or not. The rows are
  // read-only in every mode, so nothing the user typed can be lost underneath
  // them; saving is what writes them to the provider.
  useEffect(() => {
    if (!editEntry) return;
    setAltEndpoints((rows) => {
      const have = new Set<Protocol>([protocol, ...rows.map((r) => r.protocol)]);
      const missing = (editEntry.endpoints ?? []).filter((e) => !have.has(e.protocol));
      return missing.length === 0
        ? rows
        : [...rows, ...missing.map((e) => ({ protocol: e.protocol, endpoint: e.endpoint }))];
    });
  }, [editEntry, protocol]);

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

  const badPrice = prices.some((row) =>
    RATE_FIELDS.some((f) => invalidRate(row[f.key])),
  );

  // An invalid ceiling is refused rather than dropped: see `invalidPct`.
  const canSave =
    name.trim() !== "" &&
    endpoint.trim() !== "" &&
    // `both` is a question, not a mode: the row cannot hold it (the backend
    // refuses it by name), so the form does not offer to save it.
    billing !== "both" &&
    !invalidPct(planFiveHour) &&
    !invalidPct(planWeekly) &&
    !badPrice &&
    !saving;

  // Billing is intrinsic to the provider: locked to the catalog entry when
  // adding from Models, and to the stored value when editing. Vendors that
  // support several modes get one catalog entry per mode. Custom providers
  // (no catalog entry behind the form) pick freely at creation.
  // A Hub catalog row may carry a billing tag this build predates: keep it
  // visible instead of silently rewriting it to payg, and leave the picker
  // open — the backend rejects an unknown tag on save, so locking the form
  // would dead-end the entry.
  // `both` counts as known — it has a label and copy of its own — while staying
  // out of `billOptions`, which is what the pills are built from and what the
  // lock below keys on. Putting it in there is the trap: the pills would lock
  // into a read-only label saying `both`, and saving would send it.
  const billingKnown = billing === "both" || billOptions.some((b) => b.id === billing);
  /** Whether the declared-price section is offered — and whether the save
   * speaks about prices at all.
   *
   * Pay-as-you-go only: the other two modes have no per-token charge for a
   * price to describe. And only while the form is the user's own, which is the
   * same rule the protocol, the endpoints and the currency follow: a catalog
   * entry prices the models it publishes, so there is nothing to declare while
   * it owns the form. */
  const priceRowsShown = billing === "payg" && !fromEntry;
  /** A catalog entry that charges both ways at one address — the one case where
      the billing mode is the user's to pick, because the catalog names two
      arrangements for one host and cannot know which this provider is.
   *
   * It stays the user's after the fact as well: editing such a provider keeps the
   * pills live. Refusing there would leave "delete it and re-enter the key" as
   * the only way to change one's mind, which is a stiff price for a setting. Any
   * other provider keeps the lock — its entry decides, and one entry per mode is
   * how a vendor with several modes is meant to be published. */
  const bothEntry = (edit ? editEntry?.billing : shelf?.billing) === "both";
  const unresolvedBoth = bothEntry && billing === "both";
  const billingLocked =
    (!!edit || (mode === "shelf" && !!shelf)) && billingKnown && !bothEntry;

  // Whether the percent ceilings are worth offering, which is a question about
  // the vendor's API and not about how it charges. The fields are compared
  // against utilization the provider's own endpoint reports, so without one to
  // ask they would set a ceiling nothing could ever measure — and the Providers
  // page's ring reads a missing report as 0%, i.e. a permanent green zero rather
  // than "unknown".
  //
  // Adding from the shelf: only the catalog says whether an endpoint exists.
  // Editing: whatever template is selected, which is also the escape hatch for a
  // custom provider — pick one and the fields appear.
  const canQueryQuota = edit ? pqTemplate !== "" : !!shelf?.plan_query?.template;

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
                //
                // Gated on the same condition that renders the fields, because a
                // ceiling on a provider nothing can measure is worse than none:
                // the Providers ring reads a missing report as 0%, so the user
                // gets a comfortable green zero while the plan empties. Reachable
                // by clearing the template on a provider that had one — the
                // fields unmount, the numbers stay in state, and saving would
                // keep a ceiling with nothing behind it. Both blank → the backend
                // stores NULL, which is the honest state.
                plan_limits: {
                  five_hour:
                    canQueryQuota && planFiveHour.trim() ? Number(planFiveHour) : undefined,
                  weekly: canQueryQuota && planWeekly.trim() ? Number(planWeekly) : undefined,
                },
              }
            : {
                limit_value: limitValue ? Number(limitValue) : undefined,
                limit_unit: billing === "payg" ? limitCurrency : undefined,
              },
        // What this provider charges, in the currency the limit is in. Absent
        // while the section is hidden — the form is then saying nothing about
        // prices, and the stored ones stand; an empty list is it saying "none",
        // which clears them. That asymmetry is the whole reason the field is
        // sent as `undefined` rather than as an empty bundle.
        prices: priceRowsShown
          ? {
              currency: limitCurrency,
              models: prices
                .filter((row) => row.model_id.trim() !== "")
                .map((row) => ({
                  model_id: row.model_id.trim(),
                  input: row.input.trim() || "0",
                  output: row.output.trim() || "0",
                  cache_read: row.cache_read.trim() || "0",
                  cache_creation: row.cache_creation.trim() || "0",
                })),
            }
          : undefined,
        // Add only — see the binding control above: an edit leaves the bindings
        // to the agent tabs rather than re-promoting itself through them.
        ...(edit ? {} : { agents }),
        endpoints: altInputs,
        advanced: {
          timeout_secs: advTimeout ? Number(advTimeout) : null,
          retries: advRetries ? Number(advRetries) : null,
          headers: Object.keys(headerMap).length > 0 ? headerMap : undefined,
        },
        // Edit: "" = clear the config, a template = authoritative snapshot.
        // Shelf: carry the entry's own query, the same shape with no fields —
        // credentials are the user's, and the fields UI only exists in edit mode.
        // Without this the ceilings the form just offered would be stored against
        // a provider nothing can measure.
        plan_query: edit
          ? pqTemplate
            ? { template: pqTemplate, fields: pqFieldInputs() }
            : null
          : mode === "shelf" && shelf?.plan_query?.template
            ? { template: shelf.plan_query.template }
            : undefined,
        // Add from the shelf only: which catalog entry this is. The gateway
        // prices a request by catalog entry, so without it a shelf-added
        // provider is costed at the general rate rather than its own — and the
        // local id cannot stand in, since it is `<slug>-<hex>`.
        // Absent = no catalog entry (custom), and the backend keeps whatever is
        // stored when the field is missing, so edits never unlink it.
        catalog_id: !edit && mode === "shelf" && shelf ? shelf.id : undefined,
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
            {edit ? t("addProvider.titleEdit") : t("addProvider.titleAdd")}
          </DialogTitle>
        </DialogHeader>

        <div className="px-4 py-3.5">
          {/* Mode switch (hidden in edit mode: no models involved) */}
          {!edit && (
            <ModeSwitch mode={mode} shelf={shelf} setMode={setMode} />
          )}

          {/* In-modal catalog picker (shelf mode without a chosen entry) */}
          {!edit && mode === "shelf" && !shelf && (
            <CatalogPicker
              query={query}
              setQuery={setQuery}
              catalog={catalog}
              hubUrl={hubUrl}
              setShelf={setShelf}
              applyShelf={applyShelf}
            />
          )}

          {/* Chosen catalog entry (with a way back to the picker) */}
          {!edit && mode === "shelf" && shelf && (
            <EntryChip shelf={shelf} hubUrl={hubUrl} setShelf={setShelf} />
          )}

          <div className="mt-3 space-y-3">
            <IdentitySection
              name={name}
              setName={setName}
              apiKey={apiKey}
              setApiKey={setApiKey}
              showKey={showKey}
              setShowKey={setShowKey}
              edit={edit}
            />

            {/* Endpoint URLs, one row per protocol: protocol + URL + Test. The
                first row is the primary endpoint, the rest serve agents speaking
                another protocol natively (same API key pool). A catalog entry
                brings its own list and it is read-only while that entry owns the
                form; a provider typed in by hand — and an existing row, which is
                the user's own already — can add and drop them.

                There used to be a "Protocol" block above this one: two badges,
                one per protocol, lit for the ones the rows cover. It was the
                rows' own union drawn a second time — its comment already said the
                per-endpoint rows were authoritative — so it is gone, and each row
                carries its protocol itself. */}
            <EndpointsSection
              protocol={protocol}
              setProtocol={setProtocol}
              endpoint={endpoint}
              setEndpoint={setEndpoint}
              altEndpoints={altEndpoints}
              setAltEndpoints={setAltEndpoints}
              altProbes={altProbes}
              setAltProbes={setAltProbes}
              altTesting={altTesting}
              probe={probe}
              setProbe={setProbe}
              testing={testing}
              setTesting={setTesting}
              setFetchedModels={setFetchedModels}
              setFetchError={setFetchError}
              apiKey={apiKey}
              edit={edit}
              fromEntry={fromEntry}
              mode={mode}
              shelf={shelf}
              testAlt={testAlt}
            />

            <ModelSection
              model={model}
              setModel={setModel}
              modelOpen={modelOpen}
              setModelOpen={setModelOpen}
              modelOptions={modelOptions}
              showModelSelect={showModelSelect}
              fetching={fetching}
              fetchError={fetchError}
              endpoint={endpoint}
              fetchModels={fetchModels}
            />

            <BillingSection
              billing={billing}
              setBilling={setBilling}
              billOptions={billOptions}
              billingKnown={billingKnown}
              billingLocked={billingLocked}
              bothEntry={bothEntry}
              unresolvedBoth={unresolvedBoth}
              edit={edit}
            />

            {billing === "plan" && !canQueryQuota && (
              <PlanQuotaNotice />
            )}

            {billing === "plan" && canQueryQuota && (
              <PlanCeilings
                planFiveHour={planFiveHour}
                setPlanFiveHour={setPlanFiveHour}
                planWeekly={planWeekly}
                setPlanWeekly={setPlanWeekly}
              />
            )}

            {billing === "payg" && (
              <PaygLimit
                limitValue={limitValue}
                setLimitValue={setLimitValue}
                limitCurrency={limitCurrency}
                setLimitCurrency={setLimitCurrency}
                currencies={currencies}
                fromEntry={fromEntry}
              />
            )}

            {/* What this provider charges, for a pay-as-you-go provider with no
                catalog entry behind it: the Hub prices its own entries' models,
                so without these figures a hand-added provider is costed at
                whatever the general table happens to say about the model — or
                recorded unpriced, which leaves its spending limit with nothing
                to measure. Two lines per model: the id, then its four rates. */}
            {priceRowsShown && (
              <PricesSection
                prices={prices}
                setPrices={setPrices}
                model={model}
                limitCurrency={limitCurrency}
                badPrice={badPrice}
              />
            )}

            {billing === "unl" && (
              <UnlimitedNote />
            )}

            {/* Adding only. Which providers serve an agent is the Apps screen's
                job — its agent tabs already bind, unbind and set the strategy —
                and a save here rebinds as a side effect: every agent in the set
                is re-promoted to this provider as primary and has its strategy
                flattened to Single. Saving an unrelated change must not do that,
                so an edit neither shows the control nor sends the set. */}
            {!edit && (
              <AgentBinding
                agents={agents}
                setAgents={setAgents}
                agentsOpen={agentsOpen}
                setAgentsOpen={setAgentsOpen}
              />
            )}

            {/* Rotating keys (edit mode: multiple keys rotate automatically, spec §4.1 P1) */}
            {edit && (
              <RotatingKeys
                pollKeys={pollKeys}
                newKey={newKey}
                setNewKey={setNewKey}
                newKeyLabel={newKeyLabel}
                setNewKeyLabel={setNewKeyLabel}
                keyBusy={keyBusy}
                addPollKey={addPollKey}
                removePollKey={removePollKey}
              />
            )}


            {/* Advanced forwarding settings (per provider, gateway defaults
                when blank): timeout = time to response headers, never aborts
                an in-flight stream; retries cover failures before any bytes
                reach the client; headers merge over injected credentials */}
            <AdvancedSection
              advOpen={advOpen}
              setAdvOpen={setAdvOpen}
              advTimeout={advTimeout}
              setAdvTimeout={setAdvTimeout}
              advRetries={advRetries}
              setAdvRetries={setAdvRetries}
              advHeaders={advHeaders}
              setAdvHeaders={setAdvHeaders}
            />
          </div>
        </div>

        <Footer onClose={onClose} save={save} canSave={canSave} saving={saving} edit={edit} />
      </DialogContent>
    </Dialog>
  );
}
