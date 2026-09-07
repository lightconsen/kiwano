// 添加/编辑供应商弹窗（design/index.html #modal，cc-switch AddProviderDialog 模式）
import { useEffect, useState } from "react";
import { Eye, Gauge, Infinity as InfinityIcon, Plus, Store, X } from "lucide-react";
import { api } from "../api/client";
import {
  AGENTS,
  type AgentId,
  type ApiKeyEntry,
  type Billing,
  type CatalogEntry,
  type Provider,
} from "../api/types";

const BILL_OPTIONS: { id: Billing; label: string }[] = [
  { id: "plan", label: "订阅套餐" },
  { id: "payg", label: "按量付费" },
  { id: "unl", label: "不限额" },
];

export default function AddProviderModal({
  open,
  preset,
  edit,
  onClose,
  onSaved,
}: {
  open: boolean;
  preset: CatalogEntry | null;
  edit: Provider | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [mode, setMode] = useState<"shelf" | "custom">("shelf");
  const [name, setName] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [endpoint, setEndpoint] = useState("");
  const [model, setModel] = useState("");
  const [billing, setBilling] = useState<Billing>("payg");
  const [limitValue, setLimitValue] = useState("");
  const [limitUnit, setLimitUnit] = useState<"requests" | "wan_tokens" | "cny">("cny");
  const [resetPeriod, setResetPeriod] = useState<"monthly" | "weekly" | "yearly" | "none">("monthly");
  const [agents, setAgents] = useState<AgentId[]>([]);
  const [testing, setTesting] = useState(false);
  const [latency, setLatency] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);
  // 轮询 Key 管理（spec §4.1 P1 多 Key 轮询；编辑态可用）
  const [pollKeys, setPollKeys] = useState<ApiKeyEntry[]>([]);
  const [newKey, setNewKey] = useState("");
  const [newKeyLabel, setNewKeyLabel] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setShowKey(false);
    setLatency(null);
    setNewKey("");
    setNewKeyLabel("");
    if (edit) {
      setMode("custom");
      setName(edit.name);
      setApiKey(""); // 留空 = 保持原 Key
      setEndpoint(edit.endpoint);
      setModel("");
      setBilling(edit.billing);
      const q = edit.usage?.quota;
      setLimitValue(q ? String(q.limit) : "");
      setLimitUnit(q?.unit === "requests" ? "requests" : "cny");
      setResetPeriod("monthly");
      setAgents([...edit.agents]);
      api.listApiKeys(edit.id).then(setPollKeys).catch(() => setPollKeys([]));
      return;
    }
    setMode(preset ? "shelf" : "custom");
    setName(preset?.name ?? "");
    setApiKey(preset ? "sk-9f3e21a7c8d4b6e05a12" : "");
    setEndpoint(preset?.endpoint ?? "");
    setModel(preset?.models[0] ?? "");
    setBilling(preset?.billing ?? "payg");
    setLimitValue(preset?.billing === "payg" ? "50" : "");
    setLimitUnit("cny");
    setResetPeriod("monthly");
    setAgents(preset?.id === "deepseek" ? ["claude", "codex"] : []);
  }, [open, preset, edit]);

  if (!open) return null;

  const canSave = name.trim() !== "" && endpoint.trim() !== "" && !saving;

  const maskKey = (k: string) => (k.length > 12 ? `${k.slice(0, 6)}…${k.slice(-4)}` : k);

  const addPollKey = async () => {
    if (!edit || !newKey.trim() || keyBusy) return;
    setKeyBusy(true);
    try {
      const row = await api.addApiKey(edit.id, newKey.trim(), newKeyLabel.trim() || undefined);
      setPollKeys((ks) => [...ks, row]);
      setNewKey("");
      setNewKeyLabel("");
    } finally {
      setKeyBusy(false);
    }
  };

  const removePollKey = async (id: number) => {
    if (keyBusy) return;
    setKeyBusy(true);
    try {
      await api.deleteApiKey(id);
      setPollKeys((ks) => ks.filter((k) => k.id !== id));
    } finally {
      setKeyBusy(false);
    }
  };

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    try {
      const input = {
        name: name.trim(),
        api_key: apiKey,
        endpoint: endpoint.trim(),
        protocol: "openai" as const,
        model_default: model,
        billing,
        billing_config: {
          limit_value: limitValue ? Number(limitValue) : undefined,
          limit_unit: billing === "plan" ? limitUnit : billing === "payg" ? "cny" : undefined,
          reset_period: billing === "plan" ? resetPeriod : undefined,
        },
        agents,
      };
      if (edit) {
        await api.updateProvider(edit.id, input);
      } else {
        await api.addProvider(input);
      }
      onSaved();
      onClose();
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "oklch(0.08 0.01 260 / .65)", backdropFilter: "blur(3px)" }}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="max-h-[600px] w-[480px] overflow-y-auto rounded-xl border border-line shadow-2xl" style={{ background: "var(--surface)" }}>
        <div className="flex h-11 items-center justify-between border-b border-line px-4">
          <h2 className="text-[13px] font-semibold">{edit ? "编辑供应商" : "添加供应商"}</h2>
          <button className="btn btn-ghost rounded-md p-1 text-mut" onClick={onClose} aria-label="关闭">
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="px-4 py-3.5">
          {/* 模式切换（编辑态隐藏：不涉及货架） */}
          {!edit && (
            <div className="flex overflow-hidden rounded-md border border-line text-[11.5px]">
              <div
                className="btn flex h-8 flex-1 cursor-pointer items-center justify-center gap-1 font-medium"
                style={mode === "shelf" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
                onClick={() => preset && setMode("shelf")}
              >
                <Store className="h-3 w-3" />
                从货架{preset ? `：${preset.name}` : "（先在货架选择）"}
              </div>
              <div
                className="btn flex h-8 flex-1 cursor-pointer items-center justify-center"
                style={mode === "custom" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
                onClick={() => setMode("custom")}
              >
                自定义
              </div>
            </div>
          )}

          <div className="mt-3 space-y-3">
            <div>
              <label className="text-[11px] font-medium text-mut">名称</label>
              <input className="mt-1 h-8 w-full rounded-md border border-line bg-bg px-2.5 text-[12px]" value={name} onChange={(e) => setName(e.target.value)} />
            </div>

            <div>
              <label className="text-[11px] font-medium text-mut">
                API Key{" "}
                <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                  {edit ? "留空则保持原 Key" : "仅存本机钥匙串"}
                </span>
              </label>
              <div className="relative mt-1">
                <input
                  type={showKey ? "text" : "password"}
                  className="h-8 w-full rounded-md border border-line bg-bg pl-2.5 pr-8 font-mono text-[12px]"
                  value={apiKey}
                  placeholder={edit ? "••••••••" : ""}
                  onChange={(e) => setApiKey(e.target.value)}
                />
                <button className="absolute right-2 top-1/2 -translate-y-1/2 text-mut" onClick={() => setShowKey(!showKey)} aria-label="显示/隐藏">
                  <Eye className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>

            <div>
              <label className="text-[11px] font-medium text-mut">请求地址</label>
              <div className="mt-1 flex gap-1.5">
                <input
                  className="h-8 flex-1 rounded-md border border-line bg-bg px-2.5 font-mono text-[12px]"
                  value={endpoint}
                  onChange={(e) => {
                    setEndpoint(e.target.value);
                    setLatency(null);
                  }}
                />
                <button
                  className="btn btn-ghost flex h-8 items-center gap-1 rounded-md border border-line px-2.5 text-[11px]"
                  disabled={testing}
                  onClick={async () => {
                    setTesting(true);
                    try {
                      setLatency(await api.testLatency(endpoint.trim()));
                    } finally {
                      setTesting(false);
                    }
                  }}
                >
                  <Gauge className="h-3 w-3" />
                  测速{" "}
                  {latency != null && (
                    <span className="font-mono" style={{ color: "var(--kiwi)" }}>
                      {latency}ms
                    </span>
                  )}
                </button>
              </div>
            </div>

            <div>
              <label className="text-[11px] font-medium text-mut">默认模型</label>
              {!edit && mode === "shelf" && preset ? (
                <select className="mt-1 h-8 w-full rounded-md border border-line bg-bg px-2.5 text-[12px]" value={model} onChange={(e) => setModel(e.target.value)}>
                  {preset.models.map((m) => (
                    <option key={m}>{m}</option>
                  ))}
                </select>
              ) : (
                <input className="mt-1 h-8 w-full rounded-md border border-line bg-bg px-2.5 font-mono text-[12px]" value={model} onChange={(e) => setModel(e.target.value)} placeholder="model-id" />
              )}
            </div>

            <div>
              <label className="text-[11px] font-medium text-mut">计费方式</label>
              <div className="mt-1 grid grid-cols-3 gap-1.5">
                {BILL_OPTIONS.map((b) => {
                  const active = billing === b.id;
                  return (
                    <button
                      key={b.id}
                      className="btn h-8 cursor-pointer rounded-md border text-[11.5px]"
                      style={active
                        ? { borderColor: "var(--kiwi-dim)", background: "var(--kiwi-soft)", color: "var(--kiwi)" }
                        : { borderColor: "var(--line)", color: "var(--mut)" }}
                      onClick={() => setBilling(b.id)}
                    >
                      {b.label}
                    </button>
                  );
                })}
              </div>
            </div>

            {billing === "plan" && (
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="text-[11px] font-medium text-mut">每期上限</label>
                  <div className="mt-1 flex gap-1.5">
                    <input
                      className="h-8 min-w-0 flex-1 rounded-md border border-line bg-bg px-2.5 font-mono text-[12px]"
                      value={limitValue}
                      onChange={(e) => setLimitValue(e.target.value)}
                    />
                    <select
                      className="h-8 rounded-md border border-line bg-bg px-1.5 text-[12px]"
                      value={limitUnit}
                      onChange={(e) => setLimitUnit(e.target.value as typeof limitUnit)}
                    >
                      <option value="requests">请求</option>
                      <option value="wan_tokens">万 tokens</option>
                      <option value="cny">¥ 金额</option>
                    </select>
                  </div>
                </div>
                <div>
                  <label className="text-[11px] font-medium text-mut">重置周期</label>
                  <select
                    className="mt-1 h-8 w-full rounded-md border border-line bg-bg px-2.5 text-[12px]"
                    value={resetPeriod}
                    onChange={(e) => setResetPeriod(e.target.value as typeof resetPeriod)}
                  >
                    <option value="monthly">每月</option>
                    <option value="weekly">每周</option>
                    <option value="yearly">每年</option>
                    <option value="none">不重置</option>
                  </select>
                </div>
              </div>
            )}

            {billing === "payg" && (
              <div>
                <label className="text-[11px] font-medium text-mut">
                  消费限额 <span className="text-[10px]" style={{ color: "var(--kiwi)" }}>选填 · 留空则显示用量趋势</span>
                </label>
                <input
                  className="mt-1 h-8 w-full rounded-md border border-line bg-bg px-2.5 font-mono text-[12px]"
                  value={limitValue}
                  onChange={(e) => setLimitValue(e.target.value)}
                  placeholder="¥"
                />
              </div>
            )}

            {billing === "unl" && (
              <div className="flex h-8 items-center gap-1.5 text-[11.5px] text-mut">
                <InfinityIcon className="h-3.5 w-3.5" />
                无需额度配置 · 列表中不显示用量信息
              </div>
            )}

            <div>
              <label className="text-[11px] font-medium text-mut">保存后绑定到 Agent</label>
              <div className="mt-1 grid grid-cols-3 gap-1.5">
                {AGENTS.map((a) => {
                  const active = agents.includes(a.id);
                  return (
                    <label
                      key={a.id}
                      className="flex h-8 cursor-pointer items-center gap-1.5 rounded-md border px-2 text-[11.5px]"
                      style={active
                        ? { borderColor: "var(--kiwi-dim)", background: "var(--kiwi-soft)", color: "var(--ink)" }
                        : { borderColor: "var(--line)", color: "var(--mut)" }}
                    >
                      <input
                        type="checkbox"
                        checked={active}
                        onChange={() =>
                          setAgents(active ? agents.filter((x) => x !== a.id) : [...agents, a.id])
                        }
                        style={{ accentColor: "var(--kiwi)" }}
                      />
                      {a.label}
                    </label>
                  );
                })}
              </div>
            </div>

            {/* 轮询 Key（编辑态：多 Key 自动轮换，spec §4.1 P1） */}
            {edit && (
              <div>
                <label className="text-[11px] font-medium text-mut">
                  轮询 Key{" "}
                  <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                    与主 Key 轮换 · 避免限流
                  </span>
                </label>
                <div className="mt-1 space-y-1">
                  {pollKeys.map((k) => (
                    <div
                      key={k.id}
                      className="flex h-8 items-center justify-between rounded-md border border-line px-2.5"
                    >
                      <span className="flex items-center gap-2">
                        <span className="font-mono text-[11.5px]">{maskKey(k.api_key)}</span>
                        {k.label && <span className="text-[10.5px] text-mut">{k.label}</span>}
                      </span>
                      <button
                        className="btn btn-ghost rounded-md p-1 text-mut"
                        aria-label="删除 Key"
                        onClick={() => removePollKey(k.id)}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  ))}
                  <div className="flex gap-1.5">
                    <input
                      className="h-8 min-w-0 flex-1 rounded-md border border-line bg-bg px-2.5 font-mono text-[12px]"
                      value={newKey}
                      onChange={(e) => setNewKey(e.target.value)}
                      placeholder="sk-… 追加 Key"
                    />
                    <input
                      className="h-8 w-[84px] rounded-md border border-line bg-bg px-2.5 text-[11.5px]"
                      value={newKeyLabel}
                      onChange={(e) => setNewKeyLabel(e.target.value)}
                      placeholder="备注"
                    />
                    <button
                      className="btn btn-ghost flex h-8 items-center gap-1 rounded-md border border-line px-2.5 text-[11px]"
                      disabled={!newKey.trim() || keyBusy}
                      onClick={addPollKey}
                    >
                      <Plus className="h-3 w-3" />
                      添加
                    </button>
                  </div>
                </div>
              </div>
            )}

            <button className="btn btn-ghost flex h-8 w-full items-center justify-between rounded-md border border-line px-2.5 text-[11.5px] text-mut">
              <span className="flex items-center gap-1.5">
                <Gauge className="h-3 w-3" />
                高级配置（超时 / 重试 / 请求头）
              </span>
              <X className="h-3.5 w-3.5 rotate-45" />
            </button>
          </div>
        </div>

        <div className="flex justify-end gap-2 border-t border-line px-4 py-3">
          <button className="btn btn-ghost h-8 rounded-md border border-line px-3.5 text-[12px]" onClick={onClose}>
            取消
          </button>
          <button
            className="btn btn-primary h-8 rounded-md px-4 text-[12px] font-semibold disabled:opacity-50"
            style={{ background: "var(--kiwi)", color: "oklch(0.18 0.03 132)" }}
            disabled={!canSave}
            onClick={save}
          >
            {saving ? "保存中…" : edit ? "保存" : "保存并启用"}
          </button>
        </div>
      </div>
    </div>
  );
}
