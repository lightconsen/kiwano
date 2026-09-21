// The Dashboard screen's one read. Its fixtures are in `../dashboard.ts`.

import type { DashboardData, DashboardWindow, KiwanoApi } from "../../types";
import { dashboards } from "../dashboard";
import { delay } from "../delay";

export const dashboardApi: Pick<
  KiwanoApi,
  | "getDashboard"
> = {
  async getDashboard(window: DashboardWindow, providerId?: string, agentId?: string): Promise<DashboardData> {
    await delay();
    const base = dashboards[window];
    // Filter options mirror the backend: who has traffic in the window,
    // independent of the active filter. Ids resolve through the provider
    // list; by_provider labels are short display names, so unmatched ones
    // fall back to the label itself as the id.
    const filter_providers = base.by_provider.map((p) => ({ id: p.id, label: p.name }));
    const filter_agents = base.by_agent.map((a) => ({ id: a.agent, label: a.label }));
    const data: DashboardData = { ...base, filter_providers, filter_agents };
    // The mock narrows the breakdowns only (headline stats stay fixture-wide).
    // By id, which is what the screen filters on: matching on the display name
    // was a lookup that quietly stopped working the moment the two lists spelled
    // a vendor differently.
    if (providerId) {
      data.by_provider = base.by_provider
        .filter((p) => p.id === providerId)
        .map((p) => ({ ...p, pct: 100 }));
    }
    if (agentId) {
      data.by_agent = base.by_agent.filter((a) => a.agent === agentId);
    }
    return data;
  },
};
