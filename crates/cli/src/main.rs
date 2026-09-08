//! kiwano-cli — manage Kiwano providers without the desktop app (spec §4.1
//! P1 CLI mode). Talks straight to the shared SQLite store (same file as the
//! GUI/gateway) and to the gateway admin plane for status/reload, so it works
//! on headless servers. Route changes are picked up via `POST /reload` when
//! the gateway is running.

mod admin;

use std::collections::BTreeMap;

use kiwano_gateway::store::{now_rfc3339, Billing, Binding, Protocol, Store, StrategyType};

const DEFAULT_ADMIN_PORT: u16 = 8310;
const USAGE_DAYS_DEFAULT: i64 = 7;

// ── parsing ---------------------------------------------------------------

#[derive(Debug)]
struct Globals {
    db: Option<String>,
    admin_port: u16,
    json: bool,
    rest: Vec<String>,
}

fn parse_globals(argv: Vec<String>) -> Result<Globals, String> {
    let mut db: Option<String> = None;
    let mut admin_port: Option<u16> = None;
    let mut json = false;
    let mut rest = Vec::new();

    let mut it = argv.into_iter();
    while let Some(word) = it.next() {
        match word.as_str() {
            "--db" => db = Some(it.next().ok_or("--db requires a path")?),
            "--admin-port" => {
                let v = it.next().ok_or("--admin-port requires a number")?;
                admin_port = Some(
                    v.parse()
                        .map_err(|_| format!("invalid --admin-port: {v}"))?,
                );
            }
            "--json" => json = true,
            _ => rest.push(word),
        }
    }
    let admin_port = match admin_port {
        Some(p) => p,
        None => match std::env::var("KIWANO_ADMIN_PORT") {
            Ok(v) if !v.is_empty() => v
                .parse()
                .map_err(|_| format!("invalid KIWANO_ADMIN_PORT: {v}"))?,
            _ => DEFAULT_ADMIN_PORT,
        },
    };
    Ok(Globals {
        db,
        admin_port,
        json,
        rest,
    })
}

/// Remove `flag` plus its value from `words`; None when absent.
fn take1(words: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    if let Some(i) = words.iter().position(|w| w == flag) {
        words.remove(i);
        let value = words
            .get(i)
            .cloned()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        words.remove(i);
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

/// All occurrences of `flag <value>` (repeatable flags like --bind).
fn take_many(words: &mut Vec<String>, flag: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    while let Some(v) = take1(words, flag)? {
        out.push(v);
    }
    Ok(out)
}

fn require(words: &mut Vec<String>, flag: &str, what: &str) -> Result<String, String> {
    take1(words, flag)?.ok_or_else(|| format!("{what} requires --{flag}"))
}

fn ensure_empty(words: &[String]) -> Result<(), String> {
    match words.first() {
        Some(w) => Err(format!("unexpected argument: {w}")),
        None => Ok(()),
    }
}

fn positional(words: &mut Vec<String>, what: &str) -> Result<String, String> {
    if words.is_empty() {
        Err(what.to_string())
    } else {
        Ok(words.remove(0))
    }
}

#[derive(Debug, PartialEq)]
struct AddArgs {
    name: String,
    endpoint: String,
    key: Option<String>,
    protocol: Protocol,
    billing: Billing,
    limit: Option<f64>,
    unit: Option<String>,
    reset: Option<String>,
    bind: Vec<String>,
}

fn parse_providers_add(mut words: Vec<String>) -> Result<AddArgs, String> {
    let name = require(&mut words, "--name", "providers add")?;
    let endpoint = require(&mut words, "--endpoint", "providers add")?;
    let key = take1(&mut words, "--key")?.filter(|k| !k.trim().is_empty());
    let protocol = match take1(&mut words, "--protocol")? {
        Some(v) => Protocol::from_str(&v)
            .ok_or_else(|| format!("invalid --protocol: {v} (openai|anthropic|gemini)"))?,
        None => Protocol::OpenAI,
    };
    let billing = match take1(&mut words, "--billing")? {
        Some(v) => Billing::from_str(&v)
            .ok_or_else(|| format!("invalid --billing: {v} (subscription|metered|unlimited)"))?,
        None => Billing::Metered,
    };
    let limit = match take1(&mut words, "--limit")? {
        Some(v) => {
            let n: f64 = v.parse().map_err(|_| format!("invalid --limit: {v}"))?;
            if n <= 0.0 {
                return Err("--limit must be a positive number".into());
            }
            Some(n)
        }
        None => None,
    };
    let unit = match take1(&mut words, "--unit")? {
        Some(v) => match v.as_str() {
            "requests" | "wan_tokens" | "cny" => Some(v),
            other => return Err(format!("invalid --unit: {other} (requests|wan_tokens|cny)")),
        },
        None => None,
    };
    let reset = match take1(&mut words, "--reset")? {
        Some(v) => match v.as_str() {
            "monthly" | "weekly" | "yearly" | "none" => Some(v),
            other => {
                return Err(format!(
                    "invalid --reset: {other} (monthly|weekly|yearly|none)"
                ))
            }
        },
        None => None,
    };
    let bind = take_many(&mut words, "--bind")?;
    ensure_empty(&words)?;
    if name.trim().is_empty() {
        return Err("--name must not be empty".into());
    }
    if endpoint.trim().is_empty() {
        return Err("--endpoint must not be empty".into());
    }
    Ok(AddArgs {
        name,
        endpoint,
        key,
        protocol,
        billing,
        limit,
        unit,
        reset,
        bind,
    })
}

fn parse_key_id(mut words: Vec<String>, what: &str) -> Result<i64, String> {
    let id = positional(&mut words, what)?;
    ensure_empty(&words)?;
    id.parse().map_err(|_| format!("invalid key id: {id}"))
}

// ── entry ------------------------------------------------------------------

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(argv));
}

fn run(argv: Vec<String>) -> i32 {
    let globals = match parse_globals(argv) {
        Ok(g) => g,
        Err(e) => return fail(&e),
    };
    if globals.rest.is_empty() {
        print_help();
        return 0;
    }
    match dispatch(globals) {
        Ok(code) => code,
        Err(e) => fail(&e),
    }
}

fn dispatch(globals: Globals) -> Result<i32, String> {
    let Globals {
        db,
        admin_port,
        json,
        rest,
    } = globals;
    let mut words = rest;
    let cmd = positional(&mut words, "nothing to do")?;
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(0)
        }
        "status" => {
            ensure_empty(&words)?;
            cmd_status(admin_port, &db, json)
        }
        "reload" => {
            ensure_empty(&words)?;
            cmd_reload(admin_port)
        }
        "providers" => {
            let sub = positional(
                &mut words,
                "providers requires a subcommand (list|add|use|remove)",
            )?;
            cmd_providers(sub, words, &db, admin_port, json)?;
            Ok(0)
        }
        "keys" => {
            let sub = positional(&mut words, "keys requires a subcommand (list|add|remove)")?;
            cmd_keys(sub, words, &db, admin_port, json)?;
            Ok(0)
        }
        "usage" => {
            let days = match take1(&mut words, "--days")? {
                Some(v) => Some(
                    v.parse::<i64>()
                        .map_err(|_| format!("invalid --days: {v}"))?,
                ),
                None => None,
            };
            let agent = take1(&mut words, "--agent")?;
            ensure_empty(&words)?;
            let store = open_store(&db)?;
            cmd_usage(&store, days, agent, json)?;
            Ok(0)
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn fail(message: &str) -> i32 {
    eprintln!("error: {message}");
    eprintln!("run `kiwano-cli help` for usage");
    2
}

fn open_store(db: &Option<String>) -> Result<Store, String> {
    let path = match db {
        Some(p) => std::path::PathBuf::from(p),
        None => match std::env::var("KIWANO_DB_PATH") {
            Ok(v) if !v.is_empty() => std::path::PathBuf::from(v),
            _ => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home)
                    .join(".kiwano")
                    .join("kiwano.db")
            }
        },
    };
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    Store::open(&path).map_err(|e| format!("cannot open database {}: {e}", path.display()))
}

/// Best-effort hot-reload of a running gateway after a mutation.
fn after_mutation(admin_port: u16) {
    match admin::post_reload(admin_port) {
        Some(v) => println!(
            "gateway route table reloaded ({} agents)",
            v["agents_routed"].as_u64().unwrap_or(0)
        ),
        None => {
            println!("note: gateway not reachable on :{admin_port}; changes apply on next start")
        }
    }
}

// ── commands ----------------------------------------------------------------

fn cmd_status(admin_port: u16, db: &Option<String>, json: bool) -> Result<i32, String> {
    match admin::get_status(admin_port) {
        Some(v) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else {
                println!("{}", render_status(&v, admin_port));
            }
            Ok(0)
        }
        None => {
            // Gateway down: still show local store counts (useful headless).
            let store = open_store(db)?;
            let m = store.metrics().map_err(|e| e.to_string())?;
            println!("gateway: not running (admin :{admin_port})");
            println!(
                "store: providers {} · bindings {} · placeholder_keys {} · usage_rows {}",
                m.providers, m.bindings, m.placeholder_keys, m.usage_rows
            );
            Ok(1)
        }
    }
}

fn cmd_reload(admin_port: u16) -> Result<i32, String> {
    match admin::post_reload(admin_port) {
        Some(v) => {
            println!(
                "reloaded: {} agents routed",
                v["agents_routed"].as_u64().unwrap_or(0)
            );
            Ok(0)
        }
        None => Err(format!("gateway not reachable on :{admin_port}")),
    }
}

fn cmd_providers(
    sub: String,
    mut words: Vec<String>,
    db: &Option<String>,
    admin_port: u16,
    json: bool,
) -> Result<(), String> {
    match sub.as_str() {
        "list" => {
            let agent = take1(&mut words, "--agent")?;
            ensure_empty(&words)?;
            let store = open_store(db)?;
            cmd_providers_list(&store, agent, json)
        }
        "add" => {
            let args = parse_providers_add(words)?;
            let store = open_store(db)?;
            cmd_providers_add(&store, &args)?;
            after_mutation(admin_port);
            Ok(())
        }
        "use" => {
            let id = positional(&mut words, "providers use requires a PROVIDER_ID")?;
            let agent = require(&mut words, "--agent", "providers use")?;
            ensure_empty(&words)?;
            let store = open_store(db)?;
            cmd_providers_use(&store, &id, &agent)?;
            after_mutation(admin_port);
            Ok(())
        }
        "remove" => {
            let id = positional(&mut words, "providers remove requires a PROVIDER_ID")?;
            ensure_empty(&words)?;
            let store = open_store(db)?;
            cmd_providers_remove(&store, &id)?;
            after_mutation(admin_port);
            Ok(())
        }
        other => Err(format!("unknown providers subcommand: {other}")),
    }
}

fn cmd_providers_list(store: &Store, agent: Option<String>, json: bool) -> Result<(), String> {
    let providers = store.list_providers().map_err(|e| e.to_string())?;
    let rows = provider_binding_rows(store)?;
    let selected: Vec<_> = providers
        .into_iter()
        .filter(|p| match &agent {
            Some(a) => rows
                .get(&p.id)
                .is_some_and(|rs| rs.iter().any(|(ag, _)| ag == a)),
            None => true,
        })
        .collect();

    if json {
        let list: Vec<serde_json::Value> = selected
            .iter()
            .map(|p| {
                let bindings = rows.get(&p.id).cloned().unwrap_or_default();
                serde_json::json!({
                    "id": p.id,
                    "name": p.name,
                    "protocol": p.protocol.as_str(),
                    "endpoint": p.base_url,
                    "billing": p.billing.as_str(),
                    "enabled": p.enabled,
                    "agents": bindings
                        .iter()
                        .map(|(a, prio)| serde_json::json!({ "agent": a, "priority": prio }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&list).unwrap_or_default()
        );
        return Ok(());
    }

    if selected.is_empty() {
        println!("(no providers)");
        return Ok(());
    }
    let mut out = format!(
        "{:<24} {:<16} {:<9} {:<28} {:<12} {}\n",
        "ID", "NAME", "PROTO", "ENDPOINT", "BILLING", "AGENTS"
    );
    for p in &selected {
        let agents = rows
            .get(&p.id)
            .map(|rs| {
                rs.iter()
                    .map(|(a, prio)| {
                        if *prio == 0 {
                            format!("{a}*")
                        } else {
                            a.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "{:<24} {:<16} {:<9} {:<28} {:<12} {}\n",
            ellipsize(&p.id, 23),
            ellipsize(&p.name, 15),
            p.protocol.as_str(),
            ellipsize(&p.base_url, 27),
            p.billing.as_str(),
            agents,
        ));
    }
    println!("{}", out.trim_end());
    Ok(())
}

fn cmd_providers_add(store: &Store, args: &AddArgs) -> Result<(), String> {
    let now = now_rfc3339();
    let id = format!(
        "{}-{}",
        slug(&args.name),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let provider = kiwano_gateway::store::Provider {
        id: id.clone(),
        name: args.name.trim().to_string(),
        protocol: args.protocol,
        base_url: args.endpoint.trim().to_string(),
        api_path: None,
        api_key: args.key.clone(),
        billing: args.billing,
        period_limit: args.limit,
        // Unit only matters together with a limit; NULL reads as requests.
        limit_unit: args
            .limit
            .map(|_| args.unit.clone().unwrap_or_else(|| "requests".into())),
        reset_period: match args.reset.as_deref() {
            Some("monthly") | Some("weekly") | Some("yearly") => args.reset.clone(),
            _ => None,
        },
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    store
        .insert_provider(&provider)
        .map_err(|e| e.to_string())?;
    for agent in &args.bind {
        bind_as_primary(store, agent, &id)?;
    }
    println!("added {} ({})", id, provider.name);
    Ok(())
}

fn cmd_providers_use(store: &Store, id: &str, agent: &str) -> Result<(), String> {
    if store.get_provider(id).map_err(|e| e.to_string())?.is_none() {
        return Err(format!("provider not found: {id}"));
    }
    bind_as_primary(store, agent, id)?;
    println!("{agent} -> {id} (primary)");
    Ok(())
}

fn cmd_providers_remove(store: &Store, id: &str) -> Result<(), String> {
    // Agents whose primary is this provider get their next candidate promoted
    // (same semantics as the GUI, vm.rs delete_provider).
    let mut affected = Vec::new();
    for agent in store.bound_agents().map_err(|e| e.to_string())? {
        if store
            .primary_provider_id(&agent)
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some(id)
        {
            affected.push(agent);
        }
    }
    let deleted = store.delete_provider(id).map_err(|e| e.to_string())?;
    if !deleted {
        return Err(format!("provider not found: {id}"));
    }
    for agent in affected {
        let remaining = store
            .bindings_for_agent(&agent)
            .map_err(|e| e.to_string())?;
        if let Some(next) = remaining.first() {
            store
                .upsert_binding(&Binding {
                    agent: agent.clone(),
                    provider_id: next.provider_id.clone(),
                    priority: 0,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(|e| e.to_string())?;
            for (i, b) in remaining
                .iter()
                .filter(|b| b.provider_id != next.provider_id)
                .enumerate()
            {
                store
                    .upsert_binding(&Binding {
                        agent: agent.clone(),
                        provider_id: b.provider_id.clone(),
                        priority: i as i64 + 1,
                        weight: b.weight,
                        win_start: b.win_start.clone(),
                        win_end: b.win_end.clone(),
                        enabled: b.enabled,
                    })
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    println!("removed {id}");
    Ok(())
}

/// Bind `provider_id` as the agent's primary (priority 0), demoting the
/// previous primary and reindexing the rest.
fn bind_as_primary(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    store
        .upsert_strategy(agent, StrategyType::Single, None)
        .map_err(|e| e.to_string())?;
    let others: Vec<String> = store
        .bindings_for_agent(agent)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|b| b.provider_id)
        .filter(|p| p != provider_id)
        .collect();
    store
        .upsert_binding(&Binding {
            agent: agent.to_string(),
            provider_id: provider_id.to_string(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .map_err(|e| e.to_string())?;
    for (i, p) in others.into_iter().enumerate() {
        store
            .upsert_binding(&Binding {
                agent: agent.to_string(),
                provider_id: p,
                priority: i as i64 + 1,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// provider_id → [(agent, priority)] from the bindings tables.
fn provider_binding_rows(store: &Store) -> Result<BTreeMap<String, Vec<(String, i64)>>, String> {
    let mut map = BTreeMap::new();
    for agent in store.bound_agents().map_err(|e| e.to_string())? {
        for b in store
            .bindings_for_agent(&agent)
            .map_err(|e| e.to_string())?
        {
            map.entry(b.provider_id)
                .or_insert_with(Vec::new)
                .push((agent.clone(), b.priority));
        }
    }
    Ok(map)
}

fn cmd_keys(
    sub: String,
    mut words: Vec<String>,
    db: &Option<String>,
    admin_port: u16,
    json: bool,
) -> Result<(), String> {
    let store = open_store(db)?;
    match sub.as_str() {
        "list" => {
            let provider = positional(&mut words, "keys list requires a PROVIDER_ID")?;
            ensure_empty(&words)?;
            cmd_keys_list(&store, &provider, json)
        }
        "add" => {
            let provider = positional(&mut words, "keys add requires a PROVIDER_ID")?;
            let key = require(&mut words, "--key", "keys add")?;
            let label = take1(&mut words, "--label")?;
            ensure_empty(&words)?;
            if key.trim().is_empty() {
                return Err("--key must not be empty".into());
            }
            if store
                .get_provider(&provider)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err(format!("provider not found: {provider}"));
            }
            let id = store
                .insert_api_key(&provider, key.trim(), label.as_deref().map(str::trim))
                .map_err(|e| e.to_string())?;
            println!("added key #{id} for {provider}");
            after_mutation(admin_port);
            Ok(())
        }
        "remove" => {
            let id = parse_key_id(words, "keys remove requires a KEY_ID")?;
            let deleted = store.delete_api_key(id).map_err(|e| e.to_string())?;
            if !deleted {
                return Err(format!("key not found: #{id}"));
            }
            println!("removed key #{id}");
            after_mutation(admin_port);
            Ok(())
        }
        other => Err(format!("unknown keys subcommand: {other}")),
    }
}

fn cmd_keys_list(store: &Store, provider: &str, json: bool) -> Result<(), String> {
    let keys = store.list_api_keys(provider).map_err(|e| e.to_string())?;
    if json {
        let list: Vec<serde_json::Value> = keys
            .iter()
            .map(|k| {
                serde_json::json!({
                    "id": k.id,
                    "key": mask_key(&k.api_key),
                    "label": k.label,
                    "enabled": k.enabled,
                    "created_at": k.created_at,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&list).unwrap_or_default()
        );
        return Ok(());
    }
    if keys.is_empty() {
        println!("(no rotation keys for {provider})");
        return Ok(());
    }
    let mut out = String::new();
    for k in &keys {
        let label = k.label.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "#{:<4} {:<24} {:<14} {}\n",
            k.id.to_string(),
            mask_key(&k.api_key),
            label,
            k.created_at
        ));
    }
    println!("{}", out.trim_end());
    Ok(())
}

/// Keys are never printed in full, even locally.
fn mask_key(k: &str) -> String {
    let chars: Vec<char> = k.chars().collect();
    if chars.len() > 12 {
        let head: String = chars[..6].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    } else {
        k.to_string()
    }
}

fn cmd_usage(
    store: &Store,
    days: Option<i64>,
    agent: Option<String>,
    json: bool,
) -> Result<(), String> {
    let days = days.unwrap_or(USAGE_DAYS_DEFAULT);
    if days <= 0 {
        return Err("--days must be a positive number".into());
    }
    let since = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let totals = store
        .usage_totals(agent.as_deref(), Some(&since))
        .map_err(|e| e.to_string())?;
    let by_provider = store
        .usage_by_provider(agent.as_deref(), Some(&since))
        .map_err(|e| e.to_string())?;
    let names: BTreeMap<String, String> = store
        .list_providers()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();

    if json {
        let v = serde_json::json!({
            "days": days,
            "agent": agent,
            "totals": totals,
            "by_provider": by_provider
                .iter()
                .map(|u| serde_json::json!({
                    "provider_id": u.provider_id,
                    "provider_name": names
                        .get(&u.provider_id)
                        .cloned()
                        .unwrap_or_else(|| u.provider_id.clone()),
                    "totals": u.totals,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }

    println!(
        "usage ({}d{})",
        days,
        agent
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default()
    );
    println!(
        "requests {} · input {} · output {} · cache_read {}",
        totals.requests,
        fmt_tokens(totals.input_tokens),
        fmt_tokens(totals.output_tokens),
        fmt_tokens(totals.cache_read_tokens)
    );
    if by_provider.is_empty() {
        println!("(no usage in window)");
        return Ok(());
    }
    println!("by provider:");
    for u in &by_provider {
        let name = names
            .get(&u.provider_id)
            .cloned()
            .unwrap_or_else(|| u.provider_id.clone());
        println!(
            "  {:<24} {:>6} req  in {:>7}  out {:>7}",
            ellipsize(&name, 23),
            u.totals.requests,
            fmt_tokens(u.totals.input_tokens),
            fmt_tokens(u.totals.output_tokens)
        );
    }
    Ok(())
}

fn render_status(v: &serde_json::Value, admin_port: u16) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "gateway: running (v{}, uptime {}s, admin 127.0.0.1:{admin_port})\n",
        v["version"].as_str().unwrap_or("?"),
        v["uptime_secs"].as_u64().unwrap_or(0)
    ));
    out.push_str(&format!(
        "store: providers {} · bindings {} · placeholder_keys {} · usage_rows {}\n",
        v["providers"], v["bindings"], v["placeholder_keys"], v["usage_rows"]
    ));
    if let Some(routes) = v["routes"].as_array() {
        if !routes.is_empty() {
            out.push_str("routes:\n");
            for r in routes {
                let candidates: Vec<&str> = r["candidates"]
                    .as_array()
                    .map(|cs| cs.iter().filter_map(|c| c["id"].as_str()).collect())
                    .unwrap_or_default();
                out.push_str(&format!(
                    "  {:<8} {:<10} primary={:<20} candidates={}\n",
                    r["agent"].as_str().unwrap_or("?"),
                    r["strategy"].as_str().unwrap_or("?"),
                    r["primary_provider"].as_str().unwrap_or("-"),
                    candidates.join(",")
                ));
            }
        }
    }
    out.trim_end().to_string()
}

// ── formatting helpers ------------------------------------------------------

fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "provider".into()
    } else {
        s
    }
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn fmt_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

fn print_help() {
    println!(
        "kiwano-cli — manage Kiwano providers without the desktop app

USAGE:
    kiwano-cli [--db PATH] [--admin-port N] [--json] <COMMAND>

GLOBAL FLAGS:
    --db PATH         SQLite file (default ~/.kiwano/kiwano.db, env KIWANO_DB_PATH)
    --admin-port N    gateway admin port (default {DEFAULT_ADMIN_PORT}, env KIWANO_ADMIN_PORT)
    --json            machine-readable output

COMMANDS:
    status                        gateway + store summary (exit 1 when gateway is down)
    reload                        hot-reload the running gateway's route table
    providers list [--agent A]    list providers (primary marked *)
    providers add --name N --endpoint URL [--key K] [--protocol openai|anthropic|gemini]
                 [--billing subscription|metered|unlimited] [--limit N --unit U]
                 [--reset monthly|weekly|yearly|none] [--bind AGENT]...
    providers use <ID> --agent A  make the provider the primary for an agent
    providers remove <ID>         delete provider (candidates re-promote automatically)
    keys list <PROVIDER_ID>       list rotation keys of a provider
    keys add <PROVIDER_ID> --key K [--label L]
    keys remove <KEY_ID>
    usage [--days N] [--agent A]  usage totals (default 7 days)

AGENTS:
    claude, claude-desktop, codex, gemini, grokbuild,
    opencode, openclaw, hermes, pi"
    );
}

// ── tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kiwano_gateway::store::UsageRecord;

    fn globals_of(argv: &[&str]) -> Globals {
        parse_globals(argv.iter().map(|s| s.to_string()).collect()).unwrap()
    }

    fn words_of(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn globals_extract_and_strip() {
        let g = globals_of(&["--db", "/tmp/x.db", "providers", "list", "--json"]);
        assert_eq!(g.db.as_deref(), Some("/tmp/x.db"));
        assert!(g.json);
        assert_eq!(g.rest, vec!["providers".to_string(), "list".to_string()]);

        let g = globals_of(&["status"]);
        assert_eq!(g.admin_port, DEFAULT_ADMIN_PORT);
        assert!(!g.json);
    }

    #[test]
    fn globals_reject_bad_port() {
        let err =
            parse_globals(vec!["--admin-port".into(), "abc".into(), "status".into()]).unwrap_err();
        assert!(err.contains("invalid --admin-port"));
    }

    #[test]
    fn providers_add_parse_full() {
        let args = parse_providers_add(words_of(&[
            "--name",
            "DeepSeek",
            "--endpoint",
            "https://api.deepseek.com",
            "--key",
            "sk-1",
            "--protocol",
            "anthropic",
            "--billing",
            "subscription",
            "--limit",
            "50",
            "--unit",
            "wan_tokens",
            "--reset",
            "monthly",
            "--bind",
            "claude",
            "--bind",
            "codex",
        ]))
        .unwrap();
        assert_eq!(args.name, "DeepSeek");
        assert_eq!(args.protocol, Protocol::Anthropic);
        assert_eq!(args.billing, Billing::Subscription);
        assert_eq!(args.limit, Some(50.0));
        assert_eq!(args.unit.as_deref(), Some("wan_tokens"));
        assert_eq!(args.reset.as_deref(), Some("monthly"));
        assert_eq!(args.bind, vec!["claude", "codex"]);
    }

    #[test]
    fn providers_add_parse_defaults_and_errors() {
        let args = parse_providers_add(words_of(&[
            "--name",
            "X",
            "--endpoint",
            "http://localhost:11434",
        ]))
        .unwrap();
        assert_eq!(args.protocol, Protocol::OpenAI);
        assert_eq!(args.billing, Billing::Metered);
        assert!(args.key.is_none() && args.limit.is_none() && args.bind.is_empty());
        assert_eq!(args.reset, None);

        assert!(parse_providers_add(vec!["--name".into(), "X".into()]).is_err());
        assert!(parse_providers_add(words_of(&[
            "--name",
            "X",
            "--endpoint",
            "u",
            "--protocol",
            "grpc"
        ]))
        .is_err());
        assert!(parse_providers_add(words_of(&[
            "--name",
            "X",
            "--endpoint",
            "u",
            "--limit",
            "-3"
        ]))
        .is_err());
        assert!(
            parse_providers_add(words_of(&["--name", "X", "--endpoint", "u", "extra"])).is_err()
        );
    }

    fn seed_provider(store: &Store, id: &str, name: &str) {
        let now = now_rfc3339();
        store
            .insert_provider(&kiwano_gateway::store::Provider {
                id: id.into(),
                name: name.into(),
                protocol: Protocol::OpenAI,
                base_url: "https://example.com".into(),
                api_path: None,
                api_key: Some("sk-x".into()),
                billing: Billing::Metered,
                period_limit: None,
                limit_unit: None,
                reset_period: None,
                enabled: true,
                created_at: now.clone(),
                updated_at: now,
            })
            .unwrap();
    }

    #[test]
    fn add_use_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();

        let args = AddArgs {
            name: "DeepSeek".into(),
            endpoint: "https://api.deepseek.com".into(),
            key: Some("sk-1".into()),
            protocol: Protocol::OpenAI,
            billing: Billing::Metered,
            limit: Some(50.0),
            unit: Some("cny".into()),
            reset: Some("none".into()),
            bind: vec!["claude".into()],
        };
        cmd_providers_add(&store, &args).unwrap();
        seed_provider(&store, "kimi", "Kimi");

        let stored = store.list_providers().unwrap();
        let ds = stored
            .iter()
            .find(|p| p.id != "kimi")
            .map(|p| p.id.clone())
            .unwrap();
        let row = stored.iter().find(|p| p.id == ds).unwrap();
        assert_eq!(row.limit_unit.as_deref(), Some("cny"));
        assert_eq!(row.reset_period, None, "--reset none stores NULL");

        // claude primary = deepseek-* row.
        assert_eq!(
            store.primary_provider_id("claude").unwrap().as_deref(),
            Some(ds.as_str())
        );

        // use kimi → primary flips, deepseek demoted to 1.
        cmd_providers_use(&store, "kimi", "claude").unwrap();
        let bindings = store.bindings_for_agent("claude").unwrap();
        assert_eq!(bindings[0].provider_id, "kimi");
        assert_eq!(bindings[0].priority, 0);
        assert_eq!(bindings[1].provider_id, ds);
        assert_eq!(bindings[1].priority, 1);

        // remove kimi → deepseek re-promoted.
        cmd_providers_remove(&store, "kimi").unwrap();
        let bindings = store.bindings_for_agent("claude").unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].provider_id, ds);
        assert_eq!(bindings[0].priority, 0);

        // remove missing → error.
        assert!(cmd_providers_remove(&store, "nope").is_err());
    }

    #[test]
    fn keys_roundtrip_and_masking() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed_provider(&store, "ds", "DeepSeek");

        assert!(
            store
                .insert_api_key("ds", "sk-rotate-1", Some("backup"))
                .unwrap()
                > 0
        );
        assert!(store.insert_api_key("ds", "sk-rotate-2", None).unwrap() > 0);
        let keys = store.list_api_keys("ds").unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(mask_key("sk-9f3e21a7c8d4b6e05a12"), "sk-9f3…5a12");

        // FK: keys against a missing provider are rejected.
        assert!(store.insert_api_key("ghost", "sk-x", None).is_err());

        assert!(store.delete_api_key(keys[0].id).unwrap());
        assert!(!store.delete_api_key(keys[0].id).unwrap());
    }

    #[test]
    fn usage_lists_rows_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed_provider(&store, "ds", "DeepSeek");
        store
            .record_usage(&UsageRecord {
                ts: now_rfc3339(),
                agent: "claude".into(),
                provider_id: "ds".into(),
                model: Some("deepseek-chat".into()),
                input_tokens: 1_500_000,
                output_tokens: 300_000,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(120),
                status: "ok".into(),
            })
            .unwrap();

        // The command is stdout-only; verify the aggregation it renders from.
        let since = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
        let totals = store.usage_totals(None, Some(&since)).unwrap();
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.input_tokens, 1_500_000);
        assert_eq!(fmt_tokens(totals.input_tokens), "1.5M");

        let by_agent = store.usage_totals(Some("codex"), Some(&since)).unwrap();
        assert_eq!(by_agent.requests, 0);
    }

    #[test]
    fn status_render_lists_routes() {
        let v: serde_json::Value = serde_json::json!({
            "ok": true, "name": "kiwano-gateway", "version": "0.1.0",
            "uptime_secs": 42, "providers": 2, "bindings": 2,
            "placeholder_keys": 1, "usage_rows": 7,
            "routes": [
                { "agent": "claude", "strategy": "failover",
                  "primary_provider": "ds",
                  "candidates": [{ "id": "ds" }, { "id": "kimi" }] }
            ]
        });
        let text = render_status(&v, 8310);
        assert!(text.contains("running (v0.1.0, uptime 42s"));
        assert!(text.contains("providers 2"));
        assert!(text.contains("failover"));
        assert!(text.contains("primary=ds"));
        assert!(text.contains("candidates=ds,kimi"));
    }

    #[test]
    fn providers_list_text_marks_primary() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed_provider(&store, "ds", "DeepSeek");
        bind_as_primary(&store, "claude", "ds").unwrap();

        // Primary is marked with *; filtering by an unknown agent is empty.
        assert_eq!(
            store.primary_provider_id("claude").unwrap().as_deref(),
            Some("ds")
        );
        assert!(provider_binding_rows(&store).unwrap().contains_key("ds"));
    }

    #[test]
    fn helpers_edge_cases() {
        assert_eq!(slug("DeepSeek 官方!!"), "deepseek");
        assert_eq!(slug("  "), "provider");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(12_400), "12k");
        assert_eq!(fmt_tokens(7_500_000), "7.5M");
        assert_eq!(ellipsize("short", 10), "short");
        assert_eq!(ellipsize("abcdefghijk", 6), "abcde…");
        assert!(parse_key_id(vec!["12".into()], "x").unwrap() == 12);
        assert!(parse_key_id(vec!["abc".into()], "x").is_err());
    }
}
