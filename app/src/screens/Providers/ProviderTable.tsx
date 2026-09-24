// The table: the column header, the rows (the All tab's provider rows, or the
// agent tab's candidate queue), the bind-one-more row, and the two empty
// states. Which one it is comes from the same facts the container holds.
import { useT } from "../../i18n";
import { api } from "../../api/client";
import { CopyRouteRow } from "../../components/StrategyPanel";
import {
  type AgentDetect,
  type AgentRef,
  type AgentRoute,
  type CustomAgent,
  type PlanQuotaReport,
  type Provider,
} from "../../api/types";
import { AgentOnboarding } from "./agents";
import { AddBindingRow, BindingRow, BindProviderSelect } from "./BindingRow";
import { ProviderRow } from "./rows";

export function ProviderTable({
  seg,
  custom,
  takenOver,
  agentDetect,
  enabling,
  setAgentTakenOver,
  takeoverError,
  onAdd,
  route,
  byId,
  planQuotas,
  blocked,
  onMoveBinding,
  refetch,
  filtered,
  routes,
  onEdit,
  onDelete,
  providers,
}: {
  seg: AgentRef | "all";
  custom: CustomAgent | undefined;
  takenOver: Set<AgentRef> | null;
  agentDetect: AgentDetect[] | null;
  enabling: boolean;
  setAgentTakenOver: (agent: AgentRef, enabled: boolean) => Promise<boolean>;
  takeoverError: string | null;
  onAdd: () => void;
  route: AgentRoute | null;
  byId: Map<string, Provider>;
  planQuotas: Record<string, PlanQuotaReport>;
  blocked: Record<string, string>;
  onMoveBinding: (idx: number, dir: -1 | 1) => void;
  /** The screen's one "something changed" callback, passed straight through. */
  refetch: () => void;
  filtered: Provider[];
  routes: AgentRoute[] | null;
  onEdit: (p: Provider) => void;
  onDelete: (p: Provider) => void;
  providers: Provider[];
}) {
  const t = useT();
  // Disabling a takeover keeps the stored route (re-enabling restores it), but
  // the agent config no longer points at the gateway — the route is dormant.
  // An agent tab for an agent that is not taken over shows the (re-)takeover
  // onboarding instead of the dormant binding rows.
  //
  // A user-defined agent has no config to take over, so it is never in that
  // state: its tab shows its route from the start.
  const notTakenOver = seg !== "all" && !custom && !(takenOver?.has(seg) ?? false);

  return (
    <>
      {notTakenOver ? (
        <AgentOnboarding
          agent={seg}
          // No detection answer yet is no evidence of installation: the
          // honest default is "not confirmed", which only affects the OAuth
          // note's visibility.
          installed={
            agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : false
          }
          takenOver={false}
          busy={enabling}
          onTakeover={() => void setAgentTakenOver(seg, true)}
          takeoverError={takeoverError}
          onAdd={onAdd}
        />
      ) : (
        <>
          <div className="flex h-7 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
            <span className="w-[31%]">{t("providers.colProvider")}</span>
            <span className="w-[16%]">
              {route ? t("providers.colRole") : t("providers.colBoundAgents")}
            </span>
            <span className="w-[26%]">{t("providers.colUsage")}</span>
            <span className="w-[8%]" title={t("providers.cacheColTitle")}>
              {t("providers.colCache")}
            </span>
            <span className="w-[12%]">{t("providers.colStatus")}</span>
            <span className="min-w-[140px] flex-1 text-right">
              {route ? t("providers.colPriority") : t("providers.colActions")}
            </span>
          </div>

          {route
            ? route.bindings.map((b, i) => {
                const p = byId.get(b.provider_id);
                // Skip a binding whose provider row vanished (deleted mid-session)
                return p ? (
                  <BindingRow
                    key={b.provider_id}
                    p={p}
                    route={route}
                    b={b}
                    idx={i}
                    plan={planQuotas[p.id]}
                    blocked={blocked[p.id]}
                    onMove={onMoveBinding}
                    onChanged={refetch}
                  />
                ) : null;
              })
            : filtered.map((p) => (
                <ProviderRow
                  key={p.id}
                  p={p}
                  plan={planQuotas[p.id]}
                  blocked={blocked[p.id]}
                  routes={routes}
                  onEdit={onEdit}
                  onDelete={onDelete}
                  onChanged={refetch}
                />
              ))}

          {/* Bind-one-more entry under the candidate queue of an agent tab */}
          {route && (
            <AddBindingRow
              agent={route.agent}
              providers={providers}
              boundIds={new Set(route.bindings.map((b) => b.provider_id))}
              onChanged={refetch}
            />
          )}

          {filtered.length === 0 && seg !== "all" && (
            <AgentOnboarding
              agent={seg}
              kind={custom ? "route" : "takeover"}
              installed={
                agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : false
              }
              takenOver={true}
              busy={enabling}
              onTakeover={() => void setAgentTakenOver(seg, true)}
              takeoverError={takeoverError}
              onAdd={onAdd}
              bindSlot={
                <BindProviderSelect
                  providers={providers}
                  boundIds={new Set(providers.filter((p) => p.agents.includes(seg)).map((p) => p.id))}
                  onPick={(pid) => api.addAgentBinding(seg, pid).then(refetch)}
                />
              }
              copySlot={
                <CopyRouteRow agent={seg} routes={routes ?? []} onChanged={refetch} />
              }
            />
          )}

          {filtered.length === 0 && seg === "all" && (
            <div className="px-4 py-8 text-center text-[12px] text-mut">
              {t("providers.none")}{" "}
              <button className="font-semibold" style={{ color: "var(--kiwi)" }} onClick={onAdd}>
                {t("providers.addProvider")}
              </button>
            </div>
          )}
        </>
      )}
    </>
  );
}
