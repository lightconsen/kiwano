// The credentials and the ceiling of a user-defined agent, and the two-step
// delete that lives at the bottom of it.
import { useEffect, useState } from "react";

import { Check, SquarePen, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useT } from "../../i18n";
import { type AgentLimit, type AgentRef, type Protocol } from "../../api/types";
import { LimitSection } from "../../components/AgentLimit";
import { SettingsTabs, ACCESS_TABS } from "./SettingsTabs";
import { CopyRow } from "./CopyValue";
import { AgentProtocolSelect } from "./protocol";

/** One user-defined agent's settings: the two values a client is configured with
    (where the gateway is, and this agent's key), what it speaks, and how much it
    may spend.
 *
 * Opened from the icon beside the agent's name — a tab that is a route does not
 * need a permanent card of credentials on top of it. Deleting it stays outside the
 * subjects, below, where a destructive control belongs. */
export function AccessDialog({
  agent,
  label,
  keyName,
  protocol,
  listen,
  limits,
  currencies,
  open,
  onClose,
  onDelete,
  onChanged,
  onRenamed,
  onProtocolChanged,
}: {
  agent: AgentRef;
  label: string;
  keyName: string | null;
  protocol: Protocol | null;
  listen: string;
  limits: AgentLimit[];
  /** The currencies its own providers bill in — see `LimitSection`. */
  currencies: string[];
  open: boolean;
  onClose: () => void;
  onDelete: () => void;
  onChanged?: () => void;
  /** Only a user-defined agent can be renamed — a built-in's name is its
      brand, and the id is what everything else points at. Without this the
      name is shown as text rather than an editable field. */
  onRenamed?: (label: string) => Promise<unknown> | void;
  /** Only a user-defined agent declares a protocol; a built-in's is a fact
      about its clients, which its own dialog states rather than edits. */
  onProtocolChanged?: (protocol: Protocol | null) => Promise<unknown> | void;
}) {
  const t = useT();
  const [tab, setTab] = useState<"access" | "limit">("access");
  // Read-only until the pencil is pressed, then saved on blur or Enter. One
  // text field does not need a dialog, and a name that is always an input is a
  // name that gets changed by accident.
  const [name, setName] = useState(label);
  const [nameErr, setNameErr] = useState<string | null>(null);
  const [editingName, setEditingName] = useState(false);
  // The protocol saves on pick rather than behind a pencil: a Select is already
  // a deliberate act, and there is nothing to retype if it fails — the error is
  // reported under the row.
  const [protoErr, setProtoErr] = useState<string | null>(null);
  useEffect(() => {
    setName(label);
    setNameErr(null);
    setEditingName(false);
    setProtoErr(null);
  }, [label, open]);
  const commitName = async () => {
    const next = name.trim();
    if (!onRenamed || !next || next === label) {
      setName(label);
      setEditingName(false);
      return;
    }
    try {
      await onRenamed(next);
      setNameErr(null);
      setEditingName(false);
    } catch (e) {
      // Stay in the field so the name can be fixed without reopening it.
      setNameErr(e instanceof Error ? e.message : String(e));
    }
  };
  const cancelEdit = () => {
    setName(label);
    setNameErr(null);
    setEditingName(false);
  };
  const pickProtocol = async (next: Protocol | null) => {
    if (!onProtocolChanged || next === protocol) return;
    try {
      await onProtocolChanged(next);
      setProtoErr(null);
    } catch (e) {
      setProtoErr(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[460px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">
            {t("providers.agentSettingsFor", { agent: label })}
          </DialogTitle>
        </DialogHeader>
        <div className="px-5 pb-3 pt-1">
          <SettingsTabs tabs={ACCESS_TABS} tab={tab} onPick={setTab} />

          <div className="mt-2.5">
            {tab === "access" && (
              <>
                {onRenamed && (
                  <>
                    {/* One row, with the same 68px label column the endpoint
                        and key rows below it use, so the name lines up with
                        them rather than floating above its own label. */}
                    <div className="mt-1.5 flex items-center gap-2">
                      <span className="w-[68px] shrink-0 text-[10.5px] text-mut">
                        {t("providers.agentName")}
                      </span>
                      {editingName ? (
                        <>
                          <Input
                            autoFocus
                            aria-label={t("providers.agentName")}
                            className="h-7 flex-1 text-[12px]"
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") void commitName();
                              if (e.key === "Escape") cancelEdit();
                            }}
                          />
                          {/* Explicit, not on blur: leaving the field to look
                              something up is not a decision to save. */}
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            className="shrink-0 text-mut"
                            aria-label={t("common.save")}
                            title={t("common.save")}
                            disabled={!name.trim()}
                            onClick={() => void commitName()}
                          >
                            <Check className="h-3 w-3" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            className="shrink-0 text-mut"
                            aria-label={t("common.cancel")}
                            title={t("common.cancel")}
                            onClick={cancelEdit}
                          >
                            <X className="h-3 w-3" />
                          </Button>
                        </>
                      ) : (
                        <>
                          <span className="min-w-0 flex-1 truncate text-[12px]">{label}</span>
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            className="shrink-0 text-mut"
                            aria-label={t("providers.editAgentName")}
                            title={t("providers.editAgentName")}
                            onClick={() => setEditingName(true)}
                          >
                            <SquarePen className="h-3 w-3" />
                          </Button>
                        </>
                      )}
                    </div>
                    {nameErr && (
                      <p className="mt-1 text-[11px]" style={{ color: "var(--red)" }}>
                        {nameErr}
                      </p>
                    )}
                  </>
                )}
                {onProtocolChanged && (
                  <>
                    <div className="mt-1.5 flex items-center gap-2">
                      <span className="w-[68px] shrink-0 text-[10.5px] text-mut">
                        {t("providers.agentProtocol")}
                      </span>
                      <AgentProtocolSelect
                        value={protocol}
                        allowUnset
                        onChange={(p) => void pickProtocol(p)}
                        className="h-7 min-w-0 flex-1 bg-bg text-[12px] dark:bg-bg"
                      />
                    </div>
                    <p className="mt-1 text-[10.5px] leading-relaxed text-mut">
                      {t("providers.agentProtocolNote")}
                    </p>
                    {protoErr && (
                      <p className="mt-1 text-[11px]" style={{ color: "var(--red)" }}>
                        {protoErr}
                      </p>
                    )}
                  </>
                )}
                <CopyRow label={t("providers.accessEndpoint")} value={`http://${listen}`} />
                <CopyRow label={t("providers.accessKey")} value={keyName ?? "—"} />
                <p className="mt-2 text-[11px] leading-relaxed text-mut">
                  {t("providers.accessNote")}
                </p>
              </>
            )}
            {tab === "limit" && (
              <LimitSection
                agent={agent}
                limits={limits}
                currencies={currencies}
                onChanged={onChanged}
              />
            )}
          </div>
        </div>
        {/* Deleting lives with the rest of what this agent *is*, and one step
            further from the pointer than the tab's own rows. */}
        <div className="flex items-center gap-3 border-t border-line px-5 py-3">
          <DeleteAgentButton label={label} onConfirm={onDelete} />
          <span className="min-w-0 flex-1 text-[10.5px] text-mut">
            {t("providers.deleteAgentNote")}
          </span>
        </div>
      </DialogContent>
    </Dialog>
  );
}
/** Delete a user-defined agent: two steps, like a provider row — the first
    click arms it, and a stray one cannot take a route away. */
function DeleteAgentButton({ label, onConfirm }: { label: string; onConfirm: () => void }) {
  const t = useT();
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), 3000);
    return () => window.clearTimeout(timer);
  }, [armed]);
  return (
    <Button
      variant="ghost"
      size="sm"
      className="h-7 shrink-0 gap-1 border border-line px-2 text-[10.5px] text-mut hover:text-ink"
      style={armed ? { color: "var(--red)", borderColor: "var(--red)" } : undefined}
      title={armed ? t("providers.deleteAgentConfirm") : t("providers.deleteAgentNote")}
      aria-label={t("providers.deleteAgent")}
      onClick={() => (armed ? onConfirm() : setArmed(true))}
    >
      <Trash2 className="h-3.5 w-3.5" />
      {armed ? t("providers.deleteAgentConfirm") : `${t("providers.deleteAgent")} ${label}`}
    </Button>
  );
}
