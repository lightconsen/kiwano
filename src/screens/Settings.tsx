// Settings (design/index.html #s-settings)
import { useEffect, useState } from "react";
import { Check, ChevronDown, ShieldCheck, SlidersHorizontal, User } from "lucide-react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { api } from "../api/client";
import type { AppSettings } from "../api/types";

const CONFIG_FILE_FILTERS = [{ name: "Kiwano config", extensions: ["json"] }];

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
  const [importMsg, setImportMsg] = useState<string | null>(null);
  const [shareMsg, setShareMsg] = useState<string | null>(null);

  const onExport = () => {
    save({
      defaultPath: "kiwano-config.json",
      filters: CONFIG_FILE_FILTERS,
    })
      .then((path) => {
        if (!path) return;
        setShareMsg("Exporting…");
        api
          .exportConfig(path)
          .then((n) => setShareMsg(`Exported ${n} providers (includes API keys — keep the file safe)`))
          .catch((e) => setShareMsg(`Export failed: ${String(e).slice(0, 60)}`))
          .finally(() => setTimeout(() => setShareMsg(null), 5000));
      })
      .catch(() => {});
  };

  const onImport = () => {
    open({ multiple: false, filters: CONFIG_FILE_FILTERS })
      .then((path) => {
        if (!path || Array.isArray(path)) return;
        setShareMsg("Importing…");
        api
          .importConfig(path)
          .then((r) =>
            setShareMsg(
              `Imported: ${r.providers_added} added · ${r.providers_kept} reused · ${r.routes_applied} routes`,
            ),
          )
          .catch((e) => setShareMsg(`Import failed: ${String(e).slice(0, 60)}`))
          .finally(() => setTimeout(() => setShareMsg(null), 5000));
      })
      .catch(() => {});
  };

  useEffect(() => {
    api.getSettings().then(setS);
  }, []);

  if (!s) return <div className="p-8 text-center text-[12px] text-mut">Loading…</div>;

  const patch = (p: Partial<AppSettings>) => {
    api.updateSettings(p).then(setS);
  };

  return (
    <section className="space-y-3 p-4">
      {/* General */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">General</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="Language">
            <Select value={s.language ?? "zh-CN"} onValueChange={(v) => patch({ language: v ?? "zh-CN" })}>
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="zh-CN">Chinese (Simplified)</SelectItem>
                <SelectItem value="en">English</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label="Theme">
            <Select value={s.theme ?? "dark"} onValueChange={(v) => patch({ theme: v ?? "dark" })}>
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="dark">Dark</SelectItem>
                <SelectItem value="light">Light</SelectItem>
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
          <Row label="Listen address">
            <Input
              className="h-7 w-[130px] bg-surface2 font-mono text-[11.5px] dark:bg-surface2"
              value={s.gateway_listen}
              onChange={(e) => patch({ gateway_listen: e.target.value })}
            />
          </Row>
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
                <Switch
                  checked={t.enabled}
                  onCheckedChange={(v) => {
                    api.setTakeover(t.agent, v).then(() => api.getSettings().then(setS));
                  }}
                />
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

      {/* Hub & config */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">Kiwano Hub & config</h3>
        <div className="flex items-center justify-between text-[12.5px]">
          <div className="flex items-center gap-2">
            <div className="flex h-6 w-6 items-center justify-center rounded-full" style={{ background: "var(--surface2)" }}>
              <User className="h-3.5 w-3.5 text-mut" />
            </div>
            {s.hub_logged_in ? "Signed in" : "Not signed in"}
            <span className="text-[10.5px] text-mut">Sign in to sync configs and rate providers</span>
            {importMsg && importMsg !== "Importing…" && (
              <span className="text-[10.5px]" style={{ color: "var(--kiwi)" }}>
                {importMsg}
              </span>
            )}
            {shareMsg && shareMsg !== "Importing…" && shareMsg !== "Exporting…" && (
              <span className="text-[10.5px]" style={{ color: "var(--kiwi)" }}>
                {shareMsg}
              </span>
            )}
          </div>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              className="h-7 whitespace-nowrap px-3 text-[11.5px]"
              onClick={onExport}
              title="Export providers and routes as JSON (includes API keys)"
            >
              Export plan
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 whitespace-nowrap px-3 text-[11.5px]"
              onClick={onImport}
              title="Import a one-click config plan (merges by same name + endpoint)"
            >
              Import plan
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 whitespace-nowrap px-3 text-[11.5px]"
              disabled={importMsg === "Importing…"}
              title={importMsg ?? undefined}
              onClick={() => {
                setImportMsg("Importing…");
                api
                  .importCcSwitch()
                  .then((r) =>
                    setImportMsg(
                      `Imported ${r.imported} · skipped ${r.skipped}${r.detail.length ? ` · ${r.detail[0]}` : ""}`,
                    ),
                  )
                  .catch(() => setImportMsg("Import failed"))
                  .finally(() => setTimeout(() => setImportMsg(null), 4000));
              }}
            >
              Import from CC Switch
            </Button>
            <Button size="sm" className="h-7 px-3 text-[11.5px] font-semibold">
              Sign in
            </Button>
          </div>
        </div>
      </div>

      {/* Advanced collapsible placeholder (the sliders hint row in the prototype, implemented with the add modal) */}
      <Button
        variant="outline"
        size="sm"
        className="h-8 w-full justify-between px-2.5 text-[11.5px] text-mut"
      >
        <span className="flex items-center gap-1.5">
          <SlidersHorizontal className="h-3 w-3" />
          Advanced (timeout / retries / headers)
        </span>
        <ChevronDown className="h-3.5 w-3.5" />
      </Button>
    </section>
  );
}
