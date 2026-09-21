//! WorkBuddy's model list — a bare array, the shape its GUI writes.

use crate::gateway_takeover::gateway::PLACEHOLDER_MODEL_ID;
use crate::gateway_takeover::model_list::{
    new_entry, point_entry_at_gateway, takeover_target_index,
};
use serde_json::Value;

// ── workbuddy (~/.workbuddy/models.json, a bare JSON array) ──

/// Take over WorkBuddy's model list.
///
/// The file is a **bare array** — the shape its GUI writes. The published docs
/// show an object instead, which is what the CLI embedded in the app parses;
/// the GUI is what reads this file, so the array is the shape to write.
///
/// `url` is a full endpoint (WorkBuddy appends nothing), so the caller passes
/// `…/v1/chat/completions`.
pub fn upsert_workbuddy_gateway(content: &str, url: &str, key: &str) -> Result<String, String> {
    let mut entries: Vec<Value> = if content.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(content)
            .map_err(|e| format!("models.json is not a JSON array: {e}"))?
    };

    match takeover_target_index(&entries) {
        Some(i) => point_entry_at_gateway(&mut entries[i], url, key)?,
        None => entries.push(new_entry(PLACEHOLDER_MODEL_ID, url, key)),
    }

    serde_json::to_string_pretty(&Value::Array(entries)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway_takeover::model_list::GATEWAY_VENDOR;

    /// The shape a real install writes (a bare array — see any
    /// `~/.workbuddy/models.json`); the published docs' object form is what
    /// the CLI embedded in the app reads, not what the GUI writes.
    #[test]
    fn workbuddy_takes_over_the_first_entry_and_keeps_the_rest() {
        let original = r#"[
  { "id": "deepseek-v4-pro", "name": "DeepSeek-V4 Pro", "vendor": "DeepSeek",
    "url": "https://api.deepseek.com/chat/completions", "apiKey": "sk-old",
    "supportsToolCall": true, "supportsImages": false },
  { "id": "kimi-k2", "vendor": "Moonshot",
    "url": "https://api.moonshot.cn/v1/chat/completions", "apiKey": "sk-kimi" }
]"#;
        let out = upsert_workbuddy_gateway(
            original,
            "http://127.0.0.1:8317/v1/chat/completions",
            "kw-ag-workbuddy-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().expect("still an array");

        assert_eq!(arr.len(), 2, "the second model is left alone");
        assert_eq!(
            arr[0]["id"], "deepseek-v4-pro",
            "the model id survives — it is what goes upstream"
        );
        assert_eq!(arr[0]["vendor"], GATEWAY_VENDOR);
        assert_eq!(arr[0]["url"], "http://127.0.0.1:8317/v1/chat/completions");
        assert_eq!(arr[0]["apiKey"], "kw-ag-workbuddy-abcd");
        assert_eq!(arr[1]["vendor"], "Moonshot", "a later entry is untouched");
        assert_eq!(arr[1]["apiKey"], "sk-kimi");
    }

    #[test]
    fn workbuddy_empty_file_creates_one_entry() {
        let out =
            upsert_workbuddy_gateway("", "http://127.0.0.1:8317/v1/chat/completions", "k").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().expect("an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], PLACEHOLDER_MODEL_ID);
        assert_eq!(arr[0]["supportsToolCall"], true);
    }

    /// Enabling twice updates the row the first run wrote instead of taking
    /// over another one.
    #[test]
    fn workbuddy_second_takeover_updates_its_own_entry() {
        let url = "http://127.0.0.1:8317/v1/chat/completions";
        let once = upsert_workbuddy_gateway("[]", url, "kw-ag-workbuddy-1").unwrap();
        let twice = upsert_workbuddy_gateway(&once, url, "kw-ag-workbuddy-2").unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1, "not a second entry");
        assert_eq!(v[0]["apiKey"], "kw-ag-workbuddy-2");
    }
}
