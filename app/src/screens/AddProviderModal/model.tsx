// The model this provider routes by default, and the read that lists them.
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useT } from "../../i18n";

/** The default model. A live list fetched from the endpoint upgrades the box to
    a dropdown; the entry's own models are what it offers before then. */
export function ModelSection({
  model,
  setModel,
  modelOpen,
  setModelOpen,
  modelOptions,
  showModelSelect,
  fetching,
  fetchError,
  endpoint,
  fetchModels,
}: {
  model: string;
  setModel: (v: string) => void;
  modelOpen: boolean;
  setModelOpen: (open: boolean) => void;
  modelOptions: string[];
  showModelSelect: boolean;
  fetching: boolean;
  fetchError: string | null;
  /** Only read to say whether there is an endpoint to fetch from at all */
  endpoint: string;
  fetchModels: () => void;
}) {
  const t = useT();
  return (
    <div>
      <div className="flex items-center justify-between">
        <Label className="text-[11px] font-medium text-mut">{t("addProvider.defaultModel")}</Label>
        {/* Pulls the live model list from the primary endpoint; needs
            the API key, so a click without one shows an inline error */}
        <Button
          variant="ghost"
          size="sm"
          className="h-6 gap-1 px-2 text-[10.5px] text-mut"
          disabled={fetching || !endpoint.trim()}
          onClick={fetchModels}
        >
          <RefreshCw className={`h-3 w-3${fetching ? " animate-spin" : ""}`} />
          {fetching ? t("addProvider.fetching") : t("addProvider.fetch")}
        </Button>
      </div>
      {showModelSelect ? (
        <Select value={model} open={modelOpen} onOpenChange={setModelOpen} onValueChange={(v) => setModel(v ?? "")}>
          <SelectTrigger className="mt-1 w-full bg-bg text-[12px] dark:bg-bg">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {modelOptions.map((m) => (
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
          placeholder={t("addProvider.modelIdPlaceholder")}
        />
      )}
      {fetchError && (
        <p className="mt-1 text-[10.5px]" style={{ color: "var(--red)" }} title={fetchError}>
          {fetchError}
        </p>
      )}
    </div>
  );
}
