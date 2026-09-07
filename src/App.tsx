// App 壳：Overlay 标题栏 + 顶部导航 + hash 路由 + 底部状态栏（design/index.html 骨架）
import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { api } from "./api/client";
import type { CatalogEntry, FooterStats, GatewayStatus } from "./api/types";
import { Dot } from "./components/bits";
import Providers from "./screens/Providers";
import Shelf from "./screens/Shelf";
import Dashboard from "./screens/Dashboard";
import Settings from "./screens/Settings";
import AddProviderModal from "./screens/AddProviderModal";

type Route = "providers" | "shelf" | "dashboard" | "settings";

const NAV: { id: Route; label: string }[] = [
  { id: "providers", label: "我的供应商" },
  { id: "shelf", label: "货架" },
  { id: "dashboard", label: "仪表盘" },
  { id: "settings", label: "设置" },
];

function routeFromHash(): Route {
  const h = window.location.hash.slice(1) as Route;
  return NAV.some((n) => n.id === h) ? h : "providers";
}

export default function App() {
  const [route, setRoute] = useState<Route>(routeFromHash);
  const [tick, setTick] = useState(0);
  const [gw, setGw] = useState<GatewayStatus | null>(null);
  const [footer, setFooter] = useState<FooterStats | null>(null);
  const [modal, setModal] = useState<{ open: boolean; preset: CatalogEntry | null }>({
    open: false,
    preset: null,
  });

  useEffect(() => {
    const onHash = () => setRoute(routeFromHash());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  const refresh = useCallback(() => {
    setTick((t) => t + 1);
    api.getGatewayStatus().then(setGw);
    api.getFooterStats().then(setFooter);
  }, []);

  useEffect(refresh, [refresh]);

  const nav = (id: Route) => {
    setRoute(id);
    window.location.hash = id;
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden" style={{ background: "var(--bg)" }}>
      {/* 标题栏（macOS Overlay：红绿灯由系统绘制，左侧预留 72px） */}
      <header
        data-tauri-drag-region
        className="flex h-[46px] select-none items-center gap-4 border-b border-line pl-[72px] pr-4"
        style={{ background: "var(--surface)" }}
      >
        <div className="flex items-center gap-1.5">
          <div
            className="flex h-[22px] w-[22px] items-center justify-center rounded-[7px] text-[11px] font-bold"
            style={{ background: "var(--kiwi)", color: "oklch(0.18 0.03 132)" }}
          >
            K
          </div>
          <span className="text-[13px] font-semibold">Kiwano</span>
        </div>
        <nav className="ml-2 flex h-full items-center gap-1">
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`navtab h-full px-3 text-[12.5px] font-medium text-mut${route === n.id ? " active" : ""}`}
              onClick={() => nav(n.id)}
            >
              {n.label}
            </button>
          ))}
        </nav>
        <div className="ml-auto flex items-center gap-2.5">
          <span className="flex items-center gap-1.5 text-[11.5px] text-mut">
            <Dot state={gw?.running ? "ok" : "off"} size="h-[6px] w-[6px]" />
            网关 <span className="font-mono">{gw ? `:${gw.port}` : "…"}</span>
          </span>
          <button className="btn btn-ghost rounded-md p-1.5 text-mut" onClick={refresh} aria-label="刷新">
            <RefreshCw className="h-3.5 w-3.5" />
          </button>
        </div>
      </header>

      <main className="min-h-0 flex-1 overflow-y-auto">
        {route === "providers" && (
          <Providers key={`p${tick}`} onAdd={() => setModal({ open: true, preset: null })} />
        )}
        {route === "shelf" && <Shelf key={`s${tick}`} onAdd={(preset) => setModal({ open: true, preset })} />}
        {route === "dashboard" && <Dashboard key={`d${tick}`} />}
        {route === "settings" && <Settings key={`c${tick}`} />}
      </main>

      <footer
        className="flex h-[26px] items-center gap-4 border-t border-line px-4 text-[10.5px] text-mut"
        style={{ background: "var(--surface)" }}
      >
        {footer && (
          <span>
            今日 <span className="font-mono text-ink">{footer.today_requests.toLocaleString()}</span> 请求 ·{" "}
            <span className="font-mono text-ink">¥{footer.today_cost}</span>
          </span>
        )}
        {footer?.hub_synced && (
          <span className="flex items-center gap-1">
            <Dot state="ok" size="h-[5px] w-[5px]" />
            Hub 刚刚同步
          </span>
        )}
        <span className="ml-auto">{footer?.version ?? "v0.1.0 · MVP"}</span>
      </footer>

      <AddProviderModal
        open={modal.open}
        preset={modal.preset}
        onClose={() => setModal({ open: false, preset: null })}
        onSaved={refresh}
      />
    </div>
  );
}
