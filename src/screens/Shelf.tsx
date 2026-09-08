// Models: Kiwano Hub cloud catalog (design/index.html #s-shelf)
import { useEffect, useMemo, useState } from "react";
import { Search, Star } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import type { CatalogEntry } from "../api/types";
import { BillTag, Logo } from "../components/bits";

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

function Card({ entry, onAdd }: { entry: CatalogEntry; onAdd: (e: CatalogEntry) => void }) {
  return (
    <div className="pcard rounded-lg border border-line bg-surface p-3">
      <div className="flex items-center gap-2">
        <Logo char={entry.logo_char} color={entry.logo_color} border={entry.logo_border} size="w-7 h-7 text-[12px]" />
        <span className="text-[12.5px] font-semibold">{entry.name}</span>
        <span className="rounded px-1.5 text-[10px]" style={tagChipStyle(entry.tag)}>
          {entry.tag_label}
        </span>
        <span className="ml-auto flex items-center gap-1 text-[11px] text-mut">
          <Star className="h-3 w-3" style={{ color: "oklch(0.8 0.15 85)" }} />
          {entry.rating}
        </span>
      </div>
      <div className="mt-2 flex items-center gap-2 font-mono text-[11.5px]">
        {entry.price_line}
        {entry.price_note && <span className="text-[10px] text-mut">{entry.price_note}</span>}
        <span className="ml-auto">
          <BillTag billing={entry.billing} />
        </span>
      </div>
      <div className="mt-2.5 flex items-center justify-between border-t border-line pt-2.5">
        <span className="text-[10.5px] text-mut">{entry.users}</span>
        {entry.added ? (
          <span className="rounded border border-line px-1.5 py-0.5 text-[10.5px] text-mut">已添加</span>
        ) : (
          <Button size="xs" className="rounded px-2 text-[10.5px] font-semibold" onClick={() => onAdd(entry)}>
            {entry.tag === "local" ? "+ 连接" : "+ 添加"}
          </Button>
        )}
      </div>
    </div>
  );
}

export default function Shelf({ onAdd }: { onAdd: (preset: CatalogEntry) => void }) {
  const [catalog, setCatalog] = useState<{ total: number; entries: CatalogEntry[] } | null>(null);
  const [chip, setChip] = useState<(typeof CHIPS)[number]["id"]>("all");
  const [query, setQuery] = useState("");

  useEffect(() => {
    api.listCatalog().then(setCatalog);
  }, []);

  const filtered = useMemo(() => {
    if (!catalog) return [];
    return catalog.entries.filter(
      (e) =>
        (chip === "all" || e.tag === chip) &&
        e.name.toLowerCase().includes(query.trim().toLowerCase()),
    );
  }, [catalog, chip, query]);

  if (!catalog) return <div className="p-8 text-center text-[12px] text-mut">加载中…</div>;

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
      <div className="grid grid-cols-3 gap-2.5 p-4">
        {filtered.map((e) => (
          <Card key={e.id} entry={e} onAdd={onAdd} />
        ))}
      </div>
    </section>
  );
}
