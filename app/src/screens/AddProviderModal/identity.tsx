// The provider's name, and the key it answers with.
import { Eye } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useT } from "../../i18n";
import type { Provider } from "../../api/types";

/** Name and key. The key box is deliberately empty while editing — blank means
    "keep the stored one", which the note beside the label says. */
export function IdentitySection({
  name,
  setName,
  apiKey,
  setApiKey,
  showKey,
  setShowKey,
  edit,
}: {
  name: string;
  setName: (v: string) => void;
  apiKey: string;
  setApiKey: (v: string) => void;
  showKey: boolean;
  setShowKey: (v: boolean) => void;
  /** The row being edited, or null when the form is adding one */
  edit: Provider | null;
}) {
  const t = useT();
  return (
    <>
      <div>
        <Label className="text-[11px] font-medium text-mut">{t("addProvider.name")}</Label>
        <Input
          className="mt-1 h-8 bg-bg text-[12px] dark:bg-bg"
          value={name}
          aria-label={t("addProvider.name")}
          onChange={(e) => setName(e.target.value)}
        />
      </div>

      <div>
        <Label className="text-[11px] font-medium text-mut">
          {t("addProvider.apiKey")}{" "}
          <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
            {edit ? t("addProvider.keyKeep") : t("addProvider.keyLocal")}
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
            aria-label={t("addProvider.showHideKey")}
          >
            <Eye className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </>
  );
}
