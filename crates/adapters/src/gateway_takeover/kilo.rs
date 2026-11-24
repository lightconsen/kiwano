//! Kilo Code CLI: OpenCode's fork, with OpenCode's provider schema and its own
//! config directory and file-name candidates.
//!
//! # What is Kilo's own
//!
//! - The config root is `$XDG_CONFIG_HOME/kilo` (`packages/core/src/global.ts`:
//!   `const app = "kilo"` → `path.join(clean(xdgConfig)!, app)`), so it follows
//!   the XDG rules exactly as OpenCode's does — including on Windows, where the
//!   `xdg-basedir` package has no platform branch.
//! - The global config is *merged from up to five files*, in this order
//!   (`packages/opencode/src/config/config.ts`, `loadGlobal`): `config.json`,
//!   `kilo.json`, `kilo.jsonc`, `opencode.json`, `opencode.jsonc`. Later files
//!   win on conflicting keys (`mergeConfig` is a deep merge).
//!
//! # Which file the takeover writes — and why it is not the one Kilo writes
//!
//! Kilo's own writes (`updateGlobal`, i.e. anything its UI saves) go to
//! `globalConfigFile()`: the first existing file among `kilo.jsonc`,
//! `kilo.json`, `opencode.jsonc`, `opencode.json`, `config.json`, else
//! `kilo.jsonc`. That list is *not* the merge priority, and the difference
//! matters: a user with both `kilo.jsonc` and `opencode.jsonc` gets Kilo's
//! settings written into the file the other one overrides.
//!
//! A takeover that copied that behaviour would inherit the problem, so this
//! module picks the **highest-priority file that exists** instead — the one the
//! rewrite has to land in to take effect. A fresh install has none of them, and
//! gets `kilo.jsonc` (the name Kilo's own writer uses).
//!
//! # What is not honored
//!
//! `KILO_CONFIG` (an extra file merged in), `KILO_CONFIG_DIR` (a directory
//! added to the config search list) and `KILO_CONFIG_CONTENT` (inline config)
//! are additional inputs, not relocations of the global file the takeover owns
//! — the same stance `kiwano_core::takeover::paths` takes on OpenCode's
//! `OPENCODE_CONFIG`/`OPENCODE_CONFIG_DIR`. Writing into one of them would put
//! the entry where Kilo merges *from* rather than where the user's own config
//! lives. A provider merge is additive, so an entry in the global file still
//! applies alongside them; only a `model` selector in one of those inputs, naming
//! a provider of its own, would win over the one written here.

use crate::gateway_takeover::gateway::GATEWAY_PROVIDER_ID;
use crate::gateway_takeover::json::parse_jsonc;
use crate::gateway_takeover::openai_compatible::upsert_openai_compatible_provider_selecting;
use crate::gateway_takeover::readers::CurrentProvider;
use serde_json::Value;
use std::path::{Path, PathBuf};

// ── kilo (~/.config/kilo, JSONC) ──

/// The global config names, highest merge priority first — the order
/// `loadGlobal` merges them in, reversed.
pub const KILO_CONFIG_FILES: [&str; 5] = [
    "opencode.jsonc",
    "opencode.json",
    "kilo.jsonc",
    "kilo.json",
    "config.json",
];

/// The file the takeover writes inside the agent's config directory: the
/// highest-priority one that exists, else `kilo.jsonc` (what a fresh Kilo
/// writes, and what its own `updateGlobal` falls back to).
pub fn kilo_config_file(config_dir: &Path) -> PathBuf {
    for name in KILO_CONFIG_FILES {
        let candidate = config_dir.join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    config_dir.join("kilo.jsonc")
}

/// Upsert the gateway provider and rewrite the top-level `model` selector,
/// keeping the user's model id. The entry is the same OpenAI-compatible one
/// OpenCode gets (`npm: @ai-sdk/openai-compatible`) — Kilo's provider schema is
/// OpenCode's.
///
/// `file_label` is only what a parse error names, and it is a parameter
/// because the file is one of Kilo's five: an error that said `kilo.jsonc` for
/// a rewrite of `opencode.jsonc` would send the reader to the wrong file.
///
/// `fallback_selection` is the model id Kilo's merge resolves from the *other*
/// config files (see [`kilo_merged_model_id`]): the selector commonly lives in
/// a lower-priority file than the endpoint, and the delivery is only real when
/// the file that wins carries the selection too. A selector already in this
/// file wins over it.
pub fn upsert_kilo_gateway(
    content: &str,
    file_label: &str,
    base_url: &str,
    key: &str,
    fallback_selection: Option<&str>,
) -> Result<String, String> {
    upsert_openai_compatible_provider_selecting(
        content,
        file_label,
        base_url,
        key,
        fallback_selection,
    )
}

/// The model id Kilo's merge selects from `configs` (**lowest priority
/// first**), i.e. the part after the slash of a `model: "<provider>/<model>"`
/// selector.
///
/// This is the ride-along for a takeover whose written file is not the one that
/// carries the selector. Returns None when a file is unparseable — the same
/// strictness as [`read_kilo_current`], because a guess would move a user who
/// is on a provider of their own onto the gateway with the wrong model.
pub fn kilo_merged_model_id(configs_low_to_high: &[&str]) -> Option<String> {
    let merged = merge_configs(configs_low_to_high)?;
    merged
        .get("model")?
        .as_str()?
        .split_once('/')
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// [`kilo_merged_model_id`] over the config files in `config_dir`, as they are
/// on disk — the merged selector Kilo would resolve, whichever of the five
/// files each part of it lives in.
///
/// `excluded` is the file the caller is rewriting; its selector is the
/// transform's own business (it wins the merge, so it is what the rewrite
/// keeps). Skipping it here is what lets the caller ask "what would Kilo have
/// selected from the *rest* of my files?".
pub fn read_kilo_merged_model_id(config_dir: &Path, excluded: Option<&Path>) -> Option<String> {
    let contents: Vec<String> = KILO_CONFIG_FILES
        .iter()
        .rev()
        .map(|name| config_dir.join(name))
        .filter(|path| excluded != Some(path.as_path()))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect();
    let refs: Vec<&str> = contents.iter().map(String::as_str).collect();
    kilo_merged_model_id(&refs)
}

/// Kilo's own merge of `configs` (**lowest priority first**), as a reader for
/// a provider: what the agent would actually route to today.
///
/// This is a reader for the first-takeover import, and it has to merge rather
/// than look at one file: Kilo reads up to five of them and later ones win, so
/// the user's selected model and the entry it names can sit in different files.
/// The merge mirrors Kilo's `mergeDeep` for the shapes this reader walks —
/// objects merge key by key, everything else is replaced by the later value.
///
/// Returns None when any file is unparseable (a reader that quietly skipped
/// one could report a provider the agent is not actually using), when no
/// selector is set, when it does not use the `<provider>/<model>` form, or when
/// the entry it names has no readable endpoint and key.
pub fn read_kilo_current(configs_low_to_high: &[&str]) -> Option<CurrentProvider> {
    let merged = merge_configs(configs_low_to_high)?;

    let id = merged
        .get("model")?
        .as_str()?
        .split_once('/')?
        .0
        .to_string();
    if id.is_empty() || id == GATEWAY_PROVIDER_ID {
        return None;
    }
    let options = merged.get("provider")?.get(&id)?.get("options")?;
    Some(CurrentProvider {
        name: id,
        base_url: options.get("baseURL")?.as_str()?.to_string(),
        api_key: options.get("apiKey")?.as_str()?.to_string(),
    })
}

/// Kilo's `mergeDeep` over the config files, lowest priority first. None when
/// any file is unparseable or is not an object.
fn merge_configs(configs_low_to_high: &[&str]) -> Option<Value> {
    let mut merged = Value::Object(serde_json::Map::new());
    for content in configs_low_to_high {
        let parsed = parse_jsonc(content, "kilo config").ok()?;
        if !parsed.is_object() {
            return None;
        }
        deep_merge(&mut merged, parsed);
    }
    Some(merged)
}

/// `into` wins where both are objects and the key exists on both sides;
/// everything else is replaced wholesale, as Kilo's `mergeDeep` does.
fn deep_merge(into: &mut Value, from: Value) {
    match (into, from) {
        (Value::Object(target), Value::Object(source)) => {
            for (key, value) in source {
                match target.get_mut(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway_takeover::GATEWAY_PROVIDER_ID;
    use serde_json::Value;

    #[test]
    fn kilo_upserts_entry_and_rewrites_model_prefix() {
        let original = r#"{
  // Kilo's global config is JSONC
  "$schema": "https://app.kilo.ai/config.json",
  "model": "anthropic/claude-sonnet-4",
  "provider": {
    "anthropic": { "options": { "apiKey": "sk-ant-old" } }
  },
}"#;
        let out = upsert_kilo_gateway(
            original,
            "kilo.jsonc",
            "http://127.0.0.1:8317/v1",
            "kw-ag-kilo-abcd",
            None,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "kiwano-gateway/claude-sonnet-4");
        let entry = &v["provider"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(entry["options"]["baseURL"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["options"]["apiKey"], "kw-ag-kilo-abcd");
        assert!(entry["models"]["claude-sonnet-4"].is_object());
        // the user's own provider survives
        assert_eq!(
            v["provider"]["anthropic"]["options"]["apiKey"],
            "sk-ant-old"
        );
    }

    /// A fresh install has no config at all: the entry is created and no
    /// selector is invented.
    #[test]
    fn kilo_without_model_selector_only_adds_entry() {
        let out =
            upsert_kilo_gateway("", "kilo.jsonc", "http://127.0.0.1:8317/v1", "kw", None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["provider"][GATEWAY_PROVIDER_ID].is_object());
        assert!(v.get("model").is_none());
    }

    /// The selector commonly lives in a file other than the one written: the
    /// fallback is what makes the takeover real, and a selector in the file
    /// itself still wins.
    #[test]
    fn a_selector_from_another_config_file_is_carried_in() {
        let out = upsert_kilo_gateway(
            r#"{"provider":{}}"#,
            "opencode.json",
            "http://127.0.0.1:8317/v1",
            "kw",
            Some("deepseek-chat"),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "kiwano-gateway/deepseek-chat");
        assert!(v["provider"][GATEWAY_PROVIDER_ID]["models"]["deepseek-chat"].is_object());

        // The file's own selection is the higher priority one, so it wins.
        let out = upsert_kilo_gateway(
            r#"{"model":"anthropic/claude-sonnet-4"}"#,
            "opencode.json",
            "http://127.0.0.1:8317/v1",
            "kw",
            Some("deepseek-chat"),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "kiwano-gateway/claude-sonnet-4");
        assert!(v["provider"][GATEWAY_PROVIDER_ID]["models"]["claude-sonnet-4"].is_object());
    }

    /// The ride-along reader resolves the merged selector across the files, and
    /// leaves out the one being written.
    #[test]
    fn the_merged_selector_is_read_across_the_config_files() {
        let dir = tempfile::tempdir().unwrap();
        // config.json (lowest) names the model, kilo.jsonc renames it, and
        // opencode.json is the file the takeover writes.
        std::fs::write(dir.path().join("config.json"), r#"{"model":"a/one"}"#).unwrap();
        std::fs::write(
            dir.path().join("kilo.jsonc"),
            r#"{"model":"deepseek/deepseek-chat"}"#,
        )
        .unwrap();
        let written = dir.path().join("opencode.json");
        std::fs::write(&written, r#"{"provider":{}}"#).unwrap();

        assert_eq!(
            read_kilo_merged_model_id(dir.path(), Some(&written)).as_deref(),
            Some("deepseek-chat"),
            "the higher of the two remaining files wins"
        );
        // With the written file included there is still `kilo.jsonc`'s; once it
        // has a selector of its own, that one wins — it is the highest priority.
        assert_eq!(
            read_kilo_merged_model_id(dir.path(), None).as_deref(),
            Some("deepseek-chat")
        );
        std::fs::write(&written, r#"{"model":"anthropic/claude-sonnet-4"}"#).unwrap();
        assert_eq!(
            read_kilo_merged_model_id(dir.path(), None).as_deref(),
            Some("claude-sonnet-4"),
            "the file written is the one Kilo's merge resolves"
        );
        assert_eq!(
            read_kilo_merged_model_id(dir.path(), Some(&written)).as_deref(),
            Some("deepseek-chat"),
            "and the ride-along is what the other files say"
        );

        // An unparseable file leaves nothing to carry in.
        std::fs::write(dir.path().join("kilo.jsonc"), r#"{"model":"#).unwrap();
        assert_eq!(read_kilo_merged_model_id(dir.path(), Some(&written)), None);
    }

    /// The write target is the file that wins the merge, not the one Kilo's own
    /// writer would pick; with nothing there at all it is `kilo.jsonc`.
    #[test]
    fn the_write_target_is_the_highest_priority_config_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(kilo_config_file(dir.path()), dir.path().join("kilo.jsonc"));

        std::fs::write(dir.path().join("kilo.json"), "{}").unwrap();
        assert_eq!(kilo_config_file(dir.path()), dir.path().join("kilo.json"));

        // opencode.json beats kilo.jsonc on read, so it is where a rewrite has
        // to land to take effect.
        std::fs::write(dir.path().join("kilo.jsonc"), "{}").unwrap();
        assert_eq!(kilo_config_file(dir.path()), dir.path().join("kilo.jsonc"));
        std::fs::write(dir.path().join("opencode.json"), "{}").unwrap();
        assert_eq!(
            kilo_config_file(dir.path()),
            dir.path().join("opencode.json")
        );
        std::fs::write(dir.path().join("opencode.jsonc"), "{}").unwrap();
        assert_eq!(
            kilo_config_file(dir.path()),
            dir.path().join("opencode.jsonc")
        );
    }

    /// The first-takeover import has to merge the way Kilo reads: a selector in
    /// one file and the entry it names in another still resolve.
    #[test]
    fn the_import_merges_the_config_files_the_way_kilo_reads_them() {
        let low = r#"{"theme":"dark","provider":{"anthropic":{"options":{"baseURL":"https://api.anthropic.com/v1"}}}}"#;
        let high = r#"{"model":"anthropic/claude-sonnet-4","provider":{"anthropic":{"options":{"apiKey":"sk-ant"}}}}"#;

        let cur = read_kilo_current(&[low, high]).unwrap();
        assert_eq!(cur.name, "anthropic");
        assert_eq!(cur.base_url, "https://api.anthropic.com/v1");
        assert_eq!(cur.api_key, "sk-ant");

        // A later file overrides an overlapping key, objects merging key by
        // key: here the higher-priority file renames the endpoint.
        let high_endpoint = r#"{"model":"anthropic/x","provider":{"anthropic":{"options":{"apiKey":"sk-ant","baseURL":"https://proxy.internal/v1"}}}}"#;
        let cur = read_kilo_current(&[low, high_endpoint]).unwrap();
        assert_eq!(cur.base_url, "https://proxy.internal/v1");
        assert_eq!(cur.api_key, "sk-ant", "the lower file's key survives");

        // Nothing to extract, each way there is nothing.
        assert!(read_kilo_current(&[]).is_none());
        assert!(read_kilo_current(&[r#"{"provider":{}}"#]).is_none());
        assert!(read_kilo_current(&[r#"{"model":"kiwano-gateway/m"}"#]).is_none());
        assert!(read_kilo_current(&[r#"{"model":"ghost/m"}"#]).is_none());
        assert!(
            read_kilo_current(&[r#"{"model":"a/m"#]).is_none(),
            "an unparseable file is not skipped silently"
        );
    }
}
