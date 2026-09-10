//! First-takeover provider import: read the provider an agent is currently
//! using straight from its config, so "Enable Kiwano" can import it into the
//! store and bind it as the agent's sole candidate — the agent's upstream
//! stays the same on day one, just routed (and metered) through the gateway.
//!
//! Exclusive-switch agents (claude/codex/gemini/grokbuild) hold one provider
//! slot in their config; the additive agents (opencode/openclaw/hermes/pi)
//! route via a selection over multiple entries (readers live in
//! `kiwano_adapters::gateway_takeover`). Claude Desktop is not read (MVP):
//! its 3p configLibrary profile handling is macOS-only and adds little —
//! the onboarding guide steers those users to manual provider entry.
//!
//! Official-subscription logins (Claude/Gemini OAuth, Codex ChatGPT login)
//! have no extractable API key — `read_current_creds` returns None and the
//! UI's onboarding guide takes over. OAuth-token proxying is deferred (see
//! plan backlog).

use std::path::Path;

use serde_json::Value;

use kiwano_adapters::gateway_takeover::CurrentProvider;

/// The provider an agent currently routes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentCreds {
    pub base_url: String,
    pub api_key: String,
    /// Friendly name when the agent's config declares one (additive agents'
    /// provider keys); None → callers fall back to the URL host.
    pub name: Option<String>,
    /// "anthropic" | "gemini" | "openai"
    pub protocol: &'static str,
}

/// Read the agent's current provider. None = nothing extractable (official
/// login, empty config, or a config that already points at a local gateway).
pub fn read_current_creds(agent: &str, home: &Path) -> Option<CurrentCreds> {
    let creds = match agent {
        "claude" => read_claude(home),
        "codex" => read_codex(home),
        "gemini" => read_gemini(home),
        "grokbuild" => read_grokbuild(home),
        "opencode" => read_additive_one(
            home.join(".config").join("opencode").join("opencode.json"),
            |c| kiwano_adapters::gateway_takeover::read_opencode_current(c),
        ),
        "openclaw" => read_additive_one(home.join(".openclaw").join("openclaw.json"), |c| {
            kiwano_adapters::gateway_takeover::read_openclaw_current(c)
        }),
        "hermes" => {
            let dir = std::env::var_os("HERMES_HOME")
                .map(|v| v.to_string_lossy().trim().to_string())
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| home.join(".hermes"));
            read_additive_one(dir.join("config.yaml"), |c| {
                kiwano_adapters::gateway_takeover::read_hermes_current(c)
            })
        }
        "pi" => {
            let dir = home.join(".pi").join("agent");
            let models = std::fs::read_to_string(dir.join("models.json")).ok()?;
            let settings = std::fs::read_to_string(dir.join("settings.json")).ok()?;
            Some(from_additive(
                kiwano_adapters::gateway_takeover::read_pi_current(&models, &settings)?,
            ))
        }
        // claude-desktop: not extracted (MVP) — see module docs
        _ => None,
    }?;
    sanitize(creds)
}

/// Never import a local endpoint as an upstream: that is either our own
/// gateway (post-takeover config) or another local proxy — both would loop.
fn sanitize(creds: CurrentCreds) -> Option<CurrentCreds> {
    let host = host_of(&creds.base_url).to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]"
    {
        return None;
    }
    Some(creds)
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

/// Friendly brand name inferred from an endpoint host — the display-name twin
/// of the frontend icon inference (components/icons/infer.ts, same rules and
/// order). Used at import time so a row reads "DeepSeek" instead of
/// "api.deepseek.com" when the agent's config declares no provider name.
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

fn from_additive(p: CurrentProvider) -> CurrentCreds {
    CurrentCreds {
        base_url: p.base_url,
        api_key: p.api_key,
        name: Some(p.name),
        protocol: "openai",
    }
}

/// Read one file and hand its content to an additive-config reader.
fn read_additive_one(
    path: std::path::PathBuf,
    read: impl Fn(&str) -> Option<CurrentProvider>,
) -> Option<CurrentCreds> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(from_additive(read(&content)?))
}

/// claude: settings.json env.ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN (or
/// ANTHROPIC_API_KEY). Absent/official → None.
fn read_claude(home: &Path) -> Option<CurrentCreds> {
    let content = std::fs::read_to_string(home.join(".claude").join("settings.json")).ok()?;
    let v: Value = serde_json::from_str(&content).ok()?;
    let env = v.get("env")?;
    let base_url = env.get("ANTHROPIC_BASE_URL")?.as_str()?.trim().to_string();
    let api_key = ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]
        .iter()
        .find_map(|k| env.get(k)?.as_str().map(str::to_string))?;
    if api_key.trim().is_empty() {
        return None;
    }
    Some(CurrentCreds {
        base_url,
        api_key,
        name: None,
        protocol: "anthropic",
    })
}

/// codex: any `base_url = "…"` line in config.toml (same scan as the rewrite)
/// + OPENAI_API_KEY from auth.json (empty for ChatGPT-login setups → None).
fn read_codex(home: &Path) -> Option<CurrentCreds> {
    let toml = std::fs::read_to_string(home.join(".codex").join("config.toml")).ok()?;
    let base_url = toml.lines().find_map(|line| match line.split_once('=') {
        Some((k, v)) if k.trim() == "base_url" => Some(
            v.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim_end_matches('/')
                .to_string(),
        ),
        _ => None,
    })?;
    let auth = std::fs::read_to_string(home.join(".codex").join("auth.json")).ok()?;
    let v: Value = serde_json::from_str(&auth).ok()?;
    let api_key = v.get("OPENAI_API_KEY")?.as_str()?.trim().to_string();
    if api_key.is_empty() {
        return None;
    }
    Some(CurrentCreds {
        base_url,
        api_key,
        name: None,
        protocol: "openai",
    })
}

/// gemini: .env GOOGLE_GEMINI_BASE_URL + GEMINI_API_KEY.
fn read_gemini(home: &Path) -> Option<CurrentCreds> {
    let content = std::fs::read_to_string(home.join(".gemini").join(".env")).ok()?;
    let env = kiwano_adapters::gemini_config::parse_env_file(&content);
    let base_url = env.get("GOOGLE_GEMINI_BASE_URL")?.trim().to_string();
    let api_key = env.get("GEMINI_API_KEY")?.trim().to_string();
    if base_url.is_empty() || api_key.is_empty() {
        return None;
    }
    Some(CurrentCreds {
        base_url,
        api_key,
        name: None,
        protocol: "gemini",
    })
}

/// grokbuild: the selected model's base_url + credentials (adapter handles
/// the env_key indirection).
fn read_grokbuild(home: &Path) -> Option<CurrentCreds> {
    let toml = std::fs::read_to_string(home.join(".grok").join("config.toml")).ok()?;
    let (base_url, api_key) = kiwano_adapters::grok_config::extract_credentials(&toml)?;
    Some(CurrentCreds {
        base_url,
        api_key,
        name: None,
        protocol: "openai",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kiwano-creds-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn host_of_extracts_hostname() {
        assert_eq!(host_of("https://api.deepseek.com/v1"), "api.deepseek.com");
        assert_eq!(host_of("http://127.0.0.1:8317/v1"), "127.0.0.1");
        assert_eq!(host_of("api.moonshot.cn"), "api.moonshot.cn");
    }

    #[test]
    fn brand_name_infers_from_host() {
        assert_eq!(brand_name_for_host("api.deepseek.com"), Some("DeepSeek"));
        assert_eq!(brand_name_for_host("api.moonshot.cn"), Some("Kimi"));
        assert_eq!(brand_name_for_host("open.bigmodel.cn"), Some("Zhipu GLM"));
        assert_eq!(brand_name_for_host("dashscope.aliyuncs.com"), Some("Qwen"));
        assert_eq!(
            brand_name_for_host("generativelanguage.googleapis.com"),
            Some("Google Gemini")
        );
        assert_eq!(brand_name_for_host("api.x.ai"), Some("xAI"));
        assert_eq!(brand_name_for_host("example.com"), None);
    }

    #[test]
    fn sanitize_rejects_local_gateways() {
        let mk = |base: &str| CurrentCreds {
            base_url: base.into(),
            api_key: "k".into(),
            name: None,
            protocol: "openai",
        };
        assert!(sanitize(mk("http://127.0.0.1:8317/v1")).is_none());
        assert!(sanitize(mk("https://localhost:9000")).is_none());
        assert!(sanitize(mk("https://api.deepseek.com")).is_some());
    }

    #[test]
    fn claude_creds_extracted_from_settings_env() {
        let home = temp_home("claude");
        let dir = home.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://packycode.com","ANTHROPIC_AUTH_TOKEN":"sk-pk"}}"#,
        )
        .unwrap();
        let creds = read_current_creds("claude", &home).unwrap();
        assert_eq!(creds.base_url, "https://packycode.com");
        assert_eq!(creds.api_key, "sk-pk");
        assert_eq!(creds.protocol, "anthropic");

        // official login (no env) → None
        std::fs::write(dir.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        assert!(read_current_creds("claude", &home).is_none());
    }

    #[test]
    fn codex_creds_need_both_toml_and_auth() {
        let home = temp_home("codex");
        let dir = home.join(".codex");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            "[model_providers.deepseek]\nbase_url = \"https://api.deepseek.com/v1\"\n",
        )
        .unwrap();
        // auth.json missing / ChatGPT login (no key) → None
        assert!(read_current_creds("codex", &home).is_none());
        std::fs::write(dir.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-ds"}"#).unwrap();
        let creds = read_current_creds("codex", &home).unwrap();
        assert_eq!(creds.base_url, "https://api.deepseek.com/v1");
        assert_eq!(creds.api_key, "sk-ds");
    }

    #[test]
    fn gemini_creds_from_env_file() {
        let home = temp_home("gemini");
        let dir = home.join(".gemini");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".env"),
            "GOOGLE_GEMINI_BASE_URL=https://aihubmix.com/gemini\nGEMINI_API_KEY=sk-gm\n",
        )
        .unwrap();
        let creds = read_current_creds("gemini", &home).unwrap();
        assert_eq!(creds.base_url, "https://aihubmix.com/gemini");
        assert_eq!(creds.protocol, "gemini");
    }

    #[test]
    fn grokbuild_creds_via_adapter() {
        let home = temp_home("grok");
        let dir = home.join(".grok");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            "[models]\ndefault = \"main\"\n\n[model.\"main\"]\nname = \"xAI\"\nmodel = \"grok-4\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"sk-x\"\napi_backend = \"responses\"\ncontext_window = 200000\n",
        )
        .unwrap();
        let creds = read_current_creds("grokbuild", &home).unwrap();
        assert_eq!(creds.base_url, "https://api.x.ai/v1");
        assert_eq!(creds.api_key, "sk-x");
    }

    #[test]
    fn opencode_creds_from_selection() {
        let home = temp_home("opencode");
        let dir = home.join(".config").join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("opencode.json"),
            r#"{"model":"deepseek/deepseek-chat","provider":{"deepseek":{"options":{"baseURL":"https://api.deepseek.com/v1","apiKey":"sk-ds"}}}}"#,
        )
        .unwrap();
        let creds = read_current_creds("opencode", &home).unwrap();
        assert_eq!(creds.name.as_deref(), Some("deepseek"));
        assert_eq!(creds.base_url, "https://api.deepseek.com/v1");

        // post-takeover config (gateway selected) → None
        std::fs::write(
            dir.join("opencode.json"),
            r#"{"model":"kiwano-gateway/deepseek-chat","provider":{"kiwano-gateway":{"options":{"baseURL":"http://127.0.0.1:8317/v1","apiKey":"kw"}}}}"#,
        )
        .unwrap();
        assert!(read_current_creds("opencode", &home).is_none());
    }

    #[test]
    fn unknown_or_desktop_agents_yield_none() {
        let home = temp_home("none");
        assert!(read_current_creds("claude-desktop", &home).is_none());
        assert!(read_current_creds("nonexistent", &home).is_none());
    }
}
