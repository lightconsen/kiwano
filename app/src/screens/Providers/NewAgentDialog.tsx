// Making a user-defined agent: the form, and nothing else — the id is derived
// and the candidates are bound in the tab it opens.
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "../../api/client";
import { useT } from "../../i18n";
import { type Protocol } from "../../api/types";
import { AgentProtocolSelect } from "./protocol";

/** Make a user-defined agent: a name, an optional note, the protocol its
    clients speak, and that is all the form asks for — the id is derived, and
    the candidates are bound in the tab it opens. */
export function NewAgentDialog({
  open,
  onClose,
  onSaved,
}: {
  open: boolean;
  onClose: () => void;
  /** The agent that was created — the caller opens its tab. */
  onSaved: (id: string) => void;
}) {
  const t = useT();
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  // Asked rather than defaulted silently: the answer is what a client author
  // will read here, so the common case (an OpenAI-compatible client) is the
  // opening value and the field is on screen to be corrected.
  const [protocol, setProtocol] = useState<Protocol>("openai");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const create = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    setErr(null);
    try {
      const created = await api.addCustomAgent(name, note, protocol);
      setName("");
      setNote("");
      setProtocol("openai");
      onSaved(created.id);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[380px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">{t("providers.newAgent")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3 px-5 pb-4 pt-1">
          <div>
            <Label className="text-[11px] font-medium text-mut">{t("providers.agentName")}</Label>
            <Input
              autoFocus
              aria-label={t("providers.agentName")}
              className="mt-1 h-8 text-[12px]"
              value={name}
              placeholder={t("providers.agentNamePlaceholder")}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && create()}
            />
          </div>
          <div>
            <Label className="text-[11px] font-medium text-mut">{t("providers.agentNote")}</Label>
            <Input
              aria-label={t("providers.agentNote")}
              className="mt-1 h-8 text-[12px]"
              value={note}
              placeholder={t("providers.agentNotePlaceholder")}
              onChange={(e) => setNote(e.target.value)}
            />
          </div>
          <div>
            <Label className="text-[11px] font-medium text-mut">
              {t("providers.agentProtocol")}
            </Label>
            <AgentProtocolSelect
              value={protocol}
              onChange={(p) => setProtocol(p ?? "openai")}
              className="mt-1 h-8 w-full bg-bg text-[12px] dark:bg-bg"
            />
            <p className="mt-1 text-[10.5px] leading-relaxed text-mut">
              {t("providers.agentProtocolNote")}
            </p>
          </div>
          <p className="text-[11px] leading-relaxed text-mut">{t("providers.agentIdNote")}</p>
          {err && (
            <div className="text-[11px]" style={{ color: "var(--red)" }}>
              {err}
            </div>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" size="sm" className="h-7 px-3 text-[12px]" onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              className="h-7 px-3 text-[12px] font-semibold"
              disabled={!name.trim() || busy}
              onClick={create}
            >
              {t("providers.createAgent")}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
