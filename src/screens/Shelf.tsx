// Models: Kiwano Hub cloud catalog as a sortable table (desktop-tool density)
import { useEffect, useMemo, useState } from "react";
import { Boxes, Check, Gauge, Gift, Layers, RefreshCw, Search, ShieldCheck, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "../api/client";
import type { Billing, CatalogEntry, ProbeReport, Protocol } from "../api/types";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { hubAssetUrl, useHubUrl } from "../lib/hub";
import { useT, type KeyPath, type Messages, type Translate } from "../i18n";

// Chip icons speak the category's meaning; colors match the per-tag badge
// palette used in the table (tagChipStyle below). The labels are keys, not
// text: every map below is module-level, where no hook can run.
const CHIPS: {
  id: "all" | CatalogEntry["tag"];
  labelKey: KeyPath<Messages>;
  icon?: LucideIcon;
  icon_color?: string;
}[] = [
  { id: "all", labelKey: "shelf.chipAll" },
  { id: "official", labelKey: "shelf.chipOfficial", icon: ShieldCheck, icon_color: "var(--kiwi)" },
  { id: "aggregate", labelKey: "shelf.chipAggregate", icon: Layers, icon_color: "var(--violet)" },
  { id: "third", labelKey: "shelf.chipThird", icon: Boxes, icon_color: "var(--blue)" },
  { id: "free", labelKey: "shelf.chipFree", icon: Gift, icon_color: "var(--amber)" },
];

function tagChipStyle(tag: CatalogEntry["tag"]): React.CSSProperties {
  switch (tag) {
    case "official":
      return { background: "var(--kiwi-soft)", color: "var(--kiwi)" };
    case "free":
      return { background: "var(--amber-soft)", color: "var(--amber)" };
    case "aggregate":
      return { background: "var(--violet-soft)", color: "var(--violet)" };
    case "third":
      return { background: "var(--blue-soft)", color: "var(--blue)" };
    case "local":
      return { background: "var(--surface2)", color: "var(--mut)" };
  }
}

const PROTO_STYLE: Record<Protocol, React.CSSProperties> = {
  anthropic: { background: "var(--orange-soft)", color: "var(--orange)" },
  openai: { background: "var(--surface2)", color: "var(--mut)" },
  gemini: { background: "var(--indigo-soft)", color: "var(--indigo)" },
};

const TAG_RANK: Record<CatalogEntry["tag"], number> = {
  official: 0,
  aggregate: 1,
  third: 2,
  free: 3,
  local: 4,
};

const BILLING_LABEL: Record<Billing, KeyPath<Messages>> = {
  plan: "shelf.billingPlan",
  payg: "shelf.billingPayg",
  unl: "shelf.billingUnl",
};

/** Hub catalogs may carry a billing tag this build predates: show the raw
    tag rather than a blank cell. */
function billingLabel(billing: Billing, t: Translate): string {
  // The lookup can miss at runtime despite the `Record` type: a newer Hub
  // catalog may name a billing tag this build has never heard of.
  return BILLING_LABEL[billing] ? t(BILLING_LABEL[billing]) : billing;
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
function EndpointCard({ protocol, endpoint, models }: { protocol: Protocol; endpoint: string; models: string[] }) {
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

type SortKey = "name" | "protocol" | "tag";

const COLUMNS: { key: SortKey | null; labelKey: KeyPath<Messages>; className: string }[] = [
  { key: "name", labelKey: "shelf.colName", className: "w-[150px]" },
  { key: "protocol", labelKey: "shelf.colProtocol", className: "w-[190px]" },
  { key: "tag", labelKey: "shelf.colCategory", className: "w-[84px]" },
  { key: null, labelKey: "shelf.colPrice", className: "" },
  { key: null, labelKey: "shelf.colActions", className: "w-[70px] text-right" },
];

function compare(key: SortKey, a: CatalogEntry, b: CatalogEntry): number {
  switch (key) {
    case "name":
      return a.name.localeCompare(b.name);
    case "protocol":
      return a.protocol.localeCompare(b.protocol) || a.name.localeCompare(b.name);
    case "tag":
      return TAG_RANK[a.tag] - TAG_RANK[b.tag] || a.name.localeCompare(b.name);
  }
}

function Row({
  entry,
  hubUrl,
  onAdd,
  onOpen,
}: {
  entry: CatalogEntry;
  hubUrl: string | null;
  onAdd: (e: CatalogEntry) => void;
  onOpen: (e: CatalogEntry) => void;
}) {
  const t = useT();
  const logo = entry.logo && hubUrl ? hubAssetUrl(hubUrl, entry.logo) : undefined;
  return (
    <tr className="cursor-pointer border-t border-line hover:bg-surface2" onClick={() => onOpen(entry)}>
      <td className="px-4 py-2">
        <div className="flex items-center gap-2">
          <ProviderLogo logo={logo} icon={entry.icon} name={entry.name} color={entry.logo_color} />
          <span className="truncate text-[12.5px] font-semibold" title={entry.name}>
            {entry.name}
          </span>
        </div>
      </td>
      <td className="px-2 py-2">
        {/* All supported protocols on one horizontal line (endpoint URLs
            live in the detail dialog, not the table) */}
        <div className="flex items-center gap-1">
          <span className="rounded px-1.5 py-0.5 font-mono text-[10px]" style={PROTO_STYLE[entry.protocol]}>
            {entry.protocol}
          </span>
          {(entry.endpoints ?? []).map((e) => (
            <span key={e.protocol} className="rounded px-1.5 py-0.5 font-mono text-[10px]" style={PROTO_STYLE[e.protocol]}>
              {e.protocol}
            </span>
          ))}
        </div>
      </td>
      <td className="px-2 py-2">
        <span className="rounded px-1.5 text-[10px]" style={tagChipStyle(entry.tag)}>
          {entry.tag_label}
        </span>
      </td>
      <td className="px-2 py-2 text-[11.5px] text-mut">{entry.price_line}</td>
      <td className="px-4 py-2 text-right" onClick={(e) => e.stopPropagation()}>
        {entry.added ? (
          <span className="text-[10.5px] text-mut">{t("shelf.added")}</span>
        ) : (
          <Button size="xs" className="rounded px-2 text-[10.5px] font-semibold" onClick={() => onAdd(entry)}>
            {entry.tag === "local" ? t("shelf.connect") : t("shelf.add")}
          </Button>
        )}
      </td>
    </tr>
  );
}

/** Model detail dialog: clicking a catalog row opens the full listing —
    every per-protocol endpoint with its models, pricing and an Add shortcut. */
function DetailDialog({
  entry,
  hubUrl,
  onClose,
  onAdd,
}: {
  entry: CatalogEntry;
  hubUrl: string | null;
  onClose: () => void;
  onAdd: (e: CatalogEntry) => void;
}) {
  const t = useT();
  const logo = entry.logo && hubUrl ? hubAssetUrl(hubUrl, entry.logo) : undefined;
  const endpoints = [
    { protocol: entry.protocol, endpoint: entry.endpoint, models: entry.models },
    ...(entry.endpoints ?? []),
  ];
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      {/* overflow-x-hidden: same WKWebView hardening as the other modals */}
      <DialogContent className="max-h-[min(600px,100dvh)] w-[calc(100%-2rem)] max-w-[480px] gap-0 overflow-x-hidden overflow-y-auto rounded-xl p-0 sm:max-w-[480px]">
        <DialogHeader className="flex h-11 flex-row items-center justify-between border-b border-line pl-4 pr-12">
          <DialogTitle className="flex items-center gap-2 text-[13px] font-semibold">
            <ProviderLogo logo={logo} icon={entry.icon} name={entry.name} color={entry.logo_color} size={18} />
            {entry.name}
          </DialogTitle>
        </DialogHeader>

        <div className="px-4 py-3.5">
          {/* hero: category · rating · price */}
          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded px-1.5 text-[10px]" style={tagChipStyle(entry.tag)}>
              {entry.tag_label}
            </span>
            <span className="text-[11.5px] text-mut">★ {entry.rating.toFixed(1)}</span>
            <span className="ml-auto text-[12px] font-medium">{entry.price_line}</span>
            {entry.price_note && <span className="text-[10.5px] text-mut">{entry.price_note}</span>}
          </div>

          {(entry.blurb || entry.free_offer) && (
            <p className="mt-2 text-[11.5px] leading-relaxed text-mut">
              {entry.free_offer && (
                <span className="mr-2 inline-block rounded px-1.5 py-0.5 text-[10.5px]" style={tagChipStyle("free")}>
                  {entry.free_offer}
                </span>
              )}
              {entry.blurb}
            </p>
          )}

          <div className="mt-2.5 flex gap-4 text-[11.5px] text-mut">
            <span>
              {t("shelf.billing")}{" "}
              <span className="text-ink">{billingLabel(entry.billing, t)}</span>
            </span>
            <span>{entry.users}</span>
          </div>

          {/* one card per protocol: endpoint URL + keyless Test + models */}
          <div className="mt-3">
            <div className="mb-1 text-[10px] font-medium text-mut">{t("shelf.endpoints")}</div>
            <div className="space-y-1.5">
              {endpoints.map((e) => (
                <EndpointCard key={e.protocol} protocol={e.protocol} endpoint={e.endpoint} models={e.models} />
              ))}
            </div>
          </div>
        </div>

        <DialogFooter className="mx-0 mb-0 flex-row justify-end gap-2 rounded-b-xl border-t border-line bg-transparent px-4 py-3">
          {entry.added && (
            <span className="mr-auto self-center text-[11px] text-mut">
              {t("shelf.alreadyAdded")}
            </span>
          )}
          <Button variant="outline" size="sm" onClick={onClose}>
            {t("common.close")}
          </Button>
          {!entry.added && (
            <Button size="sm" className="font-semibold" onClick={() => onAdd(entry)}>
              {entry.tag === "local" ? t("shelf.connect") : t("shelf.add")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export default function Shelf({ onAdd }: { onAdd: (preset: CatalogEntry) => void }) {
  const t = useT();
  const hubUrl = useHubUrl();
  const [catalog, setCatalog] = useState<{ total: number; entries: CatalogEntry[] } | null>(null);
  const [chip, setChip] = useState<(typeof CHIPS)[number]["id"]>("all");
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 } | null>(null);
  // Catalog row clicked → model detail dialog
  const [detail, setDetail] = useState<CatalogEntry | null>(null);
  // Hub refresh: "busy" spins the glyph, "done" flashes a check. The header
  // bar has no room for a toast, so success is the icon and only a failure
  // takes up words.
  const [hub, setHub] = useState<"idle" | "busy" | "done">("idle");
  const [hubErr, setHubErr] = useState<string | null>(null);

  useEffect(() => {
    api.listCatalog().then(setCatalog);
  }, []);

  /** Pull the Hub catalog into the local cache, then re-read it. The sync is
      conditional on the Hub's side (manifest sha): a Hub that has not changed
      is still a successful check, it just skips the download. */
  const refreshFromHub = () => {
    setHub("busy");
    setHubErr(null);
    api
      .syncHub()
      .then(() => api.listCatalog())
      .then((c) => {
        setCatalog(c);
        setHub("done");
      })
      .catch((e) => {
        setHubErr(String(e));
        setHub("idle");
      });
  };

  // The check is an acknowledgement, not a state to read: fall back to the
  // refresh glyph. An error has no such timer — it holds until the next try.
  useEffect(() => {
    if (hub !== "done") return;
    // Named `timer`, not `t`: `t` is the translator here.
    const timer = setTimeout(() => setHub("idle"), 1600);
    return () => clearTimeout(timer);
  }, [hub]);

  const filtered = useMemo(() => {
    if (!catalog) return [];
    const rows = catalog.entries.filter(
      (e) =>
        (chip === "all" || e.tag === chip) &&
        e.name.toLowerCase().includes(query.trim().toLowerCase()),
    );
    const ordered = sort
      ? [...rows].sort((a, b) => compare(sort.key, a, b) * sort.dir)
      : rows;
    // Added (or connected) providers pin to the top of any ordering — they
    // are the ones already wired into the local setup
    return [...ordered].sort((a, b) => Number(b.added) - Number(a.added));
  }, [catalog, chip, query, sort]);

  if (!catalog)
    return <div className="p-8 text-center text-[12px] text-mut">{t("common.loading")}</div>;

  const toggleSort = (key: SortKey) =>
    setSort((s) => (s?.key === key ? (s.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 }));

  return (
    <section>
      <div className="sticky top-0 z-20 flex h-11 items-center gap-2 border-b border-line bg-bg px-4">
        <span className="text-[12px] text-mut">
          {t("shelf.fromHub", { n: catalog.total })}
        </span>
        <div className="ml-3 flex gap-1.5">
          {CHIPS.map((c) => (
            <button
              key={c.id}
              className={`chip inline-flex h-[26px] items-center gap-1 rounded-full border border-line px-2.5 text-[11.5px]${chip === c.id ? " active" : " text-mut"}`}
              onClick={() => setChip(c.id)}
            >
              {c.icon && <c.icon className="size-3" style={{ color: c.icon_color }} />}
              {t(c.labelKey)}
            </button>
          ))}
        </div>
        <div className="ml-auto flex items-center gap-2">
          {/* A failed sync is worth the room it takes; the search box shifts
              left to make it, which is the point. */}
          {hubErr && (
            <span
              className="max-w-[220px] truncate text-[11px]"
              style={{ color: "var(--red)" }}
              title={hubErr}
            >
              {hubErr}
            </span>
          )}
          <div className="relative">
            <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-mut" />
            <input
              className="h-7 w-[190px] rounded-md border border-line bg-surface pl-7 pr-2 text-[12px]"
              placeholder={t("shelf.searchPlaceholder")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 flex-none px-0 text-mut"
            aria-label={t("shelf.refreshAria")}
            title={t("shelf.refreshTitle")}
            disabled={hub === "busy"}
            onClick={refreshFromHub}
          >
            {hub === "done" ? (
              <Check className="h-3.5 w-3.5" style={{ color: "var(--kiwi)" }} />
            ) : (
              <RefreshCw className={`h-3.5 w-3.5${hub === "busy" ? " animate-spin" : ""}`} />
            )}
          </Button>
        </div>
      </div>
      <table className="w-full border-collapse">
        <thead className="sticky top-11 z-10 bg-surface">
          <tr className="border-b border-line">
            {COLUMNS.map((c) => (
              <th
                key={c.labelKey}
                className={`px-2 py-1.5 text-left text-[11px] font-medium text-mut ${c.className} ${c.key ? "cursor-pointer select-none hover:text-foreground" : ""} ${c.key === "name" ? "pl-4" : ""} ${c.key === null && c.labelKey === "shelf.colActions" ? "pr-4" : ""}`}
                onClick={c.key ? () => toggleSort(c.key!) : undefined}
              >
                {t(c.labelKey)}
                {sort?.key === c.key && (
                  <span className="ml-0.5 text-[9px]">{sort.dir === 1 ? "▲" : "▼"}</span>
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {filtered.map((e) => (
            <Row key={e.id} entry={e} hubUrl={hubUrl} onAdd={onAdd} onOpen={setDetail} />
          ))}
          {filtered.length === 0 && (
            <tr>
              <td colSpan={COLUMNS.length} className="px-4 py-8 text-center text-[12px] text-mut">
                {t("shelf.noMatches")}
              </td>
            </tr>
          )}
        </tbody>
      </table>
      {detail && (
        <DetailDialog
          entry={detail}
          hubUrl={hubUrl}
          onClose={() => setDetail(null)}
          onAdd={(e) => {
            setDetail(null);
            onAdd(e);
          }}
        />
      )}
    </section>
  );
}
