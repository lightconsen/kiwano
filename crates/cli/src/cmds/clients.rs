//! Client-key management: the data plane's credentials, and what each may do.

use super::render::{render_client_key, render_client_keys};
use super::runtime;
use crate::cli::ClientsCmd;
use crate::{CliError, Ctx};
use kiwano_core::vm::{ClientKeyLimitVm, ClientKeyPolicyInput, NewClientKeyInput};

/// `PERIOD:LIMIT:UNIT` — the one shape a window is typed in.
///
/// A single flag rather than three, because a key holds a *set* of windows and
/// the set is what is being replaced: `--window day:100:requests --window
/// monthly:20:USD` reads as the list it is. A colon-separated triple rather than
/// three parallel flags keeps a window from being paired with the wrong unit.
///
/// The unit may be omitted (`day:100`), which is `requests` — the same default
/// the store applies to a NULL unit, and the one an operator means when they say
/// "100 a day".
fn parse_window(raw: &str) -> Result<ClientKeyLimitVm, CliError> {
    let mut parts = raw.splitn(3, ':');
    let period = parts.next().unwrap_or_default().trim();
    let limit = parts.next().unwrap_or_default().trim();
    let unit = parts.next().map(str::trim).filter(|u| !u.is_empty());
    if period.is_empty() || limit.is_empty() {
        return Err(runtime(format!(
            "a window is PERIOD:LIMIT[:UNIT] — e.g. day:100:requests; got `{raw}`"
        )));
    }
    let period_limit: f64 = limit
        .parse()
        .map_err(|_| runtime(format!("`{limit}` is not a number, in window `{raw}`")))?;
    if !period_limit.is_finite() || period_limit <= 0.0 {
        return Err(runtime(format!(
            "a window of `{limit}` is not a ceiling: the gateway reads a zero as no limit at all, \
             so it would be stored as nothing. Drop the flag to remove the window."
        )));
    }
    Ok(ClientKeyLimitVm {
        period: period.to_string(),
        period_limit,
        limit_unit: unit.map(str::to_string),
    })
}

fn parse_windows(raw: &[String]) -> Result<Vec<ClientKeyLimitVm>, CliError> {
    raw.iter().map(|w| parse_window(w)).collect()
}

pub fn clients(cmd: &ClientsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ClientsCmd::List { agent } => {
            let keys = ctx.api.list_client_keys()?;
            let keys: Vec<_> = match agent {
                Some(a) => keys.into_iter().filter(|k| &k.agent == a).collect(),
                None => keys,
            };
            let text = render_client_keys(&keys);
            ctx.out.emit(&keys, || text);
            Ok(())
        }
        ClientsCmd::Show { id } => {
            let key = ctx.api.get_client_key(id)?;
            let text = render_client_key(&key);
            ctx.out.emit(&key, || text);
            Ok(())
        }
        ClientsCmd::Add {
            agent,
            label,
            model,
            provider,
            window,
        } => {
            let created = ctx.api.add_client_key(&NewClientKeyInput {
                agent: agent.clone(),
                model_allow: model.clone(),
                provider_allow: provider.clone(),
                label: label.clone(),
                limits: parse_windows(window)?,
            })?;
            // The secret is printed here and nowhere else. The line says so,
            // because "where do I read it again?" is the question that follows.
            let text = format!(
                "{}  →  {}\n\n  agent: {}\n  handle: {}\n\nThis key is shown once — copy it into the \
                 agent's config now.\nManage it later by its handle: `kiwano clients show {}`.\n",
                created.key, agent, created.agent, created.id, created.id
            );
            ctx.out.emit(&created, || text);
            Ok(())
        }
        ClientsCmd::Remove { id } => {
            ctx.api.delete_client_key(id)?;
            ctx.out.line(format!("removed client key {id}"));
            Ok(())
        }
        ClientsCmd::Rotate { id } => {
            let rotated = ctx.api.rotate_client_key(id)?;
            let text = format!(
                "{}  →  {}\n\nThe handle, agent, limits and spend history are unchanged; only the \
                 secret is new.\nUpdate the agent's config, then `kiwano clients show {}` to check \
                 what it now says.\n",
                rotated.key, rotated.agent, rotated.id
            );
            ctx.out.emit(&rotated, || text);
            Ok(())
        }
        ClientsCmd::Limits { id, window } => {
            let limits = parse_windows(window)?;
            let cleared = limits.is_empty();
            ctx.api.set_client_key_limits(id, &limits)?;
            ctx.out.line(if cleared {
                format!("cleared every spend window on {id}")
            } else {
                format!("set {} window(s) on {id}", limits.len())
            });
            Ok(())
        }
        ClientsCmd::Policy {
            id,
            label,
            model,
            provider,
        } => {
            // `--model`/`--provider` given at all means "this is the list", so an
            // empty set of them after typing the flag is not expressible — the
            // command clears by omitting, which is the reading an operator
            // expects from a flag that takes values.
            ctx.api.set_client_key_policy(
                id,
                &ClientKeyPolicyInput {
                    model_allow: model.clone(),
                    provider_allow: provider.clone(),
                    label: label.clone(),
                },
            )?;
            let mut said = Vec::new();
            said.push(format!("{} model(s) allowed", model.len()));
            said.push(format!("{} provider(s) allowed", provider.len()));
            if label.is_some() {
                said.push("label set".to_string());
            }
            ctx.out.line(format!("{id}: {}", said.join(", ")));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_period_limit_and_an_optional_unit() {
        let w = parse_window("day:100:requests").unwrap();
        assert_eq!(w.period, "day");
        assert_eq!(w.period_limit, 100.0);
        assert_eq!(w.limit_unit.as_deref(), Some("requests"));

        // No unit is `requests` at the store, so the flag does not have to spell
        // the default out.
        let w = parse_window("monthly:20").unwrap();
        assert_eq!(w.period, "monthly");
        assert_eq!(w.limit_unit, None);

        let w = parse_window("weekly:1.5:wan_tokens").unwrap();
        assert_eq!(w.period_limit, 1.5);
        assert_eq!(w.limit_unit.as_deref(), Some("wan_tokens"));
    }

    #[test]
    fn a_window_that_means_nothing_is_refused_rather_than_stored() {
        // A zero would be dropped by the store (the gateway reads it as "no
        // limit"), so saying so here is what stops a user believing they set one.
        let err = parse_window("day:0:requests").unwrap_err().message;
        assert!(err.contains("not a ceiling"), "{err}");
        assert!(parse_window("day:-5").is_err());
        assert!(parse_window("day").is_err());
        assert!(parse_window(":100").is_err());
        assert!(parse_window("day:many").is_err());
    }
}
