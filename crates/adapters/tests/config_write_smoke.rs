//! Self-developed smoke tests for the extracted cc-switch config writers.
//!
//! These exercise the Tier A modules end-to-end against real tempfiles:
//! Grok TOML live-config round-trips and the shared atomic JSON writer used
//! by every adapter.

use std::fs;

use kiwano_adapters::config::{atomic_write_private, read_json_file, write_json_file};
use kiwano_adapters::grok_config::{read_grok_live_settings, write_grok_live_settings};
use serde_json::json;

#[test]
fn atomic_json_write_produces_readable_sorted_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    write_json_file(&path, &json!({"b": 1, "a": {"z": true, "y": 2}})).unwrap();

    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.starts_with("{\n  \"a\""), "keys must be sorted:\n{raw}");
    let back: serde_json::Value = read_json_file(&path).unwrap();
    assert_eq!(back["a"]["y"], 2);

    let private = dir.path().join("auth.json");
    atomic_write_private(&private, b"{}").unwrap();
    assert_eq!(fs::read(&private).unwrap(), b"{}");
}

#[test]
#[serial_test::serial]
fn grok_live_settings_round_trip_on_tempfile() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CC_SWITCH_TEST_HOME", dir.path());

    let config = r#"[models]
default = "grok-4.5"

[model."grok-4.5"]
model = "grok-4.5"
base_url = "https://api.example.com/v1"
name = "Example"
api_key = "sk-grok"
api_backend = "responses"
context_window = 500000
"#;
    write_grok_live_settings(&json!({ "config": config })).unwrap();
    let live = read_grok_live_settings().unwrap();
    assert_eq!(
        live.get("config").and_then(serde_json::Value::as_str),
        Some(config)
    );

    std::env::remove_var("CC_SWITCH_TEST_HOME");
}
