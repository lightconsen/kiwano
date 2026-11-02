// The advanced forwarding settings a provider can carry.
import { ChevronDown, Gauge, Plus, RotateCcw, XIcon } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useT } from "../../i18n";

/** The headers the gateway injects as credentials (`x-api-key` / `Authorization`
    / `x-goog-api-key`, see `gateway::forward`). A custom header with one of
    these names wins over the injected credential — which is sometimes exactly
    the point (an Azure-style `api-key` endpoint), and otherwise a silent way to
    send the wrong key upstream. Warned about, not refused: this dialog cannot
    know which it is. */
export const AUTH_HEADER_NAMES = ["authorization", "x-api-key", "x-goog-api-key"];

/** Timeout, retries and custom upstream headers, behind one fold. Blank leaves
    the gateway's own default in force. */
export function AdvancedSection({
  advOpen,
  setAdvOpen,
  advTimeout,
  setAdvTimeout,
  advRetries,
  setAdvRetries,
  advHeaders,
  setAdvHeaders,
  advProxy,
  setAdvProxy,
  resetCircuit,
  resetBusy,
  resetNote,
  onResetCircuit,
}: {
  advOpen: boolean;
  setAdvOpen: Dispatch<SetStateAction<boolean>>;
  advTimeout: string;
  setAdvTimeout: (v: string) => void;
  advRetries: string;
  setAdvRetries: (v: string) => void;
  advHeaders: { name: string; value: string }[];
  setAdvHeaders: Dispatch<SetStateAction<{ name: string; value: string }[]>>;
  /** HTTP(S) proxy for this provider only; blank = the gateway default. */
  advProxy: string;
  setAdvProxy: (v: string) => void;
  /** Edit mode: whether to offer the breaker reset at all — a provider that does
      not exist yet has no breakers to close. */
  resetCircuit: boolean;
  resetBusy: boolean;
  /** What the last reset answered, already worded. `null` = not tried yet.
      `error` decides only the colour — the words are the daemon's either way. */
  resetNote: { text: string; error: boolean } | null;
  onResetCircuit: () => void;
}) {
  const t = useT();
  // Case-insensitive, like HTTP itself.
  const authHeaderOverride = advHeaders
    .map((r) => r.name.trim())
    .find((n) => AUTH_HEADER_NAMES.includes(n.toLowerCase()));

  const setHeader = (i: number, patch: Partial<{ name: string; value: string }>) =>
    setAdvHeaders((rows) => rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const removeHeader = (i: number) =>
    setAdvHeaders((rows) => rows.filter((_, j) => j !== i));

  const addHeader = () => setAdvHeaders((rows) => [...rows, { name: "", value: "" }]);

  return (
    <div>
      <Button
        variant="outline"
        size="sm"
        className="h-8 w-full justify-between text-[11.5px] text-mut"
        onClick={() => setAdvOpen((o) => !o)}
      >
        <span className="flex items-center gap-1.5">
          <Gauge className="h-3 w-3" />
          {t("addProvider.advanced")}
        </span>
        <ChevronDown
          className={`h-3.5 w-3.5 transition-transform ${advOpen ? "rotate-180" : ""}`}
        />
      </Button>
      {advOpen && (
        <div className="mt-2 space-y-2.5">
          <div className="grid grid-cols-2 gap-1.5">
            <div>
              <Label className="text-[10.5px] font-medium text-mut">{t("addProvider.timeoutLabel")}</Label>
              <Input
                type="number"
                min={1}
                max={3600}
                className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                value={advTimeout}
                onChange={(e) => setAdvTimeout(e.target.value)}
                placeholder="10"
              />
            </div>
            <div>
              <Label className="text-[10.5px] font-medium text-mut">{t("addProvider.retriesLabel")}</Label>
              <Input
                type="number"
                min={0}
                max={5}
                className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
                value={advRetries}
                onChange={(e) => setAdvRetries(e.target.value)}
                placeholder="0"
              />
            </div>
          </div>
          <p className="text-[10.5px] text-mut">{t("addProvider.advancedBody")}</p>
          {/* A proxy per provider, not one for the gateway: a vendor that is
              only reachable through one says nothing about the others, and a
              proxy that is down must not take the directly reachable ones with
              it. */}
          <div>
            <Label className="text-[10.5px] font-medium text-mut">{t("addProvider.proxyLabel")}</Label>
            <Input
              className="mt-1 h-8 bg-bg font-mono text-[12px] dark:bg-bg"
              value={advProxy}
              onChange={(e) => setAdvProxy(e.target.value)}
              placeholder={t("addProvider.proxyPlaceholder")}
            />
            <p className="mt-1 text-[10.5px] text-mut">{t("addProvider.proxyHint")}</p>
          </div>
          <div>
            <Label className="text-[10.5px] font-medium text-mut">
              {t("addProvider.customHeaders")}{" "}
              <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
                {t("addProvider.customHeadersHint")}
              </span>
            </Label>
            {/* An auth header here wins over the injected credential —
                which can be exactly what an Azure-style endpoint needs,
                and is otherwise a silent way to send the wrong key
                upstream. Named so the reader can tell which it is. */}
            {authHeaderOverride && (
              <p className="mt-1 text-[10.5px]" style={{ color: "var(--amber)" }}>
                {t("addProvider.customHeadersAuthWarning", {
                  name: authHeaderOverride,
                })}
              </p>
            )}
            <div className="mt-1 space-y-1.5">
              {advHeaders.map((r, i) => (
                <div key={i} className="flex items-center gap-1.5">
                  <Input
                    className="h-8 w-[38%] min-w-0 flex-none bg-bg font-mono text-[12px] dark:bg-bg"
                    value={r.name}
                    onChange={(e) => setHeader(i, { name: e.target.value })}
                    placeholder={t("addProvider.headerNamePlaceholder")}
                  />
                  <Input
                    className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
                    value={r.value}
                    onChange={(e) => setHeader(i, { value: e.target.value })}
                    placeholder={t("addProvider.headerValuePlaceholder")}
                  />
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className="flex-none text-mut"
                    aria-label={t("addProvider.removeHeader")}
                    onClick={() => removeHeader(i)}
                  >
                    <XIcon />
                  </Button>
                </div>
              ))}
              <Button
                variant="outline"
                size="sm"
                className="h-7 w-full gap-1 text-[11px] text-mut"
                onClick={addHeader}
              >
                <Plus className="h-3 w-3" />
                {t("addProvider.addHeader")}
              </Button>
            </div>
          </div>
          {/* Not a setting: a one-shot action, and the only one in this fold.
              It lives here because it is recovery for exactly the knobs above —
              a provider an outage or a bad key knocked out of service, and a
              proxy or key you have just fixed. It is *not* a form field, so it
              is never part of the save: the breaker lives in the gateway's
              memory, and waiting out its timeout is the only alternative. */}
          {resetCircuit && (
            <div>
              <Label className="text-[10.5px] font-medium text-mut">
                {t("addProvider.resetCircuit")}
              </Label>
              <Button
                variant="outline"
                size="sm"
                className="mt-1 h-8 w-full justify-start gap-1.5 text-[11.5px] text-mut"
                disabled={resetBusy}
                onClick={onResetCircuit}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                {resetBusy ? t("addProvider.resetCircuitBusy") : t("addProvider.resetCircuitAction")}
              </Button>
              <p className="mt-1 text-[10.5px] text-mut">{t("addProvider.resetCircuitHint")}</p>
              {/* The answer, in the two shapes it has: agents that were being
                  held back, or nothing was. An error rides the same slot — a
                  reset that failed must not look like one that found nothing. */}
              {resetNote && (
                <p
                  className="mt-1 text-[10.5px]"
                  role="status"
                  style={resetNote.error ? { color: "var(--red)" } : undefined}
                >
                  {resetNote.text}
                </p>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
