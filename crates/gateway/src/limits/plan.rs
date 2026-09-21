//! `plan_limits`: percent ceilings on the plan's own windows, judged against
//! the utilization the provider's own endpoint reports (see `crate::plan_quota`).

/// `providers.plan_limits`: percent ceilings on the plan's own windows.
#[derive(Debug, serde::Deserialize)]
pub struct PlanLimits {
    five_hour: Option<f64>,
    weekly: Option<f64>,
}

impl PlanLimits {
    /// Parse a provider's ceilings. `None` when absent, unreadable, or when
    /// neither window is configured — a row that limits nothing.
    pub fn parse(raw: Option<&str>) -> Option<PlanLimits> {
        let limits: PlanLimits = serde_json::from_str(raw?).ok()?;
        (limits.five_hour.is_some() || limits.weekly.is_some()).then_some(limits)
    }
}

/// A plan window whose live utilization has reached the ceiling set for it.
pub struct WindowHit<'a> {
    /// `five_hour` | `weekly`.
    pub window: &'static str,
    /// Percent of the window used, as the provider's own endpoint reports it.
    pub util: f64,
    /// The ceiling the user configured, in percent.
    pub pct: f64,
    /// When the window resets, as the endpoint reports it. This is what a
    /// notification dedups on: one notice per window, and a fresh one after the
    /// window rolls over. `None` when the endpoint does not say.
    pub resets_at: Option<&'a str>,
}

/// The first configured window whose live utilization has reached its ceiling.
/// A window the report does not mention counts as not over: no evidence is not
/// evidence of a hit.
pub fn window_over<'a>(
    report: &'a crate::plan_quota::PlanQuotaReport,
    limits: &PlanLimits,
) -> Option<WindowHit<'a>> {
    let tier = |name: &str| report.tiers.iter().find(|t| t.name == name);
    let hit = |window, tier: &'a crate::plan_quota::PlanTierVm, pct| WindowHit {
        window,
        util: tier.utilization,
        pct,
        resets_at: tier.resets_at.as_deref(),
    };
    if let Some(pct) = limits.five_hour {
        if let Some(t) = tier("five_hour") {
            if t.utilization >= pct {
                return Some(hit("five_hour", t, pct));
            }
        }
    }
    if let Some(pct) = limits.weekly {
        if let Some(t) = tier("weekly_limit") {
            if t.utilization >= pct {
                return Some(hit("weekly", t, pct));
            }
        }
    }
    None
}
