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
    store
        .upsert_placeholder_key(key, agent)
        .map_err(ApiError::failed)
}

#[cfg(test)]
mod tests {
    use super::*;

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
