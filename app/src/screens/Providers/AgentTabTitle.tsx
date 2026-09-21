// The row that names the tab's agent above the table, in both shapes: a
// user-defined one, whose credentials and delete hide behind the icon, and a
// built-in one, whose row also names the config file a takeover rewrote.
import { Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { useT } from "../../i18n";
import { agentMeta } from "../../lib/agents";
import { type AgentRef, type CustomAgent } from "../../api/types";
import { SEGMENT_ICON } from "./agents";

export function AgentTabTitle({
  seg,
  custom,
  takenOver,
  configPaths,
  setAccessOpen,
  setAgentSettingsOpen,
}: {
  seg: AgentRef | "all";
  /** The agent this tab belongs to, when the user defined it. */
  custom: CustomAgent | undefined;
  /** The agents the registry has taken over — the container's read, and what
      decides whether the built-in branch below renders at all. */
  takenOver: Set<AgentRef> | null;
  configPaths: string[];
  setAccessOpen: (open: boolean) => void;
  setAgentSettingsOpen: (open: boolean) => void;
}) {
  const t = useT();
  return (
    <>
      {/* A user-defined agent's tab starts with the two values a client is
          configured with — the whole of what it is from the outside. */}
      {custom && (
        // The tab names the agent: the strip shows it as a letter avatar, so a
        // page with no name on it is one you have to identify from the highlight
        // over there. Everything *about* it — the credentials, and deleting it —
        // is behind the icon, which also keeps a destructive control out of the
        // row you click around in.
        <div className="mx-4 my-0.5 flex items-center gap-1.5">
          <ProviderLogo
            char={agentMeta(seg).chip_char}
            color={agentMeta(seg).chip_color}
            name={custom.label}
            size={18}
          />
          <span className="text-[13px] font-semibold">{custom.label}</span>
          {custom.note && (
            <span className="min-w-0 truncate text-[11px] text-mut">{custom.note}</span>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
            aria-label={t("providers.agentSettingsFor", { agent: custom.label })}
            title={t("providers.agentSettingsFor", { agent: custom.label })}
            onClick={() => setAccessOpen(true)}
          >
            <Settings2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      )}

      {/* A built-in agent's tab names itself the same way a user-defined one
          does — the strip shows icons, so a page with no name on it is one you
          have to identify from the highlight over there. What this one adds is
          the file behind it: the config a takeover rewrites, which is also what
          a user has to reach for by hand when something goes wrong. */}
      {!custom && seg !== "all" && (takenOver?.has(seg) ?? false) && (
        <div className="mx-4 my-0.5 flex items-center gap-1.5">
          <ProviderLogo
            icon={SEGMENT_ICON[seg]}
            char={agentMeta(seg).chip_char}
            color={agentMeta(seg).chip_color}
            name={agentMeta(seg).label}
            size={18}
          />
          <span className="text-[13px] font-semibold">{agentMeta(seg).label}</span>
          {configPaths.length > 0 && (
            <span className="min-w-0 truncate font-mono text-[11px] text-mut">
              {configPaths[0]}
              {/* The rest are listed in the dialog rather than elided here: codex
                  keeps two files and claude-desktop four, and a row that showed
                  only the first would read as if that were the whole of it. */}
              {configPaths.length > 1 ? ` +${configPaths.length - 1}` : ""}
            </span>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
            aria-label={t("providers.agentSettingsFor", { agent: agentMeta(seg).label })}
            title={t("providers.agentSettingsFor", { agent: agentMeta(seg).label })}
            onClick={() => setAgentSettingsOpen(true)}
          >
            <Settings2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      )}
    </>
  );
}
