//! The store half of a takeover: the provider imported from the agent's own
//! credentials, and the placeholder key the agent will carry.
//!
//! Moved here from `kiwano_core::vm::takeover` (`migrate.local.md` §10.20). The
//! **file** half — rewriting the agent's config — stays on the client, and it
//! has to: §5's first constraint forbids paths in this interface, and those
//! files are the user's own. What travels is the credential the client read out
//! of them.
//!
//! The key is the **caller's**, which is the same idempotency shape the other
//! three writes use: a replay registers the key it already minted rather than a
//! second one, so no operation table is needed on this side.

use crate::store::{Binding, Provider, Store, StrategyType};
use kiwano_api::error::ApiError;
use serde::{Deserialize, Serialize};

/// The credential an agent's own config carries, as the client reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentCreds {
    pub base_url: String,
    pub api_key: String,
    /// Friendly name when the agent's config declares one; absent → the caller
    /// falls back to the URL's host.
    #[serde(default)]
    pub name: Option<String>,
    /// "anthropic" | "openai"
    pub protocol: String,
}

pub fn host_of(base_url: &str) -> String {
    base_url
        .trim()
        .trim_end_matches('/')
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url)
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_string()
}

pub fn brand_name_for_host(host: &str) -> Option<&'static str> {
    const RULES: &[(&str, &str)] = &[
        ("moonshot", "Kimi"),
        ("bigmodel", "Zhipu GLM"),
        ("zhipu", "Zhipu GLM"),
        ("chatglm", "ChatGLM"),
        ("dashscope", "Qwen"),
        ("bailian", "Bailian"),
        ("aliyun", "Alibaba Cloud"),
        ("minimax", "MiniMax"),
        ("volces", "Doubao"),
        ("volcengine", "Doubao"),
        ("doubao", "Doubao"),
        ("hunyuan", "Hunyuan"),
        ("tencent", "Tencent Cloud"),
        ("wenxin", "Wenxin"),
        ("baidubce", "Baidu"),
        ("baidu", "Baidu"),
        ("modelscope", "ModelScope"),
        ("siliconflow", "SiliconFlow"),
        ("aihubmix", "AiHubMix"),
        ("openrouter", "OpenRouter"),
        ("packycode", "PackyCode"),
        ("newapi", "New API"),
        ("novita", "Novita"),
        ("ppio", "PPIO"),
        ("stepfun", "StepFun"),
        ("longcat", "LongCat"),
        ("xiaomi", "Xiaomi MiMo"),
        ("lingyiwanwu", "Yi"),
        ("deepseek", "DeepSeek"),
        ("kimi", "Kimi"),
        ("qwen", "Qwen"),
        ("generativelanguage", "Google Gemini"),
        ("googleapis", "Google"),
        ("gemini", "Gemini"),
        ("aistudio", "Google AI Studio"),
        ("grok", "Grok"),
        ("x.ai", "xAI"),
        ("anthropic", "Anthropic"),
        ("claude", "Claude"),
        ("openai", "OpenAI"),
        ("mistral", "Mistral"),
        ("cohere", "Cohere"),
        ("perplexity", "Perplexity"),
        ("ollama", "Ollama"),
        ("nvidia", "NVIDIA"),
        ("huggingface", "Hugging Face"),
        ("amazonaws", "AWS"),
        ("azure", "Azure"),
        ("cloudflare", "Cloudflare"),
        ("vercel", "Vercel"),
        ("github", "GitHub"),
    ];
    let h = host.to_ascii_lowercase();
    RULES
        .iter()
        .find(|(needle, _)| h.contains(needle))
        .map(|(_, name)| *name)
}

pub fn import_current_provider(store: &Store, creds: &CurrentCreds) -> Result<String, ApiError> {
    // An agent's config is another tool's file, and it can hold a bare host —
    // which is how a provider ends up stored as `api.deepseek.com` and unrouted.
    // Normalizing before the dedup lookup also means a config that gains its
    // scheme later still matches the row it made without one.
    let base = super::providers_add::absolute_endpoint(creds.base_url.trim_end_matches('/'));
    for p in store.list_providers().map_err(ApiError::failed)? {
        if p.base_url.trim().trim_end_matches('/') == base {
            return Ok(p.id);
        }
    }
    let now = crate::store::now_rfc3339();
    let name = creds.name.clone().unwrap_or_else(|| {
        let host = crate::api::takeover::host_of(&creds.base_url);
        crate::api::takeover::brand_name_for_host(&host)
            .map(String::from)
            .unwrap_or_else(|| {
                if host.is_empty() {
                    "Imported provider".into()
                } else {
                    host
                }
            })
    });
    let provider = Provider {
        // Imported from another manager: no Hub catalog entry behind it.
        catalog_id: None,
        model_default: None,
        id: format!(
            "{}-{}",
            kiwano_api::ids::slug(&name),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        ),
        name,
        protocol: crate::store::Protocol::parse_str(&creds.protocol)
            .unwrap_or(crate::store::Protocol::OpenAI),
        base_url: base.to_string(),
        openai_wire: crate::store::OpenAiWire::Both,
        api_path: None,
        endpoints: Vec::new(),
        api_key: Some(creds.api_key.clone()),
        billing: crate::store::Billing::Metered,
        period_limit: None,
        limit_unit: None,
        reset_period: None,
        plan_query: None,
        plan_limits: None,
        prices: None,
        timeout_secs: None,
        retries: None,
        headers: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    let id = provider.id.clone();
    store.insert_provider(&provider).map_err(ApiError::failed)?;
    Ok(id)
}

/// The store half of starting a takeover — everything except the agent's config
/// files, which the client rewrites.
///
/// `key` is the caller's, minted client-side: registering it twice is one row,
/// which is what makes a replay safe without an operation table here. The
/// import and the binding are best-effort in the same way they always were —
/// an agent whose config carries no credentials is a legitimate takeover, and
/// the onboarding guide steers those users to add a provider by hand.
pub fn phase_state(
    store: &Store,
    agent: &str,
    key: &str,
    creds: Option<&CurrentCreds>,
) -> Result<(), ApiError> {
    if let Some(creds) = creds {
        match import_current_provider(store, creds) {
            Ok(provider_id) => {
                // Only when the agent has no route: a takeover of an agent the
                // user already configured here must not reorder their queue.
                if store
                    .bindings_for_agent(agent)
                    .map_err(ApiError::failed)?
                    .is_empty()
                {
                    store
                        .upsert_strategy(agent, StrategyType::Single, None)
                        .map_err(ApiError::failed)?;
                    store
                        .upsert_binding(&Binding {
                            agent: agent.to_string(),
                            provider_id,
                            priority: 0,
                            weight: 1,
                            win_start: None,
                            win_end: None,
                            enabled: true,
                        })
                        .map_err(ApiError::failed)?;
                }
            }
            Err(e) => tracing::warn!(error = %e, "current-provider import skipped"),
        }
        // The import leaves the link empty (the agent's config knows nothing
        // about our catalog), and this is the one path where the provider starts
        // carrying traffic before any backfill pass runs — takeover is followed
        // immediately by real requests.
        let _ = super::catalog::link_providers(store);
    }
    // The handle the registration returned is the management surface's, not the
    // takeover's: the config file carries the value, and nothing here needs to
    // name the row again.
    store
        .upsert_client_key(key, agent)
        .map_err(ApiError::failed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store with an agent mid-takeover: a registered key, a route of its own,
    /// and a provider behind it.
    fn taken_over() -> Store {
        let store = Store::open_in_memory().unwrap();
        let now = crate::store::now_rfc3339();
        store
            .insert_provider(&Provider {
                catalog_id: None,
                model_default: None,
                id: "p-1".into(),
                name: "Upstream".into(),
                protocol: crate::store::Protocol::OpenAI,
                openai_wire: crate::store::OpenAiWire::Both,
                base_url: "https://api.upstream.example".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk-upstream".into()),
                billing: crate::store::Billing::Metered,
                period_limit: None,
                limit_unit: None,
                reset_period: None,
                plan_query: None,
                plan_limits: None,
                prices: None,
                timeout_secs: None,
                retries: None,
                headers: None,
                enabled: true,
                created_at: now.clone(),
                updated_at: now,
            })
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p-1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store
            .upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        store
            .upsert_client_key("kw-ag-claude-ab12", "claude")
            .unwrap();
        store
    }

    /// The read answers both questions a client has, and the rebuild half is the
    /// one it could not answer for itself: it carries the provider's
    /// **credential**, which lives in this row (`migrate.local.md` §10.43).
    #[test]
    fn the_state_read_carries_the_key_and_the_route_to_fall_back_to() {
        let store = taken_over();
        let state = takeover_state(&store, "claude").unwrap();
        assert_eq!(state.key.as_deref(), Some("kw-ag-claude-ab12"));
        let rebuild = state.rebuild.expect("a provider to point back at");
        assert_eq!(rebuild.base_url, "https://api.upstream.example");
        assert_eq!(rebuild.api_key, "sk-upstream");

        // An agent that was never taken over answers plainly rather than failing.
        let empty = takeover_state(&store, "codex").unwrap();
        assert_eq!(empty.key, None);
        assert!(empty.rebuild.is_none());
    }

    /// The two undos are different sizes, and the difference is what keeps a
    /// failed takeover from deleting a route the user had.
    #[test]
    fn unregistering_one_key_is_not_the_same_as_tearing_the_state_down() {
        let store = taken_over();

        assert!(unregister_key(&store, "claude").unwrap());
        assert!(store.list_client_keys().unwrap().is_empty());
        assert!(
            !store.bindings_for_agent("claude").unwrap().is_empty(),
            "the route stays: phase one may not have created it"
        );
        assert!(store.get_strategy("claude").unwrap().is_some());
        assert!(!unregister_key(&store, "claude").unwrap(), "idempotent");

        // And the full teardown is the one that clears the route.
        store
            .upsert_client_key("kw-ag-claude-ab12", "claude")
            .unwrap();
        teardown(&store, "claude").unwrap();
        assert!(store.list_client_keys().unwrap().is_empty());
        assert!(store.bindings_for_agent("claude").unwrap().is_empty());
        assert!(store.get_strategy("claude").unwrap().is_none());
        assert!(
            store.get_provider("p-1").unwrap().is_some(),
            "the provider is the user's own row, with its key and its history"
        );
    }

    #[test]
    fn import_current_provider_dedups_by_base_url() {
        let s = Store::open_in_memory().unwrap();
        let creds = |base: &str, name: Option<&str>| CurrentCreds {
            base_url: base.into(),
            api_key: "sk-x".into(),
            name: name.map(String::from),
            protocol: "openai".into(),
        };
        // trailing-slash variants dedup to one row
        let id1 =
            import_current_provider(&s, &creds("https://api.deepseek.com/v1/", Some("deepseek")))
                .unwrap();
        let id2 =
            import_current_provider(&s, &creds("https://api.deepseek.com/v1", Some("deepseek")))
                .unwrap();
        assert_eq!(id1, id2);
        let list = s.list_providers().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "deepseek");
        assert_eq!(list[0].api_key.as_deref(), Some("sk-x"));
        // no declared name → brand name inferred from the host
        let id3 = import_current_provider(&s, &creds("https://api.x.ai/v1", None)).unwrap();
        let p = s.get_provider(&id3).unwrap().unwrap();
        assert_eq!(p.name, "xAI");
        assert_ne!(id1, id3);
    }
}

/// The route an agent should be pointed back at when its own config cannot be
/// handed back — the provider it was using, with the credential to reach it.
///
/// Moved here from `kiwano_core::vm::takeover` with the rest of the store half:
/// the key it needs is in the daemon's `providers` row, and a client on another
/// machine has no way to read it (`migrate.local.md` §10.43).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeoverRebuildVm {
    pub base_url: String,
    pub api_key: String,
}

/// What a client needs to know about an agent's takeover before it acts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeoverStateVm {
    /// The placeholder key registered for this agent, if any. The client looks
    /// before it mints: a replay must register the key it already has rather
    /// than a second one the config it may still read would never carry.
    #[serde(default)]
    pub key: Option<String>,
    /// Where restore should point the agent if the backup turns out to be
    /// unusable. `None` when there is nothing safe to point at.
    #[serde(default)]
    pub rebuild: Option<TakeoverRebuildVm>,
}

/// The provider to fall back to when a backup cannot be written back.
///
/// Only for the agents whose config can be rebuilt from a provider at all, and
/// only when the provider's own endpoint is *not* the gateway: pointing an agent
/// back at loopback is the one outcome a restore must never produce.
pub fn rebuild_route(store: &Store, agent: &str) -> Result<Option<TakeoverRebuildVm>, ApiError> {
    if !kiwano_api::agents::REBUILDABLE_AGENTS.contains(&agent) {
        return Ok(None);
    }
    let Some(id) = store.primary_provider_id(agent).map_err(ApiError::failed)? else {
        return Ok(None);
    };
    let Some(provider) = store.get_provider(&id).map_err(ApiError::failed)? else {
        return Ok(None);
    };
    let Some(api_key) = provider
        .api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    else {
        return Ok(None);
    };
    let base_url = join_provider_url(&provider.base_url, provider.api_path.as_deref());
    if kiwano_adapters::codex_config::is_loopback_gateway_url(&base_url) {
        return Ok(None);
    }
    Ok(Some(TakeoverRebuildVm { base_url, api_key }))
}

/// The registered key and the rebuild fallback, in one answer — the two reads a
/// client makes about an agent's takeover state.
pub fn takeover_state(store: &Store, agent: &str) -> Result<TakeoverStateVm, ApiError> {
    let key = store
        .list_client_keys()
        .map_err(ApiError::failed)?
        .into_iter()
        .find(|k| k.agent == agent)
        .map(|k| k.key);
    Ok(TakeoverStateVm {
        key,
        rebuild: rebuild_route(store, agent)?,
    })
}

/// Undo the store half: the registration, the route, and the strategy.
///
/// All three, in one call, because they are one decision — the agent has its own
/// config back, so nothing about it is ours any more. Left behind, the key is
/// one the UI keeps offering and the config no longer carries, and the bindings
/// keep an agent that sends us nothing reading as *bound* and even as *in use*
/// (see `live_bound_agents`). The **providers** stay: they are the user's own
/// rows, with their keys, plans and usage history, and they are what a later
/// takeover re-imports and binds again.
pub fn teardown(store: &Store, agent: &str) -> Result<(), ApiError> {
    for k in store.list_client_keys().map_err(ApiError::failed)? {
        if k.agent == agent {
            store.delete_client_key(&k.id).map_err(ApiError::failed)?;
        }
    }
    for b in store.bindings_for_agent(agent).map_err(ApiError::failed)? {
        store
            .delete_binding(agent, &b.provider_id)
            .map_err(ApiError::failed)?;
    }
    store.delete_strategy(agent).map_err(ApiError::failed)?;
    Ok(())
}

/// `base_url` with the provider's optional `api_path` prefix appended.
fn join_provider_url(base_url: &str, api_path: Option<&str>) -> String {
    let base = base_url.trim().trim_end_matches('/');
    match api_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => format!(
            "{base}/{}",
            path.trim_start_matches('/').trim_end_matches('/')
        ),
        None => base.to_string(),
    }
}

/// Take back **one** registration — what a failed takeover's first phase has to
/// undo, and nothing else.
///
/// Narrower than [`teardown`] on purpose, and the difference is not cosmetic: a
/// takeover that fails while rewriting the agent's config must leave the rest of
/// the state as it found it. `phase_state` binds a provider only when the agent
/// had no route, so a full teardown here would delete a route the user had
/// before Kiwano was involved.
pub fn unregister_key(store: &Store, agent: &str) -> Result<bool, ApiError> {
    let mut removed = false;
    for k in store.list_client_keys().map_err(ApiError::failed)? {
        if k.agent == agent {
            removed |= store.delete_client_key(&k.id).map_err(ApiError::failed)?;
        }
    }
    Ok(removed)
}
