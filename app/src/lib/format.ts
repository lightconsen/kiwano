// Numeric display helpers (design.md §7: numbers always mono, tokens shown in tiers)

export function fmtTokens(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return `${m >= 10 ? Math.round(m) : Math.round(m * 10) / 10}M`;
  }
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

/** Bytes as MB, to one decimal below 100 (12.3 MB, 104 MB).
 *
 * The updater downloads an installer, which is always MB-scale — a generic
 * B/KB/MB/GB ladder would be four branches nobody ever sees three of. `MB` is
 * a unit, not a word, so it is not translated. */
export function fmtMB(bytes: number): string {
  const mb = bytes / 1_000_000;
  return `${mb >= 100 ? Math.round(mb) : Math.round(mb * 10) / 10} MB`;
}

export function fmtLatency(ms: number | null): string {
  if (ms == null) return "—";
  return ms >= 1000 ? `${Math.round(ms / 100) / 10}s` : `${ms}ms`;
}

export function fmtCny(n: number, dp?: number): string {
  return `¥${dp != null ? n.toFixed(dp) : n}`;
}

/** ISO currency code -> symbol (fallback: the code itself) */
const CURRENCY_SYMBOLS: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  EUR: "€",
  GBP: "£",
  JPY: "¥",
  HKD: "HK$",
  KRW: "₩",
};

/** Format an amount in the given currency (symbol prefix; sub-unit amounts keep 4dp) */
export function fmtMoney(n: number, currency: string): string {
  const symbol = CURRENCY_SYMBOLS[currency.toUpperCase()] ?? `${currency.toUpperCase()} `;
  const dp = n !== 0 && Math.abs(n) < 1 ? 4 : 2;
  const s = n.toFixed(dp);
  const trimmed = s.includes(".") ? s.replace(/0+$/, "").replace(/\.$/, "") : s;
  return `${symbol}${trimmed}`;
}

/** Convert between currencies via the USD pivot (rates[currency] = units per 1 USD).
    Unknown currencies pass the amount through unchanged. */
export function convertAmount(
  amount: number,
  from: string,
  to: string,
  rates: Record<string, number>,
): number {
  if (from === to) return amount;
  const perUsdFrom = rates[from?.toUpperCase() ?? ""];
  const perUsdTo = rates[to?.toUpperCase() ?? ""];
  if (!perUsdFrom || !perUsdTo) return amount;
  return (amount / perUsdFrom) * perUsdTo;
}
