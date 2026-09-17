// App shell: overlay title bar + top nav + hash routing + footer status bar (design/index.html skeleton)
import { useCallback, useEffect, useState } from "react";
import { Check, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { api } from "./api/client";
import { UpdateBanner } from "./components/UpdateBanner";
import { applyTheme } from "./lib/theme";
import { ReloadRegistryProvider, useReloadRegistry } from "./lib/reload";
import { resolveLocale, setLocale, useLocale, useT, type KeyPath, type Messages } from "./i18n";
import { onOpenSettings } from "./lib/updateEvents";
import type { AgentRef } from "./api/types";
import { rememberCustomAgents } from "./lib/agents";
import type {
  AgentDetect,
  AgentId,
  CatalogEntry,
  FooterStats,
  GatewayStatus,
  Provider,
} from "./api/types";
import { Dot } from "./components/bits";
import { fmtMoney, fmtTokens } from "./lib/format";
import logoUrl from "./assets/kiwano-logo.svg";
import Providers from "./screens/Providers";
import Shelf from "./screens/Shelf";
import Dashboard from "./screens/Dashboard";
import Settings from "./screens/Settings";
import AddProviderModal from "./screens/AddProviderModal";

type Route = "providers" | "shelf" | "dashboard" | "settings";

/** Keys, not labels: this array is module-level and `t()` is a hook, so the
    label is resolved at render. */
const NAV: { id: Route; labelKey: KeyPath<Messages> }[] = [
  { id: "providers", labelKey: "app.nav.apps" },
  { id: "shelf", labelKey: "app.nav.models" },
  { id: "dashboard", labelKey: "app.nav.dashboard" },
  { id: "settings", labelKey: "app.nav.settings" },
];

/** Hash router: #settings etc., plus an optional deep-linked agent segment
    (#providers/openclaw) that the Apps screen adopts as its active tab.
    The id is *not* checked against the registry: a user-defined agent's tab is
    deep-linked the same way, and the Apps screen already falls back to "all"
    for an id nothing knows. */
function routeFromHash(): { route: Route; agent: AgentRef | null } {
  const [head, tail] = window.location.hash.slice(1).split("/");
  const route: Route = NAV.some((n) => n.id === head) ? (head as Route) : "providers";
  return { route, agent: tail ? decodeURIComponent(tail) : null };
}

export default function App() {
  const t = useT();
  const locale = useLocale();
  const [route, setRoute] = useState<{ route: Route; agent: AgentRef | null }>(routeFromHash);
  // The app's own reload: what the current screen re-reads, and whether one is
  // in flight (the button spins and disables for exactly as long as that is
  // true). It replaces a remount — see lib/reload.ts for why that was wrong.
  const { register: registerReload, reload: reloadScreen } = useReloadRegistry();
  const [reloading, setReloading] = useState(false);
  // Flashed once the reads land: this work is tens of milliseconds of local
  // reads, so the spinner is a blink and the screen looks identical afterwards
  // (nothing changed, or the data would have redrawn). The check is what tells
  // the two apart — and it clears itself, because it is an acknowledgement
  // rather than a state to read.
  const [reloaded, setReloaded] = useState(false);
  const [gw, setGw] = useState<GatewayStatus | null>(null);
  const [footer, setFooter] = useState<FooterStats | null>(null);
  const [modal, setModal] = useState<{ open: boolean; preset: CatalogEntry | null; edit: Provider | null }>({
    open: false,
    preset: null,
    edit: null,
  });
  const [agentDetect, setAgentDetect] = useState<AgentDetect[] | null>(null);
  const [agentVersions, setAgentVersions] = useState<Partial<Record<AgentId, string>>>({});
  // The user's display currency. The provider dialog seeds a new spending limit
  // with it — another local read would be a round trip, and the seed has to be
  // there before the first paint of that field.
  const [preferredCurrency, setPreferredCurrency] = useState("USD");

  useEffect(() => {
    const onHash = () => setRoute(routeFromHash());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // `<html lang>` follows the locale rather than the `index.html` default: it
  // drives per-script font fallback (the CJK chain in index.css), hyphenation
  // and screen-reader pronunciation.
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  // Settings own the theme. The pre-paint script in index.html only had the
  // cached copy, so this is what decides it for real. Same round trip keeps the
  // backend's day boundaries on the user's clock: `getTimezoneOffset()` counts
  // minutes to *subtract* to reach UTC, the backend wants minutes east of it.
  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        // The agent resolver is app-wide state (the provider dialog lists
        // agents, the Apps strip names them), and this is the one settings read
        // that happens before any screen mounts.
        rememberCustomAgents(s.custom_agents);
        applyTheme(s.theme);
        setPreferredCurrency(s.preferred_currency);
        // The stored preference is `"system"` until the user picks one, and it
        // resolves from the OS locale — so this has to run before the first
        // paint that shows text.
        setLocale(resolveLocale(s.language));
        const tz = -new Date().getTimezoneOffset();
        if (s.tz_offset_minutes !== tz) {
          api.updateSettings({ tz_offset_minutes: tz }).catch(() => {});
        }
      })
      .catch(() => {});
  }, []);

  // Tray → "Update to vX…": the About block reads the pending update on mount,
  // so routing to Settings is all it takes to show it.
  useEffect(
    () =>
      onOpenSettings(() => {
        window.location.hash = "#settings";
      }),
    [],
  );

  // The whole of the status bar's ⟳: the current screen's data, the gateway's
  // state, and these totals. Awaited together so the button is busy until the
  // last of them lands — the remount this replaces gave no such signal, and no
  // screen state survived it either.
  const refresh = useCallback(async () => {
    setReloading(true);
    try {
      await Promise.all([
        reloadScreen(),
        api.getGatewayStatus().then(setGw).catch(() => {}),
        api.getFooterStats().then(setFooter).catch(() => {}),
      ]);
      setReloaded(true);
    } finally {
      setReloading(false);
    }
  }, [reloadScreen]);

  // `timer`, not `t`: `t` is the translator here.
  //
  // The check says the reads came back, not that they succeeded — each half
  // swallows its own failure, by design, and a failure shows where it always
  // did: in what did *not* update. Here that is the gateway dot beside it, and
  // the screen's own rows.
  useEffect(() => {
    if (!reloaded) return;
    const timer = setTimeout(() => setReloaded(false), 1600);
    return () => clearTimeout(timer);
  }, [reloaded]);

  // The first load is this one's, not `refresh`'s: nothing is registered yet.
  useEffect(() => {
    api.getGatewayStatus().then(setGw).catch(() => {});
    api.getFooterStats().then(setFooter).catch(() => {});
  }, []);

  // Agent detection (phase 1 gates the Apps filter; phase 2 versions enrich
  // tooltips later). Probe failure → null → the Apps tab shows every agent.
  //
  // Run once at startup, and again on demand: the Apps refresh re-probes, so a
  // CLI installed after launch gets its tab without restarting the app. Phase 2
  // is one `--version` subprocess per agent (seconds, not milliseconds), which
  // is why it stays fire-and-forget — it fills tooltips, it is never a reason
  // to make anyone wait.
  // Returns what it found, resolved once the probe is done, so the caller that
  // asked can both show a spinner and read the answer — the Apps screen's own
  // refresh does the first, the declare dialog's "check again" the second. It is
  // the slowest part of that click by far: a login shell plus one subprocess per
  // agent. `null` is "the probe did not run", which is not the same answer as an
  // empty list and must not be read as "nothing is installed".
  const detectAgents = useCallback(() => {
    return api
      .detectAgents()
      .then((list) => {
        setAgentDetect(list);
        return api
          .probeAgentVersions()
          .then((vs) =>
            setAgentVersions(
              Object.fromEntries(vs.map((v) => [v.agent, v.version ?? ""])) as Partial<Record<AgentId, string>>,
            ),
          )
          .catch(() => {})
          .then(() => list);
      })
      .catch(() => {
        setAgentDetect(null);
        return null;
      });
  }, []);
  useEffect(() => {
    void detectAgents();
  }, [detectAgents]);

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
            a.unit === "plan_pct"
              ? t("app.usedPlanWindow", { pct: a.used })
              : a.unit === "wan_tokens"
                ? t("app.usedTokens", { count: (a.used * 10).toLocaleString() })
                : a.unit === "requests"
                  ? t("app.usedRequests", { count: a.used.toLocaleString() })
                  : fmtMoney(a.used, a.unit);
          sendNotification({
            title: a.unit === "plan_pct" ? t("app.notifyPlanTitle") : t("app.notifyCostTitle"),
            body:
              a.unit === "plan_pct"
                ? t("app.notifyPlanBody", {
                    provider: a.provider_name,
                    used,
                    limit: a.limit,
                  })
                : t("app.notifyCostBody", {
                    provider: a.provider_name,
                    used,
                    limit: a.limit,
                  }),
          });
        }
      } catch {
        // dev mode / gateway not running: stay silent
      }
    };
    check();
    // Not `t`: the translator is in scope here.
    const timer = setInterval(check, 60_000);
    return () => clearInterval(timer);
  }, []);

  const nav = (id: Route) => {
    setRoute({ route: id, agent: null });
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
              className={`navtab h-full px-3 text-[12.5px] font-medium text-mut${route.route === n.id ? " active" : ""}`}
              onClick={() => nav(n.id)}
            >
              {t(n.labelKey)}
            </button>
          ))}
        </nav>
      </header>

      <UpdateBanner />

      {/* overscroll-none: this is the app's only scroll region, and once it
          hits its end WebKit hands the remaining scroll to the document, which
          rubber-bands the whole 100vh view — status bar included, since a flex
          sibling cannot outrun its container. Chaining off, so the scroll stops
          where the content does. */}
      <main className="min-h-0 flex-1 overflow-y-auto overscroll-none">
        <ReloadRegistryProvider value={registerReload}>
        {route.route === "providers" && (
          <Providers
            agentDetect={agentDetect}
            agentVersions={agentVersions}
            onRedetect={detectAgents}
            initialAgent={route.agent}
            onAdd={() => setModal({ open: true, preset: null, edit: null })}
            onEdit={(p) => setModal({ open: true, preset: null, edit: p })}
          />
        )}
        {route.route === "shelf" && (
          <Shelf onAdd={(preset) => setModal({ open: true, preset, edit: null })} />
        )}
        {route.route === "dashboard" && <Dashboard gateway={gw} />}
        {route.route === "settings" && <Settings />}
        </ReloadRegistryProvider>
      </main>

      <footer
        className="flex h-[26px] items-center gap-4 border-t border-line px-4 text-[10.5px] text-mut"
        style={{ background: "var(--surface)" }}
      >
        {footer && (
          <span>
            {t("app.today")}{" "}
            <span className="font-mono text-ink">{footer.today_requests.toLocaleString()}</span>{" "}
            {t("app.requests")} ·{" "}
            <span className="font-mono text-ink">{fmtTokens(footer.today_tokens)}</span>{" "}
            {t("app.tokens")}
          </span>
        )}
        {/* The two readings the bar carries, in the order they are asked about:
            what this machine has done today, then when the catalog last agreed
            with the Hub. */}
        {footer?.hub_synced && (
          <span className="flex items-center gap-1">
            <Dot state="ok" size="h-[5px] w-[5px]" />
            {t("app.hubSynced")}
          </span>
        )}
        <span className="ml-auto flex items-center gap-2">
          {/* The gateway is the process that routes, so its state belongs with
              the totals rather than up in the title bar — the same move the
              reload made, and for the same reason: up there it was a badge in a
              row of controls that do not touch it. */}
          <span className="flex items-center gap-1.5">
            <Dot state={gw?.running ? "ok" : "off"} size="h-[5px] w-[5px]" />
            {t("app.gateway")} <span className="font-mono">{gw ? `:${gw.port}` : "…"}</span>
          </span>
          {/* The app's own reload, beside the figures it reloads: the screen's
              data, the gateway's state, and these totals. Spins and disables
              while they are in flight, so the click has an end as well as a
              start. It lived in the header next to the gateway's own dot, where
              it read as that indicator's control rather than as the whole
              app's. */}
          <Button
            variant="ghost"
            size="icon-xs"
            className="text-mut"
            disabled={reloading}
            onClick={refresh}
            aria-label={t("common.refresh")}
            title={t("common.refresh")}
          >
            {reloaded ? (
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
            ) : (
              <RefreshCw className={`h-3 w-3${reloading ? " animate-spin" : ""}`} />
            )}
          </Button>
        </span>
      </footer>

      <AddProviderModal
        open={modal.open}
        preset={modal.preset}
        edit={modal.edit}
        preferredCurrency={preferredCurrency}
        onClose={() => setModal({ open: false, preset: null, edit: null })}
        onSaved={refresh}
      />
    </div>
  );
}
