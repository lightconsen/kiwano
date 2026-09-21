// The agent strip and its "+" menu: one segment per agent the screen shows,
// and the way another one comes to exist.
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Menu,
  MenuContent,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  MenuTrigger,
} from "@/components/ui/menu";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { useT } from "../../i18n";
import { agentMeta } from "../../lib/agents";
import { type AgentId, type AgentRef } from "../../api/types";
import { segmentLabel } from "./agents";

/** The strip. Which agents exist is a read and which one is showing is state,
    so the container owns both and they arrive sorted and filtered; the two
    menu actions are its setters, passed through under their own names. */
export function SegmentStrip({
  visibleSegments,
  seg,
  agentVersions,
  pickSeg,
  declarable,
  setNewAgent,
  setDeclaring,
}: {
  visibleSegments: { id: AgentRef | "all"; icon?: string; label?: string }[];
  seg: AgentRef | "all";
  agentVersions: Partial<Record<AgentRef, string>>;
  pickSeg: (id: AgentRef | "all") => void;
  /** The built-ins the detector could not find, plus the declared ones — the
      container's `declarable` memo, which is what fills the second group. */
  declarable: {
    id: AgentId;
    label: string;
    icon?: string;
    declared: boolean;
    dir: string;
    missing: boolean;
  }[];
  setNewAgent: (open: boolean) => void;
  setDeclaring: (agent: {
    id: AgentId;
    label: string;
    icon?: string;
    dir?: string;
    declared?: boolean;
  }) => void;
}) {
  const t = useT();
  return (
    <div className="flex max-w-full overflow-x-auto rounded-lg border border-line text-[12px]">
      {visibleSegments.map((s, i) => {
        const label = segmentLabel(t, s.id);
        const ver = s.id === "all" ? undefined : agentVersions[s.id];
        return (
          <button
            key={s.id}
            title={s.id === "all" ? label : ver ? `${label} · v${ver}` : label}
            aria-label={label}
            className={`seg flex h-7 shrink-0 items-center justify-center px-2.5${i > 0 ? " border-l border-line" : ""}${seg === s.id ? " active" : ""}`}
            onClick={() => pickSeg(s.id)}
          >
            {s.icon ? (
              <ProviderLogo icon={s.icon} name={label} size={15} />
            ) : s.id !== "all" ? (
              // Any agent with no mark to port — a user-defined route, or a
              // built-in whose logo has not been supplied — reads as its own
              // letter, in the colour the resolver derived for it. The rule
              // used to be "a built-in always has a mark", which stopped
              // being true the moment one arrived without.
              <ProviderLogo
                char={agentMeta(s.id).chip_char}
                color={agentMeta(s.id).chip_color}
                name={label}
                size={15}
              />
            ) : (
              <span className="text-[12px] text-mut">{label}</span>
            )}
          </button>
        );
      })}
      {/* Its own button rather than part of the strip: the strip is a
          switch between agents that exist, and this is how one comes to. */}
      <Menu>
        <MenuTrigger
          render={
            <Button
              variant="ghost"
              size="sm"
              className="h-7 w-7 shrink-0 px-0 text-mut hover:text-ink"
              aria-label={t("providers.addAgentMenu")}
              title={t("providers.addAgentMenuTitle")}
            />
          }
        >
          <Plus className="h-3.5 w-3.5" />
        </MenuTrigger>
        <MenuContent>
          <MenuItem onClick={() => setNewAgent(true)}>
            <Plus className="h-3.5 w-3.5 text-mut" />
            {t("providers.newAgent")}
          </MenuItem>
          {declarable.length > 0 && (
            <>
              <MenuSeparator />
              <MenuLabel>{t("providers.notDetected")}</MenuLabel>
              {declarable.map((a) => (
                <MenuItem key={a.id} onClick={() => setDeclaring(a)}>
                  {a.icon ? (
                    <span aria-hidden className="inline-flex shrink-0">
                      <ProviderLogo icon={a.icon} name={a.label} size={14} />
                    </span>
                  ) : (
                    <span className="h-3.5 w-3.5 shrink-0" />
                  )}
                  <span className="min-w-0 truncate">{a.label}</span>
                  {a.declared && (
                    <span className="ml-auto shrink-0 text-[10.5px] text-mut">
                      {t("providers.alreadyDeclared")}
                    </span>
                  )}
                </MenuItem>
              ))}
            </>
          )}
        </MenuContent>
      </Menu>
    </div>
  );
}
