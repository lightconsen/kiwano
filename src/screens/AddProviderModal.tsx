// Add/edit provider modal (design/index.html #modal, cc-switch AddProviderDialog pattern)
// Base controls use shadcn/ui (Dialog/Input/Label/Select/Button); the segmented pills are kept as design language
import { useEffect, useState } from "react";
import { Eye, Gauge, Infinity as InfinityIcon, Plus, Store, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../api/client";
import {
  AGENTS,
  type AgentId,
  type ApiKeyEntry,
  type Billing,
  type CatalogEntry,
  type Protocol,
  type Provider,
} from "../api/types";

const BILL_OPTIONS: { id: Billing; label: string }[] = [
  { id: "plan", label: "订阅套餐" },
  { id: "payg", label: "按量付费" },
  { id: "unl", label: "不限额" },
];

const PROTOCOL_OPTIONS: { id: Protocol; label: string }[] = [
  { id: "openai", label: "OpenAI 兼容" },
  { id: "anthropic", label: "Anthropic" },
  { id: "gemini", label: "Gemini API" },
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
  const [protocol, setProtocol] = useState<Protocol>("openai");
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
  // Rotating Key management (spec §4.1 P1 multi-key rotation; available in edit mode)
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
      setProtocol(edit.protocol);
      setApiKey(""); // leave empty = keep the existing key
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
    setProtocol(preset?.protocol ?? "openai"); // catalog entries carry their protocol fingerprint
    setApiKey(preset ? "sk-9f3e21a7c8d4b6e05a12" : "");
    setEndpoint(preset?.endpoint ?? "");
    setModel(preset?.models[0] ?? "");
    setBilling(preset?.billing ?? "payg");
    setLimitValue(preset?.billing === "payg" ? "50" : "");
    setLimitUnit("cny");
    setResetPeriod("monthly");
    setAgents(preset?.id === "deepseek" ? ["claude", "codex"] : []);
  }, [open, preset, edit]);

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
        protocol,
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[min(600px,100dvh)] w-[calc(100%-2rem)] max-w-[480px] gap-0 overflow-y-auto rounded-xl p-0 sm:max-w-[480px]">
        <DialogHeader className="flex h-11 flex-row items-center justify-between border-b border-line px-4">
          <DialogTitle className="text-[13px] font-semibold">
            {edit ? "编辑供应商" : "添加供应商"}
          </DialogTitle>
        </DialogHeader>

        <div className="px-4 py-3.5">
          {/* Mode switch (hidden in edit mode: no models involved) */}
          {!edit && (
            <div className="flex overflow-hidden rounded-md border border-line text-[11.5px]">
              <div
                className="btn flex h-8 flex-1 cursor-pointer items-center justify-center gap-1 font-medium"
                style={mode === "shelf" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
                onClick={() => preset && setMode("shelf")}
              >
                <Store className="h-3 w-3" />
                从模型{preset ? `：${preset.name}` : "（先在模型选择）"}
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
              <Label className="text-[11px] font-medium text-mut">名称</Label>
              <Input
                className="mt-1 h-8 bg-bg text-[12px] dark:bg-bg"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">协议</Label>
              <Select value={protocol} onValueChange={(v) => setProtocol(v as Protocol)}>
                <SelectTrigger className="mt-1 w-full bg-bg text-[12px] dark:bg-bg">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PROTOCOL_OPTIONS.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">
                API Key{" "}
                <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                  {edit ? "留空则保持原 Key" : "仅存本机钥匙串"}
                </span>
              </Label>
              <div className="relative mt-1">
                <Input
                  type={showKey ? "text" : "password"}
                  className="h-8 bg-bg pr-8 font-mono text-[12px] dark:bg-bg"
                  value={apiKey}
                  placeholder={edit ? "••••••••" : ""}
                  onChange={(e) => setApiKey(e.target.value)}
                />
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="absolute top-1/2 right-1 -translate-y-1/2 text-mut"
                  onClick={() => setShowKey(!showKey)}
                  aria-label="显示/隐藏"
                >
                  <Eye className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">请求地址</Label>
              <div className="mt-1 flex gap-1.5">
                <Input
                  className="h-8 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                  value={endpoint}
                  onChange={(e) => {
                    setEndpoint(e.target.value);
                    setLatency(null);
                  }}
                />
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1 px-2.5 text-[11px]"
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
                </Button>
              </div>
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">默认模型</Label>
              {!edit && mode === "shelf" && preset ? (
                <Select value={model} onValueChange={(v) => setModel(v ?? "")}>
                  <SelectTrigger className="mt-1 w-full bg-bg text-[12px] dark:bg-bg">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {preset.models.map((m) => (
                      <SelectItem key={m} value={m}>
                        {m}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <Input
                  className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder="model-id"
                />
              )}
            </div>

            <div>
              <Label className="text-[11px] font-medium text-mut">计费方式</Label>
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
                  <Label className="text-[11px] font-medium text-mut">每期上限</Label>
                  <div className="mt-1 flex gap-1.5">
                    <Input
                      className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                      value={limitValue}
                      onChange={(e) => setLimitValue(e.target.value)}
                    />
                    <Select
                      value={limitUnit}
                      onValueChange={(v) => setLimitUnit(v as typeof limitUnit)}
                    >
                      <SelectTrigger className="w-[104px] bg-bg text-[12px] dark:bg-bg">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="requests">请求</SelectItem>
                        <SelectItem value="wan_tokens">万 tokens</SelectItem>
                        <SelectItem value="cny">¥ 金额</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
                <div>
                  <Label className="text-[11px] font-medium text-mut">重置周期</Label>
                  <Select
                    value={resetPeriod}
                    onValueChange={(v) => setResetPeriod(v as typeof resetPeriod)}
                  >
                    <SelectTrigger className="mt-1 w-full bg-bg text-[12px] dark:bg-bg">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="monthly">每月</SelectItem>
                      <SelectItem value="weekly">每周</SelectItem>
                      <SelectItem value="yearly">每年</SelectItem>
                      <SelectItem value="none">不重置</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
            )}

            {billing === "payg" && (
              <div>
                <Label className="text-[11px] font-medium text-mut">
                  消费限额 <span className="text-[10px]" style={{ color: "var(--kiwi)" }}>选填 · 留空则显示用量趋势</span>
                </Label>
                <Input
                  className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
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
              <Label className="text-[11px] font-medium text-mut">保存后绑定到 Agent</Label>
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

            {/* Rotating keys (edit mode: multiple keys rotate automatically, spec §4.1 P1) */}
            {edit && (
              <div>
                <Label className="text-[11px] font-medium text-mut">
                  轮询 Key{" "}
                  <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                    与主 Key 轮换 · 避免限流
                  </span>
                </Label>
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
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        className="text-mut"
                        aria-label="删除 Key"
                        onClick={() => removePollKey(k.id)}
                      >
                        <XIcon />
                      </Button>
                    </div>
                  ))}
                  <div className="flex gap-1.5">
                    <Input
                      className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                      value={newKey}
                      onChange={(e) => setNewKey(e.target.value)}
                      placeholder="sk-… 追加 Key"
                    />
                    <Input
                      className="h-8 w-[84px] bg-bg text-[11.5px] dark:bg-bg"
                      value={newKeyLabel}
                      onChange={(e) => setNewKeyLabel(e.target.value)}
                      placeholder="备注"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8 gap-1 px-2.5 text-[11px]"
                      disabled={!newKey.trim() || keyBusy}
                      onClick={addPollKey}
                    >
                      <Plus className="h-3 w-3" />
                      添加
                    </Button>
                  </div>
                </div>
              </div>
            )}

            <Button
              variant="outline"
              size="sm"
              className="h-8 w-full justify-between text-[11.5px] text-mut"
            >
              <span className="flex items-center gap-1.5">
                <Gauge className="h-3 w-3" />
                高级配置（超时 / 重试 / 请求头）
              </span>
              <XIcon className="h-3.5 w-3.5 rotate-45" />
            </Button>
          </div>
        </div>

        <DialogFooter className="mx-0 mb-0 flex-row justify-end gap-2 rounded-b-xl border-t border-line bg-transparent px-4 py-3">
          <Button variant="outline" size="sm" onClick={onClose}>
            取消
          </Button>
          <Button size="sm" className="font-semibold" disabled={!canSave} onClick={save}>
            {saving ? "保存中…" : edit ? "保存" : "保存并启用"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
