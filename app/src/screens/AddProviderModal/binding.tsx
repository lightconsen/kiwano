// The agents a new provider is bound to, as one multi-select.
import { Check, ChevronDown } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { AgentChip } from "@/components/bits";
import { Label } from "@/components/ui/label";
import { useT } from "../../i18n";
import { allAgentMetas } from "../../lib/agents";
import type { AgentRef } from "../../api/types";

/** Which agents this provider serves. Adding only: an edit leaves the bindings
    to the agent tabs, whose own strategy panels are what reorder them. */
export function AgentBinding({
  agents,
  setAgents,
  agentsOpen,
  setAgentsOpen,
}: {
  agents: AgentRef[];
  setAgents: Dispatch<SetStateAction<AgentRef[]>>;
  agentsOpen: boolean;
  setAgentsOpen: Dispatch<SetStateAction<boolean>>;
}) {
  const t = useT();
  return (
    <div className="relative">
      <Label
        className="cursor-pointer select-none text-[11px] font-medium text-mut"
        onClick={() => setAgentsOpen((o) => !o)}
      >
        {t("addProvider.bindAgents")}
      </Label>
      <button
        type="button"
        className="mt-1 flex min-h-8 w-full cursor-pointer flex-wrap items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-left text-[12px] dark:bg-bg"
        onClick={() => setAgentsOpen((o) => !o)}
      >
        {agents.length === 0 ? (
          <span className="text-mut">{t("addProvider.selectAgents")}</span>
        ) : (
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
            {allAgentMetas().filter((a) => agents.includes(a.id)).map((a) => (
              <span
                key={a.id}
                className="flex items-center gap-1 rounded bg-surface2 px-1 py-0.5 text-[10.5px]"
              >
                <AgentChip meta={a} size={12} />
                {a.label}
              </span>
            ))}
          </span>
        )}
        <ChevronDown
          className={`ml-auto h-3.5 w-3.5 flex-none text-mut transition-transform ${agentsOpen ? "rotate-180" : ""}`}
        />
      </button>
      {agentsOpen && (
        <>
          {/* Click-catcher behind the panel: any press elsewhere in the
              modal lands here and closes the dropdown */}
          <div className="fixed inset-0 z-40" onClick={() => setAgentsOpen(false)} />
          <div className="absolute z-50 mt-1 max-h-[210px] w-full overflow-y-auto rounded-md border border-line bg-bg py-1 shadow-lg">
          {allAgentMetas().map((a) => {
            const active = agents.includes(a.id);
            return (
              <button
                key={a.id}
                type="button"
                className="flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-left text-[12px] hover:bg-surface2"
                onClick={() =>
                  setAgents(active ? agents.filter((x) => x !== a.id) : [...agents, a.id])
                }
              >
                <AgentChip meta={a} size={16} />
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
                {active && <Check className="h-3.5 w-3.5 flex-none" style={{ color: "var(--kiwi)" }} />}
              </button>
            );
          })}
          </div>
        </>
      )}
    </div>
  );
}
