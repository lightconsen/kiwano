//! Takeover itself: read the live files, compute every rewrite in memory (a
//! refusal costs nothing), back up, then write — putting the writes back if one
//! of them fails.

use crate::detect::ShellVars;
use crate::takeover::backup::{copy_files, BackupFile, Files};
use crate::takeover::paths::takeover_paths;
use crate::takeover::rewrite::{codex_rewrites, rewrite};
use crate::takeover::state::live_placeholder_key;
use crate::vm::Aux;
use kiwano_adapters::config::atomic_write_private;
use std::path::{Path, PathBuf};

/// The local gateway's origin. The port comes from the caller (the sidecar's
/// data port), and the per-agent path suffix from [`gateway_target`].
const GATEWAY_HOST: &str = "http://127.0.0.1";

/// Takeover: read the original files → perform all rewrites in memory
/// (can fail as a whole, zero side effects) → back up → atomic write.
///
/// The rewrites come first so a refusal (the Codex gates live in the rewrite)
/// costs nothing; if a *write* then fails part-way, every file this call
/// replaced is put back and the backup row it created is dropped, so neither a
/// half-rewritten config nor a backup row can describe a takeover that did not
/// happen.
pub fn enable(
    aux: &Aux,
    agent: &str,
    placeholder_key: &str,
    data_port: u16,
    home: &Path,
    vars: &ShellVars,
) -> Result<(), String> {
    // Read: the files as they are now, and whether each was there at all.
    let paths = takeover_paths(agent, home, vars)?;
    let originals = read_originals(agent, &paths)?;

    // Rewrite: every new content computed in memory, so a refusal costs nothing.
    let rewritten = compute_rewrites(agent, &originals, placeholder_key, data_port)?;

    // Back up unless what is on disk is already our route: a repeated enable
    // must not record a loopback config as the user's original (escape-hatch
    // semantics). The backup *row* is not the question it used to be — it can
    // be a leftover from a takeover whose rewrite is gone (the agent's config
    // was put back by hand or by another tool), and skipping the backup then
    // would leave restore aimed at a config this takeover is not replacing.
    let first_time = live_placeholder_key(agent, home, vars).is_none();
    if first_time {
        aux.save_takeover_backup(agent, &originals)
            .map_err(|e| e.to_string())?;
        copy_files(agent, home, &originals);
    }

    // Write, and put everything back if a write fails.
    write_rewrites(aux, agent, &rewritten, &originals, first_time)
}

/// The first step of [`enable`]: read every file a takeover will touch, with
/// what each held before. Nothing is created here — a takeover writes only
/// after every rewrite has been computed.
fn read_originals(agent: &str, paths: &[PathBuf]) -> Result<Vec<BackupFile>, String> {
    let mut originals: Vec<BackupFile> = Vec::with_capacity(paths.len());
    for p in paths {
        // codex's auth.json, every additive agent's config, and
        // all claude-desktop files are allowed to be missing (treated as empty
        // files — every claude-desktop write normalizes a missing/non-object
        // document to {}). `existed` records which of the two it was, so
        // disabling can put the directory back the way it found it.
        let (content, existed) = match std::fs::read_to_string(p) {
            Ok(c) => (c, true),
            Err(_)
                if matches!(
                    agent,
                    "opencode"
                        | "openclaw"
                        | "hermes"
                        | "pi"
                        | "claude-desktop"
                        | "workbuddy"
                        | "codebuddy"
                        | "kimi"
                        | "qwen"
                        | "mimo"
                        | "mcode"
                        | "aider"
                        | "continue"
                        | "crush"
                        | "droid"
                        | "goose"
                        | "zcode"
                ) || p.ends_with("auth.json")
                    || p.ends_with(".env") =>
            {
                (String::new(), false)
            }
            Err(_) => {
                return Err(format!(
                    "{} not found — run {} at least once before takeover",
                    p.display(),
                    agent
                ))
            }
        };
        originals.push(BackupFile {
            path: p.to_string_lossy().into_owned(),
            content,
            existed,
        });
    }
    Ok(originals)
}

/// The last step of [`enable`]: write every rewrite, and on the first failure
/// put the files this call already replaced back and drop the backup row it
/// created — so a takeover that did not land leaves nothing behind that says it
/// did.
fn write_rewrites(
    aux: &Aux,
    agent: &str,
    rewritten: &Files,
    originals: &[BackupFile],
    first_time: bool,
) -> Result<(), String> {
    let mut written: Vec<usize> = Vec::with_capacity(rewritten.len());
    for (index, (path, content)) in rewritten.iter().enumerate() {
        // The config's directory may not exist at all (a first takeover), so
        // create it first
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = atomic_write_private(Path::new(path), content.as_bytes()) {
            rollback_writes(rewritten, &written, originals);
            if first_time {
                let _ = aux.delete_takeover_backup(agent);
            }
            return Err(format!(
                "{}: {e} — the takeover was not applied",
                rewritten[index].0
            ));
        }
        written.push(index);
    }
    Ok(())
}

/// The new contents of every file a takeover touches, computed entirely in
/// memory. Codex goes through the ported gate module (which may refuse the
/// config before anything on disk has changed); every other agent keeps its
/// line-level rewriter, now handed the full target URL rather than a host+port
/// pair so the same code can later point an agent at an upstream provider.
fn compute_rewrites(
    agent: &str,
    originals: &[BackupFile],
    placeholder_key: &str,
    data_port: u16,
) -> Result<Files, String> {
    if agent == "codex" {
        return codex_rewrites(originals, placeholder_key, data_port);
    }
    let target = gateway_target(agent, data_port);
    // One timestamp for the whole run: Cline's provider settings carry an
    // `updatedAt` the file's schema requires, and two files of one takeover
    // disagreeing about when it happened would be a detail with no meaning.
    let now = crate::vm::rfc3339(crate::vm::unix_now());
    // openclaw's catalogue declares the model the main config selects, so it
    // is rewritten against the main config's *original* content — the right
    // side to read, because the main config's own rewrite keeps the model id
    // and only swaps the provider prefix. Goose's provider JSON needs the same
    // ride-along: the id it declares comes from the selection file's original
    // content. `None` for every other agent.
    let openclaw_config = originals
        .iter()
        .find(|f| f.path.ends_with("openclaw.json"))
        .map(|f| f.content.as_str());
    let goose_selection = if agent == "goose" {
        originals
            .iter()
            .find(|f| f.path.ends_with("config.yaml"))
            .map(|f| f.content.as_str())
    } else {
        None
    };
    originals
        .iter()
        .map(|original| {
            Ok((
                original.path.clone(),
                rewrite(
                    agent,
                    &original.path,
                    &original.content,
                    &target,
                    placeholder_key,
                    &now,
                    openclaw_config.or(goose_selection),
                )?,
            ))
        })
        .collect()
}

/// The URL a takeover writes for `agent`: the gateway origin plus the path
/// suffix that agent's base_url carries. What the suffix is depends entirely
/// on what the agent's own client appends: the Anthropic-shaped ones take the
/// origin bare (their clients add `/v1/messages`), OpenAI-SDK-shaped ones want
/// the version root (their clients add only `/chat/completions`), and agents
/// that append nothing at all need the full route in the URL.
pub(crate) fn gateway_target(agent: &str, data_port: u16) -> String {
    let suffix = match agent {
        "codex" | "grokbuild" | "opencode" | "pi" => "/v1",
        // Kimi and Qwen want the version root; their clients append the route.
        "kimi" | "qwen" => "/v1",
        // MiMo Code passes its custom baseURL straight to an OpenAI SDK, which
        // appends only /chat/completions — the version root is required.
        "mimo" => "/v1",
        // Aider routes through LiteLLM: the `openai/` model prefix forces the
        // OpenAI-compatible provider, which appends /chat/completions to the
        // base — so the version root is what `openai-api-base` holds.
        "aider" => "/v1",
        // Continue's `provider: openai` model entry is handed to the OpenAI
        // SDK, which appends only /chat/completions to `apiBase`.
        "continue" => "/v1",
        // Crush's openai-compat providers carry the version root, the same
        // shape its own docs' DeepSeek example uses.
        "crush" => "/v1",
        // Droid's `generic-chat-completion-api` provider is the OpenAI Chat
        // Completions client — the version root is what `baseUrl` holds.
        "droid" => "/v1",
        // Goose's engine "openai" normalizes any base through its own
        // parse_openai_base_url, where a bare `/v1` path is the default
        // base_path — so the version root is what `base_url` holds.
        "goose" => "/v1",
        // ZCode's openai-chat-completions api hands the base to an OpenAI
        // Chat Completions client, which appends the route itself.
        "zcode" => "/v1",
        // MiniMax Code's official endpoint ends at the version root
        // (api.minimax.io/v1) and its custom-provider baseUrl is handed to the
        // same OpenAI-completions client — the root is required here too.
        "mcode" => "/v1",
        // Cline's `openai-compatible` provider is handed to the OpenAI client
        // as-is — `/v1` included, trailing slashes trimmed, nothing appended —
        // so the version root is what its `baseUrl` holds.
        "cline" => "/v1",
        // OpenClaw and Hermes pass their configured base URL straight to an
        // OpenAI SDK (JS and Python respectively), which appends only
        // `/chat/completions` — without the version segment here, every
        // request lands on an unrouted path and the data plane 404s it.
        "openclaw" | "hermes" => "/v1",
        // WorkBuddy and CodeBuddy take a full endpoint per model entry — they
        // append nothing, so the route has to be in the URL we write.
        "workbuddy" | "codebuddy" => "/v1/chat/completions",
        _ => "",
    };
    format!("{GATEWAY_HOST}:{data_port}{suffix}")
}

/// Put the files this call already replaced back the way they were. Best
/// effort by nature — it runs on the failure path — but it is what keeps a
/// partial write from leaving the reader a config that half-exists.
fn rollback_writes(rewritten: &Files, written: &[usize], originals: &[BackupFile]) {
    for index in written.iter().rev() {
        let Some((path, _)) = rewritten.get(*index) else {
            continue;
        };
        let Some(original) = originals.iter().find(|o| &o.path == path) else {
            continue;
        };
        let path = Path::new(path);
        if original.existed {
            let _ = atomic_write_private(path, original.content.as_bytes());
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::disable;
    use crate::takeover::state::{ProviderRoute, RestoreOutcome};
    use crate::takeover::test_support::{
        no_vars, restore, temp_home, write_codex_config, CODEX_ORIGINAL,
    };
    use serde_json::Value;

    #[test]
    fn claude_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#,
        )
        .unwrap();

        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home, &no_vars()).unwrap();
        let rewritten = std::fs::read_to_string(&settings).unwrap();
        let v: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(v["model"], "opus"); // other fields preserved
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "kw-ag-claude-abcd");

        // restore = write back byte for byte
        restore(&aux, "claude", &home).unwrap();
        let restored = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(
            restored,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#
        );
        assert!(aux.load_takeover_backup("claude").is_none());
    }

    #[test]
    fn repeated_enable_keeps_first_backup() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, r#"{"original":true}"#).unwrap();

        enable(&aux, "claude", "kw-ag-claude-1111", 8317, &home, &no_vars()).unwrap();
        enable(&aux, "claude", "kw-ag-claude-2222", 8317, &home, &no_vars()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "kw-ag-claude-2222");

        restore(&aux, "claude", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            r#"{"original":true}"#
        );
    }

    #[test]
    fn codex_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, r#"{"OPENAI_API_KEY":"sk-old"}"#);

        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home, &no_vars()).unwrap();
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("wire_api = \"responses\"")); // other lines untouched
        assert!(toml.contains("name = \"DeepSeek\"")); // the table keeps its name
        assert!(toml.contains("experimental_bearer_token = \"kw-ag-codex-abcd\""));
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "kw-ag-codex-abcd");

        let report = restore(&aux, "codex", &home).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::RestoredFromBackup);
        assert!(report.warning.is_none());
        // Byte for byte, including the `model_provider` selection.
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
            CODEX_ORIGINAL
        );
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("auth.json")).unwrap(),
            r#"{"OPENAI_API_KEY":"sk-old"}"#
        );
        assert!(aux.load_takeover_backup("codex").is_none());

        // Restoring again finds nothing of ours and says so instead of failing.
        let again = restore(&aux, "codex", &home).unwrap();
        assert_eq!(again.outcome, RestoreOutcome::NotTakenOver);
    }

    #[test]
    fn codex_without_base_url_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, "model = \"m\"\n", "{}");
        let err = enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home, &no_vars()).unwrap_err();
        assert!(err.contains("custom"), "{err}");
        // The switch did not happen: no backup row, and the live config is the
        // one the user wrote (the gates run before any write).
        assert!(aux.load_takeover_backup("codex").is_none());
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
            "model = \"m\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("auth.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn codex_write_failure_rolls_the_other_file_back() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, "{}");
        // Make the second file unwritable by handing the writer a directory
        // where the file belongs: config.toml is written first and must be put
        // back when auth.json cannot be.
        std::fs::remove_file(codex_dir.join("auth.json")).unwrap();
        std::fs::create_dir(codex_dir.join("auth.json")).unwrap();

        let err = enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home, &no_vars()).unwrap_err();
        assert!(err.contains("the takeover was not applied"), "{err}");
        assert!(aux.load_takeover_backup("codex").is_none());
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
            CODEX_ORIGINAL,
            "a half-applied takeover must leave no rewritten config behind"
        );
    }

    #[test]
    fn enabling_over_a_config_that_is_already_ours_captures_no_backup() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        // A config that is *already* taken over (a hand-restored ~/.codex, or a
        // takeover whose backup row vanished): taking it over again must not
        // capture the loopback route as the "original".
        let codex_dir = write_codex_config(
            &home,
            "model = \"m\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nname = \"DeepSeek\"\nbase_url = \"http://127.0.0.1:8317/v1\"\nwire_api = \"responses\"\nexperimental_bearer_token = \"kw-ag-codex-old\"\n",
            r#"{"OPENAI_API_KEY":"kw-ag-codex-old"}"#,
        );
        enable(&aux, "codex", "kw-ag-codex-new", 8317, &home, &no_vars()).unwrap();
        assert!(
            aux.load_takeover_backup("codex").is_none(),
            "what is on disk is our own route, so there is nothing to capture"
        );
        assert_eq!(
            live_placeholder_key("codex", &home, &no_vars()).as_deref(),
            Some("kw-ag-codex-new")
        );

        let route = ProviderRoute {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "codex", &home, Some(&route), &no_vars()).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::RebuiltFromProvider);
        assert!(report.warning.is_none(), "{:?}", report.warning);
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(toml.contains("https://api.deepseek.com/v1"), "{toml}");
        assert!(!toml.contains("127.0.0.1"), "{toml}");
    }

    #[test]
    fn missing_claude_config_errors() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        assert!(enable(&aux, "claude", "k", 8317, &home, &no_vars()).is_err());
        // disable before any takeover succeeds idempotently, and reports that
        // there was nothing of ours to undo rather than a restore it did not do
        let report = restore(&aux, "claude", &home).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::NotTakenOver);
        assert!(report.warning.is_none());
    }

    #[test]
    fn gemini_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let gemini_dir = home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join(".env"),
            "GOOGLE_GENAI_USE_VERTEXAI=false\nGEMINI_API_KEY=AIzaSy-old\n# proxy comment\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\n",
        )
        .unwrap();

        enable(&aux, "gemini", "kw-ag-gemini-abcd", 8317, &home, &no_vars()).unwrap();
        let env = std::fs::read_to_string(gemini_dir.join(".env")).unwrap();
        assert!(env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8317\n"));
        assert!(env.contains("GEMINI_API_KEY=kw-ag-gemini-abcd\n"));
        assert!(env.contains("GOOGLE_GENAI_USE_VERTEXAI=false")); // other lines untouched
        assert!(env.contains("# proxy comment")); // comment preserved

        // restore = write back byte for byte
        restore(&aux, "gemini", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(gemini_dir.join(".env")).unwrap(),
            "GOOGLE_GENAI_USE_VERTEXAI=false\nGEMINI_API_KEY=AIzaSy-old\n# proxy comment\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\n"
        );
        assert!(aux.load_takeover_backup("gemini").is_none());
    }

    #[test]
    fn gemini_takeover_creates_missing_env() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let env_path = home.join(".gemini").join(".env");
        // takeover works even when ~/.gemini/.env is missing entirely (dir + file are created automatically)
        enable(&aux, "gemini", "kw-ag-gemini-abcd", 8317, &home, &no_vars()).unwrap();
        let env = std::fs::read_to_string(&env_path).unwrap();
        assert_eq!(
            env,
            "GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8317\nGEMINI_API_KEY=kw-ag-gemini-abcd\n"
        );

        // Disabling puts the directory back the way it found it: the file we
        // created is removed, not left behind as an empty config (which an
        // agent may read as a broken one).
        restore(&aux, "gemini", &home).unwrap();
        assert!(
            !env_path.exists(),
            "a file created by the takeover must not survive disable"
        );
        assert!(aux.load_takeover_backup("gemini").is_none());
    }

    #[test]
    fn grok_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        std::fs::write(
            grok_dir.join("config.toml"),
            r#"theme = "dark"
[models]
default = "custom-grok"

[model."custom-grok"]
name = "My Grok"
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
api_key = "xai-old"
api_backend = "chat"
context_window = 131072

[model."other"]
name = "Other"
model = "grok-3"
base_url = "https://relay.example.com/v1"
"#,
        )
        .unwrap();

        enable(
            &aux,
            "grokbuild",
            "kw-ag-grokbuild-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        let toml = std::fs::read_to_string(grok_dir.join("config.toml")).unwrap();
        // selected model points at the gateway; backend pinned to responses
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("api_key = \"kw-ag-grokbuild-abcd\""));
        assert!(toml.contains("api_backend = \"responses\""));
        // rows outside the selected model table stay untouched
        assert!(toml.contains("https://relay.example.com/v1"));
        assert!(toml.contains("theme = \"dark\""));

        restore(&aux, "grokbuild", &home).unwrap();
        // restore = write back byte for byte
        assert_eq!(
            std::fs::read_to_string(grok_dir.join("config.toml")).unwrap(),
            r#"theme = "dark"
[models]
default = "custom-grok"

[model."custom-grok"]
name = "My Grok"
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
api_key = "xai-old"
api_backend = "chat"
context_window = 131072

[model."other"]
name = "Other"
model = "grok-3"
base_url = "https://relay.example.com/v1"
"#
        );
        assert!(aux.load_takeover_backup("grokbuild").is_none());
    }

    #[test]
    fn grok_missing_api_key_and_backend_are_inserted() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        std::fs::write(
            grok_dir.join("config.toml"),
            "[models]\ndefault = \"p\"\n\n[model.p]\nname = \"P\"\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\nenv_key = \"XAI_API_KEY\"\ncontext_window = 131072\n",
        )
        .unwrap();

        enable(
            &aux,
            "grokbuild",
            "kw-ag-grokbuild-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        let toml = std::fs::read_to_string(grok_dir.join("config.toml")).unwrap();
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("api_key = \"kw-ag-grokbuild-abcd\""));
        assert!(toml.contains("api_backend = \"responses\""));
        assert!(toml.contains("env_key = \"XAI_API_KEY\"")); // untouched row preserved
    }

    #[test]
    fn grok_official_oauth_config_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        // official xAI login: no [models]/[model.*] tables at all
        std::fs::write(grok_dir.join("config.toml"), "theme = \"dark\"\n").unwrap();
        assert!(enable(
            &aux,
            "grokbuild",
            "kw-ag-grokbuild-abcd",
            8317,
            &home,
            &no_vars()
        )
        .is_err());
        // no backup left behind on failure (escape hatch stays clean)
        assert!(aux.load_takeover_backup("grokbuild").is_none());
    }

    #[test]
    fn opencode_takeover_roundtrip_additive() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".config").join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"{
  "theme": "dark",
  "model": "deepseek/deepseek-chat",
  "provider": { "deepseek": { "npm": "@ai-sdk/openai", "options": { "apiKey": "sk-old" } } }
}"#;
        std::fs::write(dir.join("opencode.json"), original).unwrap();

        enable(
            &aux,
            "opencode",
            "kw-ag-opencode-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(v["model"], "kiwano-gateway/deepseek-chat");
        assert_eq!(
            v["provider"]["kiwano-gateway"]["options"]["baseURL"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(
            v["provider"]["kiwano-gateway"]["options"]["apiKey"],
            "kw-ag-opencode-abcd"
        );
        assert!(v["provider"]["deepseek"].is_object()); // additive: entry survives

        restore(&aux, "opencode", &home).unwrap();
        // restore = write back byte for byte
        assert_eq!(
            std::fs::read_to_string(dir.join("opencode.json")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("opencode").is_none());
    }

    // ── workbuddy / codebuddy / kimi / qwen ──
    //
    // Their transforms are unit-tested in adapters; these check the pipeline:
    // the right file is written, the user's own entries survive, and disable
    // puts the bytes back.

    #[test]
    fn workbuddy_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".workbuddy");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"[
  { "id": "deepseek-v4-pro", "vendor": "DeepSeek",
    "url": "https://api.deepseek.com/chat/completions", "apiKey": "sk-old" }
]"#;
        std::fs::write(dir.join("models.json"), original).unwrap();

        enable(
            &aux,
            "workbuddy",
            "kw-ag-workbuddy-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("models.json")).unwrap())
                .unwrap();
        assert_eq!(
            v[0]["url"], "http://127.0.0.1:8317/v1/chat/completions",
            "the entry points at the gateway's full endpoint"
        );
        assert_eq!(v[0]["apiKey"], "kw-ag-workbuddy-abcd");
        assert_eq!(
            v[0]["id"], "deepseek-v4-pro",
            "the model id survives — it is what goes upstream"
        );

        restore(&aux, "workbuddy", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("models.json")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("workbuddy").is_none());
    }

    #[test]
    fn codebuddy_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".codebuddy");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"{
  "models": [
    { "id": "deepseek-v3", "vendor": "DeepSeek",
      "url": "https://api.deepseek.com/v1/chat/completions", "apiKey": "sk-old" }
  ],
  "availableModels": []
}"#;
        std::fs::write(dir.join("models.json"), original).unwrap();

        enable(
            &aux,
            "codebuddy",
            "kw-ag-codebuddy-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("models.json")).unwrap())
                .unwrap();
        assert_eq!(
            v["models"][0]["url"],
            "http://127.0.0.1:8317/v1/chat/completions"
        );
        assert_eq!(v["models"][0]["apiKey"], "kw-ag-codebuddy-abcd");
        assert_eq!(v["availableModels"][0], "deepseek-v3");

        restore(&aux, "codebuddy", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("models.json")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("codebuddy").is_none());
    }

    #[test]
    fn kimi_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".kimi");
        std::fs::create_dir_all(&dir).unwrap();
        let original = "default_model = \"kimi-code/kimi-for-coding\"\n\n\
                        [providers.\"managed:kimi-code\"]\n\
                        type = \"kimi\"\n\
                        base_url = \"https://api.kimi.com/coding/v1\"\n\
                        api_key = \"sk-old\"\n";
        std::fs::write(dir.join("config.toml"), original).unwrap();

        enable(&aux, "kimi", "kw-ag-kimi-abcd", 8317, &home, &no_vars()).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(
            text.contains("base_url = \"http://127.0.0.1:8317/v1\""),
            "{text}"
        );
        assert!(text.contains("api_key = \"kw-ag-kimi-abcd\""), "{text}");
        assert!(
            text.contains("default_model = \"kiwano-gateway/kimi-for-coding\""),
            "{text}"
        );
        // Our entry — not the fixture's managed provider, which is also a
        // `kimi` but a `[providers."…"]` section — carries the type whose
        // dispatcher sends the conversation's prompt_cache_key. Ours is
        // written as an inline table under `[providers]`.
        assert!(
            text.contains("kiwano-gateway = { type = \"kimi\""),
            "the Python generation's protocol type: {text}"
        );
        // The user's own provider is untouched.
        assert!(text.contains("api_key = \"sk-old\""), "{text}");

        restore(&aux, "kimi", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config.toml")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("kimi").is_none());
    }

    /// The catalogue entry that carries openclaw's session-affinity flag has
    /// to name the model the main config selects — the two files are written
    /// in one takeover and have to agree on the id.
    #[test]
    fn openclaw_takeover_declares_the_selected_model_for_session_affinity() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".openclaw");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"{
  "models": { "providers": { "openrouter": { "baseUrl": "https://openrouter.ai/api/v1", "apiKey": "sk-or" } } },
  "agents": { "defaults": { "model": { "primary": "openrouter/deepseek-chat" } } }
}"#;
        std::fs::write(dir.join("openclaw.json"), original).unwrap();

        enable(
            &aux,
            "openclaw",
            "kw-ag-openclaw-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();

        // The main config repoints the selector, keeping the model id, and
        // its provider carries the version root — OpenClaw's OpenAI JS client
        // appends only `/chat/completions` to whatever baseUrl it is given.
        let main_text = std::fs::read_to_string(dir.join("openclaw.json")).unwrap();
        assert!(
            main_text.contains("kiwano-gateway/deepseek-chat"),
            "{main_text}"
        );
        let main: serde_json::Value = serde_json::from_str(&main_text).unwrap();
        assert_eq!(
            main["models"]["providers"]["kiwano-gateway"]["baseUrl"],
            "http://127.0.0.1:8317/v1"
        );

        // The catalogue declares that same id with the affinity flag — at the
        // runtime location for the default agent (no `agents.list` here).
        let catalog = dir
            .join("agents")
            .join("main")
            .join("agent")
            .join("models.json");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&catalog).unwrap()).unwrap();
        let entry = &v["providers"]["kiwano-gateway"];
        assert_eq!(entry["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["apiKey"], "kw-ag-openclaw-abcd");
        assert_eq!(entry["models"][0]["id"], "deepseek-chat");
        assert_eq!(
            entry["models"][0]["compat"]["sendSessionAffinityHeaders"],
            true
        );

        restore(&aux, "openclaw", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("openclaw.json")).unwrap(),
            original
        );
        assert!(
            !catalog.exists(),
            "a catalogue the takeover created must not survive disable"
        );
        assert!(aux.load_takeover_backup("openclaw").is_none());
    }

    #[test]
    fn qwen_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".qwen");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"{
  "model": { "name": "qwen3-coder-plus" },
  "security": { "auth": { "selectedType": "qwen-oauth" } }
}"#;
        std::fs::write(dir.join("settings.json"), original).unwrap();

        enable(&aux, "qwen", "kw-ag-qwen-abcd", 8317, &home, &no_vars()).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            v["modelProviders"]["openai"][0]["baseUrl"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(v["env"]["KIWANO_GATEWAY_KEY"], "kw-ag-qwen-abcd");
        assert_eq!(v["security"]["auth"]["selectedType"], "openai");
        assert_eq!(v["model"]["name"], "qwen3-coder-plus");

        restore(&aux, "qwen", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("qwen").is_none());
    }

    #[test]
    fn mimo_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".config").join("mimocode");
        std::fs::create_dir_all(&dir).unwrap();
        // The config MiMo's own docs shape for a custom provider.
        let original = r#"{
  // user comment survives the parse, not the write-back
  "model": "custom/deepseek-chat",
  "provider": {
    "custom": {
      "name": "Custom",
      "npm": "@ai-sdk/openai-compatible",
      "only_configured_models": true,
      "models": { "deepseek-chat": { "name": "deepseek-chat" } },
      "options": { "baseURL": "https://example.com/v1", "apiKey": "sk-old" }
    }
  }
}"#;
        std::fs::write(dir.join("mimocode.jsonc"), original).unwrap();

        enable(&aux, "mimo", "kw-ag-mimo-abcd", 8317, &home, &no_vars()).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("mimocode.jsonc")).unwrap())
                .unwrap();
        assert_eq!(v["model"], "custom/deepseek-chat");
        let entry = &v["provider"]["custom"];
        assert_eq!(entry["options"]["baseURL"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["options"]["apiKey"], "kw-ag-mimo-abcd");

        restore(&aux, "mimo", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("mimocode.jsonc")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("mimo").is_none());
    }

    #[test]
    fn mcode_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".minimax");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"defaultModel: minimax/minimax-m2.5
custom_provider:
  openrouter:
    name: OpenRouter
    baseUrl: https://openrouter.ai/api/v1
    apiKey: sk-or
    apiFormat: openai-completions
    models:
      - modelId: deepseek-chat
"#;
        std::fs::write(dir.join("config.yaml"), original).unwrap();

        enable(&aux, "mcode", "kw-ag-mcode-abcd", 8317, &home, &no_vars()).unwrap();
        let v: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yaml")).unwrap())
                .unwrap();
        assert_eq!(
            v["defaultModel"],
            "custom_provider:kiwano-gateway/minimax-m2.5"
        );
        let gw = &v["custom_provider"]["kiwano-gateway"];
        assert_eq!(gw["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(gw["apiKey"], "kw-ag-mcode-abcd");
        assert_eq!(gw["apiFormat"], "openai-completions");

        restore(&aux, "mcode", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config.yaml")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("mcode").is_none());
    }

    /// Aider is exclusive like claude — its custom endpoint *is* three global
    /// fields — so the takeover overwrites them and restore puts the user's
    /// bytes back the way they were, comments included.
    #[test]
    fn aider_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let original = "# aider's own settings, written by hand\nmodel: anthropic/claude-sonnet-4-6\ndark-mode: true\nauto-commits: false\n";
        let config = home.join(".aider.conf.yml");
        std::fs::write(&config, original).unwrap();

        enable(&aux, "aider", "kw-ag-aider-abcd", 8317, &home, &no_vars()).unwrap();
        let v: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["openai-api-base"], "http://127.0.0.1:8317/v1");
        assert_eq!(v["openai-api-key"], "kw-ag-aider-abcd");
        assert_eq!(
            v["model"], "openai/claude-sonnet-4-6",
            "the model id is kept, reprefixed through LiteLLM's OpenAI route"
        );
        assert_eq!(v["auto-commits"], false); // unrelated settings survive

        assert_eq!(
            live_placeholder_key("aider", &home, &no_vars()).as_deref(),
            Some("kw-ag-aider-abcd")
        );

        restore(&aux, "aider", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(aux.load_takeover_backup("aider").is_none());
    }

    /// Continue's takeover unshifts the gateway model at the head of `models`
    /// (its documented default is "first chat model"), scoped to the chat role.
    #[test]
    fn continue_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let original = r#"name: my config
version: 0.0.1
schema: v1
models:
  - name: DeepSeek
    provider: openai
    model: deepseek-chat
    apiBase: https://api.deepseek.com/v1
    apiKey: sk-ds
    roles: [chat, edit]
"#;
        let config = home.join(".continue").join("config.yaml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, original).unwrap();

        enable(
            &aux,
            "continue",
            "kw-ag-continue-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        let v: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        let models = v["models"].as_sequence().unwrap();
        assert_eq!(models[0]["name"], "Kiwano Gateway");
        assert_eq!(models[0]["model"], "deepseek-chat");
        assert_eq!(models[0]["apiBase"], "http://127.0.0.1:8317/v1");
        assert_eq!(models[0]["apiKey"], "kw-ag-continue-abcd");
        assert_eq!(models[0]["roles"][0], "chat");
        assert_eq!(models[1]["name"], "DeepSeek", "the user's own entry stays");

        assert_eq!(
            live_placeholder_key("continue", &home, &no_vars()).as_deref(),
            Some("kw-ag-continue-abcd")
        );

        restore(&aux, "continue", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(aux.load_takeover_backup("continue").is_none());
    }

    /// Crush's takeover fills the `models.large` slot (what its own
    /// `model large` command persists) and upserts the provider beside the
    /// user's own ones.
    #[test]
    fn crush_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let original = r#"{
  "$schema": "https://charm.land/crush.json",
  "models": { "large": { "provider": "deepseek", "model": "deepseek-chat" } },
  "providers": {
    "deepseek": { "type": "openai-compat", "base_url": "https://api.deepseek.com/v1",
                  "api_key": "sk-ds", "models": [] }
  }
}"#;
        let config = home.join(".config").join("crush").join("crush.json");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, original).unwrap();

        enable(&aux, "crush", "kw-ag-crush-abcd", 8317, &home, &no_vars()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["models"]["large"]["provider"], "kiwano-gateway");
        assert_eq!(v["models"]["large"]["model"], "deepseek-chat");
        assert_eq!(
            v["providers"]["kiwano-gateway"]["base_url"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(
            v["providers"]["kiwano-gateway"]["api_key"],
            "kw-ag-crush-abcd"
        );
        assert!(
            v["providers"]["deepseek"].is_object(),
            "the user's provider survives"
        );

        assert_eq!(
            live_placeholder_key("crush", &home, &no_vars()).as_deref(),
            Some("kw-ag-crush-abcd")
        );

        restore(&aux, "crush", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(aux.load_takeover_backup("crush").is_none());
    }

    /// Droid's takeover prepends the gateway model to `customModels` and makes
    /// it the default `model`.
    #[test]
    fn droid_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let original = r#"{
  "model": "claude-sonnet-4-6",
  "reasoningEffort": "high"
}"#;
        let config = home.join(".factory").join("settings.json");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, original).unwrap();

        enable(&aux, "droid", "kw-ag-droid-abcd", 8317, &home, &no_vars()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["model"], "claude-sonnet-4-6");
        let models = v["customModels"].as_array().unwrap();
        assert_eq!(models[0]["displayName"], "Kiwano Gateway");
        assert_eq!(models[0]["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(models[0]["apiKey"], "kw-ag-droid-abcd");
        assert_eq!(v["reasoningEffort"], "high");

        assert_eq!(
            live_placeholder_key("droid", &home, &no_vars()).as_deref(),
            Some("kw-ag-droid-abcd")
        );

        restore(&aux, "droid", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(aux.load_takeover_backup("droid").is_none());
    }

    /// Goose's takeover writes three files: the selection, the provider
    /// definition (whose auth.command carries the key file's absolute path),
    /// and the key file itself. All three are created; disable removes what we
    /// created. A fresh machine has no selection to copy a model id from, so
    /// the entry carries the placeholder (the user replaces it).
    #[test]
    fn goose_takeover_roundtrip_writes_the_three_files() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let root = home
            .join("Library")
            .join("Application Support")
            .join("Block")
            .join("goose");

        enable(&aux, "goose", "kw-ag-goose-abcd", 8317, &home, &no_vars()).unwrap();

        // The selection points at the gateway; nothing to keep, so the
        // placeholder model id.
        let config = root.join("config.yaml");
        let v: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["GOOSE_PROVIDER"], "kiwano-gateway");
        assert_eq!(v["GOOSE_MODEL"], "kiwano");

        // The provider JSON wires auth.command to the key file's absolute path.
        let provider = root.join("custom_providers").join("kiwano-gateway.json");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&provider).unwrap()).unwrap();
        assert_eq!(v["engine"], "openai");
        assert_eq!(v["base_url"], "http://127.0.0.1:8317/v1");
        assert_eq!(v["api_key_env"], "");
        assert_eq!(
            v["auth"]["args"][0],
            root.join("kiwano-gateway.key").to_string_lossy().as_ref()
        );
        assert_eq!(v["models"][0]["name"], "kiwano");

        // The key file holds the placeholder, which is also the live evidence.
        assert_eq!(
            std::fs::read_to_string(root.join("kiwano-gateway.key"))
                .unwrap()
                .trim(),
            "kw-ag-goose-abcd"
        );
        assert_eq!(
            live_placeholder_key("goose", &home, &no_vars()).as_deref(),
            Some("kw-ag-goose-abcd")
        );

        // The originals were missing, so restore removes what the takeover
        // created rather than leaving three empty files for goose to choke on.
        restore(&aux, "goose", &home).unwrap();
        assert!(!config.exists());
        assert!(!provider.exists());
        assert!(!root.join("kiwano-gateway.key").exists());
        assert!(aux.load_takeover_backup("goose").is_none());
    }

    /// With the config already on disk, disable puts the user's bytes back
    /// (the YAML comments do not survive the takeover; the backup restores
    /// them byte for byte).
    #[test]
    fn goose_takeover_roundtrip_restores_the_users_bytes() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let root = home
            .join("Library")
            .join("Application Support")
            .join("Block")
            .join("goose");
        let original =
            "# goose's own settings\nGOOSE_PROVIDER: anthropic\nGOOSE_MODEL: claude-sonnet-4-6\n";
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.yaml"), original).unwrap();

        enable(&aux, "goose", "kw-ag-goose-abcd", 8317, &home, &no_vars()).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(
            &std::fs::read_to_string(root.join("custom_providers").join("kiwano-gateway.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            v["auth"]["args"][0],
            root.join("kiwano-gateway.key").to_string_lossy().as_ref()
        );
        // The user's own selection survives: both files name the same id.
        let sel: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(root.join("config.yaml")).unwrap())
                .unwrap();
        assert_eq!(sel["GOOSE_PROVIDER"], "kiwano-gateway");
        assert_eq!(sel["GOOSE_MODEL"], "claude-sonnet-4-6");
        assert_eq!(v["models"][0]["name"], "claude-sonnet-4-6");

        restore(&aux, "goose", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("config.yaml")).unwrap(),
            original
        );
        assert!(
            !root
                .join("custom_providers")
                .join("kiwano-gateway.json")
                .exists(),
            "a file the takeover created must not survive disable"
        );
        assert!(!root.join("kiwano-gateway.key").exists());
        assert!(aux.load_takeover_backup("goose").is_none());
    }

    /// ZCode's takeover upserts the gateway rule into the native registry and
    /// points `defaultModelSelection` at it; disable puts the user's bytes
    /// (rules, ordering and selection included) back the way they were.
    #[test]
    fn zcode_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let original = r#"{
  "schemaVersion": 1,
  "config": {
    "providerConfigRules": { "providerRules": [
      { "providerId": "deepseek", "providerName": "DeepSeek", "enabled": true,
        "config": { "access": { "type": "api-key", "apiKey": "sk-ds" },
                    "api": { "type": "openai-chat-completions", "baseUrl": "https://api.deepseek.com/v1" },
                    "personalModelIds": ["deepseek-chat"] } }
    ] },
    "defaultModelSelection": { "providerId": "deepseek", "modelId": "deepseek-chat" }
  }
}"#;
        let config = home.join(".zcode").join("v2").join("provider_config.json");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, original).unwrap();

        enable(&aux, "zcode", "kw-ag-zcode-abcd", 8317, &home, &no_vars()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        let sel = &v["config"]["defaultModelSelection"];
        assert_eq!(sel["providerId"], "kiwano-gateway");
        assert_eq!(sel["modelId"], "deepseek-chat");
        let rules = v["config"]["providerConfigRules"]["providerRules"]
            .as_array()
            .unwrap();
        assert_eq!(
            rules[1]["config"]["api"]["baseUrl"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(rules[1]["config"]["access"]["apiKey"], "kw-ag-zcode-abcd");
        assert_eq!(rules[0]["providerId"], "deepseek", "the user's rule stays");

        assert_eq!(
            live_placeholder_key("zcode", &home, &no_vars()).as_deref(),
            Some("kw-ag-zcode-abcd")
        );

        restore(&aux, "zcode", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(aux.load_takeover_backup("zcode").is_none());
    }

    /// A config that does not exist yet is created, and disabling removes it
    /// rather than leaving a zero-byte file the agent would read as broken.
    #[test]
    fn takeovers_of_the_new_agents_create_missing_configs() {
        for (agent, relative) in [
            ("workbuddy", ".workbuddy/models.json"),
            ("codebuddy", ".codebuddy/models.json"),
            // Neither generation's directory exists on a fresh machine, so the
            // successor's path is the one written (see `takeover_paths`).
            ("kimi", ".kimi-code/config.toml"),
            ("qwen", ".qwen/settings.json"),
            ("mimo", ".config/mimocode/mimocode.jsonc"),
            ("mcode", ".minimax/config.yaml"),
            ("aider", ".aider.conf.yml"),
            ("continue", ".continue/config.yaml"),
            ("crush", ".config/crush/crush.json"),
            ("droid", ".factory/settings.json"),
            // All three of goose's files, at the root its etcetera strategy
            // resolves on this platform.
            #[cfg(target_os = "macos")]
            (
                "goose",
                "Library/Application Support/Block/goose/config.yaml",
            ),
            #[cfg(target_os = "macos")]
            (
                "goose",
                "Library/Application Support/Block/goose/custom_providers/kiwano-gateway.json",
            ),
            #[cfg(target_os = "macos")]
            (
                "goose",
                "Library/Application Support/Block/goose/kiwano-gateway.key",
            ),
            #[cfg(not(target_os = "macos"))]
            ("goose", ".config/goose/config.yaml"),
            #[cfg(not(target_os = "macos"))]
            (
                "goose",
                ".config/goose/custom_providers/kiwano-gateway.json",
            ),
            #[cfg(not(target_os = "macos"))]
            ("goose", ".config/goose/kiwano-gateway.key"),
            ("zcode", ".zcode/v2/provider_config.json"),
            // Both of openclaw's files: the main config and the per-agent
            // catalogue the session-affinity declaration lives in.
            ("openclaw", ".openclaw/openclaw.json"),
            ("openclaw", ".openclaw/agents/main/agent/models.json"),
        ] {
            let (_dir, home) = temp_home();
            let aux = Aux::open_in_memory().unwrap();
            let path = home.join(relative);

            enable(
                &aux,
                agent,
                &format!("kw-ag-{agent}-abcd"),
                8317,
                &home,
                &no_vars(),
            )
            .unwrap();
            assert!(path.exists(), "{agent}: the config is created");
            assert!(
                std::fs::read_to_string(&path).unwrap().len() > 2,
                "{agent}: and it is not empty"
            );

            restore(&aux, agent, &home).unwrap();
            assert!(
                !path.exists(),
                "{agent}: a file the takeover created must not survive disable"
            );
        }
    }

    /// Cline selects a provider slot by provider id, so a takeover replaces the
    /// slot its selector names — the user's other slots stay — and disabling
    /// puts the original bytes back.
    #[test]
    fn cline_takeover_roundtrip_replaces_the_selected_slot() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let path = home
            .join(".cline")
            .join("data")
            .join("settings")
            .join("providers.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"{
  "version": 1,
  "lastUsedProvider": "openai-compatible",
  "modes": {},
  "providers": {
    "openai-compatible": {
      "settings": { "provider": "openai-compatible", "apiKey": "sk-theirs",
                    "baseUrl": "https://api.deepseek.com/v1", "model": "deepseek-v3" },
      "updatedAt": "2026-09-01T00:00:00Z",
      "tokenSource": "manual"
    }
  }
}"#;
        std::fs::write(&path, original).unwrap();

        enable(&aux, "cline", "kw-ag-cline-abcd", 8317, &home, &no_vars()).unwrap();

        let settings = &serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap())
            .unwrap()["providers"]["openai-compatible"]["settings"];
        assert_eq!(settings["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(settings["apiKey"], "kw-ag-cline-abcd");
        assert_eq!(
            settings["model"], "deepseek-v3",
            "the model id goes upstream verbatim, so the takeover keeps it"
        );
        // The live file is what tells a second enable not to record this as the
        // user's original.
        assert_eq!(
            live_placeholder_key("cline", &home, &no_vars()).as_deref(),
            Some("kw-ag-cline-abcd")
        );

        restore(&aux, "cline", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert!(aux.load_takeover_backup("cline").is_none());
    }

    /// Unlike the additive agents, a missing config is refused: there is no
    /// slot to replace, and the file is where the user's own endpoint lives.
    #[test]
    fn cline_takeover_refuses_a_missing_config() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();

        let err = enable(&aux, "cline", "kw-ag-cline-abcd", 8317, &home, &no_vars()).unwrap_err();
        assert!(
            err.contains("not found — run cline at least once before takeover"),
            "{err}"
        );

        let report = restore(&aux, "cline", &home).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::NotTakenOver);
    }

    #[test]
    fn pi_takeover_roundtrip_writes_both_files() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        // pi's files may not exist yet (agent dir is created on takeover)
        enable(&aux, "pi", "kw-ag-pi-abcd", 8317, &home, &no_vars()).unwrap();
        let agent = home.join(".pi").join("agent");
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(agent.join("models.json")).unwrap())
                .unwrap();
        assert_eq!(
            models["providers"]["kiwano-gateway"]["baseUrl"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(
            models["providers"]["kiwano-gateway"]["apiKey"],
            "kw-ag-pi-abcd"
        );
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(agent.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["defaultProvider"], "kiwano-gateway");

        restore(&aux, "pi", &home).unwrap();
        // The originals were missing, so restore removes what the takeover
        // created instead of leaving two 0-byte JSON files for the CLI to
        // choke on; the agent recreates its own state.
        assert!(!agent.join("models.json").exists());
        assert!(!agent.join("settings.json").exists());
        assert!(aux.load_takeover_backup("pi").is_none());
    }

    #[test]
    fn hermes_takeover_roundtrip_preserves_untouched_sections() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".hermes");
        std::fs::create_dir_all(&dir).unwrap();
        let original = "agent:\n  max_turns: 50\ncustom_providers:\n  - name: openrouter\n    base_url: https://openrouter.ai/api/v1\n";
        std::fs::write(dir.join("config.yaml"), original).unwrap();

        enable(&aux, "hermes", "kw-ag-hermes-abcd", 8317, &home, &no_vars()).unwrap();
        let out = std::fs::read_to_string(dir.join("config.yaml")).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"]["provider"], "kiwano-gateway");
        assert_eq!(v["agent"]["max_turns"], 50);
        let providers = v["custom_providers"].as_sequence().unwrap();
        assert_eq!(providers.len(), 2);
        // Hermes hands base_url to the OpenAI Python SDK, which appends only
        // `/chat/completions` — the version root has to be in what we write.
        let ours = providers
            .iter()
            .find(|p| p["name"].as_str() == Some("kiwano-gateway"))
            .expect("the gateway provider is upserted");
        assert_eq!(ours["base_url"].as_str(), Some("http://127.0.0.1:8317/v1"));

        restore(&aux, "hermes", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config.yaml")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("hermes").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_desktop_takeover_roundtrip_writes_profile() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let app_support = home.join("Library").join("Application Support");
        let normal_config = app_support
            .join("Claude")
            .join("claude_desktop_config.json");
        std::fs::create_dir_all(normal_config.parent().unwrap()).unwrap();
        let original = r#"{"deploymentMode":"1p","autoUpdater":true}"#;
        std::fs::write(&normal_config, original).unwrap();
        // Claude-3p side (config, profile, _meta.json) is absent: takeover
        // must create it from scratch

        enable(
            &aux,
            "claude-desktop",
            "kw-ag-claude-desktop-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();

        let normal: Value =
            serde_json::from_str(&std::fs::read_to_string(&normal_config).unwrap()).unwrap();
        assert_eq!(normal["deploymentMode"], "3p");
        assert_eq!(normal["autoUpdater"], true); // untouched key survives

        let threep: Value = serde_json::from_str(
            &std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("claude_desktop_config.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(threep["deploymentMode"], "3p");

        let profile: Value = serde_json::from_str(
            &std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("configLibrary")
                    .join("00000000-0000-4000-8000-000000157210.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(profile["inferenceProvider"], "gateway");
        assert_eq!(profile["inferenceGatewayBaseUrl"], "http://127.0.0.1:8317");
        assert_eq!(
            profile["inferenceGatewayApiKey"],
            "kw-ag-claude-desktop-abcd"
        );
        assert_eq!(profile["inferenceModels"].as_array().unwrap().len(), 4);

        let meta: Value = serde_json::from_str(
            &std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("configLibrary")
                    .join("_meta.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(meta["appliedId"], "00000000-0000-4000-8000-000000157210");

        restore(&aux, "claude-desktop", &home).unwrap();
        // Restore is byte-for-byte for files that existed and removal for files
        // the takeover created: here the whole Claude-3p side was absent.
        assert_eq!(std::fs::read_to_string(&normal_config).unwrap(), original);
        let threep = app_support.join("Claude-3p");
        assert!(!threep.join("claude_desktop_config.json").exists());
        assert!(!threep
            .join("configLibrary")
            .join("00000000-0000-4000-8000-000000157210.json")
            .exists());
        assert!(!threep.join("configLibrary").join("_meta.json").exists());
        assert!(aux.load_takeover_backup("claude-desktop").is_none());
    }
}
