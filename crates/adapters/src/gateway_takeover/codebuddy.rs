//! CodeBuddy's model list — an object whose `models` array the picker's
//! `availableModels` list has to name.

use crate::gateway_takeover::gateway::PLACEHOLDER_MODEL_ID;
use crate::gateway_takeover::json::{parse_jsonc, require_object};
use crate::gateway_takeover::model_list::{
    new_entry, point_entry_at_gateway, takeover_target_index,
};
use serde_json::{json, Value};

// ── codebuddy (~/.codebuddy/models.json, an object with a `models` array) ──

/// Take over CodeBuddy's model list.
///
/// Same product family as WorkBuddy, different file shape: an object carrying
/// `models` plus the `availableModels` list the picker reads, and the same
/// full-endpoint `url`. The taken-over entry's id joins `availableModels` so
/// the row is selectable; an id already listed is not duplicated.
pub fn upsert_codebuddy_models_gateway(
    content: &str,
    url: &str,
    key: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "models.json")?;
    let mut obj = require_object(root, "models.json")?;
    if !obj.get("models").is_some_and(Value::is_array) {
        obj.insert("models".into(), json!([]));
    }

    let model_id = {
        let entries = obj["models"].as_array().expect("inserted above");
        let index = takeover_target_index(entries);
        let id = index
            .and_then(|i| entries[i].get("id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(PLACEHOLDER_MODEL_ID)
            .to_string();
        let entries = obj["models"].as_array_mut().expect("inserted above");
        match index {
            Some(i) => point_entry_at_gateway(&mut entries[i], url, key)?,
            None => entries.push(new_entry(&id, url, key)),
        }
        id
    };

    // The picker reads this list, not `models`: an id the picker does not know
    // is a row the user cannot choose.
    if !obj.get("availableModels").is_some_and(Value::is_array) {
        obj.insert("availableModels".into(), json!([]));
    }
    if let Some(list) = obj["availableModels"].as_array_mut() {
        if !list.iter().any(|m| m.as_str() == Some(model_id.as_str())) {
            list.push(json!(model_id));
        }
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway_takeover::model_list::GATEWAY_VENDOR;

    #[test]
    fn codebuddy_takes_over_the_first_entry_and_lists_it() {
        let original = r#"{
  "models": [
    { "id": "deepseek-v3", "name": "DeepSeek V3", "vendor": "DeepSeek",
      "apiKey": "sk-old", "url": "https://api.deepseek.com/v1/chat/completions" }
  ],
  "availableModels": []
}"#;
        let out = upsert_codebuddy_models_gateway(
            original,
            "http://127.0.0.1:8317/v1/chat/completions",
            "kw-ag-codebuddy-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(v["models"][0]["id"], "deepseek-v3");
        assert_eq!(v["models"][0]["vendor"], GATEWAY_VENDOR);
        assert_eq!(
            v["models"][0]["url"],
            "http://127.0.0.1:8317/v1/chat/completions"
        );
        assert_eq!(
            v["availableModels"][0], "deepseek-v3",
            "the picker lists availableModels, not models"
        );
    }

    #[test]
    fn codebuddy_empty_config_creates_both_sections() {
        let out =
            upsert_codebuddy_models_gateway("", "http://127.0.0.1:8317/v1/chat/completions", "k")
                .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["models"].as_array().unwrap().len(), 1);
        assert_eq!(v["models"][0]["id"], PLACEHOLDER_MODEL_ID);
        assert_eq!(v["availableModels"][0], PLACEHOLDER_MODEL_ID);
    }
}
