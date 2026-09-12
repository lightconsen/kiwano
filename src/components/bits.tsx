// Shared atomic components from the design prototype (React versions of design/index.html's .logo-c/.dot/.billtag etc.; base controls have moved to shadcn/ui)
import type { AgentMeta, Billing } from "../api/types";
import { useT, type KeyPath, type Messages } from "../i18n";
import { ProviderLogo } from "@/components/icons/ProviderLogo";

/** AgentId → brand icon key in the cc-switch registry (components/icons). */
export const AGENT_ICON: Partial<Record<AgentMeta["id"], string>> = {
  claude: "claudecode",
  codex: "openai",
  gemini: "gemini",
  grokbuild: "grok",
  "claude-desktop": "claude",
  opencode: "opencode",
  openclaw: "openclaw",
  hermes: "hermes",
  pi: "pi",
};

export function Logo({
  char,
  color,
  border,
  size = "w-8 h-8 text-[13px]",
}: {
  char: string;
  color: string;
  border?: boolean;
  size?: string;
}) {
  return (
    <div
      className={`logo-c ${size}`}
      style={border ? { background: color, border: "1px solid var(--line)" } : { background: color }}
    >
      {char}
    </div>
  );
}

export function Dot({
  state,
  size = "w-[6px] h-[6px]",
}: {
  /** `error` is not a health state: it reads "this row is not serving you". */
  state: "ok" | "idle" | "off" | "error";
  size?: string;
}) {
  const style =
    state === "ok"
      ? { background: "var(--kiwi)", boxShadow: "0 0 6px oklch(0.8 0.19 132 / .5)" }
      : state === "idle"
        ? { background: "oklch(0.55 0.12 250)" }
        : state === "error"
          ? { background: "var(--red)" }
          : { background: "oklch(0.42 0.01 260)" };
  return <span className={`dot ${size}`} style={style} />;
}

/** Keys, not labels: this map is module-level and `t()` is a hook, so the
    label is resolved at render. */
const BILL_TAG: Record<Billing, { labelKey: KeyPath<Messages>; cls: string }> = {
  payg: { labelKey: "app.billing.payg", cls: "bill-payg" },
  plan: { labelKey: "app.billing.plan", cls: "bill-plan" },
  unl: { labelKey: "app.billing.unl", cls: "bill-unl" },
};

/** Fallback class for a billing tag this build does not know (a Hub catalog
    row may carry one): a neutral chip showing the raw tag, instead of crashing
    on a missing lookup entry. */
const BILL_TAG_UNKNOWN_CLS = "bill-other";

export function BillTag({ billing }: { billing: Billing }) {
  const t = useT();
  const tag = BILL_TAG[billing] as { labelKey: KeyPath<Messages>; cls: string } | undefined;
  return (
    <span className={`billtag ${tag?.cls ?? BILL_TAG_UNKNOWN_CLS}`}>
      {tag ? t(tag.labelKey) : billing}
    </span>
  );
}

/** 28px usage ring (design.md §8: plan=purple / payg=kiwi, ≥80% amber, ≥95% red) */
export function Ring({ pct, color }: { pct: number; color?: string }) {
  const stroke = color ?? "var(--kiwi)";
  // Abnormal input like quota.limit=0 → NaN/Infinity; clamp to [0,100]
  const safe = Number.isFinite(pct) ? Math.min(100, Math.max(0, pct)) : 0;
  return (
    <svg viewBox="0 0 28 28" className="h-7 w-7 flex-none">
      <circle cx="14" cy="14" r="12" fill="none" stroke="var(--surface2)" strokeWidth="3" />
      <circle
        cx="14"
        cy="14"
        r="12"
        fill="none"
        stroke={stroke}
        strokeWidth="3"
        strokeLinecap="round"
        strokeDasharray={`${(safe / 100) * 75.4} 75.4`}
        transform="rotate(-90 14 14)"
      />
      <text x="14" y="17" textAnchor="middle" fontSize="8" fontFamily="JetBrains Mono" fill="var(--ink)">
        {safe}%
      </text>
    </svg>
  );
}

export function Sparkline({ points }: { points: number[] }) {
  const n = points.length;
  const pts = points.map((y, i) => `${(i * 80) / (n - 1)},${y}`).join(" ");
  return (
    <svg viewBox="0 0 80 14" className="h-3.5 w-20">
      <polyline points={pts} fill="none" stroke="var(--blue)" strokeWidth="1.5" />
    </svg>
  );
}

/** Bound-agents cell chip: the agent's brand logo (letter-avatar fallback
    keeps the same 18px footprint as the old .achip). chip_color is NOT passed
    as the tint — it is the old letter-chip background (near-black for
    codex/grok), which would vanish on the dark theme. Without it ProviderLogo
    tints currentColor marks with var(--ink) (theme-aware) and brand marks
    with their registry defaultColor, matching the segment bar. */
export function AgentChip({ meta, size = 18 }: { meta: AgentMeta; size?: number }) {
  return (
    <ProviderLogo
      icon={AGENT_ICON[meta.id]}
      char={meta.chip_char}
      name={meta.label}
      size={size}
    />
  );
}
