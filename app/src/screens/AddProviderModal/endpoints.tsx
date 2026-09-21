// The endpoint rows the form collects, and the probe readout beside each one.
import { Gauge, Plus, XIcon } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../../api/client";
import { useT } from "../../i18n";
import type { CatalogEntry, ProbeReport, Protocol, Provider } from "../../api/types";

export function probeColor(verdict: ProbeReport["verdict"]): string {
  return verdict === "ok" || verdict === "auth" ? "var(--kiwi)" : "var(--red)";
}

/** One URL per protocol: the primary first, then one row per additional
    protocol the same key pool answers on. A catalog entry brings its own list,
    and it is read-only while that entry owns the form; a provider typed in by
    hand — and an existing row, which is the user's own already — can add and
    drop them. */
export function EndpointsSection({
  protocol,
  setProtocol,
  endpoint,
  setEndpoint,
  altEndpoints,
  setAltEndpoints,
  altProbes,
  setAltProbes,
  altTesting,
  probe,
  setProbe,
  testing,
  setTesting,
  setFetchedModels,
  setFetchError,
  apiKey,
  edit,
  fromEntry,
  mode,
  shelf,
  testAlt,
}: {
  protocol: Protocol;
  setProtocol: (p: Protocol) => void;
  endpoint: string;
  setEndpoint: (v: string) => void;
  altEndpoints: { protocol: Protocol; endpoint: string }[];
  setAltEndpoints: Dispatch<SetStateAction<{ protocol: Protocol; endpoint: string }[]>>;
  altProbes: Record<number, ProbeReport | null>;
  setAltProbes: Dispatch<SetStateAction<Record<number, ProbeReport | null>>>;
  altTesting: Record<number, boolean>;
  probe: ProbeReport | null;
  setProbe: (r: ProbeReport | null) => void;
  testing: boolean;
  setTesting: (b: boolean) => void;
  /** A new endpoint invalidates the list fetched from the old one */
  setFetchedModels: (m: string[] | null) => void;
  setFetchError: (e: string | null) => void;
  apiKey: string;
  edit: Provider | null;
  /** True while a catalog entry declares the endpoints: the rows are read-only */
  fromEntry: boolean;
  /** The two the hint under the URL label names an entry with. `fromEntry` asks
      the same question, but the hint reads them the way the form wrote it. */
  mode: "shelf" | "custom";
  shelf: CatalogEntry | null;
  /** The container's own test of one additional endpoint, by row index */
  testAlt: (i: number) => void;
}) {
  const t = useT();
  const protocolLabel = (id: string) =>
    protocolOptions.find((p) => p.id === id)?.label ?? id;

  const protocolOptions: { id: Protocol; label: string }[] = [
    { id: "openai", label: t("addProvider.protoOpenai") },
    { id: "anthropic", label: t("addProvider.protoAnthropic") },
    { id: "gemini", label: t("addProvider.protoGemini") },
  ];

  // Inline probe readout: ok shows the measured latency, every other verdict
  // shows a short word (full detail lives in the span's title). Green/kiwi =
  // usable (ok, or route exists but key invalid), red = broken/unreachable.
  const probeText: Record<ProbeReport["verdict"], string> = {
    ok: t("addProvider.probeOk"),
    auth: t("addProvider.probeAuth"),
    unsupported: t("addProvider.probeUnsupported"),
    error: t("addProvider.probeError"),
    unreachable: t("addProvider.probeUnreachable"),
  };

  // Protocols this provider serves: the primary + every additional endpoint's
  const supportedProtocols = new Set<Protocol>([protocol, ...altEndpoints.map((r) => r.protocol)]);

  /** One endpoint row's protocol: a badge while a catalog entry owns the form, a
      picker when the user does. Both rows use it, and so the coverage badges above
      follow whatever is picked here. */
  const protocolField = (value: Protocol, onChange: (p: Protocol) => void) =>
    fromEntry ? (
      <span className="flex h-8 w-[108px] flex-none items-center overflow-hidden rounded-md border border-line bg-surface2 px-2 text-[11.5px] text-mut">
        <span className="truncate">{protocolLabel(value)}</span>
      </span>
    ) : (
      <Select value={value} onValueChange={(v) => v && onChange(v as Protocol)}>
        <SelectTrigger
          className="h-8 w-[108px] flex-none bg-bg text-[11.5px] dark:bg-bg"
          aria-label={t("addProvider.protocol")}
        >
          <SelectValue>{(v) => protocolLabel(String(v ?? value))}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          {protocolOptions.map((p) => (
            <SelectItem key={p.id} value={p.id}>
              {p.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );

  /** The protocol no endpoint covers yet, for a row that is about to be added.
      Only two exist, so the add button runs out rather than repeating one. */
  const nextFreeProtocol = (): Protocol =>
    protocolOptions.find((p) => !supportedProtocols.has(p.id))?.id ?? protocol;

  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.endpointUrl")}{" "}
        <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
          {!edit && mode === "shelf" && shelf
            ? t("addProvider.endpointHintShelf")
            : t("addProvider.endpointHint")}
        </span>
      </Label>
      <div className="mt-1 space-y-1.5">
        {/* Primary endpoint row */}
        <div className="flex items-center gap-1.5">
          {protocolField(protocol, (p) => {
            setProtocol(p);
            // The verdict was for the old protocol's endpoint call shape
            setProbe(null);
          })}
          <Input
            className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
            value={endpoint}
            readOnly={fromEntry}
            aria-label={t("addProvider.endpointUrl")}
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
                setProbe(
    await api.testEndpoint(protocol, endpoint.trim(), apiKey.trim() || undefined, edit?.id),
  );
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
            {t("addProvider.test")}{" "}
            {probe && (
              <span className="font-mono" style={{ color: probeColor(probe.verdict) }} title={probe.detail}>
                {probe.verdict === "ok" ? `${probe.latency_ms}ms` : probeText[probe.verdict]}
              </span>
            )}
          </Button>
        </div>
        {altEndpoints.map((r, i) => (
          <div key={i} className="flex items-center gap-1.5">
            {protocolField(r.protocol, (p) => {
              setAltEndpoints((rs) =>
                rs.map((x, j) => (j === i ? { ...x, protocol: p } : x)),
              );
              // The verdict was for the old protocol's call shape
              setAltProbes((m) => {
                const next = { ...m };
                delete next[i];
                return next;
              });
            })}
            <span className="min-w-0 flex-1">
              <Input
                className="h-8 w-full bg-bg font-mono text-[12px] dark:bg-bg"
                value={r.endpoint}
                readOnly={fromEntry}
                aria-label={t("addProvider.endpointUrl")}
                onChange={(e) => {
                  const value = e.target.value;
                  setAltEndpoints((rs) =>
                    rs.map((x, j) => (j === i ? { ...x, endpoint: value } : x)),
                  );
                  setAltProbes((m) => {
                    const next = { ...m };
                    delete next[i];
                    return next;
                  });
                }}
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
              {t("addProvider.test")}{" "}
              {altProbes[i] && (
                <span
                  className="font-mono"
                  style={{ color: probeColor(altProbes[i]!.verdict) }}
                  title={altProbes[i]!.detail}
                >
                  {altProbes[i]!.verdict === "ok"
                    ? `${altProbes[i]!.latency_ms}ms`
                    : probeText[altProbes[i]!.verdict]}
                </span>
              )}
            </Button>
            {!fromEntry && (
              <Button
                variant="ghost"
                size="sm"
                className="h-8 w-8 flex-none px-0 text-mut hover:text-ink"
                aria-label={t("addProvider.removeEndpoint")}
                title={t("addProvider.removeEndpoint")}
                onClick={() => {
                  setAltEndpoints((rs) => rs.filter((_, j) => j !== i));
                  setAltProbes({});
                }}
              >
                <XIcon className="h-3.5 w-3.5" />
              </Button>
            )}
          </div>
        ))}
        {/* A provider typed in by hand declares its own endpoints: one row
            per protocol it answers on. A catalog entry has already said
            which those are, so the button is not there while it owns the
            form — and with two protocols there is nothing to add once both
            are covered. */}
        {!fromEntry && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2 text-[11.5px] text-mut hover:text-ink"
            disabled={protocolOptions.every((p) => supportedProtocols.has(p.id))}
            onClick={() =>
              setAltEndpoints((rs) => [
                ...rs,
                { protocol: nextFreeProtocol(), endpoint: "" },
              ])
            }
          >
            <Plus className="h-3.5 w-3.5" />
            {t("addProvider.addEndpoint")}
          </Button>
        )}
      </div>
    </div>
  );
}
