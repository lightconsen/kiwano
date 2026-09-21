//! Template dispatch: the table mapping a `plan_query` template to its
//! adapter, the per-template credential validators, and the price hints.

use crate::plan_quota::http::client;
use crate::plan_quota::kimi::query_kimi;
use crate::plan_quota::minimax::query_minimax;
use crate::plan_quota::opencode_go::query_opencode_go;
use crate::plan_quota::types::QuotaOutcome;
use crate::plan_quota::volcengine::query_volcengine;
use crate::plan_quota::zenmux::query_zenmux;
use crate::plan_quota::zhipu::{query_zhipu, query_zhipu_team};
use std::collections::HashMap;

fn field_str<'a>(fields: &'a HashMap<String, serde_json::Value>, key: &str) -> &'a str {
    fields
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
}

/// Curated monthly price hints per template (endpoints don't report the
/// subscription price). None = unknown.
pub fn plan_monthly_price(plan_query: Option<&str>) -> Option<String> {
    let raw = plan_query?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    match v.get("template")?.as_str()? {
        "opencode_go" => Some("$10/mo".to_string()),
        _ => None,
    }
}

/// Execute a template against its credentials. Outer `Err` = transient
/// network failure (frontend retries); inner failure becomes a
/// success:false report.
pub(crate) async fn run_template(
    template: &str,
    fields: &HashMap<String, serde_json::Value>,
    base_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    let client = client()?;
    match template {
        "kimi" => query_kimi(&client, api_key).await,
        "zhipu" => query_zhipu(&client, base_url, api_key).await,
        "zhipu_team" => {
            let org = field_str(fields, "organization_id");
            let project = field_str(fields, "project_id");
            if api_key.trim().is_empty() || org.is_empty() || project.is_empty() {
                Ok(QuotaOutcome::Failed(
                    "The Zhipu team plan needs the API key + org ID + project ID".to_string(),
                ))
            } else {
                query_zhipu_team(&client, api_key, org, project).await
            }
        }
        "minimax" => query_minimax(&client, base_url, api_key).await,
        "zenmux" => {
            let quota_url = field_str(fields, "quota_url");
            if quota_url.is_empty() {
                Ok(QuotaOutcome::Failed(
                    "Fill in the ZenMux usage endpoint URL".to_string(),
                ))
            } else {
                query_zenmux(&client, quota_url, api_key).await
            }
        }
        "opencode_go" => query_opencode_go(&client, api_key).await,
        "volcengine" => {
            let ak = field_str(fields, "access_key_id");
            let sk = field_str(fields, "secret_access_key");
            if ak.is_empty() || sk.is_empty() {
                Ok(QuotaOutcome::Failed(
                    "Volcengine Ark usage queries need the account AccessKey ID + Secret (not the inference API key)".to_string(),
                ))
            } else {
                query_volcengine(&client, base_url, ak, sk).await
            }
        }
        // Grok reports quota over an undocumented gRPC-web API — stubbed.
        "grok" => Ok(QuotaOutcome::Failed(
            "Grok plan queries are not supported yet (the gRPC-web API is not adapted)".to_string(),
        )),
        other => Ok(QuotaOutcome::Failed(format!(
            "Unknown plan query template: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn unknown_template_and_grok_fail_deterministically() {
        let fields = HashMap::new();
        match run_template("grok", &fields, "https://x.grok.com", "k")
            .await
            .unwrap()
        {
            QuotaOutcome::Failed(m) => assert!(m.contains("not supported yet")),
            _ => panic!("grok must fail deterministically"),
        }
        match run_template("whatever", &fields, "https://x", "k")
            .await
            .unwrap()
        {
            QuotaOutcome::Failed(m) => assert!(m.contains("Unknown plan query template")),
            _ => panic!("unknown template must fail"),
        }
    }

    #[tokio::test]
    async fn zenmux_and_volcengine_require_fields() {
        let empty = HashMap::new();
        match run_template("zenmux", &empty, "https://x", "k")
            .await
            .unwrap()
        {
            QuotaOutcome::Failed(m) => assert!(m.contains("ZenMux")),
            _ => panic!("zenmux needs quota_url"),
        }
        match run_template(
            "volcengine",
            &empty,
            "https://ark.cn-beijing.volces.com/api/coding",
            "",
        )
        .await
        .unwrap()
        {
            QuotaOutcome::Failed(m) => assert!(m.contains("AccessKey")),
            _ => panic!("volcengine needs AK/SK"),
        }
    }

    #[test]
    fn price_hint_only_for_known_templates() {
        let pq = json!({ "template": "opencode_go", "fields": {} }).to_string();
        assert_eq!(plan_monthly_price(Some(&pq)).as_deref(), Some("$10/mo"));
        let pq2 = json!({ "template": "kimi", "fields": {} }).to_string();
        assert_eq!(plan_monthly_price(Some(&pq2)), None);
        assert_eq!(plan_monthly_price(None), None);
        assert_eq!(plan_monthly_price(Some("not json")), None);
    }
}
