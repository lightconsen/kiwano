//! Route table + strategy engine (tech.md §4.7).
//!
//! The gateway keeps an in-memory [`RouteTable`] loaded from SQLite; the admin
//! plane `/reload` rebuilds it (tech.md §4.6 control channel). MVP activates
//! the `single` strategy only: each agent routes to its unique primary
//! binding. The schema already carries candidates/priorities/weights so P1
//! strategies (failover/roundrobin) slot in without another migration.

use std::collections::{BTreeSet, HashMap};

use crate::error::{GatewayError, Result};
use crate::store::{Protocol, Store, StrategyType};

/// Canonical agent ids (tech.md §2.4 B: MVP takes over Claude Code + Codex).
pub const AGENT_CLAUDE: &str = "claude";
pub const AGENT_CODEX: &str = "codex";
pub const AGENT_GEMINI: &str = "gemini";

/// Prefix of per-agent placeholder keys: `kw-ag-<agent>-<rand>`.
pub const PLACEHOLDER_KEY_PREFIX: &str = "kw-ag-";

/// An upstream provider as seen by the routing/proxy layer.
#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamProvider {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    /// Optional upstream path prefix, e.g. `/anthropic` on compatible endpoints.
    pub api_path: Option<String>,
    pub api_key: Option<String>,
    /// roundrobin 权重（tech.md §4.7：默认 1）。
    pub weight: i64,
    /// timewindow 本地窗口 `HH:MM`（agent_bindings 透传）。
    pub win_start: Option<String>,
    pub win_end: Option<String>,
}

/// Routing configuration for one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRoute {
    pub agent: String,
    pub strategy: StrategyType,
    /// 策略 config 原始 JSON（quota 阈值等，agent_strategies.config 透传）。
    pub config: Option<String>,
    /// Ordered candidates: priority 0 (primary) first, backups after.
    pub candidates: Vec<UpstreamProvider>,
}

/// In-memory snapshot of `providers` + `agent_strategies` + `agent_bindings`
/// + `placeholder_keys`, rebuilt on `/reload`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteTable {
    /// Placeholder key → agent (attribution map, tech.md §4.6).
    pub keys: HashMap<String, String>,
    /// agent → route (strategy + ordered candidates).
    pub routes: HashMap<String, AgentRoute>,
}

/// How the agent of a request was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribution {
    /// A registered placeholder key in the auth headers.
    PlaceholderKey,
    /// Key missing/unknown → fell back by path protocol (+ warning log).
    PathFallback,
}

/// Result of routing one inbound request.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedRequest {
    pub agent: String,
    pub attribution: Attribution,
    pub provider: UpstreamProvider,
}

impl RouteTable {
    /// Rebuild the route table from the SQLite SSOT.
    pub fn load(store: &Store) -> Result<RouteTable> {
        let mut keys = HashMap::new();
        let mut agents: BTreeSet<String> = BTreeSet::new();

        for k in store.list_placeholder_keys()? {
            keys.insert(k.key, k.agent.clone());
            agents.insert(k.agent);
        }
        for a in store.bound_agents()? {
            agents.insert(a);
        }

        let providers: HashMap<String, _> = store
            .list_providers()?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();

        let mut routes = HashMap::new();
        for agent in agents {
            let strategy_row = store.get_strategy(&agent)?;
            let strategy = strategy_row
                .as_ref()
                .map(|s| s.kind)
                .unwrap_or(StrategyType::Single);
            let config = strategy_row.and_then(|s| s.config);
            let mut candidates = Vec::new();
            for b in store.bindings_for_agent(&agent)? {
                if !b.enabled {
                    continue;
                }
                match providers.get(&b.provider_id) {
                    Some(p) if p.enabled => candidates.push(UpstreamProvider {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        protocol: p.protocol,
                        base_url: p.base_url.clone(),
                        api_path: p.api_path.clone(),
                        api_key: p.api_key.clone(),
                        weight: b.weight,
                        win_start: b.win_start,
                        win_end: b.win_end,
                    }),
                    Some(_) => tracing::warn!(
                        agent = %agent,
                        provider_id = %b.provider_id,
                        "binding references disabled provider; skipped"
                    ),
                    None => tracing::warn!(
                        agent = %agent,
                        provider_id = %b.provider_id,
                        "binding references missing provider; skipped"
                    ),
                }
            }
            routes.insert(
                agent.clone(),
                AgentRoute {
                    agent,
                    strategy,
                    config,
                    candidates,
                },
            );
        }

        Ok(RouteTable { keys, routes })
    }

    /// Attribute a placeholder key to its agent, if registered.
    pub fn agent_for_key(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }

    /// Fallback attribution when the key is missing/unknown (tech.md §4.6):
    /// Anthropic paths → Claude Code, OpenAI paths → Codex. Ambiguous paths
    /// default to the primary (Claude) agent.
    pub fn fallback_agent(protocol: Option<Protocol>) -> &'static str {
        match protocol {
            Some(Protocol::OpenAI) => AGENT_CODEX,
            _ => AGENT_CLAUDE,
        }
    }

    /// Select the upstream provider for an agent under its strategy.
    ///
    /// Single/legacy path: always the first (primary) candidate. The live
    /// data plane uses [`resolve_via_engine`] instead; this stays for
    /// attribution-only callers and tests.
    pub fn select(&self, agent: &str) -> Result<&UpstreamProvider> {
        let route = self
            .routes
            .get(agent)
            .ok_or_else(|| GatewayError::NoBinding(agent.to_string()))?;
        route
            .candidates
            .first()
            .ok_or_else(|| GatewayError::NoBinding(agent.to_string()))
    }
}

/// Attribute an inbound request to an agent (placeholder key, fallback by
/// path protocol) and return its route.
fn route_agent<'t>(
    table: &'t RouteTable,
    protocol_hint: Option<Protocol>,
    placeholder_key: Option<&str>,
) -> Result<(&'t AgentRoute, Attribution)> {
    let (agent, attribution) = match placeholder_key.and_then(|k| table.agent_for_key(k)) {
        Some(a) => (a.to_string(), Attribution::PlaceholderKey),
        None => {
            let fb = RouteTable::fallback_agent(protocol_hint);
            tracing::warn!(
                key_present = placeholder_key.is_some(),
                fallback_agent = fb,
                "placeholder key missing/unknown; attributing by path protocol"
            );
            (fb.to_string(), Attribution::PathFallback)
        }
    };
    let route = table
        .routes
        .get(&agent)
        .ok_or_else(|| GatewayError::NoBinding(agent))?;
    Ok((route, attribution))
}

/// Resolve agent attribution + provider selection for one inbound request
/// via the strategy engine (tech.md §4.7: the live data-plane path).
pub async fn resolve_via_engine(
    table: &RouteTable,
    engine: &crate::strategy::StrategyEngine,
    store: &crate::store::Store,
    protocol_hint: Option<Protocol>,
    placeholder_key: Option<&str>,
    session: Option<&str>,
) -> Result<RoutedRequest> {
    let (route, attribution) = route_agent(table, protocol_hint, placeholder_key)?;
    let provider = engine.select(store, route, session).await?;
    Ok(RoutedRequest {
        agent: route.agent.clone(),
        attribution,
        provider,
    })
}

/// Attribution-only resolution with single-strategy selection (kept for
/// tests and tooling; the data plane routes through the strategy engine).
pub fn resolve(
    table: &RouteTable,
    protocol_hint: Option<Protocol>,
    placeholder_key: Option<&str>,
) -> Result<RoutedRequest> {
    let (route, attribution) = route_agent(table, protocol_hint, placeholder_key)?;
    let provider = table.select(&route.agent)?.clone();
    Ok(RoutedRequest {
        agent: route.agent.clone(),
        attribution,
        provider,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{now_rfc3339, Billing, Binding, Provider};

    fn provider(id: &str, protocol: Protocol, enabled: bool) -> Provider {
        Provider {
            id: id.to_string(),
            name: format!("prov-{id}"),
            protocol,
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            api_key: Some(format!("sk-{id}")),
            billing: Billing::Metered,
            period_limit: None,
            reset_period: None,
            enabled,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    }

    fn binding(agent: &str, provider_id: &str, priority: i64, enabled: bool) -> Binding {
        Binding {
            agent: agent.to_string(),
            provider_id: provider_id.to_string(),
            priority,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled,
        }
    }

    /// Seed a store inside a caller-owned TempDir (kept alive for the test).
    fn seeded_store(dir: &tempfile::TempDir) -> Store {
        let store = Store::open(dir.path().join("t.db")).unwrap();
        store
            .insert_provider(&provider("p-ant", Protocol::Anthropic, true))
            .unwrap();
        store
            .insert_provider(&provider("p-oai", Protocol::OpenAI, true))
            .unwrap();
        store
            .insert_provider(&provider("p-backup", Protocol::Anthropic, true))
            .unwrap();
        store
            .insert_provider(&provider("p-off", Protocol::Anthropic, false))
            .unwrap();

        store
            .upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        store
            .upsert_strategy("codex", StrategyType::Single, None)
            .unwrap();

        store
            .upsert_binding(&binding("claude", "p-ant", 0, true))
            .unwrap();
        store
            .upsert_binding(&binding("claude", "p-backup", 1, true))
            .unwrap();
        store
            .upsert_binding(&binding("claude", "p-off", 0, true))
            .unwrap();
        store
            .upsert_binding(&binding("codex", "p-oai", 0, true))
            .unwrap();

        store
            .upsert_placeholder_key("kw-ag-claude-abc123", AGENT_CLAUDE)
            .unwrap();
        store
            .upsert_placeholder_key("kw-ag-codex-xyz789", AGENT_CODEX)
            .unwrap();
        store
    }

    #[test]
    fn route_table_loads_from_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(&dir);
        let table = RouteTable::load(&store).unwrap();

        assert_eq!(
            table.agent_for_key("kw-ag-claude-abc123"),
            Some(AGENT_CLAUDE)
        );
        assert_eq!(table.agent_for_key("kw-ag-codex-xyz789"), Some(AGENT_CODEX));
        assert_eq!(table.agent_for_key("unknown"), None);

        let claude = table.routes.get(AGENT_CLAUDE).unwrap();
        assert_eq!(claude.strategy, StrategyType::Single);
        // Priority-0 enabled candidate first; disabled p-off excluded.
        assert_eq!(claude.candidates.len(), 2);
        assert_eq!(claude.candidates[0].id, "p-ant");
        assert_eq!(claude.candidates[1].id, "p-backup");

        let codex = table.routes.get(AGENT_CODEX).unwrap();
        assert_eq!(codex.candidates.len(), 1);
        assert_eq!(codex.candidates[0].protocol, Protocol::OpenAI);
    }

    #[test]
    fn single_strategy_selects_primary() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(&dir);
        let table = RouteTable::load(&store).unwrap();

        let routed = resolve(
            &table,
            Some(Protocol::Anthropic),
            Some("kw-ag-claude-abc123"),
        )
        .unwrap();
        assert_eq!(routed.agent, AGENT_CLAUDE);
        assert_eq!(routed.attribution, Attribution::PlaceholderKey);
        assert_eq!(routed.provider.id, "p-ant");

        // Disabling the primary promotes the backup on the next reload.
        store
            .upsert_binding(&binding("claude", "p-ant", 0, false))
            .unwrap();
        let table = RouteTable::load(&store).unwrap();
        let routed = resolve(
            &table,
            Some(Protocol::Anthropic),
            Some("kw-ag-claude-abc123"),
        )
        .unwrap();
        assert_eq!(routed.provider.id, "p-backup");
    }

    #[test]
    fn key_attribution_falls_back_by_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(&dir);
        let table = RouteTable::load(&store).unwrap();

        // Unknown key → path fallback (warn logged).
        let routed = resolve(&table, Some(Protocol::OpenAI), Some("sk-not-ours")).unwrap();
        assert_eq!(routed.agent, AGENT_CODEX);
        assert_eq!(routed.attribution, Attribution::PathFallback);
        assert_eq!(routed.provider.id, "p-oai");

        // No key at all → same fallback.
        let routed = resolve(&table, Some(Protocol::Anthropic), None).unwrap();
        assert_eq!(routed.agent, AGENT_CLAUDE);
        assert_eq!(routed.provider.id, "p-ant");

        // Ambiguous path without key defaults to the primary agent.
        let routed = resolve(&table, None, None).unwrap();
        assert_eq!(routed.agent, AGENT_CLAUDE);
    }

    #[test]
    fn select_errors_without_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let table = RouteTable::load(&store).unwrap();

        let err = resolve(&table, Some(Protocol::Anthropic), None).unwrap_err();
        assert!(matches!(err, GatewayError::NoBinding(a) if a == AGENT_CLAUDE));

        // Key registered but agent has no bindings → same clean failure.
        store
            .upsert_placeholder_key("kw-ag-gemini-1", AGENT_GEMINI)
            .unwrap();
        let table = RouteTable::load(&store).unwrap();
        let err = resolve(&table, None, Some("kw-ag-gemini-1")).unwrap_err();
        assert!(matches!(err, GatewayError::NoBinding(a) if a == AGENT_GEMINI));
    }

    #[test]
    fn fallback_agent_mapping() {
        assert_eq!(
            RouteTable::fallback_agent(Some(Protocol::Anthropic)),
            AGENT_CLAUDE
        );
        assert_eq!(
            RouteTable::fallback_agent(Some(Protocol::OpenAI)),
            AGENT_CODEX
        );
        assert_eq!(RouteTable::fallback_agent(None), AGENT_CLAUDE);
    }
}
