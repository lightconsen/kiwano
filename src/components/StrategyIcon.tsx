// Minimal monochrome glyphs for the five routing strategies, rendered inside
// the strategy <Select> (trigger + items). Stroke-based on currentColor so
// they follow the theme; sized to size-3.5 at call sites (a size- class keeps
// the Select's default size-4 svg rule from overriding it).
import type { ReactNode } from "react";

import type { StrategyKind } from "../api/types";

const GLYPHS: Record<StrategyKind, ReactNode> = {
  // One hub, one target: the primary only.
  single: (
    <>
      <circle cx="12" cy="12" r="8" />
      <circle cx="12" cy="12" r="2.4" fill="currentColor" stroke="none" />
    </>
  ),
  // The dashed link to the primary failed; the solid line below carries
  // traffic to the standby.
  failover: (
    <>
      <circle cx="16" cy="7" r="2.4" />
      <circle cx="16" cy="17" r="2.4" />
      <path d="M3.5 7h10" strokeDasharray="2.2 2.8" />
      <path d="M3.5 17h10" />
    </>
  ),
  // Requests rotate around the ring of candidates (rotate arrow).
  roundrobin: (
    <>
      <path d="M20 12a8 8 0 1 1-8-8c2.24 0 4.38.89 5.99 2.43L20 8.4" />
      <path d="M20 3.6v4.8h-4.8" />
    </>
  ),
  // Clock face with the active window shaded; the hour hand sits inside it.
  timewindow: (
    <>
      <path d="M12 12V4a8 8 0 0 1 6.93 4Z" fill="currentColor" stroke="none" opacity="0.2" />
      <circle cx="12" cy="12" r="8" />
      <path d="M12 12V7.5" />
      <path d="M12 12l3.5-2" />
    </>
  ),
  // Gauge with the needle in the upper zone: primary over its daily threshold.
  quota: (
    <>
      <path d="M4 14a8 8 0 0 1 16 0" />
      <circle cx="12" cy="14" r="1.6" fill="currentColor" stroke="none" />
      <path d="M12 14l4.6-4.6" />
    </>
  ),
};

export default function StrategyIcon({ id, className }: { id: StrategyKind; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden
    >
      {GLYPHS[id]}
    </svg>
  );
}
