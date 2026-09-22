// The settings the mock starts from — the takeovers, the user-defined agents
// and the feature flags. Mirrors `vm::build_settings`.

import type { AppSettings } from "../types";

export const settings: AppSettings = {
  language: "system",
  theme: "dark",
  autostart: true,
  close_to_tray: true,
  gateway_listen: "127.0.0.1:8317",
  // `config_paths` mirrors `takeover::takeover_paths` for the host platform, so
  // the mock answers what the real backend would for each agent's files.
  takeovers: [
    { agent: "claude", label: "Claude Code", placeholder_key: "kw-ag-claude-a1b2", enabled: true, additive: false, protocols: ["anthropic"], config_paths: ["~/.claude/settings.json"] },
    { agent: "codex", label: "Codex", placeholder_key: "kw-ag-codex-c3d4", enabled: true, additive: false, protocols: ["openai"], config_paths: ["~/.codex/config.toml", "~/.codex/auth.json"] },
    { agent: "gemini", label: "Gemini CLI", placeholder_key: null, enabled: false, additive: false, protocols: ["gemini"], config_paths: ["~/.gemini/.env"] },
    { agent: "grokbuild", label: "Grok Build", placeholder_key: null, enabled: false, additive: false, protocols: ["openai"], config_paths: ["~/.grok/config.toml"] },
    { agent: "claude-desktop", label: "Claude Desktop", placeholder_key: null, enabled: false, additive: false, protocols: ["anthropic"], config_paths: [
      "~/Library/Application Support/Claude/claude_desktop_config.json",
      "~/Library/Application Support/Claude-3p/claude_desktop_config.json",
      "~/Library/Application Support/Claude-3p/configLibrary/kiwano.json",
      "~/Library/Application Support/Claude-3p/configLibrary/_meta.json",
    ] },
    { agent: "opencode", label: "OpenCode", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.config/opencode/opencode.json"] },
    { agent: "openclaw", label: "OpenClaw", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.openclaw/openclaw.json"] },
    { agent: "hermes", label: "Hermes", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.hermes/config.yaml"] },
    { agent: "pi", label: "Pi", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.pi/agent/models.json", "~/.pi/agent/settings.json"] },
    { agent: "workbuddy", label: "WorkBuddy", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.workbuddy/models.json"] },
    { agent: "codebuddy", label: "CodeBuddy Code", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.codebuddy/models.json"] },
    { agent: "kimi", label: "Kimi Code CLI", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.kimi/config.toml"] },
    { agent: "qwen", label: "Qwen Code", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.qwen/settings.json"] },
    { agent: "cline", label: "Cline", placeholder_key: null, enabled: false, additive: false, protocols: ["openai"], config_paths: ["~/.cline/data/settings/providers.json"] },
  ],
  // Two user-defined agents, so `pnpm dev` can show the whole feature: a route
  // with candidates, and one that is still empty (the state the tab's bind slot
  // exists for).
  custom_agents: [
    {
      id: "long-tasks-3f9a",
      label: "Long tasks",
      note: "batch work at off-peak rates",
      protocol: "openai",
      placeholder_key: "kw-ag-long-tasks-3f9a-b7e1",
    },
    {
      // Null on purpose: this is the row a pre-v24 database has, so `pnpm dev`
      // shows the "not said" state next to the one that was answered.
      id: "scratch-91cd",
      label: "Scratch",
      note: null,
      protocol: null,
      placeholder_key: "kw-ag-scratch-91cd-40aa",
    },
    // Three more, so the routing strategies the built-in agents do not exercise
    // (roundrobin / timewindow / quota) are reachable in `pnpm dev` — they are
    // what the matrix rows below are bound to.
    {
      id: "rotating-pool-4a1f",
      label: "Rotating pool",
      note: "two providers, weighted rotation",
      protocol: "openai",
      placeholder_key: "kw-ag-rotating-pool-4a1f-0d31",
    },
    {
      id: "nightly-batch-7c2e",
      label: "Nightly batch",
      note: "one by day, one inside the night window",
      protocol: "anthropic",
      placeholder_key: "kw-ag-nightly-batch-7c2e-5a90",
    },
    {
      id: "quota-guard-5b8d",
      label: "Quota guard",
      note: "stops routing once its ceiling is reached",
      protocol: "openai",
      placeholder_key: "kw-ag-quota-guard-5b8d-c41e",
    },
  ],
  auto_failover: true,
  request_logs: true,
  compat_shim: true,
  dlp_mode: "alert",
  log_retention_days: 0,
  log_max_body_bytes: 0,
  stream_first_byte_secs: 120,
  stream_idle_secs: 120,
  cost_alert: true,
  preferred_currency: "CNY",
  auto_check_update: true,
  dismissed_update: null,
  tz_offset_minutes: 480,
  hub_logged_in: false,
  hub_url: "https://hub.kiwano.cc/catalog.json",
  shelf_sort: null,
  shelf_view: null,
  // Features panel: every flag defaults off in the backend; the dev fixture
  // turns two on so the alerts they shape are reachable in `pnpm dev`.
  feat_cost_forecast: true,
  feat_anomaly_alerts: true,
  feat_agent_limit_alerts: false,
  feat_mcp_self_query: false,
  feat_rule_injection: false,
  feat_tuning_advice: false,
  feat_cache_experiment: false,
};
