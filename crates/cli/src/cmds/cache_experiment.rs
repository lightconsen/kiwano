//! The offline cache-shaping experiment and its report.

use super::runtime;
use crate::cli::CacheExperimentArgs;
use crate::{CliError, Ctx};
use kiwano_core::vm;
use kiwanod::store::RequestLogFilter;

// ── cache experiment (Features: cache-shaping offline experiment) ────────────

pub fn cache_experiment(args: &CacheExperimentArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    if !vm::ui_settings(ctx.aux()?).feat_cache_experiment {
        return Err(CliError::usage(
            "the cache-shaping experiment is off — enable it under Settings → Features",
        ));
    }
    if args.days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    let now = vm::unix_now();
    let rows = ctx
        .store()?
        .export_request_logs_with_bodies(
            RequestLogFilter {
                agent: args.agent.as_deref(),
                from: Some(&vm::rfc3339(now - args.days * 86_400)),
                to: Some(&vm::rfc3339(now)),
                ..Default::default()
            },
            kiwanod::store::EXPORT_ROW_CAP,
        )
        .map_err(runtime)?;
    let report = kiwano_core::cache_experiment::run(args.days, &rows);
    let text = render_cache_experiment(&report);
    ctx.out.emit(&report, || text);
    Ok(())
}

fn render_cache_experiment(r: &kiwano_core::cache_experiment::ExperimentReport) -> String {
    let pct = |v: f64| format!("{:.1}%", v * 100.0);
    let verdict = if r.pairs < 5 {
        "verdict: too little data — a handful of sessions is no basis; run the agents for a few more days and re-run"
    } else if r.stripped_mean - r.raw_mean >= 0.10 || r.improved_share >= 0.3 {
        "verdict: worth building — normalization rescues a real share of the prefix; the forward-path shaper has a case"
    } else {
        "verdict: not worth it — the bodies already share about as much prefix as normalization would give them"
    };
    format!(
        "cache-shaping experiment — last {} day(s)\n\
         sessions {} · pairs {} (skipped: {} truncated, {} no body, {} unparseable)\n\
         shared prefix, mean / median:\n\
         \x20 raw            {} / {}\n\
         \x20 canonical      {} / {}\n\
         \x20 + whitelist    {} / {}\n\
         pairs improved ≥5pt by canonicalization: {}\n\
         {verdict}",
        r.days,
        r.sessions,
        r.pairs,
        r.skipped_truncated,
        r.skipped_no_body,
        r.skipped_unparseable,
        pct(r.raw_mean),
        pct(r.raw_median),
        pct(r.canonical_mean),
        pct(r.canonical_median),
        pct(r.stripped_mean),
        pct(r.stripped_median),
        pct(r.improved_share),
    )
}
