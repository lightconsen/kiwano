//! First-takeover provider import: read the provider an agent is currently
//! using straight from its config, so "Enable Kiwano" can import it into the
//! store and bind it as the agent's sole candidate — the agent's upstream
//! stays the same on day one, just routed (and metered) through the gateway.
//!
//! Exclusive-switch agents (claude/codex/grokbuild) hold one provider slot in
//! their config; the additive agents (opencode/openclaw/hermes/pi)
//! route via a selection over multiple entries (readers live in
//! `kiwano_adapters::gateway_takeover`). Claude Desktop is not read (MVP):
//! its 3p configLibrary profile handling is macOS-only and adds little —
//! the onboarding guide steers those users to manual provider entry.
//!
//! Official-subscription logins (Claude OAuth, Codex ChatGPT login) have no
//! extractable API key — `read_current_creds` returns None and the
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
    /// "anthropic" | "openai"
    pub protocol: &'static str,
}

/// Read the agent's current provider. None = nothing extractable (official
/// login, empty config, or a config that already points at a local gateway).
pub fn read_current_creds(agent: &str, home: &Path) -> Option<CurrentCreds> {
    let creds = match agent {
        "claude" => read_claude(home),
        "codex" => read_codex(home),
        "grokbuild" => read_grokbuild(home),
        "opencode" => read_additive_one(
            home.join(".config").join("opencode").join("opencode.json"),
            kiwano_adapters::gateway_takeover::read_opencode_current,
        ),
        // MiMo follows the XDG rules for where its global config lives, same
        // as OpenCode — a moved XDG_CONFIG_HOME moves this file too.
        "mimo" => read_additive_one(
            home.join(".config").join("mimocode").join("mimocode.jsonc"),
            kiwano_adapters::gateway_takeover::read_mimo_current,
        ),
        // MiniMax Code names its own relocation variable; the default root is
        // ~/.minimax (the shipped CLI's own resolution).
        "mcode" => {
            let dir = match kiwano_adapters::config::env_dir("MCODE_CONFIG_DIR") {
                kiwano_adapters::config::EnvDir::Absolute(dir) => dir,
                kiwano_adapters::config::EnvDir::Unset => home.join(".minimax"),
                kiwano_adapters::config::EnvDir::Relative(_) => return None,
            };
            read_additive_one(dir.join("config.yaml"), |c| {
                kiwano_adapters::gateway_takeover::read_mcode_current(c)
            })
        }
        "openclaw" => read_additive_one(home.join(".openclaw").join("openclaw.json"), |c| {
            kiwano_adapters::gateway_takeover::read_openclaw_current(c)
        }),
        "hermes" => {
            // A variable that names no one place is not a license to read the
            // default: that file is not what hermes would read either, and
            // reporting its provider would be a claim about a configuration
            // that is not in use. Nothing extractable is the honest answer.
            let dir = match kiwano_adapters::config::env_dir("HERMES_HOME") {
                kiwano_adapters::config::EnvDir::Absolute(dir) => dir,
                kiwano_adapters::config::EnvDir::Unset => home.join(".hermes"),
                kiwano_adapters::config::EnvDir::Relative(_) => return None,
            };
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
        "cline" => {
            let content = std::fs::read_to_string(cline_providers_path(home)?).ok()?;
            let p = kiwano_adapters::gateway_takeover::read_cline_current(&content)?;
            // Deliberately no `name`, unlike the other additive readers: what
            // precedes the entry is a provider *type* (`openai-compatible`),
            // which on a row would read as a name and is not one. Leaving it
            // out is what puts the host inference (`brand_name_for_host`) in
            // charge of naming the import.
            Some(CurrentCreds {
                base_url: p.base_url,
                api_key: p.api_key,
                name: None,
                protocol: "openai",
            })
        }
        // Aider's custom endpoint is three global fields with no provider id
        // (same stance as Cline: what names the row would be a *field*, not a
        // name, so the host inference names the import). A config without both
        // a base URL and a key is not a routed configuration.
        "aider" => {
            let content = std::fs::read_to_string(home.join(".aider.conf.yml")).ok()?;
            let p = kiwano_adapters::gateway_takeover::read_aider_current(&content)?;
            Some(CurrentCreds {
                base_url: p.base_url,
                api_key: p.api_key,
                name: None,
                protocol: "openai",
            })
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

/// cline: `~/.cline/data/settings/providers.json`, under the same three levels
/// `takeover_paths` resolves (its own `sdk/…/storage/paths.ts`): an exact file,
/// else a data directory, else a base directory whose `data/` is the data
/// directory.
///
/// The process environment is the copy a reader can see — a takeover asks the
/// login shell instead, because a GUI process does not inherit what an rc file
/// exports. Where the two disagree the takeover is the one that writes, so this
/// can only under-report, never point somewhere else. None means "not
/// determinable": a relative value is refused rather than fallen through to the
/// default, which would be a claim about a file that is not the one in use.
fn cline_providers_path(home: &Path) -> Option<std::path::PathBuf> {
    use kiwano_adapters::config::{env_dir, EnvDir};
    let data_dir = match env_dir("CLINE_PROVIDER_SETTINGS_PATH") {
        EnvDir::Absolute(path) => return Some(path),
        EnvDir::Relative(_) => return None,
        EnvDir::Unset => match env_dir("CLINE_DATA_DIR") {
            EnvDir::Absolute(dir) => dir,
            EnvDir::Relative(_) => return None,
            EnvDir::Unset => match env_dir("CLINE_DIR") {
                EnvDir::Absolute(base) => base.join("data"),
                EnvDir::Relative(_) => return None,
                EnvDir::Unset => home.join(".cline").join("data"),
            },
        },
    };
    Some(data_dir.join("settings").join("providers.json"))
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

    /// A relative `HERMES_HOME` names no one place, and the default directory is
    /// not what hermes would read either — so the honest answer is that nothing
    /// is extractable, rather than a provider read out of a config nobody is
    /// using.
    #[test]
    fn a_relative_hermes_home_extracts_nothing() {
        use crate::test_env::EnvGuard;

        let home = temp_home("hermes-relative");
        let config = home.join(".hermes").join("config.yaml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        // `custom_providers` is a *list* whose entries are matched by `name`.
        std::fs::write(
            &config,
            "model:\n  provider: custom\ncustom_providers:\n  - name: custom\n    base_url: https://api.example.com\n    api_key: sk-x\n",
        )
        .unwrap();

        // Unset: the default directory is read, and its provider comes back.
        let _unset = EnvGuard::set("HERMES_HOME", None);
        assert!(read_current_creds("hermes", &home).is_some());

        // Set to a relative path: no.
        let _relative = EnvGuard::set("HERMES_HOME", Some("relative/hermes"));
        assert!(read_current_creds("hermes", &home).is_none());
    }

    /// Cline's provider settings follow three variables, and a reader aimed at
    /// the default while the config lives elsewhere reports a provider the user
    /// is not using — the same trap the takeover's path resolution avoids.
    #[test]
    fn cline_reads_the_file_its_variables_point_at() {
        use crate::test_env::EnvGuard;

        let write = |dir: &Path| {
            let path = dir.join("settings").join("providers.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                r#"{"lastUsedProvider": "openai-compatible", "providers": {"openai-compatible":
                   {"settings": {"provider": "openai-compatible", "apiKey": "sk-theirs",
                                 "baseUrl": "https://api.deepseek.com/v1"}}}}"#,
            )
            .unwrap();
        };

        let home = temp_home("cline");
        write(&home.join(".cline").join("data"));
        let _cleared = (
            EnvGuard::set("CLINE_PROVIDER_SETTINGS_PATH", None),
            EnvGuard::set("CLINE_DATA_DIR", None),
            EnvGuard::set("CLINE_DIR", None),
        );

        let creds = read_current_creds("cline", &home).expect("the default file");
        // Cline's `baseUrl` carries the version root; a stored provider does not,
        // because the gateway appends the protocol path itself. Importing it
        // verbatim is what put `/v1/v1/chat/completions` on the wire.
        assert_eq!(creds.base_url, "https://api.deepseek.com");
        assert_eq!(creds.api_key, "sk-theirs");
        assert_eq!(creds.protocol, "openai");
        // The slot is named after a provider *type*, which is not a name —
        // leaving it out is what puts the host inference in charge instead.
        assert_eq!(creds.name, None);

        // A moved data directory is where the file now is.
        let moved = temp_home("cline-moved");
        write(&moved);
        {
            let _data = EnvGuard::set("CLINE_DATA_DIR", Some(moved.to_str().unwrap()));
            assert!(read_current_creds("cline", &home).is_some());
        }
        // …and a relative one names no place a reader can be sure of.
        let _relative = EnvGuard::set("CLINE_DATA_DIR", Some("relative/cline"));
        assert!(read_current_creds("cline", &home).is_none());
    }

    /// What a takeover leaves in the file is the gateway's own loopback
    /// address, and importing that as an upstream would point the agent at
    /// itself. This matters more here than for the other additive agents: the
    /// takeover replaces the very entry the selector names, so there is no
    /// second entry to hide behind.
    #[test]
    fn a_taken_over_cline_config_is_not_imported() {
        let home = temp_home("cline-loopback");
        let path = home
            .join(".cline")
            .join("data")
            .join("settings")
            .join("providers.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"lastUsedProvider": "openai-compatible", "providers": {"openai-compatible":
               {"settings": {"provider": "openai-compatible", "apiKey": "kw-ag-cline-abcd",
                             "baseUrl": "http://127.0.0.1:8317/v1"}}}}"#,
        )
        .unwrap();

        let _cleared = (
            crate::test_env::EnvGuard::set("CLINE_PROVIDER_SETTINGS_PATH", None),
            crate::test_env::EnvGuard::set("CLINE_DATA_DIR", None),
            crate::test_env::EnvGuard::set("CLINE_DIR", None),
        );
        assert!(read_current_creds("cline", &home).is_none());
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
    fn mimo_creds_read_the_custom_provider_the_selector_names() {
        let home = temp_home("mimo");
        let dir = home.join(".config").join("mimocode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mimocode.jsonc"),
            r#"{"model":"custom/deepseek-chat","provider":{"custom":{"options":{"baseURL":"https://example.com/v1","apiKey":"sk-old"}}}}"#,
        )
        .unwrap();
        let creds = read_current_creds("mimo", &home).unwrap();
        assert_eq!(creds.name.as_deref(), Some("custom"));
        assert_eq!(creds.base_url, "https://example.com/v1");
        assert_eq!(creds.api_key, "sk-old");

        // Post-takeover config never re-imports our own loopback endpoint.
        std::fs::write(
            dir.join("mimocode.jsonc"),
            r#"{"model":"custom/deepseek-chat","provider":{"custom":{"options":{"baseURL":"http://127.0.0.1:8317/v1","apiKey":"kw"}}}}"#,
        )
        .unwrap();
        assert!(read_current_creds("mimo", &home).is_none());
    }

    #[test]
    fn mcode_creds_read_the_selector_named_provider() {
        let home = temp_home("mcode");
        let dir = home.join(".minimax");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.yaml"),
            r#"defaultModel: custom_provider:openrouter/deepseek-chat
custom_provider:
  openrouter:
    name: OpenRouter
    baseUrl: https://openrouter.ai/api/v1
    apiKey: sk-or
"#,
        )
        .unwrap();
        let creds = read_current_creds("mcode", &home).unwrap();
        assert_eq!(creds.name.as_deref(), Some("openrouter"));
        assert_eq!(creds.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(creds.api_key, "sk-or");

        // Post-takeover config never re-imports our own loopback endpoint.
        std::fs::write(
            dir.join("config.yaml"),
            r#"defaultModel: custom_provider:kiwano-gateway/deepseek-chat
custom_provider:
  kiwano-gateway:
    name: Kiwano Gateway
    baseUrl: http://127.0.0.1:8317/v1
    apiKey: kw
"#,
        )
        .unwrap();
        assert!(read_current_creds("mcode", &home).is_none());
    }

    /// Aider's three global fields are the import: both a base URL and a key
    /// must be present, and — as for Cline — no `name` is offered, because
    /// aider has no provider id for the row to carry.
    #[test]
    fn aider_creds_read_the_custom_endpoint_when_both_fields_exist() {
        let home = temp_home("aider");
        std::fs::write(
            home.join(".aider.conf.yml"),
            "model: openai/deepseek-chat\nopenai-api-base: https://api.deepseek.com/v1\nopenai-api-key: sk-ds\n",
        )
        .unwrap();
        let creds = read_current_creds("aider", &home).expect("a routed configuration");
        assert_eq!(creds.base_url, "https://api.deepseek.com/v1");
        assert_eq!(creds.api_key, "sk-ds");
        assert_eq!(creds.protocol, "openai");
        assert_eq!(creds.name, None, "no provider id: host inference names it");

        // A config without a key is not a routed configuration.
        std::fs::write(
            home.join(".aider.conf.yml"),
            "openai-api-base: https://api.deepseek.com/v1\n",
        )
        .unwrap();
        assert!(read_current_creds("aider", &home).is_none());
    }

    #[test]
    fn unknown_or_desktop_agents_yield_none() {
        let home = temp_home("none");
        assert!(read_current_creds("claude-desktop", &home).is_none());
        assert!(read_current_creds("nonexistent", &home).is_none());
    }
}
