// The usage/quota column: what a provider's numbers are, how a cell is tinted
// when they move, and the two cells that read them.
import { useEffect, useRef, useState, type ReactNode } from "react";

import { useT, type Translate } from "../../i18n";
import { BillTag, Ring, Sparkline } from "../../components/bits";
import { fmtLatency, fmtMoney, fmtTokens } from "../../lib/format";
import { PLAN_TIER_LABEL_KEYS, type PlanQuotaReport, type Provider } from "../../api/types";

function ringColor(billing: Provider["billing"], pct: number): string {
  if (billing === "plan") return "var(--violet)";
  if (pct >= 95) return "var(--red)";
  if (pct >= 80) return "var(--amber)";
  return "var(--kiwi)";
}

/** One quota tier, e.g. `5h window 42%`. Falls back to the backend's own tier
    name, so a tier added there before this list knows about it still reads as
    something rather than disappearing. */
function tierLabel(name: string, t: Translate): string {
  const key = PLAN_TIER_LABEL_KEYS[name];
  return key ? t(key) : name;
}

/** The usage cell's tooltip. It is one sentence per billing shape, so each
    branch is a single key with named placeholders rather than a join: the
    optional 5h/weekly bounds arrive already joined as `windows`, and the
    optional in/out suffix as `tokens`. */
function usageTitle(t: Translate, p: Provider): string {
  if (p.billing === "plan" && p.plan_limits) {
    const l = p.plan_limits;
    const windows = [
      l.five_hour != null ? t("providers.usageWindowFive", { percent: l.five_hour }) : null,
      l.weekly != null ? t("providers.usageWindowWeekly", { percent: l.weekly }) : null,
    ]
      .filter(Boolean)
      .join(" · ");
    const tokens = p.usage
      ? t("providers.usageInOut", {
          input: fmtTokens(p.usage.input_tokens),
          output: fmtTokens(p.usage.output_tokens),
        })
      : "";
    return t("providers.usagePlanLimits", { windows, tokens });
  }
  const u = p.usage;
  if (!u) return "";
  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    // Counted units ("requests", "wan_tokens") print as-is; anything else is
    // a currency code, which is also a data value rather than a label.
    return t("providers.usagePlanQuota", {
      used: u.quota.used,
      limit: u.quota.limit,
      unit: u.quota.unit,
      percent: pct,
      resets: u.quota.resets_at ?? "",
      input: fmtTokens(u.input_tokens),
      output: fmtTokens(u.output_tokens),
    });
  }
  if (p.billing === "unl") return t("providers.usageLocalInference");
  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return t("providers.usagePaygQuota", {
      used: fmtMoney(u.quota.used, u.quota.unit),
      limit: fmtMoney(u.quota.limit, u.quota.unit),
      percent: pct,
      input: fmtTokens(u.input_tokens),
      cache: fmtTokens(u.cache_read_tokens),
      output: fmtTokens(u.output_tokens),
      latency: fmtLatency(u.latency_ms),
    });
  }
  return t("providers.usagePaygTrend");
}

/** The provider's own billing currency, when its limit unit names one.
    Counted units ("requests", "wan_tokens") are not currencies. */
function providerCurrency(p: Provider): string | undefined {
  const unit = p.limit_unit;
  return unit && unit.length === 3 ? unit : undefined;
}

/** Quota amount text: counted units render raw, currencies via fmtMoney */
function quotaAmountText(used: number, limit: number, unit: string): string {
  if (unit === "requests") return `${used}/${limit}`;
  if (unit === "wan_tokens") return `${used}/${limit}`;
  return `${fmtMoney(used, unit)} / ${fmtMoney(limit, unit)}`;
}

/** Tooltip of the live plan-quota line: the failure reason when the query
    failed, otherwise the template id plus a cached marker. `template` is a
    backend identifier and stays untranslated. */
function planLineTitle(t: Translate, plan: PlanQuotaReport | undefined): string {
  if (!plan) return "";
  if (!plan.success) return plan.error ?? "";
  return plan.cached
    ? t("providers.planTemplateCached", { template: plan.template })
    : plan.template;
}

/** Percent-limit plan cell: the ring shows how close the live plan-quota
    utilization is to its configured ceiling (tightest window wins); the
    limit line reads from plan_limits. Independent of usage history, so it
    renders for brand-new providers too. */
function planPercentCell(
  t: Translate,
  p: Provider,
  plan: PlanQuotaReport | undefined,
  title: string,
): ReactNode {
  const l = p.plan_limits!;
  const util = (name: string) =>
    plan?.success ? plan.tiers.find((t) => t.name === name)?.utilization : undefined;
  const five = util("five_hour");
  const week = util("weekly_limit");
  const ratios = [
    l.five_hour != null && five != null ? (five / l.five_hour) * 100 : 0,
    l.weekly != null && week != null ? (week / l.weekly) * 100 : 0,
  ];
  const pct = Math.min(100, Math.round(Math.max(...ratios, 0)));
  const color = pct >= 95 ? "var(--red)" : pct >= 80 ? "var(--amber)" : "var(--kiwi)";
  // Two optional whole phrases with a fixed order, joined by a neutral
  // separator — each is translated on its own so the join needs no key.
  const limitLine = [
    l.five_hour != null ? t("providers.planLimitFive", { percent: l.five_hour }) : null,
    l.weekly != null ? t("providers.planLimitWeekly", { percent: l.weekly }) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const planLine = plan?.success
    ? plan.tiers
        .map((tier) => `${tierLabel(tier.name, t)} ${Math.round(tier.utilization)}%`)
        .join(" · ")
    : plan && !plan.success
      ? plan.error
      : null;
  const maxUtil = plan?.success ? Math.max(...plan.tiers.map((t) => t.utilization), 0) : 0;
  return (
    <div className="flex w-full items-center gap-2" title={title}>
      <Ring pct={pct} color={color} />
      <div className="min-w-0">
        <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
          <BillTag billing="plan" />
          <span>{limitLine}</span>
        </div>
        {/* Live plan quota: per-window utilization from the provider's own endpoint */}
        {planLine && (
          <div
            className="mt-0.5 truncate text-[10.5px]"
            style={{ color: maxUtil >= 95 ? "var(--red)" : maxUtil >= 80 ? "var(--amber)" : "var(--mut)" }}
            title={planLineTitle(t, plan)}
          >
            {planLine}
          </div>
        )}
      </div>
    </div>
  );
}

/** How long a usage cell keeps its tint after its numbers last moved. */
const USAGE_FLASH_MS = 1200;

/** Everything a usage cell shows, as one string: what "these numbers changed"
    is judged on. Built from the same fields the cell renders, so a value that
    moves on screen is a value that moves here — and empty when there is nothing
    to show yet, which the flash reads as "not a number" rather than as one. */
function usageFingerprint(p: Provider, plan?: PlanQuotaReport): string {
  const u = p.usage;
  // The plan line's own numbers, which move without the totals above moving.
  const tiers = plan?.success
    ? plan.tiers.map((tier) => Math.round(tier.utilization)).join(",")
    : "";
  if (!u && !tiers) return "";
  return [
    u?.requests ?? "",
    u?.input_tokens ?? "",
    u?.output_tokens ?? "",
    u?.cache_read_tokens ?? "",
    u?.cache_creation_tokens ?? "",
    u?.quota ? `${u.quota.used}/${u.quota.limit}/${u.quota.unit}` : "",
    tiers,
  ].join("|");
}

/** Whether the numbers behind `fingerprint` have moved since the last render.
 *
 * Judged on the values rather than on a re-render, because re-reads are not
 * news: the gateway ticks once per recorded request, and a screen showing a
 * dozen providers re-reads all of them for one row that changed. A cell that
 * flashed on every re-read would be flashing all the time, which is the same as
 * never.
 *
 * The tint is re-armed rather than restarted while it is up, so a run of
 * requests holds it: the cell then reads as "still moving", which is what it
 * is. */
function useChangeFlash(fingerprint: string): boolean {
  const [changed, setChanged] = useState(false);
  const last = useRef(fingerprint);
  useEffect(() => {
    const previous = last.current;
    last.current = fingerprint;
    if (previous === fingerprint) return;
    // A cell that was empty and now is not is a first read landing, not a number
    // that moved: the row showed nothing because there was nothing to show.
    if (previous === "") return;
    setChanged(true);
    const timer = window.setTimeout(() => setChanged(false), USAGE_FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [fingerprint]);
  return changed;
}

/** Sits in the Usage/quota column.
 *
 * A provider the gateway is refusing to route keeps its numbers here, dimmed:
 * what the period cost is still true and still worth seeing, it is just no
 * longer what is happening. The block itself belongs in the status column,
 * which is the one that answers "what is this row doing right now". */
export function UsageCell({
  p,
  plan,
  blocked,
}: {
  p: Provider;
  plan?: PlanQuotaReport;
  /** Why the gateway is refusing to route here, when it is. */
  blocked?: string;
}) {
  const t = useT();
  const changed = useChangeFlash(usageFingerprint(p, plan));
  return (
    <div
      className={`w-[26%]${blocked ? " opacity-55" : ""}`}
      title={blocked ? t("providers.notRoutingTitle", { reason: blocked }) : undefined}
    >
      {/* The tint goes on the numbers, not on the column: `w-fit` is what keeps
          it to them (see `.usage-cell`). */}
      <div className={`usage-cell w-fit max-w-full${changed ? " changed" : ""}`}>
        <UsageCellBody p={p} plan={plan} />
      </div>
    </div>
  );
}

/** Prompt-cache hit rate: the share of input-side tokens (new input + cache
    reads + cache writes) that came back from cache. "–" when the window holds
    no input-side tokens at all — a 0% there would read as "the cache never
    hit" where the truth is "nothing has been cached yet". */
export function CacheCell({ u }: { u: Provider["usage"] }) {
  const t = useT();
  const denom = u ? u.input_tokens + u.cache_read_tokens + u.cache_creation_tokens : 0;
  if (!u || denom === 0) {
    return (
      <div className="w-[8%] text-[11px] text-mut" title={t("providers.cacheNoData")}>
        –
      </div>
    );
  }
  const pct = Math.round((u.cache_read_tokens / denom) * 100);
  return (
    <div
      className="w-[8%] font-mono text-[11px] text-mut"
      title={t("providers.cacheHitTitle", {
        pct: String(pct),
        read: fmtTokens(u.cache_read_tokens),
        denom: fmtTokens(denom),
      })}
    >
      {pct}%
    </div>
  );
}

function UsageCellBody({ p, plan }: { p: Provider;
  /** Plan-quota report for providers with a plan query (undefined = not fetched yet) */
  plan?: PlanQuotaReport;
}) {
  const t = useT();
  const u = p.usage;

  // Percent-limit rows render even before the provider has any usage
  // history (usage summary still null): the ring/limit line need no totals.
  if (p.billing === "plan" && p.plan_limits) {
    return planPercentCell(t, p, plan, usageTitle(t, p));
  }

  if (!u) return <div className="w-full" />;
  const title = usageTitle(t, p);

  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    const planLine = plan?.success
      ? plan.tiers
          .map((tier) => `${tierLabel(tier.name, t)} ${Math.round(tier.utilization)}%`)
          .join(" · ")
      : plan && !plan.success
        ? plan.error
        : null;
    const maxUtil = plan?.success ? Math.max(...plan.tiers.map((t) => t.utilization), 0) : 0;
    return (
      <div className="flex w-full items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("plan", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="plan" />
            {u.quota.unit === "requests" || u.quota.unit === "wan_tokens" ? (
              <>
                {u.quota.used}/{u.quota.limit}{" "}
                <span className="font-normal text-mut">
                  {u.quota.unit === "requests" ? t("providers.unitReq") : t("providers.unitTenKTok")}
                </span>
              </>
            ) : (
              <span>{quotaAmountText(u.quota.used, u.quota.limit, u.quota.unit)}</span>
            )}
          </div>
          {/* Live plan quota: per-window utilization from the provider's own endpoint */}
          {planLine && (
            <div
              className="mt-0.5 truncate text-[10.5px]"
              style={{ color: maxUtil >= 95 ? "var(--red)" : maxUtil >= 80 ? "var(--amber)" : "var(--mut)" }}
              title={planLineTitle(t, plan)}
            >
              {planLine}
            </div>
          )}
        </div>
      </div>
    );
  }

  // Percent-limit cell (extracted so it renders before usage history exists):
  // the ring tracks how close the live utilization is to its configured
  // ceiling (tightest window wins); legacy used/limit rows render above.
  if (p.billing === "unl") {
    return (
      <div className="w-full" title={title}>
        <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
          <BillTag billing="unl" />
          {u.requests}{" "}
          <span className="font-normal text-mut">
            {t("providers.reqTokSuffix", { tokens: fmtTokens(u.input_tokens + u.output_tokens) })}
          </span>
        </div>
        <div className="mt-0.5 text-[10.5px] text-mut">
          {t("providers.inOutTokens", {
            input: fmtTokens(u.input_tokens),
            output: fmtTokens(u.output_tokens),
          })}
        </div>
      </div>
    );
  }

  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return (
      <div className="flex w-full items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("payg", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="payg" />
            {quotaAmountText(u.quota.used, u.quota.limit, u.quota.unit).split(" / ")[0]}{" "}
            <span className="font-normal text-mut">
              {t("providers.limitSuffix", {
                limit:
                  u.quota.unit === "requests" || u.quota.unit === "wan_tokens"
                    ? u.quota.limit
                    : fmtMoney(u.quota.limit, u.quota.unit),
              })}
            </span>
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {t("providers.tokensLatency", {
              tokens: fmtTokens(u.input_tokens + u.output_tokens),
              latency: fmtLatency(u.latency_ms),
            })}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="w-full" title={title}>
      <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
        <BillTag billing="payg" />
        {fmtMoney(u.cost ?? 0, u.cost_currency ?? providerCurrency(p) ?? "USD")}{" "}
        <span className="font-normal text-mut">
          {t("providers.reqSuffix", { requests: u.requests })}
        </span>
      </div>
      {/* One second line, never two. It says what the spend bought — the tokens
          and the latency — unless there is a 7-day trend to draw, which *is*
          that same thing over time and is the one that gets the line. Both are
          two-line cells then, and the column keeps one rhythm instead of
          growing a third line on exactly the rows that have the most to show.

          "A trend" means two points or more. The backend derives the series and
          the totals from one 7-day window (`vm::usage_vm` / `Aux::provider_daily`),
          so any provider with usage at all has a series — and the common case is
          a *one-element* one, a provider whose traffic all landed on a single
          day. That draws a single dot, which is not a trend and is less than the
          numbers it would replace. */}
      <div className="mt-0.5 flex h-4 items-center text-[10.5px] text-mut">
        {u.spark && u.spark.length > 1 ? (
          <Sparkline points={u.spark} />
        ) : (
          <span>
            {t("providers.tokensLatency", {
              tokens: fmtTokens(u.input_tokens + u.output_tokens),
              latency: fmtLatency(u.latency_ms),
            })}
          </span>
        )}
      </div>
    </div>
  );
}
