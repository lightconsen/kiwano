//! Rule injection: render the rules, and apply them over a window.
//!
//! The rules come from the same report `insights` builds, so this imports it back
//! rather than re-deriving the findings.

use super::insights::build_insights_report;
use crate::cli::RulesCmd;
use crate::{CliError, Ctx};
use kiwano_core::vm;

// ── rules (Features: rule injection) ─────────────────────────────────────────
pub fn rules(cmd: &RulesCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        RulesCmd::Apply { agent, days } => rules_apply(agent, *days, ctx),
        RulesCmd::Remove { agent } => {
            let st =
                kiwano_core::rules_inject::remove(ctx.aux()?, agent, &ctx.home, ctx.config_vars())?;
            ctx.out
                .emit(&st, || format!("removed: no kiwano rules in {}", st.file));
            Ok(())
        }
        RulesCmd::Status { agent } => {
            let st =
                kiwano_core::rules_inject::status(ctx.aux()?, agent, &ctx.home, ctx.config_vars())?;
            let text = if st.applied {
                format!(
                    "{}: {} rule(s) in {}{}\n{}",
                    st.agent,
                    st.rules.len(),
                    st.file,
                    st.applied_at
                        .as_deref()
                        .map(|t| format!(" (applied {t})"))
                        .unwrap_or_default(),
                    st.rules
                        .iter()
                        .map(|r| format!("  - {r}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            } else {
                format!("{}: no rules applied ({})", st.agent, st.file)
            };
            ctx.out.emit(&st, || text);
            Ok(())
        }
    }
}

/// The rules the window's findings teach, applied to the agent's instruction
/// file. Only findings about *this* agent (or about no agent in particular)
/// contribute — another agent's lesson is not this one's to learn.
fn rules_apply(agent: &str, days: i64, ctx: &mut Ctx) -> Result<(), CliError> {
    if !vm::ui_settings(ctx.aux()?).feat_rule_injection {
        return Err(CliError::usage(
            "rule injection is off — enable it under Settings → Features",
        ));
    }
    if days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    let report = build_insights_report(
        ctx.store()?,
        days,
        Some(agent.to_string()),
        vm::ui_settings(ctx.aux()?).feat_tuning_advice,
    )?;
    let mut rules: Vec<String> = Vec::new();
    for f in &report.findings {
        let Some(rule) = &f.rule else { continue };
        if f.agent.as_deref().is_some_and(|a| a != agent) {
            continue;
        }
        if !rules.contains(rule) {
            rules.push(rule.clone());
        }
    }
    if rules.is_empty() {
        ctx.out.line(format!(
            "no rules in the last {days} day(s) of insights for {agent}"
        ));
        return Ok(());
    }
    let st =
        kiwano_core::rules_inject::apply(ctx.aux()?, agent, &rules, &ctx.home, ctx.config_vars())?;
    let text = format!(
        "applied {} rule(s) to {}\n{}",
        st.rules.len(),
        st.file,
        st.rules
            .iter()
            .map(|r| format!("  - {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    ctx.out.emit(&st, || text);
    Ok(())
}
