// The protocol chip a row wears and the endpoint card the detail dialog opens
// on: what a provider speaks, at what address, and whether that address answers
// without a key.
import { useState } from "react";
import { Gauge } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../../api/client";
import { useT, type KeyPath, type Messages, type Translate } from "../../i18n";
import type { ProbeReport, Protocol } from "../../api/types";
import { PROTO_ABBR, PROTO_LABEL, PROTO_STYLE } from "./labels";

/** One protocol as a marked letter — see PROTO_ABBR for why, and for what
    carries the full name. A protocol the vocabulary does not know (a Hub
    catalog entry that names no primary protocol — its top-level `protocol`
    is empty — or names a new one) renders no chip: the endpoint cards in the
    detail dialog spell the protocols out regardless. */
export function ProtoChip({ protocol, t }: { protocol: Protocol; t: Translate }) {
  const labelKey = PROTO_LABEL[protocol];
  if (!labelKey) return null;
  const name = t(labelKey);
  return (
    <span
      className="rounded px-1.5 py-0.5 font-mono text-[10px]"
      style={PROTO_STYLE[protocol]}
      title={name}
    >
      {/* The letter is the visual stand-in, the name is what a screen reader
          reads — an `aria-label` on a bare span is not reliably announced, and
          a chip that announces "O" is worse than no chip at all. */}
      <span aria-hidden>{PROTO_ABBR[protocol]}</span>
      <span className="sr-only">{name}</span>
    </span>
  );
}

// Probe verdict chip: green=usable, amber=route exists but needs a key,
// gray=route missing, red=broken/unreachable
const PROBE_LABEL: Record<ProbeReport["verdict"], KeyPath<Messages>> = {
  ok: "shelf.probeOk",
  auth: "shelf.probeAuth",
  unsupported: "shelf.probeUnsupported",
  error: "shelf.probeError",
  unreachable: "shelf.probeUnreachable",
};

function probeChipStyle(verdict: ProbeReport["verdict"]): React.CSSProperties {
  switch (verdict) {
    case "ok":
      return { background: "oklch(0.72 0.15 155 / .14)", color: "oklch(0.62 0.15 155)" };
    case "auth":
      return { background: "oklch(0.8 0.13 85 / .14)", color: "oklch(0.7 0.13 85)" };
    case "unsupported":
      return { background: "var(--surface2)", color: "var(--mut)" };
    case "error":
    case "unreachable":
      return { background: "oklch(0.6 0.2 25 / .14)", color: "var(--red)" };
  }
}

/** One endpoint card of the detail dialog: URL + keyless protocol probe.
    A 401/403 still counts as "auth" — the route exists, so the protocol is
    supported even without a key. */
export function EndpointCard({ protocol, endpoint, models }: { protocol: Protocol; endpoint: string; models: string[] }) {
  const t = useT();
  const [probe, setProbe] = useState<ProbeReport | "loading" | null>(null);
  return (
    <div className="rounded-md border border-line px-2.5 py-2">
      <div className="flex items-center gap-2">
        <span className="rounded px-1.5 py-0.5 font-mono text-[10px]" style={PROTO_STYLE[protocol]}>
          {protocol}
        </span>
        <span className="min-w-0 truncate font-mono text-[11px] text-mut" title={endpoint}>
          {endpoint.replace(/^https?:\/\//, "")}
        </span>
        <Button
          variant="outline"
          size="xs"
          className="ml-auto flex-none gap-1 rounded px-2 text-[10.5px]"
          disabled={probe === "loading"}
          onClick={() => {
            setProbe("loading");
            api
              .testEndpoint(protocol, endpoint)
              .then(setProbe)
              .catch(() =>
                setProbe({
                  verdict: "error",
                  status: null,
                  latency_ms: 0,
                  detail: t("shelf.probeFailed"),
                }),
              );
          }}
        >
          <Gauge className="h-3 w-3" />
          {t("shelf.test")}
        </Button>
      </div>
      {probe && probe !== "loading" && (
        <div className="mt-1.5 flex items-center gap-2">
          <span className="rounded px-1.5 text-[10px]" style={probeChipStyle(probe.verdict)}>
            {t(PROBE_LABEL[probe.verdict])}
          </span>
          <span className="min-w-0 truncate text-[10.5px] text-mut" title={probe.detail}>
            {probe.detail}
          </span>
          <span className="ml-auto flex-none font-mono text-[10.5px] text-mut">{probe.latency_ms}ms</span>
        </div>
      )}
      {models.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {models.map((m) => (
            <span key={m} className="rounded bg-surface2 px-1.5 py-0.5 font-mono text-[10px] text-mut">
              {m}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
