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
import { applyTheme } from "../lib/theme";
import { onUpdateAvailable } from "../lib/updateEvents";
import { startUpdateInstall, useUpdateInstall } from "../lib/updateInstall";
import {
  LANGUAGE_PREFS,
  languageLabel,
  resolveLocale,
  setLocale,
  useT,
  type LanguagePref,
} from "../i18n";
import type { AgentId, AppSettings, UpdateInfo } from "../api/types";

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
  const t = useT();
  const [s, setS] = useState<AppSettings | null>(null);
  const [currencies, setCurrencies] = useState<string[]>([]);
  const [version, setVersion] = useState("");
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  // Both the download state and its error belong to lib/updateInstall: the
  // banner can start the transfer before this screen mounts, and this screen
  // unmounts mid-download. `checkErr` stays local — it is the version check's,
  // a different failure the user retries with a different button.
  const install = useUpdateInstall();
  const [checkErr, setCheckErr] = useState<string | null>(null);
  const [upToDate, setUpToDate] = useState(false);
  const [takeoverBusy, setTakeoverBusy] = useState<string | null>(null);
  const [takeoverErr, setTakeoverErr] = useState<string | null>(null);
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

    // No progress subscription here: lib/updateInstall owns the one long-lived
    // listener, so a download started from the banner is already being tracked
    // by the time this screen mounts.
    return () => offUpdate();
  }, []);

  // Turning a takeover off is one call — the backend restores the agent's own
  // configuration. Turning one on is not: it walks the Apps onboarding
  // (enable → route → rewrite), so that direction stays a deep link.
  const disableTakeover = (agent: AgentId) => {
    setTakeoverBusy(agent);
    setTakeoverErr(null);
    api
      .setTakeover(agent, false)
      .then(() => api.getSettings().then(setS))
      .catch((e) => setTakeoverErr(`${agent}: ${String(e)}`))
      .finally(() => setTakeoverBusy(null));
  };

  // "Up to date" is a confirmation, not a mode: it clears itself.
  useEffect(() => {
    if (!upToDate) return;
    const timer = setTimeout(() => setUpToDate(false), 5000);
    return () => clearTimeout(timer);
  }, [upToDate]);

  if (!s) return <div className="p-8 text-center text-[12px] text-mut">{t("common.loading")}</div>;

  const patch = (p: Partial<AppSettings>) => {
    api.updateSettings(p).then(setS);
  };

  const checkUpdate = () => {
    setChecking(true);
    setCheckErr(null);
    setUpdate(null);
    setUpToDate(false);
    api
      .checkAppUpdate()
      .then((u) => {
        setUpdate(u);
        // Silence used to be the answer to "no update"; say so instead.
        if (!u) setUpToDate(true);
      })
      .catch((e) => setCheckErr(String(e)))
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
          r.unchanged
            ? t("settings.syncUnchanged", { count: r.fetched })
            : t("settings.syncSynced", { count: r.fetched }),
        ];
        if (r.pricing_version != null && !r.pricing_unchanged) {
          bits.push(t("settings.syncPricing", { version: r.pricing_version }));
        }
        setSyncNote(bits.join(" · "));
      })
      .catch((e) => setSyncErr(String(e)))
      .finally(() => setSyncing(false));
  };

  return (
    <section className="space-y-3 p-4">
      {/* General */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">{t("settings.general")}</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label={t("common.language")}>
            <Select
              value={s.language || "system"}
              onValueChange={(v) => {
                if (!v) return;
                // Apply now, persist after — the same order the theme switch
                // below uses, so the screen does not wait on a round trip to
                // change its own language.
                setLocale(resolveLocale(v));
                patch({ language: v });
              }}
            >
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                {/* A bare <SelectValue /> renders the raw value ("system"), not
                    the item's label. */}
                <SelectValue>
                  {(v) => languageLabel(t, (v as LanguagePref) ?? "system")}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {LANGUAGE_PREFS.map((pref) => (
                  <SelectItem key={pref} value={pref}>
                    {languageLabel(t, pref)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          {/* Costs are printed in this currency across the app, so it reads as
              a display preference — it sits with language, not with the
              usage-threshold settings it used to live among. */}
          <Row label={t("settings.displayCurrency")} note={t("settings.displayCurrencyNote")}>
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
          <Row label={t("settings.theme")}>
            <Select
              value={s.theme}
              onValueChange={(v) => {
                if (!v) return;
                // Apply now, persist after: the switch should feel immediate.
                applyTheme(v);
                patch({ theme: v });
              }}
            >
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                {/* Same reason as Language: the value is lowercase ("dark"). */}
                <SelectValue>
                  {(v) => (v === "light" ? t("settings.themeLight") : t("settings.themeDark"))}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="dark">{t("settings.themeDark")}</SelectItem>
                <SelectItem value="light">{t("settings.themeLight")}</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label={t("settings.launchAtLogin")}>
            <Switch checked={s.autostart} onCheckedChange={(v) => patch({ autostart: v })} />
          </Row>
          <Row label={t("settings.minimizeToTray")}>
            <Switch checked={s.close_to_tray} onCheckedChange={(v) => patch({ close_to_tray: v })} />
          </Row>
          {/* The app's own trail — sidecar starts, sync failures, panics — which
              a packaged app has nowhere to print. */}
          <Row label={t("settings.logs")} note={t("settings.logsNote")}>
            <Button
              variant="outline"
              size="sm"
              className="h-7 px-2.5 text-[11px]"
              onClick={() => api.openLogFolder().catch(() => {})}
            >
              {t("settings.openFolder")}
            </Button>
          </Row>
        </div>
      </div>

      {/* Local gateway */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">{t("settings.localGateway")}</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <div className="pb-0.5 pt-1 text-[11px] font-medium text-mut">
            {t("settings.agentTakeover")}{" "}
            <span className="font-normal">{t("settings.agentTakeoverNote")}</span>
          </div>
          {/* `tk`, not `t`: the translator is in scope here. */}
          {s.takeovers.map((tk) => (
            <div key={tk.agent} className="flex items-center justify-between">
              <span className="flex items-center gap-2">
                {tk.label}{" "}
                {tk.additive && (
                  <span className="text-[10px] text-mut">{t("settings.coexist")}</span>
                )}
                {tk.placeholder_key ? (
                  <span
                    className="text-[10px] font-mono text-mut"
                    title={t("settings.placeholderKeyTitle")}
                  >
                    {tk.placeholder_key}
                  </span>
                ) : (
                  <span className="text-[10px] font-mono text-mut">{t("common.none")}</span>
                )}
              </span>
              <span className="flex items-center gap-2">
                <span className="text-[10.5px]" style={tk.enabled ? { color: "var(--kiwi)" } : { color: "var(--mut)" }}>
                  {tk.enabled ? t("settings.takenOver") : t("settings.notTakenOver")}
                </span>
                {tk.enabled ? (
                  // Off is one call, so it lives here as a switch. Turning one
                  // back on is not (enable → route → rewrite in Apps), which is
                  // why that direction keeps its own button below.
                  <Switch
                    checked
                    disabled={takeoverBusy === tk.agent}
                    onCheckedChange={() => disableTakeover(tk.agent)}
                  />
                ) : (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 px-2.5 text-[11px]"
                    onClick={() => {
                      window.location.hash = `providers/${tk.agent}`;
                    }}
                  >
                    {t("settings.enableInApps")}
                  </Button>
                )}
              </span>
            </div>
          ))}
          {takeoverErr ? <div className="text-[11px] text-red-400">{takeoverErr}</div> : null}
          <Row label={t("settings.autoFailover")} note={t("settings.autoFailoverNote")}>
            <Switch checked={s.auto_failover} onCheckedChange={(v) => patch({ auto_failover: v })} />
          </Row>
          <Row label={t("settings.requestLogs")} note={t("settings.requestLogsNote")}>
            <Switch checked={s.request_logs} onCheckedChange={(v) => patch({ request_logs: v })} />
          </Row>
          <Row label={t("settings.logRetention")} note={t("settings.logRetentionNote")}>
            <Select
              value={String(s.log_retention_days ?? 30)}
              onValueChange={(v) => patch({ log_retention_days: Number(v) })}
            >
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                {/* Would otherwise read "30" rather than "30 days". */}
                <SelectValue>{(v) => t("settings.days", { count: v ?? "" })}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="7">{t("settings.days", { count: 7 })}</SelectItem>
                <SelectItem value="30">{t("settings.days", { count: 30 })}</SelectItem>
                <SelectItem value="90">{t("settings.days", { count: 90 })}</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label={t("settings.costAlert")} note={t("settings.costAlertNote")}>
            <Switch checked={s.cost_alert} onCheckedChange={(v) => patch({ cost_alert: v })} />
          </Row>
        </div>
      </div>

      {/* Privacy pledge */}
      <div className="rounded-lg border p-4" style={{ background: "var(--kiwi-soft)", borderColor: "var(--kiwi-dim)" }}>
        <h3 className="mb-3 flex items-center gap-1.5 text-[12.5px] font-semibold" style={{ color: "var(--kiwi)" }}>
          <ShieldCheck className="h-3.5 w-3.5" />
          {t("settings.privacyTitle")}
        </h3>
        <div className="space-y-2 text-[12.5px]">
          {/* No switch here. A toggle implies a pipeline behind it, and there
              is none: nothing is collected, so "off by default" was promising
              a choice that did not exist. */}
          <div className="text-mut">{t("settings.privacyUsage")}</div>
          <div className="space-y-1 text-[10.5px]" style={{ color: "var(--mut)" }}>
            <div className="flex items-center gap-1.5">
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
              {t("settings.privacyNoKeys")}
            </div>
            <div className="flex items-center gap-1.5">
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
              {t("settings.privacyKeysLocal")}
            </div>
          </div>
        </div>
      </div>
      {/* Hub catalog — model/provider list source */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">{t("settings.hub")}</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label={t("settings.catalog")} note={t("settings.catalogNote")}>
            <span className="flex items-center gap-2">
              {syncNote && <span className="text-[11px] text-mut">{syncNote}</span>}
              <Button
                size="sm"
                variant="outline"
                className="h-7 px-2.5 text-[11px]"
                onClick={syncNow}
                disabled={syncing}
              >
                {syncing ? t("settings.syncing") : t("settings.syncNow")}
              </Button>
            </span>
          </Row>
          {syncErr ? <div className="text-[11px] text-red-400">{syncErr}</div> : null}
        </div>
      </div>
      {/* About / update */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">{t("settings.about")}</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label={t("settings.version")}>
            <span className="text-mut">{version || t("common.none")}</span>
          </Row>
          <Row label={t("settings.autoCheckUpdates")} note={t("settings.autoCheckUpdatesNote")}>
            <Switch checked={s.auto_check_update} onCheckedChange={(v) => patch({ auto_check_update: v })} />
          </Row>
          <Row label={t("settings.updates")}>
            <span className="flex items-center gap-2">
              {/* Progress first, ahead of the version check: the banner can
                  start a download before this screen has read the pending
                  update back, and the in-flight transfer is the more specific
                  answer either way. */}
              {install.installing ? (
                <>
                  <span
                    className="h-1 w-24 overflow-hidden rounded-full"
                    style={{ background: "var(--surface2)" }}
                  >
                    <span
                      className="block h-full rounded-full transition-[width] duration-200"
                      style={{ width: `${install.progress ?? 0}%`, background: "var(--kiwi)" }}
                    />
                  </span>
                  <span className="text-[11px] text-mut">
                    {install.progress == null ? t("settings.downloading") : `${install.progress}%`}
                  </span>
                </>
              ) : update ? (
                <>
                  <span className="text-[11px]">
                    {t("settings.versionAvailable", { version: update.version })}
                  </span>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-7 px-2.5 text-[11px]"
                    onClick={startUpdateInstall}
                  >
                    {/* Same button, honest label: a failure lands back here. */}
                    {install.err ? t("settings.retryDownload") : t("settings.downloadInstall")}
                  </Button>
                </>
              ) : (
                <>
                  {upToDate ? <span className="text-[11px] text-mut">{t("settings.upToDate")}</span> : null}
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-7 px-2.5 text-[11px]"
                    onClick={checkUpdate}
                    disabled={checking}
                  >
                    {checking ? t("settings.checking") : t("settings.checkForUpdates")}
                  </Button>
                </>
              )}
            </span>
          </Row>
          {update?.notes ? (
            <div className="whitespace-pre-wrap text-[11px] text-mut">{update.notes}</div>
          ) : null}
          {install.err ?? checkErr ? (
            <div className="text-[11px] text-red-400">{install.err ?? checkErr}</div>
          ) : null}
        </div>
      </div>
    </section>
  );
}
