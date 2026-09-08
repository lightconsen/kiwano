// Numeric display helpers (design.md §7: numbers always mono, tokens shown in tiers)

export function fmtTokens(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return `${m >= 10 ? Math.round(m) : Math.round(m * 10) / 10}M`;
  }
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

export function fmtLatency(ms: number | null): string {
  if (ms == null) return "—";
  return ms >= 1000 ? `${Math.round(ms / 100) / 10}s` : `${ms}ms`;
}

export function fmtCny(n: number, dp?: number): string {
  return `¥${dp != null ? n.toFixed(dp) : n}`;
}
