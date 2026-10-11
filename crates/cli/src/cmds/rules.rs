//! Rule injection: render the rules, and apply them over a window.
//!
//! The rules come from the same report `insights` builds, so this imports it back
//! rather than re-deriving the findings.

use super::insights::build_insights_report;
use crate::cli::RulesCmd;
use crate::{CliError, Ctx};

// ── rules (Features: rule injection) ─────────────────────────────────────────
pub fn rules(cmd: &RulesCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        RulesCmd::Apply { agent, days } => rules_apply(agent, *days, ctx),
        RulesCmd::User { rule, clear } => rules_user(rule, *clear, ctx),
        RulesCmd::Remove { agent } => {
            // Both blocks: "give me my file back" is one intention, whether what
            // is in there came from insights or from the user.
            let st = kiwano_core::rules_inject::remove_all(
                ctx.aux()?,
                agent,
                &ctx.home,
                ctx.config_vars(),
            )?;
            ctx.out
                .emit(&st, || format!("removed: no kiwano rules in {}", st.file));
            Ok(())
        }
        RulesCmd::Status { agent } => {
            let st =
                kiwano_core::rules_inject::status(ctx.aux()?, agent, &ctx.home, ctx.config_vars())?;
            let block = |title: &str, rules: &[String], at: Option<&str>| {
                if rules.is_empty() {
                    return format!("  {title}: none");
                }
                format!(
                    "  {title}: {} rule(s){} \n{}",
                    rules.len(),
                    at.map(|t| format!(" (applied {t})")).unwrap_or_default(),
                    rules
                        .iter()
                        .map(|r| format!("    - {r}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            let text = format!(
                "{} ({})\n{}\n{}",
                st.agent,
                st.file,
                block("insights", &st.rules, st.applied_at.as_deref()),
                block("yours", &st.user_rules, st.user_applied_at.as_deref()),
            );
            ctx.out.emit(&st, || text);
            Ok(())
        }
    }
}

/// The user's own rules: show the library, replace it, or empty it.
///
/// The library is per **machine** while what gets written is per **agent** — that
/// split is the point. A rule someone writes is usually a convention they hold
/// everywhere, and which agents it reaches is `rules apply <agent>`'s decision;
/// storing it per agent would mean typing the same sentence once per agent and
/// keeping the copies in step by hand.
fn rules_user(rule: &[String], clear: bool, ctx: &mut Ctx) -> Result<(), CliError> {
    if rule.is_empty() && !clear {
        let rules = kiwano_core::rules_inject::user_rules(ctx.aux()?);
        let text = if rules.is_empty() {
            "no rules of your own — `kiwano rules user --rule \"…\"` writes one".to_string()
        } else {
            rules
                .iter()
                .map(|r| format!("  - {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        ctx.out.emit(&rules, || text);
        return Ok(());
    }
    let stored = kiwano_core::rules_inject::set_user_rules(ctx.aux()?, rule)?;
    let text = if stored.is_empty() {
        "cleared your rules; `kiwano rules apply <agent>` takes them out of the files".to_string()
    } else {
        format!(
            "{} rule(s) stored; `kiwano rules apply <agent>` writes them in",
            stored.len()
        )
    };
    ctx.out.emit(&stored, || text);
    Ok(())
}

/// The rules the window's findings teach, applied to the agent's instruction
/// file. Only findings about *this* agent (or about no agent in particular)
/// contribute — another agent's lesson is not this one's to learn.
fn rules_apply(agent: &str, days: i64, ctx: &mut Ctx) -> Result<(), CliError> {
    if days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    let settings = ctx.api.get_settings()?;

    // The gate, kept exactly as it was for the case it was written for: nothing
    // to write and the insights half switched off is still a usage error, so the
    // loud refusal a Features flag promises does not become a quiet success just
    // because the command grew a second input.
    let user_early = kiwano_core::rules_inject::user_rules(ctx.aux()?);
    if !settings.feat_rule_injection && user_early.is_empty() {
        return Err(CliError::usage(
            "rule injection is off — enable it under Settings → Features",
        ));
    }

    // The insights half: advice we derived, gated by the Features flag because
    // that is what the flag is about — a panel offering to write the advice it
    // found. Its absence is not a failure of the command: a machine with no
    // findings still has the user's own rules to write, and the two halves are
    // reported separately below.
    let mut insights: Vec<String> = Vec::new();
    if settings.feat_rule_injection {
        let report = build_insights_report(
            &ctx.api,
            days,
            Some(agent.to_string()),
            settings.feat_tuning_advice,
        )?;
        for f in &report.findings {
            let Some(rule) = &f.rule else { continue };
            if f.agent.as_deref().is_some_and(|a| a != agent) {
                continue;
            }
            if !insights.contains(rule) {
                insights.push(rule.clone());
            }
        }
    }

    // The user's half. **Not** behind that flag: these rules come from the user,
    // not from a feature we are still tuning, and making one depend on the other
    // is how "I wrote a rule and nothing happened" happens.
    let user = user_early;

    if insights.is_empty() && user.is_empty() {
        ctx.out.line(format!(
            "no rules in the last {days} day(s) of insights for {agent}, and none of your own"
        ));
        return Ok(());
    }

    let mut st =
        kiwano_core::rules_inject::status(ctx.aux()?, agent, &ctx.home, ctx.config_vars())?;
    if !insights.is_empty() {
        st = kiwano_core::rules_inject::apply(
            ctx.aux()?,
            agent,
            &insights,
            &ctx.home,
            ctx.config_vars(),
        )?;
    }
    if !user.is_empty() {
        st =
            kiwano_core::rules_inject::apply_user(ctx.aux()?, agent, &ctx.home, ctx.config_vars())?;
    }

    let listed = |rules: &[String]| {
        rules
            .iter()
            .map(|r| format!("  - {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut text = format!(
        "applied {} insight rule(s) and {} of yours to {}",
        st.rules.len(),
        st.user_rules.len(),
        st.file
    );
    if !st.rules.is_empty() {
        text.push_str(&format!("\n{}", listed(&st.rules)));
    }
    if !st.user_rules.is_empty() {
        text.push_str(&format!("\n{}", listed(&st.user_rules)));
    }
    ctx.out.emit(&st, || text);
    Ok(())
}
