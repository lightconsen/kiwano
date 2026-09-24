//! Agent takeover/restore (tech.md §4.3-3 takeover flow, §2.4 B key point 3).
//!
//! Takeover = back up the original config (SQLite `takeover_backups` table +
//! file copies) → rewrite base_url to point at the local gateway + inject an
//! Agent-specific placeholder key; restore = write the backup back (the
//! escape hatch). Backup semantics: re-enabling while already taken over
//! never overwrites the first backup, so the user's original config can be
//! restored no matter how many takeovers happen in a row.
//!
//! The MVP covered claude (settings.json), codex (config.toml + auth.json)
//! and gemini (~/.gemini/.env: the two variables Gemini CLI reads to point at
//! a different endpoint). The expanded registry adds grokbuild
//! (~/.grok/config.toml, selected model's base_url/api_key/api_backend), the
//! additive-mode agents opencode/openclaw/hermes/pi — whose takeover upserts
//! a `kiwano-gateway` provider entry and selects it via
//! `kiwano_adapters::gateway_takeover` — and claude-desktop (macOS only:
//! deploymentMode 3p + a gateway profile in the Claude-3p configLibrary via
//! `kiwano_adapters::claude_desktop_config`). Pre-existing provider
//! entries survive, and disable still restores the original bytes verbatim.
//!
//! # Honesty rules (borrowed in concept from cc-switch's takeover/codex paths;
//! this file is Kiwano-authored, so it carries no derived-from header)
//!
//! Three things can claim a takeover: this module's backup row, the
//! `placeholder_keys` table, and the agent's live config. They can disagree —
//! a rolled-back key registration over a rewritten config, a delete that
//! failed, a `~/.codex` restored by hand or rewritten by another tool — so
//! **state is derived from the live file and nothing else**:
//! [`live_placeholder_key`] reads the route out of it and `vm::build_settings`
//! reports what it finds. The key table is a registration side-effect, and
//! [`restorable_backup`] answers a different question — whether restore has an
//! original it can write back — which is why neither counts as state: an agent
//! whose config was put back behind us reads as not taken over even while we
//! still hold a backup for it, and one whose row registration was rolled back
//! reads as taken over because the file says so.
//!
//! [`enable`] refuses before it touches anything (the Codex gates in
//! `kiwano_adapters::codex_config` run first, on the text that would be
//! written) and rolls its own writes back if one of them fails, so a refused
//! or half-done takeover leaves neither a backup row nor a rewritten file to
//! mislead the reader.
//!
//! [`disable`] degrades instead of stranding: the backup if it is usable,
//! otherwise a rebuild from the provider the gateway serves, otherwise
//! stripping the gateway route out of the live config. Losing the backup never
//! leaves an agent pointed at a dead loopback port, and never reports a
//! success it did not achieve — a restore that could not give the original
//! config back says so.
//!
//! The module is split by domain. Every item that was `pub` at the `takeover`
//! root keeps the path it had when this was one file — the facade below
//! re-exports it — so `takeover::enable`, `takeover::BackupFile` and
//! `takeover::home_owned_by_other_user` resolve for `crates/cli`,
//! `app/src-tauri` and the sibling modules of this crate exactly as before.
//!
//! `paths` and `rewrite` are leaves: a path is decided from the environment the
//! caller hands in and a rewrite is a pure text transform, so neither reads a
//! store, a backup row or the live file it is about. `backup` owns what a
//! capture is (`BackupFile`, `Files`); `state` owns the "is this ours?" question
//! and the contract types a caller reads back; `enable`, `disable` and `strip`
//! are the write path, the restore chain and its last tier.
//!
//! One edge looks backwards and is deliberate: `rewrite` imports `enable`'s
//! [`gateway_target`], because codex's two files are transformed together by
//! `codex_rewrites` — the arm `rewrite` refuses on — and it needs the URL a
//! takeover points at. Moving the gateway origin into `rewrite` to buy a
//! one-way edge would put a const about the gateway in the text-transform
//! module, which is the less honest of the two.

pub mod backup;
pub mod disable;
pub mod enable;
pub mod paths;
pub mod rewrite;
pub mod state;
pub mod strip;

/// The service/home account-mismatch check, shared with the CLI and the
/// gateway — see `kiwano_adapters::config::home_owned_by_other_user`.
pub use kiwano_adapters::config::home_owned_by_other_user;

// ── the public surface, re-exported so every `takeover::x` path still resolves ──

pub use backup::BackupFile;
pub use disable::{disable, REBUILDABLE_AGENTS};
pub use enable::enable;
pub use paths::CONFIG_DIR_VARS;
pub use state::{
    live_placeholder_key, restorable_backup, ProviderRoute, RestoreOutcome, RestoreReport,
};

/// In-crate: the resolver `rules_inject` and the view models read paths through
/// as well, so it keeps the path it had rather than moving under `takeover::paths`.
pub(crate) use paths::takeover_paths;

#[cfg(test)]
pub(crate) mod test_support {
    use crate::detect::ShellVars;
    use crate::takeover::{disable, RestoreReport};
    use crate::vm::Aux;
    use std::path::{Path, PathBuf};

    /// The tests run with no shell environment: a takeover here is rooted at the
    /// temp home the test injected, which is the whole point of injecting it.
    /// The variables are covered on their own in `config_dir`'s tests.
    pub(crate) fn no_vars() -> ShellVars {
        ShellVars::new()
    }

    /// A directory to name in a variable, under `tmp` so that it is absolute on
    /// every platform: a literal `/srv/...` is absolute on unix and *relative* on
    /// Windows, where the rule under test refuses exactly that — the test would
    /// be asserting the opposite of what it reads.
    pub(crate) fn abs_dir(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn temp_home() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        (dir, home)
    }

    /// Goose's config root as `takeover_paths` resolves it on this platform:
    /// the etcetera strategy lands on `Library/Application Support/Block` on
    /// macOS and on the XDG config dir elsewhere. The goose tests assert
    /// against the same root the takeover writes, so this is the one
    /// definition of it — a hardcoded macOS path passed CI on a Mac and
    /// failed everywhere else.
    pub(crate) fn goose_test_root(home: &Path) -> PathBuf {
        crate::takeover::paths::takeover_paths("goose", home, &no_vars())
            .expect("goose paths resolve")
            .into_iter()
            .next()
            .expect("goose lists config.yaml first")
            .parent()
            .expect("config.yaml sits in the root")
            .to_path_buf()
    }

    /// Restore with no provider to rebuild from. This is the plain "put my
    /// config back" call; the degradation chain's middle tier is exercised by
    /// the tests that pass a [`ProviderRoute`](crate::takeover::ProviderRoute) explicitly.
    pub(crate) fn restore(aux: &Aux, agent: &str, home: &Path) -> Result<RestoreReport, String> {
        disable(aux, agent, home, None, &no_vars())
    }

    /// A Codex config the gates accept: the active provider is named by a
    /// top-level `model_provider` **string** and its table lives under
    /// `[model_providers.<id>]`. The pre-0.48 `[model_provider.custom]` singular
    /// table several older Kiwano fixtures used is not a shape Codex 0.149 reads
    /// at all — there is no active provider for the gateway to route, so the
    /// takeover is refused — which is why the fixture is written this way.
    pub(crate) const CODEX_ORIGINAL: &str = r#"model = "m"
model_provider = "custom"

[model_providers.custom]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#;

    pub(crate) fn write_codex_config(home: &Path, config: &str, auth: &str) -> std::path::PathBuf {
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(codex_dir.join("config.toml"), config).unwrap();
        std::fs::write(codex_dir.join("auth.json"), auth).unwrap();
        codex_dir
    }
}
