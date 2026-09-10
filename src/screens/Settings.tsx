// Settings (design/index.html #s-settings)
import { useEffect, useState } from "react";
import { Check, ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { api } from "../api/client";
import { onUpdateAvailable } from "../lib/updateEvents";
import type { AppSettings, UpdateInfo } from "../api/types";

function Row({ label, note, children }: { label: React.ReactNode; note?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between">
      <span>
        {label}
        {note && <span className="text-[10.5px] text-mut"> {note}</span>}
      </span>
      {children}
    </div>
  );
}

export default function Settings() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [currencies, setCurrencies] = useState<string[]>([]);
  const [version, setVersion] = useState("");
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [upToDate, setUpToDate] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [syncNote, setSyncNote] = useState<string | null>(null);
  const [syncErr, setSyncErr] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then(setS);
    api.getCurrencyMeta().then((m) => setCurrencies(m.currencies)).catch(() => {});
    api.getFooterStats().then((f) => setVersion(f.version)).catch(() => {});

    // About is the one place that answers "is there an update?", so it reads
    // what the silent startup check found — and re-reads when the check finds
    // one while this page is already open.
    const pullPendingUpdate = () => {
      api
        .getPendingUpdate()
        .then((u) => {
          if (u) setUpdate(u);
        })
        .catch(() => {});
    };
    pullPendingUpdate();
    const offUpdate = onUpdateAvailable(pullPendingUpdate);

    const offProgress = api.onUpdateProgress((p) => {
      setProgress(p.total ? Math.round((p.downloaded / p.total) * 100) : null);
    });
    return () => {
      offUpdate();
      offProgress.then((off) => off()).catch(() => {});
    };
  }, []);

  // "Up to date" is a confirmation, not a mode: it clears itself.
  useEffect(() => {
    if (!upToDate) return;
    const t = setTimeout(() => setUpToDate(false), 5000);
    return () => clearTimeout(t);
  }, [upToDate]);

  if (!s) return <div className="p-8 text-center text-[12px] text-mut">Loading…</div>;

  const patch = (p: Partial<AppSettings>) => {
    api.updateSettings(p).then(setS);
  };

  const checkUpdate = () => {
    setChecking(true);
    setErr(null);
    setUpdate(null);
    setUpToDate(false);
    api
      .checkAppUpdate()
      .then((u) => {
        setUpdate(u);
        // Silence used to be the answer to "no update"; say so instead.
        if (!u) setUpToDate(true);
      })
      .catch((e) => setErr(String(e)))
      .finally(() => setChecking(false));
  };

  // Conditional sync: the backend compares the Hub manifest's sha256 with the
  // cached artifacts and skips the download of whichever is unchanged.
  const syncNow = () => {
    setSyncing(true);
    setSyncErr(null);
    setSyncNote(null);
    api
      .syncHub()
      .then((r) => {
        const bits = [
          r.unchanged ? `Catalog up to date · ${r.fetched} providers` : `Synced ${r.fetched} providers`,
        ];
        if (r.pricing_version != null && !r.pricing_unchanged) {
          bits.push(`pricing v${r.pricing_version}`);
        }
        setSyncNote(bits.join(" · "));
      })
      .catch((e) => setSyncErr(String(e)))
      .finally(() => setSyncing(false));
  };

  const installUpdate = () => {
    setInstalling(true);
    setProgress(0);
    setErr(null);
    api.downloadAndInstallAppUpdate().catch((e) => {
      setErr(String(e));
      setInstalling(false);
    });
  };

  return (
    <section className="space-y-3 p-4">
      {/* General */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">General</h3>
        <div className="space-y-2.5 text-[12.5px]">
          {/* Language / theme ship with a single supported value — the selects
              stay (visual consistency, desktop-tool convention) but disabled
              until i18n and theming actually land. */}
          <Row label="Language" note="English only">
            <Select value="en" disabled>
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="en">English</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label="Theme" note="Dark only">
            <Select value="dark" disabled>
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="dark">Dark</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label="Launch at login">
            <Switch checked={s.autostart} onCheckedChange={(v) => patch({ autostart: v })} />
          </Row>
          <Row label="Minimize to tray on close">
            <Switch checked={s.close_to_tray} onCheckedChange={(v) => patch({ close_to_tray: v })} />
          </Row>
        </div>
      </div>

      {/* Local gateway */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">Local gateway</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <div className="pb-0.5 pt-1 text-[11px] font-medium text-mut">
            Agent takeover <span className="font-normal">· hot-switching once pointed at the local gateway</span>
          </div>
          {s.takeovers.map((t) => (
            <div key={t.agent} className="flex items-center justify-between">
              <span className="flex items-center gap-2">
                {t.label}{" "}
                {t.additive && (
                  <span className="text-[10px] text-mut">Coexist · multi-provider</span>
                )}
                {t.placeholder_key ? (
                  <span className="text-[10px] font-mono text-mut" title="Placeholder key assigned by the gateway, used for request attribution">
                    {t.placeholder_key}
                  </span>
                ) : (
                  <span className="text-[10px] font-mono text-mut">—</span>
                )}
              </span>
              <span className="flex items-center gap-2">
                <span className="text-[10.5px]" style={t.enabled ? { color: "var(--kiwi)" } : { color: "var(--mut)" }}>
                  {t.enabled ? "Taken over" : "Not taken over"}
                </span>
                {/* Takeover is performed in Apps/<agent> so the user walks the
                    onboarding flow (enable → route → config rewrite); this row
                    only navigates there (deep link #providers/<agent>). */}
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 px-2.5 text-[11px]"
                  onClick={() => {
                    window.location.hash = `providers/${t.agent}`;
                  }}
                >
                  {t.enabled ? "Manage in Apps" : "Enable in Apps"}
                </Button>
              </span>
            </div>
          ))}
          <Row label="Auto failover" note="Switch to a standby when the primary fails">
            <Switch checked={s.auto_failover} onCheckedChange={(v) => patch({ auto_failover: v })} />
          </Row>
          <Row label="Request logs" note="Record every request with bodies, local only">
            <Switch checked={s.request_logs} onCheckedChange={(v) => patch({ request_logs: v })} />
          </Row>
          <Row label="Log retention" note="Rows older than this are pruned every 6h">
            <Select
              value={String(s.log_retention_days ?? 30)}
              onValueChange={(v) => patch({ log_retention_days: Number(v) })}
            >
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="7">7 days</SelectItem>
                <SelectItem value="30">30 days</SelectItem>
                <SelectItem value="90">90 days</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label="Cost alert" note="System notification when a period limit is reached">
            <Switch checked={s.cost_alert} onCheckedChange={(v) => patch({ cost_alert: v })} />
          </Row>
          <Row label="Display currency" note="For cost cards and usage limits">
            <Select
              value={s.preferred_currency}
              onValueChange={(v) => patch({ preferred_currency: v ?? "CNY" })}
            >
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(currencies.length ? currencies : [s.preferred_currency]).map((c) => (
                  <SelectItem key={c} value={c}>
                    {c}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
        </div>
      </div>

      {/* Privacy pledge */}
      <div className="rounded-lg border p-4" style={{ background: "var(--kiwi-soft)", borderColor: "var(--kiwi-dim)" }}>
        <h3 className="mb-3 flex items-center gap-1.5 text-[12.5px] font-semibold" style={{ color: "var(--kiwi)" }}>
          <ShieldCheck className="h-3.5 w-3.5" />
          Privacy (Kiwano pledge)
        </h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="Anonymous usage reporting" note="Off by default · sanitized stats only">
            <Switch checked={s.telemetry} onCheckedChange={(v) => patch({ telemetry: v })} />
          </Row>
          <div className="space-y-1 text-[10.5px]" style={{ color: "var(--mut)" }}>
            <div className="flex items-center gap-1.5">
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
              API request content never passes through the Kiwano cloud
            </div>
            <div className="flex items-center gap-1.5">
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
              API keys never leave your device
            </div>
          </div>
        </div>
      </div>
      {/* Hub catalog — model/provider list source */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">Kiwano Hub</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="Catalog source">
            <span
              className="max-w-[280px] truncate font-mono text-[11px] text-mut"
              title={s.hub_url}
            >
              {s.hub_url}
            </span>
          </Row>
          <Row label="Catalog" note="skips the download when unchanged">
            <span className="flex items-center gap-2">
              {syncNote && <span className="text-[11px] text-mut">{syncNote}</span>}
              <Button
                size="sm"
                variant="outline"
                className="h-7 px-2.5 text-[11px]"
                onClick={syncNow}
                disabled={syncing}
              >
                {syncing ? "Syncing…" : "Sync now"}
              </Button>
            </span>
          </Row>
          {syncErr ? <div className="text-[11px] text-red-400">{syncErr}</div> : null}
        </div>
      </div>
      {/* About / update */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">About</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="Version">
            <span className="text-mut">{version || "—"}</span>
          </Row>
          <Row label="Auto-check for updates" note="Silent check at startup">
            <Switch checked={s.auto_check_update} onCheckedChange={(v) => patch({ auto_check_update: v })} />
          </Row>
          <Row label="Updates">
            <span className="flex items-center gap-2">
              {update ? (
                installing ? (
                  <>
                    <span
                      className="h-1 w-24 overflow-hidden rounded-full"
                      style={{ background: "var(--surface2)" }}
                    >
                      <span
                        className="block h-full rounded-full transition-[width] duration-200"
                        style={{ width: `${progress ?? 0}%`, background: "var(--kiwi)" }}
                      />
                    </span>
                    <span className="text-[11px] text-mut">
                      {progress == null ? "Downloading…" : `${progress}%`}
                    </span>
                  </>
                ) : (
                  <>
                    <span className="text-[11px]">v{update.version} available</span>
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-7 px-2.5 text-[11px]"
                      onClick={installUpdate}
                    >
                      {/* Same button, honest label: a failure lands back here. */}
                      {err ? "Retry download" : "Download & install"}
                    </Button>
                  </>
                )
              ) : (
                <>
                  {upToDate ? <span className="text-[11px] text-mut">Up to date</span> : null}
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-7 px-2.5 text-[11px]"
                    onClick={checkUpdate}
                    disabled={checking}
                  >
                    {checking ? "Checking…" : "Check for updates"}
                  </Button>
                </>
              )}
            </span>
          </Row>
          {update?.notes ? (
            <div className="whitespace-pre-wrap text-[11px] text-mut">{update.notes}</div>
          ) : null}
          {err ? <div className="text-[11px] text-red-400">{err}</div> : null}
        </div>
      </div>
    </section>
  );
}
