// Home screen: Apps (local provider list, design/index.html #s-providers)
import { useCallback, useEffect, useMemo, useState, type FocusEvent, type ReactNode } from "react";

import {
  ArrowDown,
  ArrowUp,
  Check,
  Copy,
  Gauge,
  SquarePen,
  Pin,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Settings2,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
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
import { agentMeta, isBuiltinAgent, rememberCustomAgents } from "../lib/agents";
import { useT, type KeyPath, type Messages, type Translate } from "../i18n";
import {
  AGENTS,
  PLAN_TIER_LABEL_KEYS,
  type AgentDetect,
  type AgentLimit,
  type AgentRef,
  type CustomAgent,
  type AgentRoute,
  type PlanQuotaReport,
  type Provider,
  type StrategyBinding,
} from "../api/types";
import { AgentChip, BillTag, Dot, Logo, Ring, Sparkline } from "../components/bits";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { iconForEndpoint } from "@/components/icons/infer";
import StrategyPanel, { CopyRouteRow } from "../components/StrategyPanel";
import { LimitSection } from "../components/AgentLimit";
import { fmtLatency, fmtMoney, fmtTokens } from "../lib/format";

// Agent filter segments — each renders the agent's brand logo (ported with
// the cc-switch icon set, see components/icons). Hover shows the full name.
const SEGMENTS: { id: AgentRef | "all"; icon?: string; label?: string }[] = [
  { id: "all" },
  { id: "claude", icon: "claudecode" },
  { id: "codex", icon: "openai" },
  { id: "grokbuild", icon: "grok" },
  { id: "claude-desktop", icon: "claude" },
  { id: "opencode", icon: "opencode" },
  { id: "openclaw", icon: "openclaw" },
  { id: "hermes", icon: "hermes" },
  { id: "pi", icon: "pi" },
];

// Agent labels are brand names and stay as they are; only the "all" segment
// has a translatable label.
function segmentLabel(t: Translate, id: AgentRef | "all"): string {
  if (id === "all") return t("providers.all");
  // The registry, the user's own agents, or — for an id nothing knows — the id.
  return agentMeta(id).label;
}

const SEGMENT_ICON: Partial<Record<AgentRef, string>> = Object.fromEntries(
  SEGMENTS.filter((s) => s.icon).map((s) => [s.id, s.icon!]),
);

/** First-touch state for an agent tab with no bound providers: one click to
    back up + take over (importing the provider the agent already uses), or
    manual entry. When already taken over but unbound, steer to binding an
    existing provider (bindSlot) or manual add. */
function AgentOnboarding({
  agent,
  installed,
  takenOver,
  kind = "takeover",
  busy,
  onTakeover,
  onAdd,
  bindSlot,
  copySlot,
}: {
  /** "takeover": a built-in agent Kiwano has not taken over yet.
      "route": a user-defined agent with no candidates — there is nothing to
      take over, so the panel is only about assigning one. */
  kind?: "takeover" | "route";
  agent: AgentRef;
  installed: boolean;
  takenOver: boolean;
  busy: boolean;
  onTakeover: () => void;
  onAdd: () => void;
  /** Extra first-candidate entry (bind an existing provider) for the taken-over branch */
  bindSlot?: ReactNode;
  /** Copy another agent's whole route — available right after Enable, before any binding exists */
  copySlot?: ReactNode;
}) {
  const t = useT();
  const meta = agentMeta(agent);
  const routeOnly = kind === "route";
  return (
    <div className="flex flex-col items-center gap-3 px-4 py-10 text-center">
      <ProviderLogo
        icon={SEGMENT_ICON[agent]}
        char={meta.chip_char}
        color={meta.chip_color}
        name={meta.label}
        size={36}
      />
      {routeOnly ? (
        <>
          <div className="text-[13px] font-semibold">
            {t("providers.routeEmptyTitle", { agent: meta.label })}
          </div>
          <div className="max-w-[460px] text-[12px] text-mut">{t("providers.routeEmptyBody")}</div>
          <div className="flex items-center gap-2">
            {bindSlot}
            <Button size="sm" className="h-7 gap-1 px-3 text-[12px] font-semibold" onClick={onAdd}>
              <Plus className="h-3.5 w-3.5" />
              {t("providers.addProvider")}
            </Button>
          </div>
          {copySlot}
        </>
      ) : takenOver ? (
        <>
          <div className="text-[13px] font-semibold">
            {t("providers.takenOverTitle", { agent: meta.label })}
          </div>
          <div className="max-w-[430px] text-[12px] text-mut">{t("providers.takenOverBody")}</div>
          <div className="flex items-center gap-2">
            {bindSlot}
            <Button size="sm" className="h-7 gap-1 px-3 text-[12px] font-semibold" onClick={onAdd}>
              <Plus className="h-3.5 w-3.5" />
              {t("providers.addProvider")}
            </Button>
          </div>
          {copySlot}
        </>
      ) : (
        <>
          <div className="text-[13px] font-semibold">
            {t("providers.startManaging", { agent: meta.label })}
          </div>
          <div className="max-w-[460px] text-[12px] text-mut">
            {t("providers.startManagingBody", { agent: meta.label })}
          </div>
          <div className="flex gap-2">
            <Button
              size="sm"
              className="h-7 px-3 text-[12px] font-semibold"
              disabled={busy}
              onClick={onTakeover}
            >
              {busy ? t("providers.enabling") : t("providers.enableKiwano")}
            </Button>
            <Button variant="ghost" size="sm" className="h-7 px-3 text-[12px]" onClick={onAdd}>
              {t("providers.addProviderFirst")}
            </Button>
          </div>
          {installed && (
            <div className="max-w-[470px] text-[11px] text-mut">{t("providers.oauthNote")}</div>
          )}
        </>
      )}
    </div>
  );
}

/** The two values a client is configured with: where the gateway is, and this
    agent's key. They are the whole of what a user-defined agent *is* from the
    outside, so they get a dialog of their own, opened from the icon beside the
    agent's name — a tab that is a route does not need a permanent card of
    credentials on top of it. */
function AccessDialog({
  agent,
  label,
  keyName,
  listen,
  limit,
  currency,
  open,
  onClose,
  onDelete,
  onChanged,
}: {
  agent: AgentRef;
  label: string;
  keyName: string | null;
  listen: string;
  limit: AgentLimit | null;
  currency: string;
  open: boolean;
  onClose: () => void;
  onDelete: () => void;
  onChanged?: () => void;
}) {
  const t = useT();
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[460px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">
            {t("providers.accessFor", { agent: label })}
          </DialogTitle>
        </DialogHeader>
        <div className="px-5 pb-3 pt-1">
          <CopyRow label={t("providers.accessEndpoint")} value={`http://${listen}`} />
          <CopyRow label={t("providers.accessKey")} value={keyName ?? "—"} />
          <p className="mt-2 text-[11px] leading-relaxed text-mut">{t("providers.accessNote")}</p>
          <div className="mt-3 border-t border-line pt-2">
            {/* This dialog has no tabs, so the section is named here — the same
                words the other dialog puts on the tab that shows it. */}
            <div className="text-[10.5px] text-mut">{t("strategy.limitLabel")}</div>
            <LimitSection
              agent={agent}
              limit={limit}
              currency={currency}
              onChanged={onChanged}
            />
          </div>
        </div>
        {/* Deleting lives with the rest of what this agent *is*, and one step
            further from the pointer than the tab's own rows. */}
        <div className="flex items-center gap-3 border-t border-line px-5 py-3">
          <DeleteAgentButton label={label} onConfirm={onDelete} />
          <span className="min-w-0 flex-1 text-[10.5px] text-mut">
            {t("providers.deleteAgentNote")}
          </span>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** One built-in agent's settings: whether Kiwano routes it, and the files a
    takeover rewrites.
 *
 * The switch and the file list belong together because they are the same
 * question from two sides — turning the takeover off is what puts those files
 * back. Read-only apart from that switch: the config is the agent's, and this
 * dialog is not an editor for someone else's format.
 *
 * A user-defined agent has no such dialog: there is no file behind it, and its
 * own row already carries what it does have (its key, and deleting it). */
/** The subjects of one agent's settings, switched between rather than stacked:
    what the agent is here (the files a takeover rewrites), and how much it may
    spend. The same segment group the Dashboard's window and the shelf's view use —
    both fit on a row, and naming the other reading without opening it is the
    point. */
function SettingsTabs({
  tab,
  onPick,
}: {
  tab: AgentSettingsTab;
  onPick: (t: AgentSettingsTab) => void;
}) {
  const t = useT();
  const tabs: { id: AgentSettingsTab; labelKey: KeyPath<Messages> }[] = [
    { id: "general", labelKey: "providers.agentGeneralTab" },
    { id: "limit", labelKey: "strategy.limitLabel" },
  ];
  return (
    <div className="mt-1 flex overflow-hidden rounded-lg border border-line text-[12px]">
      {tabs.map((x, i) => (
        <button
          key={x.id}
          className={`seg h-7 flex-1 border-line px-3 text-mut${i > 0 ? " border-l" : ""}${tab === x.id ? " active" : ""}`}
          onClick={() => onPick(x.id)}
        >
          {t(x.labelKey)}
        </button>
      ))}
    </div>
  );
}

type AgentSettingsTab = "general" | "limit";

function AgentSettingsDialog({
  agent,
  label,
  paths,
  limit,
  currency,
  open,
  onClose,
  onChanged,
}: {
  agent: AgentRef;
  label: string;
  paths: string[];
  limit: AgentLimit | null;
  /** Display currency, for a money ceiling's unit. */
  currency: string;
  open: boolean;
  onClose: () => void;
  onChanged?: () => void;
}) {
  const t = useT();
  const [tab, setTab] = useState<AgentSettingsTab>("general");
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[460px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">
            {t("providers.agentSettingsFor", { agent: label })}
          </DialogTitle>
        </DialogHeader>
        <div className="px-5 pb-4 pt-1">
          <SettingsTabs tab={tab} onPick={setTab} />

          <div className="mt-2.5">
            {tab === "general" && (
              <>
                {paths.map((p) => (
                  <div key={p} className="mt-1 flex items-center gap-2">
                    <CopyValue value={p} />
                  </div>
                ))}
                <p className="mt-1.5 text-[10.5px] leading-relaxed text-mut">
                  {t("providers.agentConfigFilesNote")}
                </p>
              </>
            )}
            {tab === "limit" && (
              <LimitSection
                agent={agent}
                limit={limit}
                currency={currency}
                onChanged={onChanged}
              />
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** A value and the button that copies it. The button is an icon: these are short
    values, and a word beside each of them competes with the thing being copied. */
function CopyValue({ value }: { value: string }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    // No clipboard in a browser without a user gesture (or in a test): a copy
    // that cannot happen must not take the row down with it.
    navigator.clipboard?.writeText(value).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1200);
      },
      () => {},
    );
  };
  return (
    <>
      <code className="min-w-0 flex-1 truncate font-mono text-[11px]">{value}</code>
      <Button
        variant="ghost"
        size="sm"
        className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
        aria-label={copied ? t("common.copied") : t("common.copy")}
        title={copied ? t("common.copied") : t("common.copy")}
        onClick={copy}
      >
        {/* The tick is the confirmation: no layout shift, and the row keeps its
            width whether or not it was just used. */}
        {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
      </Button>
    </>
  );
}

/** One line of the access card: a labelled value, with the copy button. */
function CopyRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-1.5 flex items-center gap-2">
      <span className="w-[68px] shrink-0 text-[10.5px] text-mut">{label}</span>
      <CopyValue value={value} />
    </div>
  );
}

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
 * It reports on the button rather than in the Status cell on purpose: it is a
 * measurement the reader just asked for — a real completion, so a slow model
 * reads slow — and the cell is for standing facts (parked, over a limit). A
 * number that only exists until the next refresh belongs where it was asked for.
 */
function TestLatencyButton({ provider }: { provider: Provider }) {
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

/** Delete a user-defined agent: two steps, like a provider row — the first
    click arms it, and a stray one cannot take a route away. */
function DeleteAgentButton({ label, onConfirm }: { label: string; onConfirm: () => void }) {
  const t = useT();
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), 3000);
    return () => window.clearTimeout(timer);
  }, [armed]);
  return (
    <Button
      variant="ghost"
      size="sm"
      className="h-7 shrink-0 gap-1 border border-line px-2 text-[10.5px] text-mut hover:text-ink"
      style={armed ? { color: "var(--red)", borderColor: "var(--red)" } : undefined}
      title={armed ? t("providers.deleteAgentConfirm") : t("providers.deleteAgentNote")}
      aria-label={t("providers.deleteAgent")}
      onClick={() => (armed ? onConfirm() : setArmed(true))}
    >
      <Trash2 className="h-3.5 w-3.5" />
      {armed ? t("providers.deleteAgentConfirm") : `${t("providers.deleteAgent")} ${label}`}
    </Button>
  );
}

/** Make a user-defined agent: a name, an optional note, and that is all the
    form asks for — the id is derived, and the candidates are bound in the tab
    it opens. */
function NewAgentDialog({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const t = useT();
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const create = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    setErr(null);
    try {
      const created = await api.addCustomAgent(name, note);
      setName("");
      setNote("");
      onCreated(created.id);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[380px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">{t("providers.newAgent")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3 px-5 pb-4 pt-1">
          <div>
            <Label className="text-[11px] font-medium text-mut">{t("providers.agentName")}</Label>
            <Input
              autoFocus
              aria-label={t("providers.agentName")}
              className="mt-1 h-8 text-[12px]"
              value={name}
              placeholder={t("providers.agentNamePlaceholder")}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && create()}
            />
          </div>
          <div>
            <Label className="text-[11px] font-medium text-mut">{t("providers.agentNote")}</Label>
            <Input
              aria-label={t("providers.agentNote")}
              className="mt-1 h-8 text-[12px]"
              value={note}
              placeholder={t("providers.agentNotePlaceholder")}
              onChange={(e) => setNote(e.target.value)}
            />
          </div>
          <p className="text-[11px] leading-relaxed text-mut">{t("providers.agentIdNote")}</p>
          {err && (
            <div className="text-[11px]" style={{ color: "var(--red)" }}>
              {err}
            </div>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" size="sm" className="h-7 px-3 text-[12px]" onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              className="h-7 px-3 text-[12px] font-semibold"
              disabled={!name.trim() || busy}
              onClick={create}
            >
              {t("providers.createAgent")}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ringColor(billing: Provider["billing"], pct: number): string {
  if (billing === "plan") return "var(--violet)";
  if (pct >= 95) return "var(--red)";
  if (pct >= 80) return "var(--amber)";
  return "var(--kiwi)";
}

/** One quota tier, e.g. `5h window 42%`. Falls back to the backend's own tier
    name, so a tier added there before this list knows about it still reads as
    something rather than disappearing. */
function tierLabel(name: string, t: Translate): string {
  const key = PLAN_TIER_LABEL_KEYS[name];
  return key ? t(key) : name;
}

/** The usage cell's tooltip. It is one sentence per billing shape, so each
    branch is a single key with named placeholders rather than a join: the
    optional 5h/weekly bounds arrive already joined as `windows`, and the
    optional in/out suffix as `tokens`. */
function usageTitle(t: Translate, p: Provider): string {
  if (p.billing === "plan" && p.plan_limits) {
    const l = p.plan_limits;
    const windows = [
      l.five_hour != null ? t("providers.usageWindowFive", { percent: l.five_hour }) : null,
      l.weekly != null ? t("providers.usageWindowWeekly", { percent: l.weekly }) : null,
    ]
      .filter(Boolean)
      .join(" · ");
    const tokens = p.usage
      ? t("providers.usageInOut", {
          input: fmtTokens(p.usage.input_tokens),
          output: fmtTokens(p.usage.output_tokens),
        })
      : "";
    return t("providers.usagePlanLimits", { windows, tokens });
  }
  const u = p.usage;
  if (!u) return "";
  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    // Counted units ("requests", "wan_tokens") print as-is; anything else is
    // a currency code, which is also a data value rather than a label.
    return t("providers.usagePlanQuota", {
      used: u.quota.used,
      limit: u.quota.limit,
      unit: u.quota.unit,
      percent: pct,
      resets: u.quota.resets_at ?? "",
      input: fmtTokens(u.input_tokens),
      output: fmtTokens(u.output_tokens),
    });
  }
  if (p.billing === "unl") return t("providers.usageLocalInference");
  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return t("providers.usagePaygQuota", {
      used: fmtMoney(u.quota.used, u.quota.unit),
      limit: fmtMoney(u.quota.limit, u.quota.unit),
      percent: pct,
      input: fmtTokens(u.input_tokens),
      cache: fmtTokens(u.cache_read_tokens),
      output: fmtTokens(u.output_tokens),
      latency: fmtLatency(u.latency_ms),
    });
  }
  return t("providers.usagePaygTrend");
}

/** The provider's own billing currency, when its limit unit names one.
    Counted units ("requests", "wan_tokens") are not currencies. */
function providerCurrency(p: Provider): string | undefined {
  const unit = p.limit_unit;
  return unit && unit.length === 3 ? unit : undefined;
}

/** Quota amount text: counted units render raw, currencies via fmtMoney */
function quotaAmountText(used: number, limit: number, unit: string): string {
  if (unit === "requests") return `${used}/${limit}`;
  if (unit === "wan_tokens") return `${used}/${limit}`;
  return `${fmtMoney(used, unit)} / ${fmtMoney(limit, unit)}`;
}

/** Tooltip of the live plan-quota line: the failure reason when the query
    failed, otherwise the template id plus a cached marker. `template` is a
    backend identifier and stays untranslated. */
function planLineTitle(t: Translate, plan: PlanQuotaReport | undefined): string {
  if (!plan) return "";
  if (!plan.success) return plan.error ?? "";
  return plan.cached
    ? t("providers.planTemplateCached", { template: plan.template })
    : plan.template;
}

/** Percent-limit plan cell: the ring shows how close the live plan-quota
    utilization is to its configured ceiling (tightest window wins); the
    limit line reads from plan_limits. Independent of usage history, so it
    renders for brand-new providers too. */
function planPercentCell(
  t: Translate,
  p: Provider,
  plan: PlanQuotaReport | undefined,
  title: string,
): ReactNode {
  const l = p.plan_limits!;
  const util = (name: string) =>
    plan?.success ? plan.tiers.find((t) => t.name === name)?.utilization : undefined;
  const five = util("five_hour");
  const week = util("weekly_limit");
  const ratios = [
    l.five_hour != null && five != null ? (five / l.five_hour) * 100 : 0,
    l.weekly != null && week != null ? (week / l.weekly) * 100 : 0,
  ];
  const pct = Math.min(100, Math.round(Math.max(...ratios, 0)));
  const color = pct >= 95 ? "var(--red)" : pct >= 80 ? "var(--amber)" : "var(--kiwi)";
  // Two optional whole phrases with a fixed order, joined by a neutral
  // separator — each is translated on its own so the join needs no key.
  const limitLine = [
    l.five_hour != null ? t("providers.planLimitFive", { percent: l.five_hour }) : null,
    l.weekly != null ? t("providers.planLimitWeekly", { percent: l.weekly }) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const planLine = plan?.success
    ? plan.tiers
        .map((tier) => `${tierLabel(tier.name, t)} ${Math.round(tier.utilization)}%`)
        .join(" · ")
    : plan && !plan.success
      ? plan.error
      : null;
  const maxUtil = plan?.success ? Math.max(...plan.tiers.map((t) => t.utilization), 0) : 0;
  return (
    <div className="flex w-full items-center gap-2" title={title}>
      <Ring pct={pct} color={color} />
      <div className="min-w-0">
        <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
          <BillTag billing="plan" />
          <span>{limitLine}</span>
        </div>
        <div className="mt-0.5 text-[10.5px] text-mut">{p.plan_price ?? ""}</div>
        {/* Live plan quota: per-window utilization from the provider's own endpoint */}
        {planLine && (
          <div
            className="mt-0.5 truncate text-[10.5px]"
            style={{ color: maxUtil >= 95 ? "var(--red)" : maxUtil >= 80 ? "var(--amber)" : "var(--mut)" }}
            title={planLineTitle(t, plan)}
          >
            {planLine}
          </div>
        )}
      </div>
    </div>
  );
}

/** Sits in the Usage/quota column.
 *
 * A provider the gateway is refusing to route keeps its numbers here, dimmed:
 * what the period cost is still true and still worth seeing, it is just no
 * longer what is happening. The block itself belongs in the status column,
 * which is the one that answers "what is this row doing right now". */
function UsageCell({
  p,
  plan,
  blocked,
}: {
  p: Provider;
  plan?: PlanQuotaReport;
  /** Why the gateway is refusing to route here, when it is. */
  blocked?: string;
}) {
  const t = useT();
  return (
    <div
      className={`w-[28%]${blocked ? " opacity-55" : ""}`}
      title={blocked ? t("providers.notRoutingTitle", { reason: blocked }) : undefined}
    >
      <UsageCellBody p={p} plan={plan} />
    </div>
  );
}

function UsageCellBody({ p, plan }: { p: Provider;
  /** Plan-quota report for providers with a plan query (undefined = not fetched yet) */
  plan?: PlanQuotaReport;
}) {
  const t = useT();
  const u = p.usage;

  // Percent-limit rows render even before the provider has any usage
  // history (usage summary still null): the ring/limit line need no totals.
  if (p.billing === "plan" && p.plan_limits) {
    return planPercentCell(t, p, plan, usageTitle(t, p));
  }

  if (!u) return <div className="w-full" />;
  const title = usageTitle(t, p);

  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    const planLine = plan?.success
      ? plan.tiers
          .map((tier) => `${tierLabel(tier.name, t)} ${Math.round(tier.utilization)}%`)
          .join(" · ")
      : plan && !plan.success
        ? plan.error
        : null;
    const maxUtil = plan?.success ? Math.max(...plan.tiers.map((t) => t.utilization), 0) : 0;
    return (
      <div className="flex w-full items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("plan", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="plan" />
            {u.quota.unit === "requests" || u.quota.unit === "wan_tokens" ? (
              <>
                {u.quota.used}/{u.quota.limit}{" "}
                <span className="font-normal text-mut">
                  {u.quota.unit === "requests" ? t("providers.unitReq") : t("providers.unitTenKTok")}
                </span>
              </>
            ) : (
              <span>{quotaAmountText(u.quota.used, u.quota.limit, u.quota.unit)}</span>
            )}
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {[
              p.plan_price,
              u.quota.resets_at
                ? t("providers.resetsAt", { date: u.quota.resets_at.slice(5) })
                : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </div>
          {/* Live plan quota: per-window utilization from the provider's own endpoint */}
          {planLine && (
            <div
              className="mt-0.5 truncate text-[10.5px]"
              style={{ color: maxUtil >= 95 ? "var(--red)" : maxUtil >= 80 ? "var(--amber)" : "var(--mut)" }}
              title={planLineTitle(t, plan)}
            >
              {planLine}
            </div>
          )}
        </div>
      </div>
    );
  }

  // Percent-limit cell (extracted so it renders before usage history exists):
  // the ring tracks how close the live utilization is to its configured
  // ceiling (tightest window wins); legacy used/limit rows render above.
  if (p.billing === "unl") {
    return (
      <div className="w-full" title={title}>
        <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
          <BillTag billing="unl" />
          {u.requests}{" "}
          <span className="font-normal text-mut">
            {t("providers.reqTokSuffix", { tokens: fmtTokens(u.input_tokens + u.output_tokens) })}
          </span>
        </div>
        <div className="mt-0.5 text-[10.5px] text-mut">
          {t("providers.inOutTokens", {
            input: fmtTokens(u.input_tokens),
            output: fmtTokens(u.output_tokens),
          })}
        </div>
      </div>
    );
  }

  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return (
      <div className="flex w-full items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("payg", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="payg" />
            {quotaAmountText(u.quota.used, u.quota.limit, u.quota.unit).split(" / ")[0]}{" "}
            <span className="font-normal text-mut">
              {t("providers.limitSuffix", {
                limit:
                  u.quota.unit === "requests" || u.quota.unit === "wan_tokens"
                    ? u.quota.limit
                    : fmtMoney(u.quota.limit, u.quota.unit),
              })}
            </span>
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {t("providers.tokensLatency", {
              tokens: fmtTokens(u.input_tokens + u.output_tokens),
              latency: fmtLatency(u.latency_ms),
            })}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="w-full" title={title}>
      <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
        <BillTag billing="payg" />
        {fmtMoney(u.cost ?? 0, u.cost_currency ?? providerCurrency(p) ?? "USD")}{" "}
        <span className="font-normal text-mut">
          {t("providers.reqSuffix", { requests: u.requests })}
        </span>
      </div>
      {u.spark && (
        <span className="mt-1 block w-20">
          <Sparkline points={u.spark} />
        </span>
      )}
    </div>
  );
}

/** First grid cell: brand mark + name + endpoint subtitle. Shared by the All-tab
    management rows and the agent-tab strategy rows. The All tab badges the
    collapsed is_current; agent tabs pass inUse to badge membership in that
    agent's serving set only. */
function IdentityCell({ p, inUse }: { p: Provider; inUse?: boolean }) {
  const t = useT();
  // Brand mark inferred from the endpoint host; unknown hosts keep the letter avatar
  const brandIcon = iconForEndpoint(p.endpoint);
  const showInUse = inUse ?? p.is_current;
  return (
    <div className="flex min-w-0 w-[34%] items-center gap-2.5">
      {brandIcon ? (
        <ProviderLogo icon={brandIcon} name={p.name} size={32} />
      ) : (
        <Logo char={p.logo_char} color={p.logo_color} border={p.logo_border} />
      )}
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="text-[13px] font-semibold">{p.name}</span>
          {showInUse && (
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
 * "Reachable" is deliberately not claimed. A background prober used to write a
 * verdict into `provider_health` every 30s and this cell showed it as a green dot
 * and a latency — a signal that never routed anything (the breaker, fed by real
 * traffic, decides), bought with one HTTP request per provider per half-minute.
 * It is gone, so an enabled provider says nothing here rather than asserting
 * health on the strength of a 30s-old probe. A parked provider still says so, and
 * one over a billing limit fills the cell above. */
function HealthCell({ p, blocked }: { p: Provider; blocked?: string }) {
  const t = useT();
  // The provider is up and reachable — that is not what changed. What changed
  // is whether Kiwano will use it, which is this column's question.
  if (blocked) {
    return (
      <div className="w-[14%]" title={t("providers.notRoutingTitle", { reason: blocked })}>
        <span className="flex items-center gap-1.5 text-[11.5px]" style={{ color: "var(--red)" }}>
          <Dot state="error" />
          {t("providers.blocked")}
        </span>
        <div className="mt-0.5 truncate text-[10.5px] text-mut">{blocked}</div>
      </div>
    );
  }
  return (
    <div className="w-[14%]">
      {/* Nothing to say is a legitimate answer: the row is enabled and nothing
          has gone wrong that the gateway knows of. */}
      {p.health.note && (
        <span className="flex items-center gap-1.5 text-[11.5px] text-mut">
          <Dot state={p.health.state} />
          {p.health.note}
        </span>
      )}
    </div>
  );
}

function ProviderRow({
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
  return (
    <>
    <div className={`row group flex h-[58px] items-center border-b border-line px-4${p.is_current ? " current" : ""}`}>
      <IdentityCell p={p} />

      <div className="flex w-[18%] items-center">
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

      <HealthCell p={p} blocked={blocked} />

      {/* Actions stay out of the resting row: reveal on hover / keyboard focus,
          or while a delete confirmation is pending. Binding / unbinding happens
          in the agent tabs, so there is no Enable action here. */}
      <div
        className={`flex flex-1 items-center justify-end gap-1.5 transition-opacity${
          confirmDel ? "" : " opacity-0 group-hover:opacity-100 focus-within:opacity-100"
        }`}
      >
        <TestLatencyButton provider={p} />
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

// ── Agent-tab strategy rows ──
//
// Inside an agent tab the list is the agent's routing candidate queue: each row
// shows its role under the active strategy (primary / standby order / weight /
// time window) with that strategy's per-binding parameters editable inline.
// Provider management (edit / enable / delete) stays on the All tab.

/** Roundrobin weight editor: commits on blur or Enter, clamped to ≥ 1. */
function WeightEditor({
  agent,
  b,
  onChanged,
}: {
  agent: AgentRef;
  b: StrategyBinding;
  onChanged: () => void;
}) {
  const t = useT();
  const [val, setVal] = useState(String(b.weight));
  useEffect(() => setVal(String(b.weight)), [b.weight]);
  const commit = () => {
    const n = Math.max(1, Math.round(Number(val) || 1));
    if (n === b.weight) {
      setVal(String(b.weight));
      return;
    }
    api.updateAgentBinding(agent, b.provider_id, { weight: n }).then(onChanged);
  };
  return (
    <span className="flex flex-none items-center gap-1" title={t("providers.weightTitle")}>
      <span className="text-[10.5px] text-mut">{t("providers.weight")}</span>
      <Input
        type="number"
        min={1}
        className="h-6 w-12 rounded-md bg-transparent px-1.5 text-right font-mono text-[11px] dark:bg-transparent"
        value={val}
        onChange={(e) => setVal(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
    </span>
  );
}

/** Timewindow range editor: one control owning both bounds, committed as a
    pair (a half window would never match — the backend clears them together).
    Validation: each bound must be a valid HH:MM and the end must be later
    than the start; invalid fields are highlighted red and the pair is simply
    not committed. Plain "HH:MM" text fields instead of <input type="time">:
    the native control's click/stepper UI varies per WebView (Tauri's WKWebView
    offers nothing to click), while a masked text field behaves identically
    everywhere. */
function TimeRangeEditor({
  agent,
  b,
  onChanged,
}: {
  agent: AgentRef;
  b: StrategyBinding;
  onChanged: () => void;
}) {
  const t = useT();
  const [s, setS] = useState(b.win_start ?? "");
  const [e, setE] = useState(b.win_end ?? "");
  const [left, setLeft] = useState(false); // focus has left the pair once
  useEffect(() => {
    setS(b.win_start ?? "");
    setE(b.win_end ?? "");
    setLeft(false);
  }, [b.win_start, b.win_end]);
  // Digits only; the colon is inserted automatically once past the hours
  const mask = (raw: string) => {
    const d = raw.replace(/[^0-9]/g, "").slice(0, 4);
    return d.length > 2 ? `${d.slice(0, 2)}:${d.slice(2)}` : d;
  };
  // Complete a masked entry: "9" -> "09:00", "0930" -> "09:30"; null = invalid
  const complete = (t: string): string | null => {
    const d = t.replace(/[^0-9]/g, "");
    if (!d) return "";
    if (d.length <= 2) {
      const h = Number(d);
      return h <= 23 ? `${d.padStart(2, "0")}:00` : null;
    }
    const h = Number(d.slice(0, 2));
    const m = Number(d.slice(2, 4));
    return h <= 23 && m <= 59 ? `${d.slice(0, 2).padStart(2, "0")}:${d.slice(2, 4).padStart(2, "0")}` : null;
  };

  const cs = complete(s);
  const ce = complete(e);
  // An incomplete pair only matters once editing is done: while the focus is
  // still inside, a single filled bound is just work in progress
  const half = left && !!cs !== !!ce;
  // Per-field red highlight: bad text flags its own bound, an end that is not
  // later than the start flags the end bound, an incomplete pair flags the
  // missing side
  const badS = cs === null || (half && !s);
  const badE = ce === null || (!!cs && !!ce && ce <= cs) || (half && !e);

  const commit = () => {
    setLeft(true);
    if (badS || badE) return; // invalid: stay red until fixed
    if (cs === (b.win_start ?? "") && ce === (b.win_end ?? "")) return;
    api.updateAgentBinding(agent, b.provider_id, { win_start: cs, win_end: ce }).then(onChanged);
  };
  // Commit only when the focus leaves the pair: moving between the two bounds
  // is mid-edit, and committing on each blur would wipe the first field before
  // the second one is filled.
  const onBlur = (ev: FocusEvent) => {
    if (ev.currentTarget.contains(ev.relatedTarget as Node | null)) return;
    commit();
  };
  const field =
    "h-5 w-[46px] border-0 bg-transparent p-0 text-center font-mono text-[11px] focus-visible:ring-0 dark:bg-transparent";
  return (
    <span
      className="flex flex-none items-center gap-1.5 rounded-md border border-line px-1.5 py-0.5"
      title={t("providers.windowTitle")}
      onBlur={onBlur}
    >
      <Input
        inputMode="numeric"
        placeholder="--:--"
        aria-label={t("providers.windowStart")}
        className={field}
        aria-invalid={badS}
        value={s}
        onChange={(ev) => setS(mask(ev.target.value))}
        onKeyDown={(ev) => ev.key === "Enter" && commit()}
      />
      <span className="text-[10.5px] text-mut">–</span>
      <Input
        inputMode="numeric"
        placeholder="--:--"
        aria-label={t("providers.windowEnd")}
        className={field}
        aria-invalid={badE}
        value={e}
        onChange={(ev) => setE(mask(ev.target.value))}
        onKeyDown={(ev) => ev.key === "Enter" && commit()}
      />
    </span>
  );
}

/** Role column: what this binding does under the active strategy, plus that
    strategy's per-binding parameters (weight / window) editable inline. */
function RoleCell({
  agent,
  route,
  b,
  idx,
  onChanged,
}: {
  agent: AgentRef;
  route: AgentRoute;
  b: StrategyBinding;
  idx: number;
  onChanged: () => void;
}) {
  const t = useT();
  const badge = (label: string, title: string) => (
    <span className="rounded px-1 text-[9.5px] font-medium" style={{ background: "var(--kiwi-soft)", color: "var(--kiwi)" }} title={title}>
      {label}
    </span>
  );
  return (
    <div className="flex w-[18%] min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
      {route.strategy === "roundrobin" ? (
        <WeightEditor agent={agent} b={b} onChanged={onChanged} />
      ) : route.strategy === "timewindow" ? (
        <>
          {idx === 0 &&
            !b.win_start &&
            badge(t("providers.fallback"), t("providers.fallbackTitle"))}
          {idx > 0 && !b.win_start && (
            <span className="text-[10.5px] text-mut" title={t("providers.noWindowTitle")}>
              {t("providers.noWindow")}
            </span>
          )}
          {/* The Fallback slot has no window by definition — no range editor
              there; give it one by pinning a windowed candidate to the top */}
          {(idx > 0 || !!b.win_start) && <TimeRangeEditor agent={agent} b={b} onChanged={onChanged} />}
        </>
      ) : idx === 0 ? (
        badge(t("providers.primary"), t("providers.primaryTitle"))
      ) : (
        <span className="text-[11px] text-mut">{t("providers.standby", { index: idx })}</span>
      )}
    </div>
  );
}

/** One agent-tab row: the provider joined with its strategy binding. */
function BindingRow({
  p,
  route,
  b,
  idx,
  plan,
  blocked,
  onMove,
  onChanged,
}: {
  p: Provider;
  route: AgentRoute;
  b: StrategyBinding;
  idx: number;
  plan?: PlanQuotaReport;
  blocked?: string;
  onMove: (idx: number, dir: -1 | 1) => void;
  onChanged: () => void;
}) {
  const t = useT();
  // Pin: promote this candidate to the queue head (the primary slot) — the
  // same reorder the arrows do, just straight to the top. Meaningless under
  // roundrobin (every candidate serves by weight), so it is hidden there.
  const makePrimary = () => {
    if (idx === 0) return;
    const ids = route.bindings.map((x) => x.provider_id);
    ids.unshift(ids.splice(idx, 1)[0]);
    api.reorderAgentBindings(route.agent, ids).then(onChanged);
  };
  // Unbind: removes only this agent's binding; other agents keep theirs.
  // Unbinding the last candidate drops the tab back to the onboarding state.
  const unbind = () => api.removeAgentBinding(route.agent, b.provider_id).then(onChanged);

  // Local "In use": serving this agent right now, not merely any agent
  const inUse = p.serving_agents.includes(route.agent);

  return (
    <div className={`row group flex h-[58px] items-center border-b border-line px-4${inUse ? " current" : ""}`}>
      <IdentityCell p={p} inUse={inUse} />

      <RoleCell agent={route.agent} route={route} b={b} idx={idx} onChanged={onChanged} />

      <UsageCell p={p} plan={plan} blocked={blocked} />

      <HealthCell p={p} blocked={blocked} />

      {/* Candidate ordering / membership is the only action here — provider
          management (edit / enable / delete) stays on the All tab. Actions
          reveal on hover; the Pin slot renders on every row (invisible for the
          primary) so resting rows keep identical column alignment. */}
      <div className="flex flex-1 items-center justify-end gap-1.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
        {route.bindings.length > 1 && (
          <span className="flex flex-col">
            <button
              className="text-mut hover:text-ink disabled:opacity-30"
              aria-label={t("providers.moveUp")}
              disabled={idx === 0}
              onClick={() => onMove(idx, -1)}
            >
              <ArrowUp className="h-3 w-3" />
            </button>
            <button
              className="text-mut hover:text-ink disabled:opacity-30"
              aria-label={t("providers.moveDown")}
              disabled={idx === route.bindings.length - 1}
              onClick={() => onMove(idx, 1)}
            >
              <ArrowDown className="h-3 w-3" />
            </button>
          </span>
        )}
        {route.strategy !== "roundrobin" && (
          <Button
            variant="ghost"
            size="sm"
            className={`h-7 border border-line px-1.5 text-mut${idx === 0 ? " invisible" : ""}`}
            aria-label={t("providers.makePrimary")}
            title={t("providers.makePrimaryTitle")}
            onClick={makePrimary}
          >
            <Pin className="h-3.5 w-3.5" />
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 border border-line px-1.5 text-mut hover:text-ink"
          aria-label={t("providers.removeFromRouteAria")}
          title={t("providers.removeFromRoute")}
          onClick={unbind}
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

/** Select that offers providers not yet bound to the agent; picking one binds
    it to the route tail. Shared by the list-bottom add row and the onboarding
    empty state. When every provider is already in the route the trigger stays
    visible but disabled, so the row keeps its shape. */
function BindProviderSelect({
  providers,
  boundIds,
  onPick,
}: {
  providers: Provider[];
  boundIds: Set<string>;
  onPick: (providerId: string) => void;
}) {
  const t = useT();
  const available = providers.filter((p) => !boundIds.has(p.id));
  const [sel, setSel] = useState("");
  const full = available.length === 0;
  return (
    <Select
      value={sel}
      disabled={full}
      onValueChange={(pid) => {
        setSel("");
        if (pid) onPick(pid);
      }}
    >
      <SelectTrigger
        size="sm"
        aria-label={t("providers.bindAria")}
        className="h-7 w-[210px] bg-transparent text-[12px] dark:bg-transparent disabled:opacity-50"
      >
        <SelectValue
          placeholder={full ? t("providers.allBound") : t("providers.bindExisting")}
        />
      </SelectTrigger>
      <SelectContent className="min-w-[220px]">
        {available.map((p) => {
          const icon = iconForEndpoint(p.endpoint);
          return (
            <SelectItem key={p.id} value={p.id} className="text-[12px]">
              {icon && <ProviderLogo icon={icon} name={p.name} size={14} />}
              {p.name}
            </SelectItem>
          );
        })}
      </SelectContent>
    </Select>
  );
}

/** Bottom row of an agent tab with candidates: bind one more provider to this
    route (it joins the queue tail as a standby). */
function AddBindingRow({
  agent,
  providers,
  boundIds,
  onChanged,
}: {
  agent: AgentRef;
  providers: Provider[];
  boundIds: Set<string>;
  onChanged: () => void;
}) {
  const t = useT();
  const available = providers.filter((p) => !boundIds.has(p.id));
  return (
    <div className="flex h-11 items-center gap-3 border-b border-line px-4">
      <BindProviderSelect
        providers={providers}
        boundIds={boundIds}
        onPick={(pid) => api.addAgentBinding(agent, pid).then(onChanged)}
      />
      <span className="flex-1 truncate text-[11px] text-mut">
        {available.length === 0
          ? t("providers.allInRoute")
          : t("providers.bindAnother")}
      </span>
    </div>
  );
}

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
  onRedetect?: () => void;
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
  // The credentials dialog, opened from the icon beside a user-defined agent's
  // name (there is no card on the tab itself).
  const [accessOpen, setAccessOpen] = useState(false);
  // The built-in agent's settings dialog (its takeover, and the files behind it).
  const [agentSettingsOpen, setAgentSettingsOpen] = useState(false);
  // The files behind each built-in agent, for the row above the table. Its own
  // state because the takeover *set* below throws the rest of that read away.
  const [configPathsByAgent, setConfigPathsByAgent] = useState<Record<string, string[]>>({});
  // What a client is pointed at: the gateway's own listen address from settings.
  const [listen, setListen] = useState("127.0.0.1:8317");
  // Display currency, which is also the unit a money ceiling is written in.
  const [currency, setCurrency] = useState("USD");
  const [enabling, setEnabling] = useState(false);
  // Plan-quota reports per provider, auto-refreshed on load
  const [planQuotas, setPlanQuotas] = useState<Record<string, PlanQuotaReport>>({});
  const [quotaBusy, setQuotaBusy] = useState(false);

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

  const refetch = useCallback(() => {
    api.listProviders().then(setProviders);
    // Per-agent strategy routes drive the agent-tab candidate rows
    api.getAgentRoutes().then(setRoutes).catch(() => setRoutes(null));
    // Takeover state drives the per-agent onboarding panel; the same read
    // carries the user's own agents, which drive their segments and resolver.
    api
      .getSettings()
      .then((s) => {
        setTakenOver(new Set(s.takeovers.filter((t) => t.enabled).map((t) => t.agent)));
        setConfigPathsByAgent(
          Object.fromEntries(s.takeovers.map((k) => [k.agent, k.config_paths ?? []])),
        );
        setCustomAgents(s.custom_agents);
        rememberCustomAgents(s.custom_agents);
        if (s.gateway_listen) setListen(s.gateway_listen);
        if (s.preferred_currency) setCurrency(s.preferred_currency);
      })
      .catch(() => {});
  }, []);
  useEffect(refetch, [refetch]);

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
  const refreshQuotas = async () => {
    if (!providers || quotaBusy) return;
    setQuotaBusy(true);
    try {
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
    } finally {
      setQuotaBusy(false);
    }
  };

  // Built-ins (in the registry's order), then the agents the user defined —
  // which are never "not installed": there is nothing to install.
  const segments = useMemo(
    () => [
      ...SEGMENTS,
      ...customAgents.map((a) => ({ id: a.id as AgentRef, label: a.label, icon: undefined })),
    ],
    [customAgents],
  );

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
   * back to the agent's own configuration, the same call the Settings switch
   * makes. Either way the screen re-reads afterwards instead of assuming the
   * outcome, and a refusal is left to the caller to show. */
  const setAgentTakenOver = async (agent: AgentRef, enabled: boolean) => {
    if (!isBuiltinAgent(agent)) return;
    setEnabling(true);
    try {
      await api.setTakeover(agent, enabled);
    } finally {
      setEnabling(false);
      refetch();
    }
  };

  if (!providers) return <div className="p-8 text-center text-[12px] text-mut">{t("common.loading")}</div>;

  const filtered = providers.filter((p) => seg === "all" || p.agents.includes(seg));
  const agentsBound = new Set(providers.flatMap((p) => p.agents)).size;
  // The files a takeover would rewrite for the tab in hand, read out of the
  // settings read this screen already makes. Empty while that read is in flight
  // or for an id the registry does not know.
  const configPaths = seg === "all" ? [] : (configPathsByAgent[seg] ?? []);
  // Inside an agent tab with bindings the list IS the strategy candidate queue
  const route = seg === "all" ? null : (routes?.find((r) => r.agent === seg) ?? null);
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
                ) : s.id !== "all" && !AGENTS.some((a) => a.id === s.id) ? (
                  // A user-defined agent has no brand mark to port: its own
                  // letter, in the colour the resolver derived for it.
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
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 shrink-0 px-0 text-mut hover:text-ink"
            aria-label={t("providers.newAgent")}
            title={t("providers.newAgentTitle")}
            onClick={() => setNewAgent(true)}
          >
            <Plus className="h-3.5 w-3.5" />
          </Button>
        </div>
        <span className="ml-1.5 text-[11.5px] text-mut">
          {t("providers.counts", { providers: providers.length, agents: agentsBound })}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-7 w-7 px-0 text-mut"
          aria-label={t("common.refresh")}
          title={t("providers.refreshTitle")}
          disabled={quotaBusy}
          onClick={() => {
            refetch();
            refreshQuotas();
            // Deliberately separate from `refetch`: that one is the generic
            // "something changed" callback handed to every child, and re-probing
            // spawns a login shell plus one subprocess per agent, so only a
            // click should pay for it.
            onRedetect?.();
          }}
        >
          <RefreshCw className={`h-3.5 w-3.5${quotaBusy ? " animate-spin" : ""}`} />
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
            aria-label={t("providers.accessFor", { agent: custom.label })}
            title={t("providers.accessFor", { agent: custom.label })}
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
          onTakeover={() => void setAgentTakenOver(seg, true).catch(() => {})}
          onAdd={onAdd}
        />
      ) : (
        <>
          <div className="flex h-7 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
            <span className="w-[32%]">{t("providers.colProvider")}</span>
            <span className="w-[18%]">
              {route ? t("providers.colRole") : t("providers.colBoundAgents")}
            </span>
            <span className="w-[28%]">{t("providers.colUsage")}</span>
            <span className="w-[14%]">{t("providers.colStatus")}</span>
            <span className="flex-1 text-right">
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
              onTakeover={() => void setAgentTakenOver(seg, true).catch(() => {})}
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
        onCreated={onAgentCreated}
      />

      {!custom && seg !== "all" && (takenOver?.has(seg) ?? false) && (
        <AgentSettingsDialog
          agent={seg}
          label={agentMeta(seg).label}
          paths={configPaths}
          limit={route?.limit ?? null}
          currency={currency}
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
          listen={listen}
          limit={route?.limit ?? null}
          currency={currency}
          open={accessOpen}
          onClose={() => setAccessOpen(false)}
          onDelete={() => onDeleteAgent(custom.id)}
          onChanged={refetch}
        />
      )}

      {/* Single closing note. In a taken-over agent tab with a route it also
          carries the strategy context (the StrategyPanel select row has no
          header of its own). */}
      <div className="mt-auto truncate px-4 py-3 text-[10.5px] text-mut">
        {notTakenOver
          ? t("providers.footerNotTakenOver")
          : custom
            ? t("providers.footerRoute")
            : seg !== "all" &&
                (takenOver?.has(seg) ?? false) &&
                (routes?.some((r) => r.agent === seg) ?? false)
              ? t("providers.footerStrategy")
              : t("providers.footerDefault")}
      </div>
    </section>
  );
}
