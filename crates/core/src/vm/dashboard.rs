//! The dashboard and the footer: the windowed trend, the provider and agent
//! breakdowns, latency, and the gateway status the app bar shows.

use crate::vm::agents::list_agents;
use crate::vm::fmt::{chart_palette, fmt_tokens};
use crate::vm::settings::tz_offset;
use crate::vm::time::{
    day_index_of_key, hh00, hour_key, local_day_key, local_day_start, local_day_start_from, mmdd,
    unix_now,
};
use crate::vm::{e2s, Aux};
use kiwanod::store::{Store, UsageTotals};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct TrendVm {
    pub date: String,
    pub requests: i64,
    pub tokens: i64,
}

#[derive(Serialize)]
pub struct ProviderDistVm {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Requests attributed to this provider in the window — what `pct` is a
    /// share of, so a chart can size its segments without re-deriving them.
    pub requests: i64,
    pub pct: i64,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
}

#[derive(Serialize)]
pub struct AgentDistVm {
    pub agent: String,
    pub label: String,
    pub requests: i64,
    pub tokens: String,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
}

/// One select option of the dashboard's provider/agent filters.
#[derive(Serialize)]
pub struct FilterOptionVm {
    pub id: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct DashboardVm {
    pub window: String,
    pub requests: i64,
    /// How this window's request count compares with the window before it, in
    /// percent. `None` when there is nothing to compare against — the "all"
    /// window has no earlier period, and an earlier period with no traffic in it
    /// has no percentage to give. Absent is not 0: a delta of zero says the two
    /// windows matched, which is a claim of its own.
    pub requests_delta_pct: Option<i64>,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
    pub latency_ms: i64,
    /// The same comparison for the average latency, and absent for the same
    /// reasons. Positive means slower than the window before it; the screen
    /// colours it as the bad direction.
    pub latency_delta_pct: Option<i64>,
    pub trend: Vec<TrendVm>,
    pub by_provider: Vec<ProviderDistVm>,
    pub by_agent: Vec<AgentDistVm>,
    /// Filter select options: providers/agents with traffic in the window,
    /// computed independent of the active filter (otherwise the option list
    /// would collapse to the current selection).
    pub filter_providers: Vec<FilterOptionVm>,
    pub filter_agents: Vec<FilterOptionVm>,
}

#[derive(Serialize)]
pub struct GatewayStatusVm {
    pub running: bool,
    pub port: u16,
    /// Providers the gateway is refusing to route, with the reason it gave.
    ///
    /// Read from the gateway rather than recomputed here: the block is decided
    /// there, and a card that worked out its own answer could disagree with the
    /// process actually turning requests away.
    pub blocked: Vec<BlockedProviderVm>,
}

#[derive(Serialize)]
pub struct BlockedProviderVm {
    pub id: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct FooterStatsVm {
    pub today_requests: i64,
    /// Tokens consumed today (input + output) — the footer's headline metric;
    /// cost stays out of the status bar until price tables land (P1).
    pub today_tokens: i64,
    pub hub_synced: bool,
    pub version: String,
}

// ── Dashboard ──

/// How many days the trend chart draws one bar per day for. Past it the bars
/// are summed a week at a time — see the trend branch below for why a limit
/// rather than always-weekly.
const TREND_DAILY_LIMIT_DAYS: i64 = 62;

/// How much a figure moved against the same figure measured over the window
/// before it, as a whole percent.
///
/// `None` when the earlier figure is nothing at all: a percentage of zero has no
/// value, and "+∞%" is not one. Reporting 0 there would be a claim that the two
/// windows matched, which is a different — and false — statement.
fn delta_pct(current: f64, previous: f64) -> Option<i64> {
    (previous > 0.0).then(|| ((current - previous) / previous * 100.0).round() as i64)
}

/// The window a dashboard request resolved to: the key it echoes back, the lower
/// bound the queries take, the local day index today sits on, and the window
/// before this one.
struct DashWindow {
    key: &'static str,
    since: Option<String>,
    days: i64,
    today_days: i64,
    tz: i64,
    previous: Option<(String, String)>,
}

fn resolve_window(aux: &Aux, now: i64, window: &str) -> DashWindow {
    let tz = tz_offset(aux);
    // Whole local calendar days, so a stat and its chart describe the same
    // span: 7 days is today plus the six before it, not a rolling 168 hours
    // (which would count the hours between 6 and 7 days back that the chart's
    // seven daily points cannot show).
    let (key, since, days) = match window {
        "today" => ("today", Some(local_day_start(tz, now)), 1),
        "30d" => ("30d", Some(local_day_start(tz, now - 29 * 86_400)), 30),
        // Everything the store holds. No lower bound is invented here: a window
        // that began whenever this install did is the question being asked, and
        // the chart finds its own left edge from the oldest bucket below.
        "all" => ("all", None, 0),
        _ => ("7d", Some(local_day_start(tz, now - 6 * 86_400)), 7),
    };
    // The local day index today sits on (the chart's right edge), and the window
    // before this one: the same length, ending exactly where this one starts.
    // `None` for "all", which has no earlier period — it *is* every period.
    let today_days = (now + tz * 60).div_euclid(86_400);
    let previous: Option<(String, String)> = since.as_ref().map(|start| {
        (
            local_day_start_from(tz, today_days - 2 * days + 1),
            start.clone(),
        )
    });
    DashWindow {
        key,
        since,
        days,
        today_days,
        tz,
        previous,
    }
}

/// What the costs panel needs, resolved once for the page.
struct DashboardCosts {
    /// Headline cost for the window in the preferred currency, **unrounded**:
    /// the caller rounds it once, where it becomes a `DashboardVm` field.
    cost: f64,
    /// The off-peak equivalent of the same rows, rounded here as it always was.
    cost_off_peak: f64,
    by_pid: HashMap<String, f64>,
    off_peak_by_pid: HashMap<String, f64>,
    preferred: String,
    rates: HashMap<String, f64>,
}

impl DashboardCosts {
    /// The same roll-up the headline used, so the per-agent card prices its
    /// buckets exactly the way the stat above it does.
    fn convert(&self, buckets: &[(Option<String>, f64)]) -> f64 {
        crate::pricing::convert_cost_buckets(buckets, &self.preferred, &self.rates)
    }
}

fn dashboard_costs(
    store: &Store,
    aux: &Aux,
    agent: Option<&str>,
    provider_id: Option<&str>,
    since: Option<&str>,
) -> Result<DashboardCosts, String> {
    // Cost rolls up per-currency buckets (each row's cost_currency) into the
    // user's preferred display currency via the Hub's published rates, so this
    // agrees with the currency selector. Before the first sync there are no
    // rates and the buckets are summed as-is.
    let rates = crate::pricing::effective_rates(aux);
    let preferred = crate::pricing::preferred_currency(aux);

    // Headline cost for the window, with the off-peak equivalent of the same
    // rows beside it. Both sums come from one query over one row set, which is
    // what makes their difference "what running at peak cost you" rather than a
    // comparison of two different populations.
    let headline = store
        .usage_cost_with_off_peak_by_currency(agent, provider_id, since)
        .map_err(e2s)?;
    let pairs = |pick: fn(&kiwanod::store::CostBucket) -> f64| -> Vec<(Option<String>, f64)> {
        headline
            .iter()
            .map(|b| (b.currency.clone(), pick(b)))
            .collect()
    };
    let cost = crate::pricing::convert_cost_buckets(&pairs(|b| b.cost), &preferred, &rates);
    let cost_off_peak =
        (crate::pricing::convert_cost_buckets(&pairs(|b| b.cost_off_peak), &preferred, &rates)
            * 1e6)
            .round()
            / 1e6;

    // Per-provider cost for the distribution card.
    let mut by_pid: HashMap<String, f64> = HashMap::new();
    let mut off_peak_by_pid: HashMap<String, f64> = HashMap::new();
    for b in store.usage_cost_by_provider(agent, since).map_err(e2s)? {
        let convert = |c: f64| match b.currency.as_deref() {
            Some(cur) => crate::pricing::convert_amount(c, cur, &preferred, &rates),
            None => 0.0,
        };
        *by_pid.entry(b.provider_id.clone()).or_default() += convert(b.cost);
        *off_peak_by_pid.entry(b.provider_id).or_default() += convert(b.cost_off_peak);
    }

    Ok(DashboardCosts {
        cost,
        cost_off_peak,
        by_pid,
        off_peak_by_pid,
        preferred,
        rates,
    })
}

/// The "today" chart: the day's 24 local hours, zero-filled.
fn trend_today(
    store: &Store,
    agent: Option<&str>,
    provider_id: Option<&str>,
    since: Option<&str>,
    today_days: i64,
    tz: i64,
) -> Result<Vec<TrendVm>, String> {
    let mut trend = Vec::new();
    // "today" plots the day's hours, not one bar for the whole day: 24 local
    // hour buckets, zero-filled exactly like the daily axis so the chart
    // spans the same day the stat above it counts (and its bars still sum
    // to that stat).
    let mut hourly: HashMap<String, UsageTotals> = HashMap::new();
    for b in store
        .usage_hourly(agent, provider_id, since, tz)
        .map_err(e2s)?
    {
        hourly.insert(b.day, b.totals);
    }
    // The local day index, turned back into the 24 hour keys of that day.
    for h in 0..24 {
        let key = hour_key(today_days * 86_400 + h * 3_600);
        let t = hourly.get(&key).cloned().unwrap_or_default();
        trend.push(TrendVm {
            date: hh00(&key),
            requests: t.requests,
            tokens: t.input_tokens + t.output_tokens,
        });
    }
    Ok(trend)
}

/// Every other window: one bar per local day, or per week once the days stop
/// fitting side by side.
fn trend_daily(
    store: &Store,
    agent: Option<&str>,
    provider_id: Option<&str>,
    w: &DashWindow,
) -> Result<Vec<TrendVm>, String> {
    let mut trend = Vec::new();
    // One bar per local day, zero-filled: the chart draws exactly the days
    // the window selected, so its bars sum to the stat above it and each
    // label names the one day its own bar covers. Merging days (30d used to
    // draw six five-day blocks) made a bar mean something the axis could
    // not say.
    let mut daily: HashMap<String, UsageTotals> = HashMap::new();
    for d in store
        .usage_daily(agent, provider_id, w.since.as_deref(), w.tz)
        .map_err(e2s)?
    {
        daily.insert(d.day, d.totals);
    }
    // Local day index: the buckets have to be the same days the window
    // above selected, or the chart and its stat disagree again.
    // Where the chart starts. A counted window is a fixed number of days
    // back from today; "all" is wherever the oldest recorded row is, which
    // only the buckets themselves can say.
    let first_days = if w.key == "all" {
        daily
            .keys()
            .filter_map(|k| day_index_of_key(k))
            .min()
            .unwrap_or(w.today_days)
    } else {
        w.today_days - (w.days - 1)
    };
    let span = (w.today_days - first_days + 1).max(1);
    // A day per bar reads while the days fit side by side. Past the point
    // where they stop fitting, the same rows are summed a week at a time and
    // a bar says which week it starts — an installation older than that has
    // stopped asking about individual days, and a hundred hairline columns
    // answer nothing.
    let step = if span > TREND_DAILY_LIMIT_DAYS { 7 } else { 1 };
    let mut start = first_days;
    while start <= w.today_days {
        let mut t = UsageTotals::default();
        for offset in 0..step {
            let day = start + offset;
            if day > w.today_days {
                break;
            }
            let key = local_day_key(w.tz, day * 86_400);
            if let Some(row) = daily.get(&key) {
                t.requests += row.requests;
                t.input_tokens += row.input_tokens;
                t.output_tokens += row.output_tokens;
                t.cache_read_tokens += row.cache_read_tokens;
                t.cache_creation_tokens += row.cache_creation_tokens;
            }
        }
        let key = local_day_key(w.tz, start * 86_400);
        trend.push(TrendVm {
            date: mmdd(&key),
            requests: t.requests,
            tokens: t.input_tokens + t.output_tokens,
        });
        start += step;
    }
    Ok(trend)
}

/// The provider distribution card, colours included: they are assigned last
/// because they depend on the whole roster (see `chart_palette`), and assigning
/// them after the sort keeps the largest slice's slot stable.
fn provider_distribution(
    store: &Store,
    agent: Option<&str>,
    provider_id: Option<&str>,
    since: Option<&str>,
    names: &HashMap<String, String>,
    total_req: i64,
    costs: &DashboardCosts,
) -> Result<Vec<ProviderDistVm>, String> {
    let mut by_provider: Vec<ProviderDistVm> = store
        .usage_by_provider(agent, provider_id, since)
        .map_err(e2s)?
        .into_iter()
        .filter(|pu| pu.totals.requests > 0)
        .map(|pu| {
            let name = names
                .get(&pu.provider_id)
                .cloned()
                .unwrap_or(pu.provider_id.clone());
            ProviderDistVm {
                id: pu.provider_id.clone(),
                name,
                color: String::new(), // assigned below, once the list is fixed
                requests: pu.totals.requests,
                pct: pu.totals.requests * 100 / total_req,
                cost: (costs.by_pid.get(&pu.provider_id).copied().unwrap_or(0.0) * 1e6).round()
                    / 1e6,
                cost_off_peak: (costs
                    .off_peak_by_pid
                    .get(&pu.provider_id)
                    .copied()
                    .unwrap_or(0.0)
                    * 1e6)
                    .round()
                    / 1e6,
            }
        })
        .collect();
    by_provider.sort_by_key(|p| std::cmp::Reverse(p.pct));
    // Colours last: they depend on the whole roster (see `chart_palette`), and
    // assigning them after the sort keeps the largest slice's slot stable.
    let ids: Vec<String> = by_provider.iter().map(|p| p.id.clone()).collect();
    for (entry, color) in by_provider.iter_mut().zip(chart_palette(&ids)) {
        entry.color = color.to_string();
    }
    Ok(by_provider)
}

/// Who gets a row: the registry in its own order (which is the order this
/// table has always used), then the user's own agents, then anything else
/// with traffic in the window. That last group is an agent whose route was
/// deleted: its usage rows stay, and its traffic should not disappear from
/// the page just because the name did.
fn agent_roster(store: &Store, since: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let mut roster: Vec<(String, String)> = list_agents(store)?;
    for id in store.usage_agents(since).map_err(e2s)? {
        if !roster.iter().any(|(a, _)| *a == id) {
            roster.push((id.clone(), id));
        }
    }
    Ok(roster)
}

/// The per-agent breakdown. `roster` is passed in rather than rebuilt: the
/// filter options below walk the same list.
fn agent_distribution(
    store: &Store,
    agent: Option<&str>,
    provider_id: Option<&str>,
    since: Option<&str>,
    roster: &[(String, String)],
    costs: &DashboardCosts,
) -> Result<Vec<AgentDistVm>, String> {
    let mut by_agent = Vec::new();
    for (name, label) in roster {
        // The agent filter narrows this breakdown like every other panel,
        // leaving one row at 100% when one agent is selected. `name` is the
        // loop's, not the filter's, so skipping here is what applies it — a
        // table that kept every agent would total more than the headline.
        if agent.is_some_and(|a| a != name.as_str()) {
            continue;
        }
        let t = store
            .usage_totals(Some(name), provider_id, since)
            .map_err(e2s)?;
        if t.requests > 0 {
            let buckets = store
                .usage_cost_with_off_peak_by_currency(Some(name), provider_id, since)
                .unwrap_or_default();
            let off_peak = costs.convert(
                &buckets
                    .iter()
                    .map(|b| (b.currency.clone(), b.cost_off_peak))
                    .collect::<Vec<_>>(),
            );
            by_agent.push(AgentDistVm {
                agent: name.clone(),
                label: label.clone(),
                requests: t.requests,
                tokens: fmt_tokens(t.input_tokens + t.output_tokens),
                cost: (costs.convert(
                    &buckets
                        .iter()
                        .map(|b| (b.currency.clone(), b.cost))
                        .collect::<Vec<_>>(),
                ) * 1e6)
                    .round()
                    / 1e6,
                cost_off_peak: (off_peak * 1e6).round() / 1e6,
            });
        }
    }
    Ok(by_agent)
}

/// The headline latency and its change against the window before.
fn latency_pair(
    aux: &Aux,
    provider_id: Option<&str>,
    agent: Option<&str>,
    since: Option<&str>,
    previous: Option<&(String, String)>,
) -> (i64, Option<i64>) {
    let latency_now = aux.avg_latency(provider_id, agent, since, None);
    let latency_before =
        previous.and_then(|(from, to)| aux.avg_latency(provider_id, agent, Some(from), Some(to)));
    // Both halves have to exist: an average with nothing to compare it against is
    // not a change, and neither is one measured over no rows. `avg_latency`
    // already ignores rows with no latency, so "no average" means "nothing was
    // measured", which is the honest absence this leaves as None.
    let delta = match (latency_now, latency_before) {
        (Some(cur), Some(before)) => delta_pct(cur as f64, before as f64),
        _ => None,
    };
    (latency_now.unwrap_or(0), delta)
}

/// The two filter selects: who has traffic in the window, independent of the
/// active filter. The provider side reuses the same per-provider aggregation
/// (query already orders by request count DESC); the agent side mirrors the
/// `agent_distribution` loop without its provider narrowing.
fn filter_options(
    store: &Store,
    since: Option<&str>,
    roster: &[(String, String)],
    names: &HashMap<String, String>,
) -> Result<(Vec<FilterOptionVm>, Vec<FilterOptionVm>), String> {
    let providers: Vec<FilterOptionVm> = store
        .usage_by_provider(None, None, since)
        .map_err(e2s)?
        .into_iter()
        .filter(|pu| pu.totals.requests > 0)
        .map(|pu| FilterOptionVm {
            id: pu.provider_id.clone(),
            label: names
                .get(&pu.provider_id)
                .cloned()
                .unwrap_or(pu.provider_id.clone()),
        })
        .collect();
    let agents: Vec<FilterOptionVm> = roster
        .iter()
        .filter_map(|(name, label)| {
            let t = store.usage_totals(Some(name), None, since).ok()?;
            (t.requests > 0).then(|| FilterOptionVm {
                id: name.clone(),
                label: label.clone(),
            })
        })
        .collect();
    Ok((providers, agents))
}

/// Dashboard aggregation. `provider_id`/`agent` narrow every stat (headline,
/// trend, distributions, latency) to that slice; None means all.
pub fn build_dashboard(
    store: &Store,
    aux: &Aux,
    window: &str,
    provider_id: Option<&str>,
    agent: Option<&str>,
) -> Result<DashboardVm, String> {
    let w = resolve_window(aux, unix_now(), window);

    let cur = store
        .usage_totals(agent, provider_id, w.since.as_deref())
        .map_err(e2s)?;
    // Headline request count shares the Logs card's source (request_logs):
    // usage rows only cover forwarded requests, so failures before the forward
    // leg (no provider bound, protocol mismatch…) would vanish from the top
    // stat while the Logs card below still shows them. Token/cost/latency stay
    // usage-based — failed requests carry none.
    let requests = store
        .count_request_logs(agent, provider_id, w.since.as_deref(), None)
        .map_err(e2s)?;
    // The same count over the window before, from the same source: two numbers
    // compared as a percentage have to be measured the same way.
    let previous_requests = match &w.previous {
        Some((from, to)) => store
            .count_request_logs(agent, provider_id, Some(from), Some(to))
            .map_err(e2s)?,
        None => 0,
    };
    let name_by_id: HashMap<String, String> = store
        .list_providers()
        .map_err(e2s)?
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();

    let costs = dashboard_costs(store, aux, agent, provider_id, w.since.as_deref())?;

    let trend = if w.key == "today" {
        trend_today(
            store,
            agent,
            provider_id,
            w.since.as_deref(),
            w.today_days,
            w.tz,
        )?
    } else {
        trend_daily(store, agent, provider_id, &w)?
    };

    let by_provider = provider_distribution(
        store,
        agent,
        provider_id,
        w.since.as_deref(),
        &name_by_id,
        cur.requests.max(1),
        &costs,
    )?;
    // Computed once: the agent breakdown and the filter selects walk the same list.
    let roster = agent_roster(store, w.since.as_deref())?;
    let by_agent = agent_distribution(
        store,
        agent,
        provider_id,
        w.since.as_deref(),
        &roster,
        &costs,
    )?;
    let (latency, latency_delta_pct) = latency_pair(
        aux,
        provider_id,
        agent,
        w.since.as_deref(),
        w.previous.as_ref(),
    );
    let (filter_providers, filter_agents) =
        filter_options(store, w.since.as_deref(), &roster, &name_by_id)?;

    Ok(DashboardVm {
        window: w.key.to_string(),
        requests,
        requests_delta_pct: delta_pct(requests as f64, previous_requests as f64),
        input_tokens: cur.input_tokens,
        cache_read_tokens: cur.cache_read_tokens,
        output_tokens: cur.output_tokens,
        cost: (costs.cost * 1e6).round() / 1e6,
        cost_off_peak: costs.cost_off_peak,
        latency_ms: latency,
        latency_delta_pct,
        trend,
        by_provider,
        by_agent,
        filter_providers,
        filter_agents,
    })
}

pub fn build_footer_stats(
    store: &Store,
    aux: &Aux,
    version: &str,
) -> Result<FooterStatsVm, String> {
    let tz = tz_offset(aux);
    let now = unix_now();
    let today = local_day_key(tz, now);
    // The same boundary the dashboard's "today" window uses: local midnight as
    // the UTC instant a `ts >=` filter needs. Spelling it `{today}T00:00:00Z`
    // reads as *UTC* midnight, which for a UTC+8 user drops the day's first
    // eight hours — and, seen just after midnight, the whole day.
    let since = local_day_start(tz, now);
    let t = store.usage_totals(None, None, Some(&since)).map_err(e2s)?;
    // hub_synced = catalog synced today (first 10 chars of the cache
    // timestamp are the date)
    let hub_synced = aux
        .load_hub_cache()
        .map(|(_, ts)| ts.starts_with(&today))
        .unwrap_or(false);
    Ok(FooterStatsVm {
        today_requests: t.requests,
        today_tokens: t.input_tokens + t.output_tokens,
        hub_synced,
        version: version.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::settings::update_settings;
    use crate::vm::test_support::{
        no_vars, provider, seed_usage_rows, store, store_and_aux_on_one_file, usage_row,
    };
    use crate::vm::time::{day_key, hh00, hour_key, mmdd, rfc3339, unix_now};
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Store};

    /// Seed `requests` rows `offset_days` back (at 23:00 UTC of that calendar
    /// day, so day-bucket boundaries are unambiguous), each with `tokens` in.
    fn seed_usage_at(s: &Store, offset_days: i64, requests: i64, tokens: i64) {
        let now = unix_now();
        let day = now.div_euclid(86_400) - offset_days;
        seed_usage_rows(s, day * 86_400 + 23 * 3600, requests, tokens);
    }

    #[test]
    fn dashboard_windows_cover_the_right_days() {
        // One file, two handles — as production has it, where the app's `Aux`
        // and the gateway's `Store` open the same SQLite file. It is also the
        // only way this test can see a latency average at all: `avg_latency`
        // reads through the `Aux` connection, which in-memory would be a
        // database of its own.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let s = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();
        // Display conversion uses the Hub's published rates and there is no
        // compiled snapshot behind them, so the fixture publishes the rate the
        // cost assertion below converts with.
        let hub = serde_json::json!({
            "version": 1,
            "exchange_rates": { "USD": 1.0, "CNY": 7.1 },
            "models": []
        })
        .to_string();
        aux.save_hub_models_cache(1, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
        // Distinct magnitudes per age so a window that is too wide or too
        // narrow cannot cancel out: today 2, 3 days back 4, 10 days back 6,
        // 40 days back 8, plus one row 7 calendar days back.
        seed_usage_at(&s, 0, 2, 1_000);
        seed_usage_at(&s, 3, 4, 2_000);
        seed_usage_at(&s, 10, 6, 3_000);
        seed_usage_at(&s, 40, 8, 4_000);
        seed_usage_at(&s, 7, 1, 5_000);
        // Old enough that "all" has to start saying weeks rather than days.
        seed_usage_at(&s, 100, 3, 900);

        let d = |w: &str| build_dashboard(&s, &aux, w, None, None).unwrap();

        // Today: the current UTC day only — the 23:00 rows of the days before
        // it stay out, and so does the one from 40 days back. The chart splits
        // that day into its 24 hours, so the 23:00 rows land in the last bucket.
        let today = d("today");
        assert_eq!((today.requests, today.input_tokens), (2, 2_000));
        assert_eq!(today.trend.len(), 24, "today plots one bar per hour");
        assert_eq!(today.trend[23].date, "23:00");
        assert_eq!(today.trend[23].requests, 2);
        assert_eq!(today.trend[0].requests, 0, "an idle hour is still a bucket");
        assert_eq!(
            today.trend.iter().map(|p| p.requests).sum::<i64>(),
            today.requests,
            "the hourly bars cover the stat's whole window"
        );

        // 7d is today plus the six days before it, so the chart's seven points
        // cover exactly the same span as the stat above them.
        let week = d("7d");
        assert_eq!((week.requests, week.input_tokens), (6, 10_000));
        assert_eq!(week.trend.len(), 7);
        assert_eq!(
            week.trend.iter().map(|p| p.requests).sum::<i64>(),
            week.requests,
            "the chart covers the stat's whole window"
        );

        // 30d adds the 7- and 10-day-old groups (1 + 6) and still excludes the
        // 40-day-old one: 6 + 7 = 13 requests, 10k + 5k + 18k tokens.
        let month = d("30d");
        assert_eq!((month.requests, month.input_tokens), (13, 33_000));
        assert_eq!(month.trend.len(), 30, "30d is one bar per day");
        assert_eq!(month.trend.iter().map(|p| p.requests).sum::<i64>(), 13);
        // Bar i is the day 29 - i days ago, so each seeded group lands where its
        // own label says it does — no bar covering more than the day it names.
        assert_eq!(month.trend[29].requests, 2, "the last bar is today");
        assert_eq!(month.trend[26].requests, 4, "three days back");
        assert_eq!(month.trend[22].requests, 1, "seven days back");
        assert_eq!(month.trend[19].requests, 6, "ten days back");
        assert_eq!(
            month.trend[18].requests, 0,
            "and the quiet days are bars too"
        );

        // Cost is summed in the window and converted for display (default CNY).
        assert!((month.cost - month.requests as f64 * 0.5 * 7.1).abs() < 0.01);

        // All: everything the store holds, including the row no counted window
        // reaches. Its bars are whole local days like every other window's, and
        // they add up to the stat above them — which is the point of a chart
        // whose left edge is "wherever the oldest row is".
        let all = d("all");
        assert_eq!((all.requests, all.input_tokens), (24, 67_700));
        assert_eq!(
            all.trend.iter().map(|p| p.requests).sum::<i64>(),
            all.requests,
            "the bars cover the stat's whole window"
        );
        assert_eq!(
            all.trend[0].requests, 3,
            "the oldest group is the first bar, not a day count back from today"
        );
        // 101 days no longer fit a bar each, so the rows are summed a week at a
        // time: 15 bars of seven days, the last one clipped at today.
        assert_eq!(all.trend.len(), 15, "a long history is summed by the week");

        // The deltas compare each window with the one before it, over the same
        // source: 7d is six requests this week against seven last week.
        assert_eq!(week.requests_delta_pct, Some(-14));
        // 30d against the 30 days before it: 13 against the 40-day-old group's
        // eight.
        assert_eq!(month.requests_delta_pct, Some(63));
        // Yesterday had no traffic, so today has nothing to be a percentage of.
        // Zero would be the claim that the two days matched.
        assert_eq!(today.requests_delta_pct, None);
        // And "all" has no earlier window at all — it *is* every window.
        assert_eq!(all.requests_delta_pct, None);

        // Every seeded row is 100ms, so the two windows really do have the same
        // average: a flat 0%, which is a finding, not the absence of one.
        assert_eq!(week.latency_delta_pct, Some(0));
        assert_eq!(today.latency_delta_pct, None, "nothing ran yesterday");
    }

    #[test]
    fn footer_today_counts_from_local_midnight() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "tz_offset_minutes": 480 }),
            &no_vars(),
        )
        .unwrap();
        // One row a second after *local* midnight at UTC+8 — 16:00:01Z the day
        // before. It is the first moment of the user's day and the stretch a
        // UTC-midnight boundary silently dropped.
        let now = unix_now();
        let local_day = (now + 480 * 60).div_euclid(86_400);
        seed_usage_rows(&s, local_day * 86_400 - 480 * 60 + 1, 1, 1_000);

        let f = build_footer_stats(&s, &aux, "test").unwrap();
        assert_eq!(
            f.today_requests, 1,
            "00:00:01 local is today, not yesterday"
        );
        assert_eq!(f.today_tokens, 1_000);
    }

    #[test]
    fn day_boundaries_follow_the_configured_offset() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        let utc_day = now.div_euclid(86_400);
        let local_day = (now + 480 * 60).div_euclid(86_400);
        // One row the two clocks date differently. Which way it can be built
        // depends on the hour, because the offset only opens a gap once one
        // date has rolled over and the other has not. While UTC's date still
        // matches the local one, the local day's first second (16:00Z the day
        // before) is what UTC calls yesterday; once UTC has caught up, a row in
        // UTC's morning is what the user's clock calls yesterday. Only one of
        // the two exists at any given moment — a fixed "23:00Z yesterday",
        // which is what this used to seed, is yesterday on *both* clocks for
        // the first eight hours of every local day, and the test failed there.
        let (row, in_utc, in_local) = if local_day == utc_day {
            (local_day * 86_400 - 480 * 60 + 1, 0, 1)
        } else {
            (utc_day * 86_400 + 3_600, 1, 0)
        };
        seed_usage_rows(&s, row, 1, 1_000);

        // UTC (the default, and what an older settings blob yields).
        assert_eq!(
            build_dashboard(&s, &aux, "today", None, None)
                .unwrap()
                .requests,
            in_utc
        );

        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "tz_offset_minutes": 480 }),
            &no_vars(),
        )
        .unwrap();
        let shifted = build_dashboard(&s, &aux, "today", None, None).unwrap();
        assert_eq!(shifted.requests, in_local, "the two clocks disagree");
        assert_eq!(shifted.trend.len(), 24);
        assert_eq!(
            shifted.trend.iter().filter(|t| t.requests > 0).count(),
            in_local as usize,
            "and the chart plots the day the stat counts"
        );
    }

    #[test]
    fn dashboard_shape_with_usage() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Prov", Billing::Metered))
            .unwrap();
        let now = rfc3339(unix_now());
        s.record_usage(&kiwanod::store::UsageRecord {
            ts: now.clone(),
            agent: "claude".into(),
            provider_id: "p1".into(),
            model: None,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cache_creation_tokens: 0,
            latency_ms: Some(1200),
            status: "ok".into(),
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
        })
        .unwrap();
        // Seed the request-log rows the headline counts: the forwarded request
        // above plus a pre-forward failure (usage tables never see the latter).
        let log = |status: i64, tokens: (i64, i64)| kiwanod::store::RequestLogNew {
            ts: now.clone(),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: Some("claude".into()),
            attribution: Some("key".into()),
            provider_id: Some("p1".into()),
            model: None,
            status_code: status,
            error_kind: None,
            error_message: None,
            session_id: None,
            is_streaming: false,
            input_tokens: tokens.0,
            output_tokens: tokens.1,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms: Some(1200),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: None,
            response_body: None,
            request_size: 0,
            response_size: 0,
            truncated: false,
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
            request_notes: None,
        };
        s.insert_request_log(&log(200, (1000, 500))).unwrap();
        s.insert_request_log(&log(503, (0, 0))).unwrap();
        // The aux connection is a separate in-memory DB in tests (one shared
        // file in production); mirror the usage row so avg-latency reads see it.
        {
            let c = aux.conn.lock().unwrap();
            c.execute(
                "CREATE TABLE usage (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     ts TEXT NOT NULL, agent TEXT NOT NULL, provider_id TEXT NOT NULL,
                     model TEXT, input_tokens INTEGER NOT NULL DEFAULT 0,
                     output_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                     latency_ms INTEGER, status TEXT NOT NULL DEFAULT 'ok')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO usage (ts, agent, provider_id, input_tokens, output_tokens,
                                    cache_read_tokens, latency_ms)
                 VALUES (?1, 'claude', 'p1', 1000, 500, 100, 1200)",
                rusqlite::params![now],
            )
            .unwrap();
        }
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        // The headline reads request_logs (Logs-card source): both the
        // forwarded and the failed request count, while the usage-derived
        // totals stay limited to the forwarded one.
        assert_eq!(d.requests, 2);
        assert_eq!(d.input_tokens, 1000);
        assert_eq!(d.latency_ms, 1200);
        assert_eq!(d.by_agent[0].tokens, "2k");
        assert_eq!(d.by_provider[0].pct, 100);
        assert!(d.trend.iter().map(|t| t.requests).sum::<i64>() >= 1);

        // Filters narrow every stat to the matching slice — and zero out on
        // a provider with no traffic.
        let fp = build_dashboard(&s, &aux, "7d", Some("p1"), Some("claude")).unwrap();
        assert_eq!(fp.requests, 2);
        assert_eq!(fp.by_provider.len(), 1);
        assert_eq!(fp.by_provider[0].id, "p1");
        assert_eq!(fp.by_agent.len(), 1);
        let fo = build_dashboard(&s, &aux, "7d", Some("ghost"), None).unwrap();
        assert_eq!(fo.requests, 0);
        assert!(fo.by_provider.is_empty());
        assert!(fo.by_agent.is_empty());
    }

    /// The peak premium is the difference between two sums over the *same* rows,
    /// which is only meaningful if both come out of one roll-up and are converted
    /// the same way. Two currencies in the window make the conversion part of the
    /// assertion rather than an identity, and a row whose model publishes no
    /// schedule (off-peak == cost) contributes nothing to the premium.
    #[test]
    fn dashboard_prices_the_peak_premium_over_one_row_set() {
        let (_dir, s, aux) = store_and_aux_on_one_file();
        // A Hub rate table — the quote is deliberately not the real one; what is
        // under test is that it is applied, not what it says.
        let hub = serde_json::json!({
            "version": 99,
            "exchange_rates": { "USD": 1.0, "CNY": 6.0 },
            "models": []
        })
        .to_string();
        aux.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
        for (id, name) in [("p1", "Alpha"), ("p2", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }

        // p1: 10 USD at peak against 4 USD off-peak — a 6 USD premium — plus a
        // row in another currency with no schedule at all (one vendor's off-peak
        // halving applies to some models and not others).
        let mut peak = usage_row("p1");
        peak.cost = Some(10.0);
        peak.cost_currency = Some("USD".into());
        peak.cost_off_peak = Some(4.0);
        s.record_usage(&peak).unwrap();
        let mut flat = usage_row("p1");
        flat.cost = Some(3.0);
        flat.cost_currency = Some("CNY".into());
        flat.cost_off_peak = Some(3.0);
        s.record_usage(&flat).unwrap();
        // p2 is billed the same either way: no premium to report.
        let mut even = usage_row("p2");
        even.cost = Some(2.0);
        even.cost_currency = Some("USD".into());
        even.cost_off_peak = Some(2.0);
        s.record_usage(&even).unwrap();

        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        // (10 + 2) USD × 6 + 3 CNY; off-peak (4 + 2) × 6 + 3.
        assert!((d.cost - 75.0).abs() < 1e-6, "{}", d.cost);
        assert!((d.cost_off_peak - 39.0).abs() < 1e-6, "{}", d.cost_off_peak);
        // What the reader subtracts: 36 CNY of peak rates, in the reader's own
        // currency — the same rows priced the other way, not two populations.
        assert!((d.cost - d.cost_off_peak - 36.0).abs() < 1e-6);

        let p1 = d.by_provider.iter().find(|p| p.id == "p1").unwrap();
        let p2 = d.by_provider.iter().find(|p| p.id == "p2").unwrap();
        assert!((p1.cost - 63.0).abs() < 1e-6, "{}", p1.cost);
        assert!(
            (p1.cost_off_peak - 27.0).abs() < 1e-6,
            "{}",
            p1.cost_off_peak
        );
        assert!((p2.cost - 12.0).abs() < 1e-6);
        assert!(
            (p2.cost_off_peak - 12.0).abs() < 1e-6,
            "no schedule, no premium: {}",
            p2.cost_off_peak
        );
        // The slices add up to the headline they were converted from.
        let sum: f64 = d.by_provider.iter().map(|p| p.cost).sum();
        assert!((sum - d.cost).abs() < 1e-6, "{sum} vs {}", d.cost);
        // …and the per-agent row is the same pair for that agent's rows.
        assert!((d.by_agent[0].cost - 75.0).abs() < 1e-6);
        assert!((d.by_agent[0].cost_off_peak - 39.0).abs() < 1e-6);
    }

    #[test]
    fn the_agent_filter_narrows_its_own_breakdown() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        // Two agents with traffic, so the table has something it could fail to
        // leave out. Only the first gets the request_logs twin the headline
        // counts — the filtered slice's total is all this needs.
        seed_usage_rows(&s, now - 90, 3, 1_000);
        for i in 0..2 {
            s.record_usage(&kiwanod::store::UsageRecord {
                ts: rfc3339(now - 60 - i),
                agent: "codex".into(),
                provider_id: "demo-alpha".into(),
                model: Some("demo-model".into()),
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(100),
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
            })
            .unwrap();
        }

        let all = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        assert_eq!(all.by_agent.len(), 2, "both agents have traffic");

        let one = build_dashboard(&s, &aux, "7d", None, Some("claude")).unwrap();
        assert_eq!(one.by_agent.len(), 1, "the table narrows with the filter");
        assert_eq!(one.by_agent[0].agent, "claude");
        assert_eq!(one.by_agent[0].requests, 3);
        assert_eq!(
            one.by_agent.iter().map(|a| a.requests).sum::<i64>(),
            one.requests,
            "the breakdown totals the same slice the headline counts"
        );
    }

    #[test]
    fn rfc3339_and_day_helpers() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(mmdd("2026-09-07"), "09-07");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // Hour keys match the `YYYY-MM-DDTHH` shape `usage_hourly` groups by,
        // and roll over at midnight like `day_key` does.
        assert_eq!(hour_key(0), "1970-01-01T00");
        assert_eq!(hour_key(7 * 3_600 + 59 * 60), "1970-01-01T07");
        assert_eq!(hour_key(86_400 + 3_600), "1970-01-02T01");
        assert_eq!(hh00("1970-01-01T07"), "07:00");
    }
}
