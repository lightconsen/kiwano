// Models: Kiwano Hub cloud catalog as a sortable table (desktop-tool density)
import { useEffect, useMemo, useState } from "react";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import type { CatalogEntry, Protocol } from "../api/types";
import { Logo } from "../components/bits";

const CHIPS: { id: "all" | CatalogEntry["tag"]; label: string }[] = [
  { id: "all", label: "全部" },
  { id: "official", label: "官方" },
  { id: "aggregate", label: "聚合" },
  { id: "third", label: "三方" },
  { id: "free", label: "有免费额度" },
];

function tagChipStyle(tag: CatalogEntry["tag"]): React.CSSProperties {
  switch (tag) {
    case "official":
      return { background: "var(--kiwi-soft)", color: "var(--kiwi)" };
    case "free":
      return { background: "oklch(0.8 0.15 85 / .12)", color: "oklch(0.82 0.13 85)" };
    case "aggregate":
      return { background: "oklch(0.62 0.16 300 / .18)", color: "oklch(0.75 0.14 300)" };
    case "third":
      return { background: "oklch(0.62 0.14 250 / .16)", color: "oklch(0.72 0.12 250)" };
    case "local":
      return { background: "var(--surface2)", color: "var(--mut)" };
  }
}

const PROTO_STYLE: Record<Protocol, React.CSSProperties> = {
  anthropic: { background: "oklch(0.68 0.11 55 / .14)", color: "oklch(0.72 0.11 55)" },
  openai: { background: "var(--surface2)", color: "var(--mut)" },
  gemini: { background: "oklch(0.62 0.14 260 / .16)", color: "oklch(0.7 0.13 260)" },
};

const TAG_RANK: Record<CatalogEntry["tag"], number> = {
  official: 0,
  aggregate: 1,
  third: 2,
  free: 3,
  local: 4,
};

type SortKey = "name" | "protocol" | "tag" | "endpoint";

const COLUMNS: { key: SortKey | null; label: string; className: string }[] = [
  { key: "name", label: "名称", className: "w-[190px]" },
  { key: "protocol", label: "协议", className: "w-[86px]" },
  { key: "tag", label: "分类", className: "w-[64px]" },
  { key: "endpoint", label: "端点", className: "" },
  { key: null, label: "价格", className: "w-[92px]" },
  { key: null, label: "操作", className: "w-[70px] text-right" },
];

function compare(key: SortKey, a: CatalogEntry, b: CatalogEntry): number {
  switch (key) {
    case "name":
      return a.name.localeCompare(b.name, "zh-Hans-CN");
    case "protocol":
      return a.protocol.localeCompare(b.protocol) || a.name.localeCompare(b.name, "zh-Hans-CN");
    case "tag":
      return TAG_RANK[a.tag] - TAG_RANK[b.tag] || a.name.localeCompare(b.name, "zh-Hans-CN");
    case "endpoint":
      return a.endpoint.localeCompare(b.endpoint);
  }
}

function Row({ entry, onAdd }: { entry: CatalogEntry; onAdd: (e: CatalogEntry) => void }) {
  return (
    <tr className="border-t border-line hover:bg-surface2">
      <td className="px-4 py-2">
        <div className="flex items-center gap-2">
          <Logo char={entry.logo_char} color={entry.logo_color} border={entry.logo_border} size="w-6 h-6 text-[11px]" />
          <span className="truncate text-[12.5px] font-semibold" title={entry.name}>
            {entry.name}
          </span>
        </div>
      </td>
      <td className="px-2 py-2">
        <span className="rounded px-1.5 py-0.5 font-mono text-[10px]" style={PROTO_STYLE[entry.protocol]}>
          {entry.protocol}
        </span>
      </td>
      <td className="px-2 py-2">
        <span className="rounded px-1.5 text-[10px]" style={tagChipStyle(entry.tag)}>
          {entry.tag_label}
        </span>
      </td>
      <td className="px-2 py-2">
        <span className="block max-w-[300px] truncate font-mono text-[11px] text-mut" title={entry.endpoint}>
          {entry.endpoint.replace(/^https?:\/\//, "")}
        </span>
      </td>
      <td className="px-2 py-2 text-[11.5px] text-mut">{entry.price_line}</td>
      <td className="px-4 py-2 text-right">
        {entry.added ? (
          <span className="text-[10.5px] text-mut">已添加</span>
        ) : (
          <Button size="xs" className="rounded px-2 text-[10.5px] font-semibold" onClick={() => onAdd(entry)}>
            {entry.tag === "local" ? "+ 连接" : "+ 添加"}
          </Button>
        )}
      </td>
    </tr>
  );
}

export default function Shelf({ onAdd }: { onAdd: (preset: CatalogEntry) => void }) {
  const [catalog, setCatalog] = useState<{ total: number; entries: CatalogEntry[] } | null>(null);
  const [chip, setChip] = useState<(typeof CHIPS)[number]["id"]>("all");
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 } | null>(null);

  useEffect(() => {
    api.listCatalog().then(setCatalog);
  }, []);

  const filtered = useMemo(() => {
    if (!catalog) return [];
    const rows = catalog.entries.filter(
      (e) =>
        (chip === "all" || e.tag === chip) &&
        e.name.toLowerCase().includes(query.trim().toLowerCase()),
    );
    if (!sort) return rows;
    return [...rows].sort((a, b) => compare(sort.key, a, b) * sort.dir);
  }, [catalog, chip, query, sort]);

  if (!catalog) return <div className="p-8 text-center text-[12px] text-mut">加载中…</div>;

  const toggleSort = (key: SortKey) =>
    setSort((s) => (s?.key === key ? (s.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 }));

  return (
    <section>
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <span className="text-[12px] text-mut">
          来自 Kiwano Hub · <span className="font-mono">{catalog.total}</span> 个供应商
        </span>
        <div className="ml-3 flex gap-1.5">
          {CHIPS.map((c) => (
            <button
              key={c.id}
              className={`chip h-[26px] rounded-full border border-line px-2.5 text-[11.5px]${chip === c.id ? " active" : " text-mut"}`}
              onClick={() => setChip(c.id)}
            >
              {c.label}
            </button>
          ))}
        </div>
        <div className="relative ml-auto">
          <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-mut" />
          <input
            className="h-7 w-[190px] rounded-md border border-line bg-surface pl-7 pr-2 text-[12px]"
            placeholder="搜索供应商…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </div>
      <table className="w-full border-collapse">
        <thead className="sticky top-0 z-10 bg-surface">
          <tr className="border-b border-line">
            {COLUMNS.map((c) => (
              <th
                key={c.label}
                className={`px-2 py-1.5 text-left text-[11px] font-medium text-mut ${c.className} ${c.key ? "cursor-pointer select-none hover:text-foreground" : ""} ${c.key === "name" ? "pl-4" : ""} ${c.key === null && c.label === "操作" ? "pr-4" : ""}`}
                onClick={c.key ? () => toggleSort(c.key!) : undefined}
              >
                {c.label}
                {sort?.key === c.key && (
                  <span className="ml-0.5 text-[9px]">{sort.dir === 1 ? "▲" : "▼"}</span>
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {filtered.map((e) => (
            <Row key={e.id} entry={e} onAdd={onAdd} />
          ))}
          {filtered.length === 0 && (
            <tr>
              <td colSpan={COLUMNS.length} className="px-4 py-8 text-center text-[12px] text-mut">
                没有匹配的供应商
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
