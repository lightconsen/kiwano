// One built-in agent's settings: the files a takeover rewrites, the key its
// clients are pointed at with, turning the takeover off, and its ceiling.
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useT } from "../../i18n";
import { type AgentLimit, type AgentRef, type Protocol } from "../../api/types";
import { LimitSection } from "../../components/AgentLimit";
import { SettingsTabs, AGENT_TABS } from "./SettingsTabs";
import { CopyValue } from "./CopyValue";
import { protocolName } from "./protocol";

/// How long the second click stays armed, in seconds.
const ARM_SECONDS = 3;

/** One built-in agent's settings: the files a takeover rewrites, the key its
    clients are pointed at with, turning the takeover off, and how much the agent
    may spend.
 *
 * A user-defined agent has a dialog of its own below — there is no file behind it
 * — and the two share this shape: a strip of subjects, and one showing. */
export function AgentSettingsDialog({
  agent,
  label,
  paths,
  protocols,
  placeholderKey,
  additive,
  limits,
  currencies,
  busy,
  disableError,
  onDisable,
  open,
  onClose,
  onChanged,
}: {
  agent: AgentRef;
  label: string;
  paths: string[];
  /** What this agent's own clients speak ([`AGENT_PROTOCOLS`]). Stated, not
      asked: it is read off the wire format Kiwano already writes into the
      agent's config, and a user editing it here would be editing a fact. */
  protocols: Protocol[];
  /** The key the gateway assigns this agent, which is what its clients are
      configured with. Null only in the states where there is no takeover to
      have assigned one yet. */
  placeholderKey: string | null;
  /** Additive agents: the rewrite leaves the agent's other providers in place. */
  additive: boolean;
  limits: AgentLimit[];
  /** The currencies its own providers bill in — see `LimitSection`. */
  currencies: string[];
  /** A takeover change is in flight. */
  busy?: boolean;
  /** Why turning the takeover off was refused. */
  disableError?: string | null;
  onDisable: () => void;
  open: boolean;
  onClose: () => void;
  onChanged?: () => void;
}) {
  const t = useT();
  const [tab, setTab] = useState<"general" | "limit">("general");
  // Turning the takeover off rewrites the agent's config back, so it takes two
  // clicks: the first arms, the second does it, and arming lapses on its own.
  // The same two steps a provider row's delete takes, for the same reason.
  //
  // The deadline is a number rather than a boolean so the button can say how
  // long the second click stays valid: a two-step control whose window is
  // invisible reads as a control that ignored the first click.
  const [armedUntil, setArmedUntil] = useState<number | null>(null);
  const [remaining, setRemaining] = useState(ARM_SECONDS);
  const armed = armedUntil !== null;
  useEffect(() => {
    if (armedUntil === null) return;
    const tick = () => {
      const left = Math.ceil((armedUntil - Date.now()) / 1000);
      if (left <= 0) {
        setArmedUntil(null);
        setRemaining(ARM_SECONDS);
        return;
      }
      setRemaining(left);
    };
    tick();
    const timer = window.setInterval(tick, 200);
    return () => window.clearInterval(timer);
  }, [armedUntil]);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[460px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">
            {t("providers.agentSettingsFor", { agent: label })}
          </DialogTitle>
        </DialogHeader>
        <div className="px-5 pb-4 pt-1">
          <SettingsTabs tabs={AGENT_TABS} tab={tab} onPick={setTab} />

          <div className="mt-2.5">
            {tab === "general" && (
              <>
                {protocols.length > 0 && (
                  <div className="mt-1.5 flex items-center gap-2">
                    <span className="w-[68px] shrink-0 text-[10.5px] text-mut">
                      {t("providers.agentProtocol")}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[12px]">
                      {protocols.map((p) => protocolName(t, p)).join(" · ")}
                    </span>
                  </div>
                )}
                <div className="mt-1 flex items-center gap-2">
                  <span
                    className="w-[68px] shrink-0 text-[10.5px] text-mut"
                    title={t("providers.agentPlaceholderKeyTitle")}
                  >
                    {t("providers.agentPlaceholderKey")}
                  </span>
                  {placeholderKey ? (
                    <CopyValue value={placeholderKey} />
                  ) : (
                    <span className="text-[11px] text-mut">{t("common.none")}</span>
                  )}
                </div>
                {paths.map((p) => (
                  <div key={p} className="mt-1 flex items-center gap-2">
                    <CopyValue value={p} />
                  </div>
                ))}
                <p className="mt-1.5 text-[10.5px] leading-relaxed text-mut">
                  {t("providers.agentConfigFilesNote")}
                </p>
                {/* How much the rewrite keeps, right under the files it rewrote:
                    an additive agent's config holds the other providers it
                    already had alongside the gateway. */}
                {additive && (
                  <p className="mt-1 text-[10.5px] text-mut">{t("providers.agentAdditive")}</p>
                )}
                {/* The escape hatch back to the agent's own configuration. It
                    sits in the agent's settings rather than in a list of every
                    agent the app knows: this is the one place it is about *this*
                    agent, and the only one that also says what the takeover
                    rewrote. Turning one on stays the onboarding's job — it is a
                    three-step walk, not a toggle. */}
                <div className="mt-3 flex items-center justify-between gap-3 border-t border-line pt-2.5">
                  <span className="text-[10.5px] leading-relaxed text-mut">
                    {t("providers.agentDisableNote")}
                  </span>
                  <Button
                    variant="destructive"
                    size="sm"
                    className="h-7 shrink-0 px-2.5 text-[11px]"
                    // Armed, the border joins in: the colour says "careful",
                    // and the border says "this click is the one that does it".
                    style={armed ? { borderColor: "var(--red)" } : undefined}
                    disabled={busy}
                    onClick={() => {
                      if (armed) {
                        onDisable();
                        return;
                      }
                      setRemaining(ARM_SECONDS);
                      setArmedUntil(Date.now() + ARM_SECONDS * 1000);
                    }}
                  >
                    {armed ? t("providers.agentDisableConfirm") : t("providers.agentDisable")}
                    {armed && (
                      // Visual only: the button's own label is the accessible
                      // name, and a number that changes every second would
                      // make it a moving target for a screen reader.
                      <span aria-hidden className="ml-1.5 text-[10px] tabular-nums opacity-70">
                        {t("providers.agentDisableCountdown", { n: remaining })}
                      </span>
                    )}
                  </Button>
                </div>
                {disableError ? (
                  <div className="mt-1 text-[11px] text-red-400">{disableError}</div>
                ) : null}
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
      </DialogContent>
    </Dialog>
  );
}
