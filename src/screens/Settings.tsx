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

const CONFIG_FILE_FILTERS = [{ name: "Kiwano 配置", extensions: ["json"] }];

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
        setShareMsg("导出中…");
        api
          .exportConfig(path)
          .then((n) => setShareMsg(`已导出 ${n} 个 Provider（含 API Key，请妥善保管）`))
          .catch((e) => setShareMsg(`导出失败: ${String(e).slice(0, 60)}`))
          .finally(() => setTimeout(() => setShareMsg(null), 5000));
      })
      .catch(() => {});
  };

  const onImport = () => {
    open({ multiple: false, filters: CONFIG_FILE_FILTERS })
      .then((path) => {
        if (!path || Array.isArray(path)) return;
        setShareMsg("导入中…");
        api
          .importConfig(path)
          .then((r) =>
            setShareMsg(
              `已导入：新增 ${r.providers_added} · 复用 ${r.providers_kept} · 路由 ${r.routes_applied}`,
            ),
          )
          .catch((e) => setShareMsg(`导入失败: ${String(e).slice(0, 60)}`))
          .finally(() => setTimeout(() => setShareMsg(null), 5000));
      })
      .catch(() => {});
  };

  useEffect(() => {
    api.getSettings().then(setS);
  }, []);

  if (!s) return <div className="p-8 text-center text-[12px] text-mut">加载中…</div>;

  const patch = (p: Partial<AppSettings>) => {
    api.updateSettings(p).then(setS);
  };

  return (
    <section className="space-y-3 p-4">
      {/* General */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">通用</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="语言">
            <Select value={s.language ?? "zh-CN"} onValueChange={(v) => patch({ language: v ?? "zh-CN" })}>
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="zh-CN">简体中文</SelectItem>
                <SelectItem value="en">English</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label="主题">
            <Select value={s.theme ?? "dark"} onValueChange={(v) => patch({ theme: v ?? "dark" })}>
              <SelectTrigger size="sm" className="h-7 w-[130px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="dark">深色</SelectItem>
                <SelectItem value="light">浅色</SelectItem>
              </SelectContent>
            </Select>
          </Row>
          <Row label="开机自启">
            <Switch checked={s.autostart} onCheckedChange={(v) => patch({ autostart: v })} />
          </Row>
          <Row label="关闭时最小化到托盘">
            <Switch checked={s.close_to_tray} onCheckedChange={(v) => patch({ close_to_tray: v })} />
          </Row>
        </div>
      </div>

      {/* Local gateway */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">本地网关</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="监听地址">
            <Input
              className="h-7 w-[130px] bg-surface2 font-mono text-[11.5px] dark:bg-surface2"
              value={s.gateway_listen}
              onChange={(e) => patch({ gateway_listen: e.target.value })}
            />
          </Row>
          <div className="pb-0.5 pt-1 text-[11px] font-medium text-mut">
            Agent 接管 <span className="font-normal">· 指向本地网关后切换即热切</span>
          </div>
          {s.takeovers.map((t) => (
            <div key={t.agent} className="flex items-center justify-between">
              <span className="flex items-center gap-2">
                {t.label}{" "}
                {t.placeholder_key ? (
                  <span className="text-[10px] font-mono text-mut" title="网关分配的占位 Key，用于请求归因">
                    {t.placeholder_key}
                  </span>
                ) : (
                  <span className="text-[10px] font-mono text-mut">—</span>
                )}
              </span>
              <span className="flex items-center gap-2">
                <span className="text-[10.5px]" style={t.enabled ? { color: "var(--kiwi)" } : { color: "var(--mut)" }}>
                  {t.enabled ? "已接管" : "未接管"}
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
          <Row label="自动故障转移" note="主供应商失败时切备用">
            <Switch checked={s.auto_failover} onCheckedChange={(v) => patch({ auto_failover: v })} />
          </Row>
          <Row label="请求日志留存" note="仅本地 · 30 天">
            <Switch checked={s.request_logs} onCheckedChange={(v) => patch({ request_logs: v })} />
          </Row>
          <Row label="费用预警" note="用量达每期上限时系统通知">
            <Switch checked={s.cost_alert} onCheckedChange={(v) => patch({ cost_alert: v })} />
          </Row>
        </div>
      </div>

      {/* Privacy pledge */}
      <div className="rounded-lg border p-4" style={{ background: "var(--kiwi-soft)", borderColor: "var(--kiwi-dim)" }}>
        <h3 className="mb-3 flex items-center gap-1.5 text-[12.5px] font-semibold" style={{ color: "var(--kiwi)" }}>
          <ShieldCheck className="h-3.5 w-3.5" />
          隐私（Kiwano 承诺）
        </h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="匿名用量上报" note="默认关闭 · 仅脱敏统计">
            <Switch checked={s.telemetry} onCheckedChange={(v) => patch({ telemetry: v })} />
          </Row>
          <div className="space-y-1 text-[10.5px]" style={{ color: "var(--mut)" }}>
            <div className="flex items-center gap-1.5">
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
              API 请求内容永不经过 Kiwano 云端
            </div>
            <div className="flex items-center gap-1.5">
              <Check className="h-3 w-3" style={{ color: "var(--kiwi)" }} />
              API Key 永不离开用户设备
            </div>
          </div>
        </div>
      </div>

      {/* Hub & config */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">Kiwano Hub 与配置</h3>
        <div className="flex items-center justify-between text-[12.5px]">
          <div className="flex items-center gap-2">
            <div className="flex h-6 w-6 items-center justify-center rounded-full" style={{ background: "var(--surface2)" }}>
              <User className="h-3.5 w-3.5 text-mut" />
            </div>
            {s.hub_logged_in ? "已登录" : "未登录"}
            <span className="text-[10.5px] text-mut">登录后同步配置、参与评分</span>
            {importMsg && importMsg !== "导入中…" && (
              <span className="text-[10.5px]" style={{ color: "var(--kiwi)" }}>
                {importMsg}
              </span>
            )}
            {shareMsg && shareMsg !== "导入中…" && shareMsg !== "导出中…" && (
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
              title="导出 Provider 与路由方案为 JSON（含 API Key）"
            >
              导出方案
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 whitespace-nowrap px-3 text-[11.5px]"
              onClick={onImport}
              title="导入一键配置方案（同名同端点合并）"
            >
              导入方案
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 whitespace-nowrap px-3 text-[11.5px]"
              disabled={importMsg === "导入中…"}
              title={importMsg ?? undefined}
              onClick={() => {
                setImportMsg("导入中…");
                api
                  .importCcSwitch()
                  .then((r) =>
                    setImportMsg(
                      `已导入 ${r.imported} · 跳过 ${r.skipped}${r.detail.length ? ` · ${r.detail[0]}` : ""}`,
                    ),
                  )
                  .catch(() => setImportMsg("导入失败"))
                  .finally(() => setTimeout(() => setImportMsg(null), 4000));
              }}
            >
              从 CC Switch 导入
            </Button>
            <Button size="sm" className="h-7 px-3 text-[11.5px] font-semibold">
              登录
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
          高级配置（超时 / 重试 / 请求头）
        </span>
        <ChevronDown className="h-3.5 w-3.5" />
      </Button>
    </section>
  );
}
