// 设计原型的共享原子组件（design/index.html 的 .logo-c/.dot/.billtag/.tswitch 等的 React 版）
import type { AgentMeta, Billing } from "../api/types";

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

export function Dot({ state, size = "w-[6px] h-[6px]" }: { state: "ok" | "idle" | "off"; size?: string }) {
  const style =
    state === "ok"
      ? { background: "var(--kiwi)", boxShadow: "0 0 6px oklch(0.8 0.19 132 / .5)" }
      : state === "idle"
        ? { background: "oklch(0.55 0.12 250)" }
        : { background: "oklch(0.42 0.01 260)" };
  return <span className={`dot ${size}`} style={style} />;
}

const BILL_TAG: Record<Billing, { label: string; cls: string }> = {
  payg: { label: "按量", cls: "bill-payg" },
  plan: { label: "订阅", cls: "bill-plan" },
  unl: { label: "不限", cls: "bill-unl" },
};

export function BillTag({ billing }: { billing: Billing }) {
  const t = BILL_TAG[billing];
  return <span className={`billtag ${t.cls}`}>{t.label}</span>;
}

/** 28px 用量圆环（design.md §8：订阅=紫 / 按量=kiwi，≥80% amber，≥95% red） */
export function Ring({ pct, color }: { pct: number; color?: string }) {
  const stroke = color ?? "var(--kiwi)";
  // quota.limit=0 等异常输入 → NaN/Infinity，钳到 [0,100]
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
      <polyline points={pts} fill="none" stroke="oklch(0.62 0.14 250)" strokeWidth="1.5" />
    </svg>
  );
}

export function AgentChip({ meta }: { meta: AgentMeta }) {
  return (
    <span
      className="achip"
      title={meta.label}
      style={meta.chip_border ? { background: meta.chip_color, border: "1px solid var(--line)" } : { background: meta.chip_color }}
    >
      {meta.chip_char}
    </span>
  );
}

export function Toggle({ on, onChange }: { on: boolean; onChange?: (next: boolean) => void }) {
  return (
    <div
      className={`tswitch${on ? " on" : ""}`}
      onClick={() => onChange?.(!on)}
      role="switch"
      aria-checked={on}
    />
  );
}
