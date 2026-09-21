//! The agent commands: detection, versions, custom agents, and takeover.
//!
//! `AgentRow` is the shared row shape `agents_list` and `render_agent_rows` pass
//! between them.

use super::render::{render_agents, render_versions};
use super::runtime;
use crate::cli::AgentsCmd;
use crate::output::{ellipsize, render_table};
use crate::{CliError, Ctx};
use kiwano_core::detect;
use kiwano_core::vm;

// ── agents ──────────────────────────────────────────────────────────────────

pub fn agents(cmd: &AgentsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        AgentsCmd::List => agents_list(ctx),
        AgentsCmd::Add {
            name,
            note,
            protocol,
        } => agents_add(ctx, name, note.as_deref(), protocol.as_deref()),
        AgentsCmd::Remove { id } => agents_remove(ctx, id),
        AgentsCmd::Detect => {
            let declared = ctx.store()?.manual_agent_dirs();
            let found = detect::detect_agents(&ctx.home, &declared);
            let text = render_agents(&found);
            ctx.out.emit(&found, || text);
            Ok(())
        }
        AgentsCmd::Versions => {
            let declared = ctx.store()?.manual_agent_dirs();
            let versions = detect::probe_agent_versions(&ctx.home, &declared);
            let text = render_versions(&versions);
            ctx.out.emit(&versions, || text);
            Ok(())
        }
        AgentsCmd::Takeover { agent } => set_takeover(ctx, agent, true),
        AgentsCmd::Restore { agent } => set_takeover(ctx, agent, false),
    }
}

/// One row of `agents list`: a built-in or a user-defined agent, and the one
/// question that differs between them — whether its traffic reaches the gateway.
#[derive(serde::Serialize)]
struct AgentRow {
    id: String,
    label: String,
    /// "builtin" (a CLI this app knows how to route) or "custom" (a route you
    /// defined, with no config file behind it).
    kind: &'static str,
    routed: bool,
    placeholder_key: Option<String>,
    note: Option<String>,
    /// Built-ins: the protocols their clients speak (a list, since some carry
    /// more than one). A user-defined agent: the one chosen when it was defined,
    /// or null for one defined before the field existed. A label — nothing in
    /// the gateway routes or validates by it.
    protocols: Option<String>,
}

fn agents_list(ctx: &mut Ctx) -> Result<(), CliError> {
    let rows = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        let settings = vm::build_settings_with_home(store, aux, &ctx.home, ctx.config_vars())?;
        let mut rows: Vec<AgentRow> = settings
            .takeovers
            .into_iter()
            .map(|t| AgentRow {
                id: t.agent,
                label: t.label,
                kind: "builtin",
                // A built-in is routed when its own config points here — which
                // is what `takeovers[].enabled` reports, read from the file.
                routed: t.enabled,
                placeholder_key: t.placeholder_key,
                note: None,
                protocols: Some(t.protocols.join(",")),
            })
            .collect();
        rows.extend(settings.custom_agents.into_iter().map(|a| AgentRow {
            id: a.id,
            label: a.label,
            kind: "custom",
            // A user-defined agent has no config to point anywhere: it exists,
            // so it routes.
            routed: true,
            placeholder_key: a.placeholder_key,
            note: a.note,
            // None when the agent never said — which is a different thing from
            // any of the three words, and reads as `-` in the table.
            protocols: a.protocol,
        }));
        rows
    };
    let text = render_agent_rows(&rows);
    ctx.out.emit(&rows, || text);
    Ok(())
}

fn agents_add(
    ctx: &mut Ctx,
    name: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<(), CliError> {
    let created = {
        let store = ctx.store()?;
        vm::add_custom_agent(store, name, note, protocol)?
    };
    // The key is the whole integration surface, so it is printed rather than
    // left to be looked up: this is the credential a client is configured with,
    // and the gateway attributes traffic by nothing else.
    let text = format!(
        "added {} ({})\nplaceholder key: {}\nbind a provider: kiwano routes binding add {} <provider_id>",
        created.id,
        created.label,
        created.placeholder_key.as_deref().unwrap_or("-"),
        created.id
    );
    ctx.out.emit(&created, || text);
    ctx.after_mutation();
    Ok(())
}

fn agents_remove(ctx: &mut Ctx, id: &str) -> Result<(), CliError> {
    {
        let store = ctx.store()?;
        vm::remove_custom_agent(store, id)?;
    }
    ctx.out
        .line(format!("removed {id} (its usage history stays)"));
    ctx.after_mutation();
    Ok(())
}

fn render_agent_rows(rows: &[AgentRow]) -> String {
    if rows.is_empty() {
        return "(no agents)".to_string();
    }
    let head = ["ID", "LABEL", "KIND", "ROUTED", "PROTOCOL", "KEY"];
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                ellipsize(&r.id, 28),
                ellipsize(&r.label, 20),
                r.kind.to_string(),
                if r.routed { "yes" } else { "no" }.to_string(),
                r.protocols.clone().unwrap_or_else(|| "-".to_string()),
                r.placeholder_key.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&head, &table_rows, &[])
}

/// Take over, or restore, one agent — the operation that makes a server
/// usable, and the one the CLI could not do at all.
fn set_takeover(ctx: &mut Ctx, agent: &str, enabled: bool) -> Result<(), CliError> {
    let (home, port) = (ctx.home.clone(), ctx.data_port);
    {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::set_agent_takeover(store, aux, agent, enabled, port, &home, ctx.config_vars())?;
    }

    if enabled {
        // The placeholder key is the part that is invisible from outside, and
        // getting it wrong is the difference between a routed agent and a 401 —
        // the data plane only routes keys it minted itself. Printed as stored.
        let key = {
            let store = ctx.store()?;
            store
                .list_placeholder_keys()
                .map_err(runtime)?
                .into_iter()
                .find(|k| k.agent == agent)
                .map(|k| k.key)
        };
        ctx.out.line(format!(
            "{agent}: routed through the gateway on 127.0.0.1:{port}"
        ));
        match key {
            Some(key) => ctx.out.line(format!("placeholder key: {key}")),
            None => ctx.out.note(format!(
                "note: {agent} has no placeholder key; the gateway will refuse its requests"
            )),
        }
    } else {
        ctx.out.line(format!("{agent}: configuration restored"));
    }
    ctx.after_mutation();
    Ok(())
}
