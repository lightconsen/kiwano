//! Self-developed smoke tests for the extracted cc-switch config writers.
//!
//! These exercise the Tier A modules end-to-end against real tempfiles:
//! Gemini env serialization, Grok TOML live-config round-trips, and the
//! shared atomic JSON writer used by every adapter.

use std::fs;

use kiwano_adapters::config::{atomic_write_private, read_json_file, write_json_file};
use kiwano_adapters::gemini_config::{
    get_gemini_env_path, parse_env_file, serialize_env_file, write_gemini_env_text_atomic,
};
use kiwano_adapters::grok_config::{read_grok_live_settings, write_grok_live_settings};
use serde_json::json;

#[test]
#[serial_test::serial]
fn gemini_env_write_round_trips_through_tempfile() {
    let dir = tempfile::tempdir().unwrap();
    // get_gemini_env_path() honors CC_SWITCH_TEST_HOME via get_home_dir().
    std::env::set_var("CC_SWITCH_TEST_HOME", dir.path());

    let mut map =
        parse_env_file("GEMINI_API_KEY=k-123\nGOOGLE_GEMINI_BASE_URL=https://x.example\n");
    assert_eq!(map.get("GEMINI_API_KEY").map(String::as_str), Some("k-123"));

    write_gemini_env_text_atomic(&serialize_env_file(&map)).unwrap();
    let env_path = get_gemini_env_path();
    assert!(env_path.starts_with(dir.path()));
    let written = fs::read_to_string(&env_path).unwrap();
    map = parse_env_file(&written);
    assert_eq!(
        map.get("GOOGLE_GEMINI_BASE_URL").map(String::as_str),
        Some("https://x.example")
    );

    std::env::remove_var("CC_SWITCH_TEST_HOME");
}

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
