// The advanced forwarding settings a provider can carry.
import { ChevronDown, Gauge, Plus, XIcon } from "lucide-react";
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
}: {
  advOpen: boolean;
  setAdvOpen: Dispatch<SetStateAction<boolean>>;
  advTimeout: string;
  setAdvTimeout: (v: string) => void;
  advRetries: string;
  setAdvRetries: (v: string) => void;
  advHeaders: { name: string; value: string }[];
  setAdvHeaders: Dispatch<SetStateAction<{ name: string; value: string }[]>>;
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
        </div>
      )}
    </div>
  );
}
