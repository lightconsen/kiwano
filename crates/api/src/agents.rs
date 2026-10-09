//! User-defined agents: the wire shape of one.

use serde::{Deserialize, Serialize};

/// A user-defined agent's view: a name for a route, the key its traffic is
/// attributed by, and nothing else — there is no config file to report on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAgentVm {
    pub id: String,
    pub label: String,
    pub note: Option<String>,
    /// Always present: the key is minted with the agent and deleted with it, so
    /// unlike a built-in's (read out of its live config), this one cannot be
    /// stale — the row *is* the truth here, and it is the same row the gateway
    /// attributes by.
    pub placeholder_key: Option<String>,
    /// What this agent's clients speak, chosen when it was defined. `None` for
    /// one defined before the field existed — "not said", which is why the UI
    /// reads it as such rather than showing a protocol nobody picked.
    pub protocol: Option<String>,
}

/// One prompt round trip against a provider, for the Apps screen's Test button.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptLatencyVm {
    pub provider_id: String,
    /// The model the ping was sent with — it decides the number as much as the
    /// network does, so the UI can say which one was measured.
    pub model: String,
    pub latency_ms: u64,
    pub status: u16,
    /// The upstream's own words when it refused, so a failure reads as one.
    pub error: Option<String>,
}

/// Every built-in agent: its id and the name a person reads.
///
/// Here because both sides label rows with it — the daemon's dashboard, the
/// client's everything-else — and a table copied to two crates is a table that
/// disagrees with itself the first time an agent is added.
pub const AGENTS: [(&str, &str); 26] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("gemini", "Gemini CLI"),
    ("grokbuild", "Grok Build"),
    ("claude-desktop", "Claude Desktop"),
    ("opencode", "OpenCode"),
    ("openclaw", "OpenClaw"),
    ("hermes", "Hermes"),
    ("pi", "Pi"),
    ("omp", "oh-my-pi"),
    ("commandcode", "Command Code"),
    ("dsh", "DeepSeek Harness"),
    ("hanaagent", "HanaAgent"),
    ("workbuddy", "WorkBuddy"),
    ("codebuddy", "CodeBuddy Code"),
    ("kimi", "Kimi Code CLI"),
    ("qwen", "Qwen Code"),
    ("cline", "Cline"),
    ("mimo", "MiMo Code"),
    ("mcode", "MiniMax Code"),
    ("aider", "Aider"),
    ("continue", "Continue"),
    ("crush", "Crush"),
    ("droid", "Droid"),
    ("goose", "Goose"),
    ("zcode", "ZCode"),
];
