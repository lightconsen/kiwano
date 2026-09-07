// 设置（design/index.html #s-settings）
import { useEffect, useState } from "react";
import { Check, ChevronDown, ShieldCheck, SlidersHorizontal, User } from "lucide-react";
import { api } from "../api/client";
import type { AppSettings } from "../api/types";
import { Toggle } from "../components/bits";

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

  useEffect(() => {
    api.getSettings().then(setS);
  }, []);

  if (!s) return <div className="p-8 text-center text-[12px] text-mut">加载中…</div>;

  const patch = (p: Partial<AppSettings>) => {
    api.updateSettings(p).then(setS);
  };

  return (
    <section className="space-y-3 p-4">
      {/* 通用 */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">通用</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="语言">
            <select
              className="h-7 w-[130px] rounded-md border border-line bg-surface2 px-2 text-[11.5px]"
              value={s.language}
              onChange={(e) => patch({ language: e.target.value })}
            >
              <option value="zh-CN">简体中文</option>
              <option value="en">English</option>
            </select>
          </Row>
          <Row label="主题">
            <select
              className="h-7 w-[130px] rounded-md border border-line bg-surface2 px-2 text-[11.5px]"
              value={s.theme}
              onChange={(e) => patch({ theme: e.target.value })}
            >
              <option value="dark">深色</option>
              <option value="light">浅色</option>
            </select>
          </Row>
          <Row label="开机自启">
            <Toggle on={s.autostart} onChange={(v) => patch({ autostart: v })} />
          </Row>
          <Row label="关闭时最小化到托盘">
            <Toggle on={s.close_to_tray} onChange={(v) => patch({ close_to_tray: v })} />
          </Row>
        </div>
      </div>

      {/* 本地网关 */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">本地网关</h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="监听地址">
            <input
              className="h-7 w-[130px] rounded-md border border-line bg-surface2 px-2 font-mono text-[11.5px]"
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
                <Toggle
                  on={t.enabled}
                  onChange={(v) =>
                    patch({
                      takeovers: s.takeovers.map((x) => (x.agent === t.agent ? { ...x, enabled: v } : x)),
                    })
                  }
                />
              </span>
            </div>
          ))}
          <Row label="自动故障转移" note="主供应商失败时切备用">
            <Toggle on={s.auto_failover} onChange={(v) => patch({ auto_failover: v })} />
          </Row>
          <Row label="请求日志留存" note="仅本地 · 30 天">
            <Toggle on={s.request_logs} onChange={(v) => patch({ request_logs: v })} />
          </Row>
        </div>
      </div>

      {/* 隐私承诺 */}
      <div className="rounded-lg border p-4" style={{ background: "var(--kiwi-soft)", borderColor: "var(--kiwi-dim)" }}>
        <h3 className="mb-3 flex items-center gap-1.5 text-[12.5px] font-semibold" style={{ color: "var(--kiwi)" }}>
          <ShieldCheck className="h-3.5 w-3.5" />
          隐私（Kiwano 承诺）
        </h3>
        <div className="space-y-2.5 text-[12.5px]">
          <Row label="匿名用量上报" note="默认关闭 · 仅脱敏统计">
            <Toggle on={s.telemetry} onChange={(v) => patch({ telemetry: v })} />
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

      {/* Hub 与配置 */}
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-3 text-[12.5px] font-semibold">Kiwano Hub 与配置</h3>
        <div className="flex items-center justify-between text-[12.5px]">
          <div className="flex items-center gap-2">
            <div className="flex h-6 w-6 items-center justify-center rounded-full" style={{ background: "var(--surface2)" }}>
              <User className="h-3.5 w-3.5 text-mut" />
            </div>
            {s.hub_logged_in ? "已登录" : "未登录"}
            <span className="text-[10.5px] text-mut">登录后同步配置、参与评分</span>
          </div>
          <div className="flex gap-2">
            <button className="btn btn-ghost h-7 rounded-md border border-line px-3 text-[11.5px]">从 CC Switch 导入</button>
            <button
              className="btn btn-primary h-7 rounded-md px-3 text-[11.5px] font-semibold"
              style={{ background: "var(--kiwi)", color: "oklch(0.18 0.03 132)" }}
            >
              登录
            </button>
          </div>
        </div>
      </div>

      {/* 高级折叠占位（原型内的 sliders 提示行，随添加弹窗实现） */}
      <button className="btn btn-ghost flex h-8 w-full items-center justify-between rounded-md border border-line px-2.5 text-[11.5px] text-mut">
        <span className="flex items-center gap-1.5">
          <SlidersHorizontal className="h-3 w-3" />
          高级配置（超时 / 重试 / 请求头）
        </span>
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
    </section>
  );
}
