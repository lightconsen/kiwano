// App shell: overlay title bar + top nav + hash routing + footer status bar (design/index.html skeleton)
import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { api } from "./api/client";
import type {
  AgentDetect,
  AgentId,
  CatalogEntry,
  FooterStats,
  GatewayStatus,
  Provider,
} from "./api/types";
import { Dot } from "./components/bits";
import { fmtTokens } from "./lib/format";
import logoUrl from "./assets/kiwano-logo.svg";
import Providers from "./screens/Providers";
import Shelf from "./screens/Shelf";
import Dashboard from "./screens/Dashboard";
import Settings from "./screens/Settings";
import AddProviderModal from "./screens/AddProviderModal";

type Route = "providers" | "shelf" | "dashboard" | "settings";

const NAV: { id: Route; label: string }[] = [
  { id: "providers", label: "Apps" },
  { id: "shelf", label: "Models" },
  { id: "dashboard", label: "Dashboard" },
  { id: "settings", label: "Settings" },
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
  const [modal, setModal] = useState<{ open: boolean; preset: CatalogEntry | null; edit: Provider | null }>({
    open: false,
    preset: null,
    edit: null,
  });
  const [agentDetect, setAgentDetect] = useState<AgentDetect[] | null>(null);
  const [agentVersions, setAgentVersions] = useState<Partial<Record<AgentId, string>>>({});

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

  // Agent detection (phase 1 gates the Apps filter; phase 2 versions enrich
  // tooltips later). Probe failure → null → the Apps tab shows every agent.
  useEffect(() => {
    api
      .detectAgents()
      .then((list) => {
        setAgentDetect(list);
        api
          .probeAgentVersions()
          .then((vs) =>
            setAgentVersions(
              Object.fromEntries(vs.map((v) => [v.agent, v.version ?? ""])) as Partial<Record<AgentId, string>>,
            ),
          )
          .catch(() => {});
      })
      .catch(() => setAgentDetect(null));
  }, []);

  // Cost alert patrol (spec §4.1 P1): poll every 60s; the backend dedupes per period,
  // so a returned alert is the first hit of that period — forward it as a system notification.
  useEffect(() => {
    const check = async () => {
      try {
        const alerts = await api.checkUsageAlerts();
        for (const a of alerts) {
          if (!(await isPermissionGranted())) {
            const st = await requestPermission();
            if (st !== "granted") return;
          }
          const used =
            a.unit === "wan_tokens" ? `${(a.used * 10).toLocaleString()}k tokens` : `${a.used.toLocaleString()} requests`;
          sendNotification({
            title: "Kiwano cost alert",
            body: `${a.provider_name} used ${used} this period and has hit its limit of ${a.limit} — watch your spending`,
          });
        }
      } catch {
        // dev mode / gateway not running: stay silent
      }
    };
    check();
    const t = setInterval(check, 60_000);
    return () => clearInterval(t);
  }, []);

  const nav = (id: Route) => {
    setRoute(id);
    window.location.hash = id;
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden" style={{ background: "var(--bg)" }}>
      {/* Title bar (macOS overlay: traffic lights drawn by the system, 72px reserved on the left) */}
      <header
        data-tauri-drag-region
        className="flex h-[46px] select-none items-center gap-4 border-b border-line pl-[72px] pr-4"
        style={{ background: "var(--surface)" }}
      >
        <div className="flex items-center gap-1.5">
          <img src={logoUrl} alt="Kiwano" className="h-[22px] w-[22px] rounded-[6px]" />
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
            Gateway <span className="font-mono">{gw ? `:${gw.port}` : "…"}</span>
          </span>
          <Button variant="ghost" size="icon-sm" className="text-mut" onClick={refresh} aria-label="Refresh">
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
        </div>
      </header>

      <main className="min-h-0 flex-1 overflow-y-auto">
        {route === "providers" && (
          <Providers
            key={`p${tick}`}
            agentDetect={agentDetect}
            agentVersions={agentVersions}
            onAdd={() => setModal({ open: true, preset: null, edit: null })}
            onEdit={(p) => setModal({ open: true, preset: null, edit: p })}
          />
        )}
        {route === "shelf" && <Shelf key={`s${tick}`} onAdd={(preset) => setModal({ open: true, preset, edit: null })} />}
        {route === "dashboard" && <Dashboard key={`d${tick}`} />}
        {route === "settings" && <Settings key={`c${tick}`} />}
      </main>

      <footer
        className="flex h-[26px] items-center gap-4 border-t border-line px-4 text-[10.5px] text-mut"
        style={{ background: "var(--surface)" }}
      >
        {footer && (
          <span>
            Today <span className="font-mono text-ink">{footer.today_requests.toLocaleString()}</span> requests ·{" "}
            <span className="font-mono text-ink">{fmtTokens(footer.today_tokens)} tokens</span>
          </span>
        )}
        {footer?.hub_synced && (
          <span className="ml-auto flex items-center gap-1">
            <Dot state="ok" size="h-[5px] w-[5px]" />
            Hub just synced
          </span>
        )}
      </footer>

      <AddProviderModal
        open={modal.open}
        preset={modal.preset}
        edit={modal.edit}
        onClose={() => setModal({ open: false, preset: null, edit: null })}
        onSaved={refresh}
      />
    </div>
  );
}
