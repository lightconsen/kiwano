# Agent takeover

Takeover is how an agent ends up routed through the local gateway: Kiwano
rewrites the agent's **own** config file under the agent's own `$HOME`, with
the original backed up first and restored byte-for-byte when you switch the
takeover off. No proxy binary is injected, no environment variables are
exported on your behalf — the agent reads its usual config and finds the
gateway there.

This page is the per-agent reference: what file each takeover writes, in
which mode, and the behavior worth knowing. The agent set and these paths are
pinned by a consistency test (`docs` table ↔ built-in registry), so this page
cannot silently drift from what a build does.

## The three modes

- **Additive** — the agent's config holds a provider list; the takeover
  upserts a `kiwano-gateway` entry and selects it. Your own providers stay.
- **Exclusive** — the agent's config is one provider slot; the takeover
  overwrites it. Restore puts your bytes back.
- **Replaces the selected slot** (Cline only) — the selection names a provider
  id, so the takeover rewrites the slot that id points at rather than adding
  a second entry.

## The agents

| Agent | Config file(s) rewritten | Mode | Worth knowing |
| --- | --- | --- | --- |
| `claude` | `~/.claude/settings.json` | exclusive | `CLAUDE_CONFIG_DIR` moves the whole profile. |
| `codex` | `~/.codex/config.toml` + `auth.json` | exclusive | `CODEX_HOME` honored. Both files rewritten together; refuses a missing `config.toml` — run Codex once first. |
| `gemini` | `~/.gemini/.env` | exclusive | Missing file is created; comments in it are preserved. |
| `grokbuild` | `~/.grok/config.toml` | exclusive | Refuses the official xAI login config — set up a custom model first. |
| `claude-desktop` | two `claude_desktop_config.json` + configLibrary profile + `_meta.json` | exclusive | macOS only; flips deployment mode to 3p and writes a gateway profile. |
| `opencode` | `~/.config/opencode/opencode.json` | additive | XDG rules (`XDG_CONFIG_HOME`). |
| `openclaw` | `~/.openclaw/openclaw.json` + `agents/<id>/agent/models.json` | additive | The catalogue file declares the session-affinity flag the main config's validator rejects. |
| `hermes` | `~/.hermes/config.yaml` | additive | `HERMES_HOME` honored. |
| `pi` | `~/.pi/agent/models.json` + `settings.json` | additive | The settings file carries the selection. |
| `workbuddy` | `~/.workbuddy/models.json` | additive | `WORKBUDDY_CONFIG_DIR` honored. |
| `codebuddy` | `~/.codebuddy/models.json` | additive | `CODEBUDDY_CONFIG_DIR` honored. |
| `kimi` | `~/.kimi-code/config.toml` | additive | `KIMI_CODE_HOME` honored; the legacy `~/.kimi` is read when the successor is absent. |
| `qwen` | `~/.qwen/settings.json` | additive | `QWEN_HOME` honored. |
| `cline` | `~/.cline/data/settings/providers.json` | replaces the selected slot | `CLINE_PROVIDER_SETTINGS_PATH` / `CLINE_DATA_DIR` / `CLINE_DIR` honored, in that order. |
| `mimo` | `~/.config/mimocode/mimocode.jsonc` | additive | XDG rules. |
| `mcode` | `~/.minimax/config.yaml` | additive | `MCODE_CONFIG_DIR` honored. |
| `aider` | `~/.aider.conf.yml` | exclusive | Three global fields, not a provider list. The model gets an `openai/` prefix to force LiteLLM's OpenAI-compatible route; the model id itself reaches the upstream verbatim. `AIDER_CONFIG` is deliberately not honored. |
| `continue` | `~/.continue/config.yaml` | additive | The gateway model is inserted at the head of `models` with `roles: [chat]` — Continue's documented default is "first model for the role". |
| `crush` | `~/.config/crush/crush.json` | additive | Fills the `models.large` slot its own `model large` command persists; the `small` slot is untouched. |
| `droid` | `~/.factory/settings.json` | additive | Prepends a `customModels` entry and sets the default `model`. Refused when org policy sets `allowCustomModels: false`. |
| `goose` | `<goose config root>/config.yaml` + `custom_providers/kiwano-gateway.json` + `kiwano-gateway.key` | additive | macOS root is `~/Library/Application Support/Block/goose` (not `~/.config/goose`); Linux follows XDG. The key reaches goose through the provider's `auth.command` (`/bin/cat` of the key file), because goose's keyring-based secret store is not file-reachable. First-takeover import: none — the key lives in the keyring. |
| `zcode` | `~/.zcode/v2/provider_config.json` | additive | The native registry shared between the ZCode CLI and Desktop — a takeover affects both. `ZCODE_PERSONAL_PROVIDER_CONFIG_FILE` is honored; `ZCODE_DATA_BASE_DIR` is not. |

## Environment variables

A takeover resolves paths through the user's **login shell** environment (a
GUI app cannot see what rc files export otherwise). A variable that is set to
a *relative* path is refused rather than guessed at.

Honored: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_CONFIG_HOME` (opencode,
mimo, crush; goose on Linux), `HERMES_HOME`, `WORKBUDDY_CONFIG_DIR`,
`CODEBUDDY_CONFIG_DIR`, `QWEN_HOME`, `KIMI_CODE_HOME` (+ legacy
`KIMI_SHARE_DIR`), `CLINE_PROVIDER_SETTINGS_PATH`, `CLINE_DATA_DIR`,
`CLINE_DIR`, `MCODE_CONFIG_DIR`, `ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`.

Deliberately not honored: `AIDER_CONFIG` (aider v1.24 does not read it when
loading), `GOOSE_PATH_ROOT` and `ZCODE_DATA_BASE_DIR` (relocating a whole
tree would split files a takeover writes together), `OPENCODE_CONFIG` /
`OPENCODE_CONFIG_DIR` and the OpenClaw state overrides (they do not move the
file a takeover has to write).

## First-takeover import

When an agent already has a custom endpoint configured, enabling the takeover
offers to import it: the same base URL and key become a registered provider
bound to the agent, so day one routes the same upstream. Import never
re-imports a loopback endpoint (that is the gateway itself, post-takeover),
and it reads nothing where no plaintext key exists — official-subscription
logins (Claude OAuth, Codex ChatGPT) and goose's keyring extract nothing, and
the onboarding guide offers manual entry instead.

## Restore semantics

Switching a takeover off: the backed-up originals are written back byte for
byte (files the takeover created are removed). If the backup is gone — or it
was captured while the config was already ours — the route is rebuilt from
the provider the gateway serves the agent; failing that, our route is
stripped from the live config so the agent is never left pointing at a
loopback port nothing listens on. The strip is surgical: only rows/values
that are ours are removed.
