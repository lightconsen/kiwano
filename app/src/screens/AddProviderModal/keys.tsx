// The rotating keys a provider answers with, and the box that adds one.
import { Plus, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useT } from "../../i18n";
import type { ApiKeyEntry } from "../../api/types";

/** Edit mode only: several keys on one provider rotate automatically. The two
    calls behind the buttons are the container's. */
export function RotatingKeys({
  pollKeys,
  newKey,
  setNewKey,
  newKeyLabel,
  setNewKeyLabel,
  keyBusy,
  addPollKey,
  removePollKey,
}: {
  pollKeys: ApiKeyEntry[];
  newKey: string;
  setNewKey: (v: string) => void;
  newKeyLabel: string;
  setNewKeyLabel: (v: string) => void;
  keyBusy: boolean;
  addPollKey: () => void;
  removePollKey: (id: number) => void;
}) {
  const t = useT();
  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.rotatingKeys")}{" "}
        <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
          {t("addProvider.rotatingKeysHint")}
        </span>
      </Label>
      <div className="mt-1 space-y-1">
        {pollKeys.map((k) => (
          <div
            key={k.id}
            className="flex h-8 items-center justify-between rounded-md border border-line px-2.5"
          >
            <span className="flex items-center gap-2">
              <span className="font-mono text-[11.5px]">{k.masked}</span>
              {k.label && <span className="text-[10.5px] text-mut">{k.label}</span>}
            </span>
            <Button
              variant="ghost"
              size="icon-xs"
              className="text-mut"
              aria-label={t("addProvider.deleteKey")}
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
            placeholder={t("addProvider.addKeyPlaceholder")}
          />
          <Input
            className="h-8 w-[84px] bg-bg text-[11.5px] dark:bg-bg"
            value={newKeyLabel}
            onChange={(e) => setNewKeyLabel(e.target.value)}
            placeholder={t("addProvider.labelPlaceholder")}
          />
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-1 px-2.5 text-[11px]"
            disabled={!newKey.trim() || keyBusy}
            onClick={addPollKey}
          >
            <Plus className="h-3 w-3" />
            {t("common.add")}
          </Button>
        </div>
      </div>
    </div>
  );
}
