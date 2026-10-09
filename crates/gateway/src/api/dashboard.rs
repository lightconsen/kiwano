//! The Dashboard, the footer's totals and the currency picker — assembled by the
//! daemon.
//!
//! Moved from `kiwano_core::vm::dashboard` and `kiwano_core::pricing`
//! (`migrate.local.md` §10.21). None of these needs a machine fact: they are
//! reads of the usage table, the price cache and the settings blob, all of which
//! the daemon owns — which is why they move whole, with no input from the client
//! beyond the window the screen is showing.
//!
//! `kiwano-core` re-exports every one of them, so its call sites are unchanged.

use crate::store::{Store, UsageTotals};
use kiwano_api::dashboard::{
    AgentDistVm, BlockedProviderVm, CurrencyMetaVm, DashboardVm, FilterOptionVm, FooterStatsVm,
    ProviderDistVm, TrendVm,
};
use kiwano_api::error::ApiError;
use serde::Serialize;
use std::collections::HashMap;

use crate::api::views::{chart_palette, fmt_tokens};
use crate::store::time::{local_day_key, local_day_start, unix_now};

use crate::store::time::{day_index_of_key, hh00, hour_key, local_day_start_from, mmdd};

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
    /// The gateway is up and writing to a *different* database than this app
    /// reads — so every number in this window is about a store the gateway is
    /// not filling. No error reports this on its own; the identity comparison
    /// does (see `crate::sidecar::database_agreement`).
    pub db_mismatch: bool,
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

fn resolve_window(store: &Store, now: i64, window: &str) -> DashWindow {
    let tz = store.ui_tz_offset_minutes();
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
        kiwano_adapters::model_pricing::convert_cost_buckets(buckets, &self.preferred, &self.rates)
    }
}

fn dashboard_costs(
    store: &Store,
    agent: Option<&str>,
    provider_id: Option<&str>,
    since: Option<&str>,
) -> Result<DashboardCosts, ApiError> {
    // Cost rolls up per-currency buckets (each row's cost_currency) into the
    // user's preferred display currency via the Hub's published rates, so this
    // agrees with the currency selector. Before the first sync there are no
    // rates and the buckets are summed as-is.
    let rates = super::pricing_sync::effective_rates(store);
    let preferred = preferred_currency(store);

    // Headline cost for the window, with the off-peak equivalent of the same
    // rows beside it. Both sums come from one query over one row set, which is
    // what makes their difference "what running at peak cost you" rather than a
    // comparison of two different populations.
    let headline = store
        .usage_cost_with_off_peak_by_currency(agent, provider_id, since)
        .map_err(ApiError::failed)?;
    let pairs = |pick: fn(&crate::store::CostBucket) -> f64| -> Vec<(Option<String>, f64)> {
        headline
            .iter()
            .map(|b| (b.currency.clone(), pick(b)))
            .collect()
    };
    let cost = kiwano_adapters::model_pricing::convert_cost_buckets(
        &pairs(|b| b.cost),
        &preferred,
        &rates,
    );
    let cost_off_peak = (kiwano_adapters::model_pricing::convert_cost_buckets(
        &pairs(|b| b.cost_off_peak),
        &preferred,
        &rates,
    ) * 1e6)
        .round()
        / 1e6;

    // Per-provider cost for the distribution card.
    let mut by_pid: HashMap<String, f64> = HashMap::new();
    let mut off_peak_by_pid: HashMap<String, f64> = HashMap::new();
    for b in store
        .usage_cost_by_provider(agent, since)
        .map_err(ApiError::failed)?
    {
        let convert = |c: f64| match b.currency.as_deref() {
            Some(cur) => kiwano_adapters::model_pricing::convert_amount(c, cur, &preferred, &rates),
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
) -> Result<Vec<TrendVm>, ApiError> {
    let mut trend = Vec::new();
    // "today" plots the day's hours, not one bar for the whole day: 24 local
    // hour buckets, zero-filled exactly like the daily axis so the chart
    // spans the same day the stat above it counts (and its bars still sum
    // to that stat).
    let mut hourly: HashMap<String, UsageTotals> = HashMap::new();
    for b in store
        .usage_hourly(agent, provider_id, since, tz)
        .map_err(ApiError::failed)?
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
) -> Result<Vec<TrendVm>, ApiError> {
    let mut trend = Vec::new();
    // One bar per local day, zero-filled: the chart draws exactly the days
    // the window selected, so its bars sum to the stat above it and each
    // label names the one day its own bar covers. Merging days (30d used to
    // draw six five-day blocks) made a bar mean something the axis could
    // not say.
    let mut daily: HashMap<String, UsageTotals> = HashMap::new();
    for d in store
        .usage_daily(agent, provider_id, w.since.as_deref(), w.tz)
        .map_err(ApiError::failed)?
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
) -> Result<Vec<ProviderDistVm>, ApiError> {
    let mut by_provider: Vec<ProviderDistVm> = store
        .usage_by_provider(agent, provider_id, since)
        .map_err(ApiError::failed)?
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
fn agent_roster(store: &Store, since: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
    let mut roster: Vec<(String, String)> =
        super::agents::list_agents(store).map_err(ApiError::failed)?;
    for id in store.usage_agents(since).map_err(ApiError::failed)? {
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
) -> Result<Vec<AgentDistVm>, ApiError> {
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
            .map_err(ApiError::failed)?;
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
    store: &Store,
    provider_id: Option<&str>,
    agent: Option<&str>,
    since: Option<&str>,
    previous: Option<&(String, String)>,
) -> (i64, Option<i64>) {
    let latency_now = store.avg_latency(provider_id, agent, since, None);
    let latency_before =
        previous.and_then(|(from, to)| store.avg_latency(provider_id, agent, Some(from), Some(to)));
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
) -> Result<(Vec<FilterOptionVm>, Vec<FilterOptionVm>), ApiError> {
    let providers: Vec<FilterOptionVm> = store
        .usage_by_provider(None, None, since)
        .map_err(ApiError::failed)?
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
    window: &str,
    provider_id: Option<&str>,
    agent: Option<&str>,
) -> Result<DashboardVm, ApiError> {
    let w = resolve_window(store, unix_now(), window);

    let cur = store
        .usage_totals(agent, provider_id, w.since.as_deref())
        .map_err(ApiError::failed)?;
    // Headline request count shares the Logs card's source (request_logs):
    // usage rows only cover forwarded requests, so failures before the forward
    // leg (no provider bound, protocol mismatch…) would vanish from the top
    // stat while the Logs card below still shows them. Token/cost/latency stay
    // usage-based — failed requests carry none.
    let requests = store
        .count_request_logs(agent, provider_id, w.since.as_deref(), None)
        .map_err(ApiError::failed)?;
    // The same count over the window before, from the same source: two numbers
    // compared as a percentage have to be measured the same way.
    let previous_requests = match &w.previous {
        Some((from, to)) => store
            .count_request_logs(agent, provider_id, Some(from), Some(to))
            .map_err(ApiError::failed)?,
        None => 0,
    };
    let name_by_id: HashMap<String, String> = store
        .list_providers()
        .map_err(ApiError::failed)?
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();

    let costs = dashboard_costs(store, agent, provider_id, w.since.as_deref())?;

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
        store,
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

pub fn build_footer_stats(store: &Store, version: &str) -> Result<FooterStatsVm, ApiError> {
    let tz = store.ui_tz_offset_minutes();
    let now = unix_now();
    let today = local_day_key(tz, now);
    // The same boundary the dashboard's "today" window uses: local midnight as
    // the UTC instant a `ts >=` filter needs. Spelling it `{today}T00:00:00Z`
    // reads as *UTC* midnight, which for a UTC+8 user drops the day's first
    // eight hours — and, seen just after midnight, the whole day.
    let since = local_day_start(tz, now);
    let t = store
        .usage_totals(None, None, Some(&since))
        .map_err(ApiError::failed)?;
    // hub_synced = catalog synced today (first 10 chars of the cache
    // timestamp are the date)
    // The cache is the daemon's table now, so the badge reads the store — the
    // same row the sync writes.
    let hub_synced = store
        .hub_cache()
        .map(|(_, ts)| ts.starts_with(&today))
        .unwrap_or(false);
    Ok(FooterStatsVm {
        today_requests: t.requests,
        today_tokens: t.input_tokens + t.output_tokens,
        hub_synced,
        version: version.to_string(),
    })
}

/// Re-exported: the app's `list_model_prices` command hands these rows to the
/// frontend, and the app deliberately depends on `kiwano-core` alone rather than
/// reaching into the adapters crate for a type.
pub use kiwano_adapters::model_pricing::ModelPriceEntry;
use kiwano_adapters::model_pricing::ModelsDoc;
/// Re-exported for the same reason, and for a better one: the gateway's limit
/// check converts with these too, and one implementation is the only way the
/// figure it enforces and the figure the app shows can be the same number.
pub use kiwano_adapters::model_pricing::{convert_amount, convert_cost_buckets};

/// The price table to seed from, with the sha256 of the bytes it came from, or
/// `None` when the Hub has never been synced or its cache is unusable.
///
/// There is no compiled fallback: like the catalog, prices come from the Hub
/// alone. `None` is therefore "nothing to seed", never "an empty price
/// document" — the difference matters, because seeding an empty document
/// clears the table.
pub fn effective_doc(store: &crate::store::Store) -> Option<(ModelsDoc, String)> {
    let (version, payload, sha, _) = store.hub_models_cache()?;
    match serde_json::from_str::<ModelsDoc>(&payload) {
        Ok(doc) => Some((doc, sha)),
        Err(e) => {
            // Not the same as "the Hub published nothing". A cached document
            // that will not parse is a price table that has quietly stopped
            // updating: the seed reports itself skipped, the old rows stay, and
            // the gateway goes on billing them. The sync validates a document
            // before caching it, so reaching this means the stored row changed
            // underneath the app — a truncated write, or a schema this build no
            // longer reads. Say so, because the symptom is otherwise invisible.
            tracing::warn!(
                version,
                error = %e,
                "cached pricing document does not parse; the previously seeded prices stand"
            );
            None
        }
    }
}

/// The exchange rates to convert with, or an empty table before the first sync
/// (every conversion then passes amounts through unchanged).
pub fn effective_rates(store: &crate::store::Store) -> HashMap<String, f64> {
    effective_doc(store)
        .map(|(doc, _)| doc.exchange_rates)
        .unwrap_or_default()
}

// The report type travels with the function that produces it.

/// The user's preferred display currency (default CNY). Stored inside the
/// ui settings blob (same source the Settings page round-trips).
pub fn preferred_currency(store: &crate::store::Store) -> String {
    super::settings::ui_settings(store)
        .map(|s| s.preferred_currency)
        .unwrap_or_else(|_| kiwano_api::settings::default_preferred_currency())
}

/// The currencies a user may display in. The rate table's keys are the ones
/// conversion can actually target; `models[].currency` says what things are
/// *priced* in, which is a different question — the published table prices
/// everything in USD, so deriving the list from it offered USD alone while the
/// default preference was CNY, and picking USD left nothing to switch back to.
///
/// The current preference is kept on the list even when no rate names it: a
/// selector that cannot show its own value is a dead end.
pub fn displayable_currencies(rates: &HashMap<String, f64>, preferred: &str) -> Vec<String> {
    let mut out: Vec<String> = rates.keys().cloned().collect();
    if !out.iter().any(|c| c == preferred) {
        out.push(preferred.to_string());
    }
    out.sort();
    out.dedup();
    out
}

pub fn currency_meta(store: &crate::store::Store) -> Result<CurrencyMetaVm, ApiError> {
    let rates = effective_rates(store);
    let preferred = preferred_currency(store);
    let currencies = displayable_currencies(&rates, &preferred);
    Ok(CurrencyMetaVm {
        currencies,
        exchange_rates: rates,
        preferred,
    })
}
