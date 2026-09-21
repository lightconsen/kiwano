//! Flag-to-view-model mapping, shared by `providers` and `settings::import`.
//!
//! This is a layer, not a command. Both callers build the same
//! `vm::NewProviderInput`, so the parsing lives once, here; the helpers that cross
//! a module boundary are `pub(crate)` and nothing wider.

use super::providers::checked_unit;
use crate::cli::{AddArgs, ForwardArgs};
use crate::CliError;
use kiwano_core::vm;
use kiwanod::store::Provider;
use std::collections::BTreeMap;

// ── input mapping ───────────────────────────────────────────────────────────

/// Turn the flags into the app's `NewProviderInput`.
pub(crate) fn new_provider_input(
    args: &AddArgs,
    known: &[String],
) -> Result<vm::NewProviderInput, CliError> {
    let billing = parse_billing(&args.billing)?;
    let protocol = parse_protocol(&args.protocol)?;

    // Plan providers track quota from the plan query, not from the legacy
    // number+unit+cycle columns — `vm::add_provider` writes those NULL for
    // them. Accepting the flags and dropping the value would be worse than
    // saying no.
    if billing == "plan" && (args.limit.is_some() || args.unit.is_some() || args.reset.is_some()) {
        return Err(CliError::usage(
            "--limit/--unit/--reset do not apply to plan providers; \
             their quota comes from the plan query instead",
        ));
    }
    let reset_period = match args.reset.as_deref() {
        None | Some("none") => None,
        Some(p @ ("monthly" | "weekly" | "yearly")) => Some(p.to_string()),
        Some(other) => {
            return Err(CliError::usage(format!(
                "invalid --reset: {other} (monthly|weekly|yearly|none)"
            )))
        }
    };
    if let Some(days) = args.limit {
        if days <= 0.0 {
            return Err(CliError::usage("--limit must be positive"));
        }
    }
    check_plan_limits(billing, &args.forward)?;

    Ok(vm::NewProviderInput {
        // No shelf on the command line; see the update path for why this is
        // absent rather than empty.
        catalog_id: None,
        // `providers add` declares no prices either — see the update path. The
        // provider is priced from the Hub's table, and the app's Custom form is
        // where its own rates are typed in.
        prices: None,
        name: args.name.trim().to_string(),
        api_key: args.key.clone().unwrap_or_default(),
        endpoint: args.endpoint.trim().to_string(),
        protocol: protocol.to_string(),
        model_default: String::new(),
        billing: billing.to_string(),
        billing_config: vm::BillingConfigInput {
            limit_value: args.limit,
            limit_unit: checked_unit(args.unit.as_deref(), known)?,
            reset_period,
            plan_limits: plan_limits_input(&args.forward, None),
        },
        agents: Some(args.bind.clone()),
        endpoints: parse_extra_endpoints(&args.forward.endpoint_extra)?,
        // Absent, not empty: `advanced` is an authoritative snapshot when
        // present, so sending an empty object would clear settings the user
        // never mentioned. There is nothing to preserve on add, so it is only
        // built when a flag actually asks for it.
        advanced: advanced_input(&args.forward, None, false)?,
        plan_query: plan_query_input(&args.forward, false)?,
    })
}

/// Plan limits are only meaningful for a plan provider, and `vm` silently drops
/// them otherwise — the same shape of mistake as `--limit` on a plan. Refuse
/// rather than accept a flag that does nothing.
pub(crate) fn check_plan_limits(billing: &str, forward: &ForwardArgs) -> Result<(), CliError> {
    let asked = forward.plan_limit_5h.is_some() || forward.plan_limit_weekly.is_some();
    if asked && billing != "plan" {
        return Err(CliError::usage(
            "--plan-limit-5h/--plan-limit-weekly apply to plan providers only \
             (use --limit for payg)",
        ));
    }
    Ok(())
}

/// `Name: value` → a header map.
///
/// Split on the *first* colon, so a value may contain one — which it very often
/// does: `Authorization: Bearer abc:def`, or a URL.
fn parse_headers(raw: &[String]) -> Result<BTreeMap<String, String>, CliError> {
    let mut map = BTreeMap::new();
    for entry in raw {
        let (name, value) = entry.split_once(':').ok_or_else(|| {
            CliError::usage(format!("--header expects 'Name: value', got {entry:?}"))
        })?;
        let name = name.trim();
        if name.is_empty() {
            return Err(CliError::usage(format!(
                "--header has an empty name: {entry:?}"
            )));
        }
        map.insert(name.to_string(), value.trim().to_string());
    }
    Ok(map)
}

/// The stored `providers.headers` JSON column back into a map.
fn headers_from_column(raw: Option<&str>) -> Option<BTreeMap<String, String>> {
    serde_json::from_str(raw?).ok()
}

/// `PROTO=URL` → the additional-endpoint list.
pub(crate) fn parse_extra_endpoints(raw: &[String]) -> Result<Vec<vm::NewEndpointInput>, CliError> {
    raw.iter()
        .map(|entry| {
            let (protocol, endpoint) = entry.split_once('=').ok_or_else(|| {
                CliError::usage(format!("--endpoint-extra expects PROTO=URL, got {entry:?}"))
            })?;
            let protocol = parse_protocol(protocol)?;
            let endpoint = endpoint.trim();
            if endpoint.is_empty() {
                return Err(CliError::usage(format!(
                    "--endpoint-extra has an empty URL: {entry:?}"
                )));
            }
            Ok(vm::NewEndpointInput {
                protocol: protocol.to_string(),
                endpoint: endpoint.to_string(),
            })
        })
        .collect()
}

/// The `advanced` object, or `None` for "leave it alone".
///
/// `vm` recomputes all three columns from this object whenever it is present,
/// so on an edit the fields the flags did not mention are carried over from the
/// stored row. Sending a partial object would clear them.
pub(crate) fn advanced_input(
    forward: &ForwardArgs,
    current: Option<&Provider>,
    clear_headers: bool,
) -> Result<Option<vm::AdvancedInput>, CliError> {
    let touched = forward.timeout.is_some()
        || forward.retries.is_some()
        || !forward.headers.is_empty()
        || clear_headers;
    if !touched {
        return Ok(None);
    }
    let stored_headers = current.and_then(|p| headers_from_column(p.headers.as_deref()));
    let headers = if clear_headers {
        None
    } else if !forward.headers.is_empty() {
        Some(parse_headers(&forward.headers)?)
    } else {
        stored_headers
    };
    Ok(Some(vm::AdvancedInput {
        timeout_secs: forward
            .timeout
            .or_else(|| current.and_then(|p| p.timeout_secs)),
        retries: forward.retries.or_else(|| current.and_then(|p| p.retries)),
        headers,
    }))
}

/// `{"five_hour":20,"weekly":60}` (the stored shape) back into the input type.
fn parse_plan_limits(raw: &str) -> Option<vm::PlanLimitsInput> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let number = |key: &str| v.get(key).and_then(|x| x.as_f64());
    Some(vm::PlanLimitsInput {
        five_hour: number("five_hour"),
        weekly: number("weekly"),
    })
}

/// The plan-mode percent limits: flags override, the stored row fills the rest.
///
/// `vm` recomputes the column from this object, so a partial view would clear
/// the window the caller did not mention.
pub(crate) fn plan_limits_input(
    forward: &ForwardArgs,
    current: Option<&Provider>,
) -> Option<vm::PlanLimitsInput> {
    let stored = current
        .and_then(|p| p.plan_limits.as_deref())
        .and_then(parse_plan_limits);
    if forward.plan_limit_5h.is_none() && forward.plan_limit_weekly.is_none() {
        return stored;
    }
    let stored = stored.unwrap_or(vm::PlanLimitsInput {
        five_hour: None,
        weekly: None,
    });
    Some(vm::PlanLimitsInput {
        five_hour: forward.plan_limit_5h.or(stored.five_hour),
        weekly: forward.plan_limit_weekly.or(stored.weekly),
    })
}

/// The quota-query payload, or `None` for "keep what is stored".
///
/// A JSON `null` is how the API spells "clear this", which is not the same as
/// absent — hence the separate `--clear-plan-query` flag rather than treating an
/// empty value as a removal.
pub(crate) fn plan_query_input(
    forward: &ForwardArgs,
    clear: bool,
) -> Result<Option<serde_json::Value>, CliError> {
    if clear {
        return Ok(Some(serde_json::Value::Null));
    }
    match &forward.plan_query {
        None => Ok(None),
        Some(raw) => {
            let parsed: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| CliError::usage(format!("--plan-query is not valid JSON: {e}")))?;
            if !parsed.is_object() {
                return Err(CliError::usage(
                    "--plan-query must be a JSON object, e.g. '{\"template\":\"kimi\",\"fields\":{}}'",
                ));
            }
            Ok(Some(parsed))
        }
    }
}

/// The app's billing vocabulary, with the older CLI's words kept as aliases.
pub(crate) fn parse_billing(raw: &str) -> Result<&'static str, CliError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "plan" | "subscription" => Ok("plan"),
        "payg" | "metered" => Ok("payg"),
        "unl" | "unlimited" => Ok("unl"),
        other => Err(CliError::usage(format!(
            "invalid --billing: {other} (plan|payg|unl)"
        ))),
    }
}

pub(crate) fn parse_protocol(raw: &str) -> Result<&'static str, CliError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Ok("anthropic"),
        "openai" => Ok("openai"),
        "gemini" => Ok("gemini"),
        other => Err(CliError::usage(format!(
            "invalid --protocol: {other} (anthropic|openai|gemini)"
        ))),
    }
}
