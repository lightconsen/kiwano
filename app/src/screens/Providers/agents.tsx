// Agent tabs: the segments the strip offers, what each one is called, and the
// first-touch panel an agent tab with no bound providers shows.
import { type ReactNode } from "react";

import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { useT, type Translate } from "../../i18n";
import { agentMeta } from "../../lib/agents";
import { type AgentId, type AgentRef } from "../../api/types";

// Built-ins with no command-line tool to point at: a directory declaration
// cannot make either of them appear, so they are left out of that menu. The
// same two are the ones missing from `CLI_AGENTS` on the Rust side.
export const NO_CLI_AGENTS: AgentId[] = ["claude-desktop", "workbuddy"];

// Agent filter segments — each renders the agent's brand logo (ported with
// the cc-switch icon set, see components/icons). Hover shows the full name.
export const SEGMENTS: { id: AgentRef | "all"; icon?: string; label?: string }[] = [
  { id: "all" },
  { id: "claude", icon: "claudecode" },
  { id: "codex", icon: "openai" },
  { id: "gemini", icon: "gemini" },
  { id: "grokbuild", icon: "grok" },
  { id: "claude-desktop", icon: "claude" },
  { id: "opencode", icon: "opencode" },
  { id: "openclaw", icon: "openclaw" },
  { id: "hermes", icon: "hermes" },
  { id: "pi", icon: "pi" },
  { id: "workbuddy", icon: "workbuddy" },
  { id: "codebuddy", icon: "codebuddy" },
  { id: "kimi", icon: "kimi" },
  { id: "qwen", icon: "qwen" },
  { id: "cline", icon: "cline" },
  { id: "mimo", icon: "xiaomimimo" },
  { id: "mcode", icon: "minimax" },
  { id: "aider", icon: "aider" },
];

// Agent labels are brand names and stay as they are; only the "all" segment
// has a translatable label.
export function segmentLabel(t: Translate, id: AgentRef | "all"): string {
  if (id === "all") return t("providers.all");
  // The registry, the user's own agents, or — for an id nothing knows — the id.
  return agentMeta(id).label;
}

export const SEGMENT_ICON: Partial<Record<AgentRef, string>> = Object.fromEntries(
  SEGMENTS.filter((s) => s.icon).map((s) => [s.id, s.icon!]),
);

/** First-touch state for an agent tab with no bound providers: one click to
    back up + take over (importing the provider the agent already uses), or
    manual entry. When already taken over but unbound, steer to binding an
    existing provider (bindSlot) or manual add. */
export function AgentOnboarding({
  agent,
  installed,
  takenOver,
  kind = "takeover",
  busy,
  onTakeover,
  onAdd,
  bindSlot,
  copySlot,
  takeoverError,
}: {
  /** "takeover": a built-in agent Kiwano has not taken over yet.
      "route": a user-defined agent with no candidates — there is nothing to
      take over, so the panel is only about assigning one. */
  kind?: "takeover" | "route";
  agent: AgentRef;
  installed: boolean;
  takenOver: boolean;
  busy: boolean;
  onTakeover: () => void;
  onAdd: () => void;
  /** Why the last attempt to take this agent over was refused, if it was. The
      backend refuses for reasons the user has to act on (a config that is not
      there yet, a variable that points nowhere), so silence here is a dead end. */
  takeoverError?: string | null;
  /** Extra first-candidate entry (bind an existing provider) for the taken-over branch */
  bindSlot?: ReactNode;
  /** Copy another agent's whole route — available right after Enable, before any binding exists */
  copySlot?: ReactNode;
}) {
  const t = useT();
  const meta = agentMeta(agent);
  const routeOnly = kind === "route";
  return (
    <div className="flex flex-col items-center gap-3 px-4 py-10 text-center">
      <ProviderLogo
        icon={SEGMENT_ICON[agent]}
        char={meta.chip_char}
        color={meta.chip_color}
        name={meta.label}
        size={36}
      />
      {routeOnly ? (
        <>
          <div className="text-[13px] font-semibold">
            {t("providers.routeEmptyTitle", { agent: meta.label })}
          </div>
          <div className="max-w-[460px] text-[12px] text-mut">{t("providers.routeEmptyBody")}</div>
          <div className="flex items-center gap-2">
            {bindSlot}
            <Button size="sm" className="h-7 gap-1 px-3 text-[12px] font-semibold" onClick={onAdd}>
              <Plus className="h-3.5 w-3.5" />
              {t("providers.addProvider")}
            </Button>
          </div>
          {copySlot}
        </>
      ) : takenOver ? (
        <>
          <div className="text-[13px] font-semibold">
            {t("providers.takenOverTitle", { agent: meta.label })}
          </div>
          <div className="max-w-[430px] text-[12px] text-mut">{t("providers.takenOverBody")}</div>
          <div className="flex items-center gap-2">
            {bindSlot}
            <Button size="sm" className="h-7 gap-1 px-3 text-[12px] font-semibold" onClick={onAdd}>
              <Plus className="h-3.5 w-3.5" />
              {t("providers.addProvider")}
            </Button>
          </div>
          {copySlot}
        </>
      ) : (
        <>
          <div className="text-[13px] font-semibold">
            {t("providers.startManaging", { agent: meta.label })}
          </div>
          <div className="max-w-[460px] text-[12px] text-mut">
            {t("providers.startManagingBody", { agent: meta.label })}
          </div>
          <div className="flex gap-2">
            <Button
              size="sm"
              className="h-7 px-3 text-[12px] font-semibold"
              disabled={busy}
              onClick={onTakeover}
            >
              {busy ? t("providers.enabling") : t("providers.enableKiwano")}
            </Button>
            <Button variant="ghost" size="sm" className="h-7 px-3 text-[12px]" onClick={onAdd}>
              {t("providers.addProviderFirst")}
            </Button>
          </div>
          {takeoverError && (
            <div
              className="max-w-[470px] text-[11px]"
              style={{ color: "var(--red)" }}
              role="alert"
            >
              {takeoverError}
            </div>
          )}
          {installed && (
            <div className="max-w-[470px] text-[11px] text-mut">{t("providers.oauthNote")}</div>
          )}
        </>
      )}
    </div>
  );
}
