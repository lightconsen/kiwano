// The panel's read-only reports: the alert list (beside the once-per-session
// dedup in `../alerts.ts`), a plan's quota, and the currencies the app may
// display in.

import type { CurrencyMeta, KiwanoApi, PlanQuotaReport, UsageAlert } from "../../types";
import { devAlerted } from "../alerts";
import { delay } from "../delay";
import { providers } from "../providers";
import { settings } from "../settings";

export const alertsApi: Pick<
  KiwanoApi,
  | "checkUsageAlerts"
  | "getPlanQuota"
  | "getCurrencyMeta"
> = {
  async checkUsageAlerts(): Promise<UsageAlert[]> {
    await delay();
    // The two feature flags the fixture turns on each get their sample alert,
    // so the notification path is exercisable in `pnpm dev` — but firing them
    // on every 60s poll would make the dev session unusable, so they answer
    // once per page load, the way the backend's per-period dedup reads.
    const out: UsageAlert[] = [];
    if (settings.feat_cost_forecast && !devAlerted.has("forecast")) {
      devAlerted.add("forecast");
      out.push({
        provider_id: "deepseek",
        provider_name: "DeepSeek",
        used: 31.2,
        limit: 50,
        unit: "CNY",
        kind: "cost_forecast",
        message: "DeepSeek: on pace for 62 CNY this period — past the 50 limit",
      });
    }
    if (settings.feat_anomaly_alerts && !devAlerted.has("anomaly")) {
      devAlerted.add("anomaly");
      out.push({
        provider_id: "",
        provider_name: "",
        used: 0,
        limit: 0,
        unit: "",
        kind: "anomaly",
        message: "Error spike: 19 of the last hour's 23 requests failed (83% — 7-day baseline 2%)",
      });
    }
    return out;
  },

  async getPlanQuota(providerId: string, force?: boolean): Promise<PlanQuotaReport> {
    await delay(600);
    const p = providers.find((x) => x.id === providerId);
    const template = p?.plan_query?.template ?? "kimi";
    if (!p?.plan_query) {
      return {
        provider_id: providerId,
        template,
        success: false,
        error: "This provider has no plan query configured",
        note: null,
        tiers: [],
        queried_at: Date.now(),
        cached: false,
      };
    }
    return {
      provider_id: providerId,
      template,
      success: true,
      error: null,
      note: "Standard plan",
      tiers: [
        { name: "five_hour", utilization: 42.5, resets_at: "2026-09-10T18:00:00Z", used: null, limit: null, unit: null },
        { name: "monthly", utilization: 18.3, resets_at: "2026-09-30", used: 295, limit: 460, unit: "requests" },
      ],
      queried_at: Date.now(),
      cached: !force && Math.random() < 0.5,
    };
  },

  async getCurrencyMeta(): Promise<CurrencyMeta> {
    await delay();
    return {
      currencies: ["USD", "CNY"],
      exchange_rates: { USD: 1.0, CNY: 7.1 },
      preferred: settings.preferred_currency,
    };
  },
};
