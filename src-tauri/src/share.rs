//! 配置分享（spec §4.1 P1 配置分享）：一键配置方案的导出/导入。
//!
//! 文件格式 kiwano-config v1：
//! `providers`（gateway Provider 全行，含 api_key——本机备份/跨设备迁移用，
//! 分享给他人前请自行脱敏）+ `routes`（每 Agent 的策略与候选顺序）。
//!
//! 导入语义（合并而非覆盖）：按 `(name, base_url)` 匹配已有 Provider ——
//! 命中则本地行保留，仅在本地无 Key 且方案带 Key 时回填；未命中则新建
//! （id 重新生成，避免与本地冲突）。绑定经 导出 id → 最终 id 重映射后
//! 按序 upsert（priority = 顺序），策略行直接 upsert；未涉及的 Agent
//! 绑定不动。调用方负责触发 admin /reload。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use kiwano_gateway::store::{Binding, Provider, Store, StrategyType};

use crate::vm;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct ConfigShare {
    kiwano_config: u32,
    exported_at: String,
    providers: Vec<Provider>,
    routes: Vec<ShareRoute>,
}

#[derive(Serialize, Deserialize)]
struct ShareRoute {
    agent: String,
    strategy: String,
    config: Option<String>,
    /// 导出时的 provider id 顺序（即优先级）。
    candidates: Vec<String>,
}

/// 导入结果（前端提示用）。
#[derive(Serialize)]
pub struct ImportReport {
    pub providers_added: usize,
    pub providers_kept: usize,
    pub routes_applied: usize,
}

/// 导出当前全部 Provider 与 Agent 路由方案为可分享 JSON。
pub fn export_config(store: &Store) -> Result<String, String> {
    let providers = store.list_providers().map_err(|e| e.to_string())?;
    let mut routes = Vec::new();
    for agent in store.bound_agents().map_err(|e| e.to_string())? {
        let (strategy, config) = store
            .get_strategy(&agent)
            .map_err(|e| e.to_string())?
            .map(|s| (s.kind.as_str().to_string(), s.config))
            .unwrap_or_else(|| ("single".to_string(), None));
        let candidates = store
            .bindings_for_agent(&agent)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|b| b.provider_id)
            .collect();
        routes.push(ShareRoute {
            agent,
            strategy,
            config,
            candidates,
        });
    }
    let share = ConfigShare {
        kiwano_config: FORMAT_VERSION,
        exported_at: vm::rfc3339(vm::unix_now()),
        providers,
        routes,
    };
    serde_json::to_string_pretty(&share).map_err(|e| e.to_string())
}

/// 导入方案（语义见模块注释）；返回计数报告。
pub fn import_config(store: &Store, json: &str) -> Result<ImportReport, String> {
    let share: ConfigShare =
        serde_json::from_str(json).map_err(|e| format!("不是有效的 Kiwano 配置文件: {e}"))?;
    if share.kiwano_config != FORMAT_VERSION {
        return Err(format!("不支持的配置版本 {}", share.kiwano_config));
    }

    let now = vm::rfc3339(vm::unix_now());
    // (name, base_url) → 本地 id
    let mut by_identity: HashMap<(String, String), String> = HashMap::new();
    for p in store.list_providers().map_err(|e| e.to_string())? {
        by_identity.insert((p.name.clone(), p.base_url.clone()), p.id.clone());
    }
    let mut remap: HashMap<String, String> = HashMap::new();
    let mut added = 0usize;
    let mut kept = 0usize;

    for sp in &share.providers {
        let key = (sp.name.clone(), sp.base_url.clone());
        if let Some(local_id) = by_identity.get(&key).cloned() {
            // 本地已有：仅回填缺失 Key（其余字段以本地为准）
            if sp.api_key.as_deref().is_some_and(|k| !k.is_empty()) {
                if let Some(mut local) = store.get_provider(&local_id).map_err(|e| e.to_string())? {
                    if local.api_key.as_deref().unwrap_or("").is_empty() {
                        local.api_key = sp.api_key.clone();
                        local.updated_at = now.clone();
                        store.update_provider(&local).map_err(|e| e.to_string())?;
                    }
                }
            }
            remap.insert(sp.id.clone(), local_id);
            kept += 1;
        } else {
            let new_id = format!(
                "{}-{}",
                vm::slug(&sp.name),
                &uuid::Uuid::new_v4().simple().to_string()[..6]
            );
            store
                .insert_provider(&Provider {
                    id: new_id.clone(),
                    name: sp.name.clone(),
                    protocol: sp.protocol,
                    base_url: sp.base_url.clone(),
                    api_path: sp.api_path.clone(),
                    api_key: sp.api_key.clone(),
                    billing: sp.billing,
                    period_limit: sp.period_limit,
                    reset_period: sp.reset_period.clone(),
                    enabled: sp.enabled,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                })
                .map_err(|e| e.to_string())?;
            by_identity.insert(key, new_id.clone());
            remap.insert(sp.id.clone(), new_id);
            added += 1;
        }
    }

    let mut routes_applied = 0usize;
    for r in &share.routes {
        let kind = StrategyType::from_str(&r.strategy).unwrap_or(StrategyType::Single);
        store
            .upsert_strategy(&r.agent, kind, r.config.as_deref())
            .map_err(|e| e.to_string())?;
        for (i, pid) in r.candidates.iter().enumerate() {
            let Some(final_id) = remap.get(pid) else {
                continue; // 方案引用了文件外的 Provider → 跳过该候选
            };
            store
                .upsert_binding(&Binding {
                    agent: r.agent.clone(),
                    provider_id: final_id.clone(),
                    priority: i as i64,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(|e| e.to_string())?;
        }
        routes_applied += 1;
    }
    Ok(ImportReport {
        providers_added: added,
        providers_kept: kept,
        routes_applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwano_gateway::store::{Billing, Protocol};

    fn provider(id: &str, name: &str, base_url: &str, api_key: Option<&str>) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            protocol: Protocol::OpenAI,
            base_url: base_url.into(),
            api_path: None,
            api_key: api_key.map(Into::into),
            billing: Billing::Metered,
            period_limit: None,
            reset_period: None,
            enabled: true,
            created_at: now_stamp(),
            updated_at: now_stamp(),
        }
    }

    fn now_stamp() -> String {
        vm::rfc3339(vm::unix_now())
    }

    #[test]
    fn export_roundtrip_and_merge_by_identity() {
        let src = Store::open_in_memory().unwrap();
        src.insert_provider(&provider("p1", "Alpha", "https://a.example.com", Some("sk-a")))
            .unwrap();
        src.insert_provider(&provider("p2", "Beta", "https://b.example.com", Some("sk-b")))
            .unwrap();
        src.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (pid, pr) in [("p1", 0), ("p2", 1)] {
            src.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: pid.into(),
                priority: pr,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let json = export_config(&src).unwrap();
        assert!(json.contains("kiwano_config"));

        // 空库导入 → 全部新建 + 路由重映射生效
        let dst = Store::open_in_memory().unwrap();
        let report = import_config(&dst, &json).unwrap();
        assert_eq!(report.providers_added, 2);
        assert_eq!(report.routes_applied, 1);
        assert_eq!(dst.list_providers().unwrap().len(), 2);
        assert!(dst.primary_provider_id("claude").unwrap().is_some());
        let bs = dst.bindings_for_agent("claude").unwrap();
        assert_eq!(bs.len(), 2);
        assert_eq!(dst.get_strategy("claude").unwrap().unwrap().kind, StrategyType::Failover);

        // 带本地同名同端点（无 Key）导入 → 保留本地 + 回填 Key
        let dst2 = Store::open_in_memory().unwrap();
        dst2.insert_provider(&provider("local-1", "Alpha", "https://a.example.com", None))
            .unwrap();
        let report2 = import_config(&dst2, &json).unwrap();
        assert_eq!(report2.providers_kept, 1);
        assert_eq!(report2.providers_added, 1);
        let local = dst2.get_provider("local-1").unwrap().unwrap();
        assert_eq!(local.api_key.as_deref(), Some("sk-a"));
        // claude 主选映射到本地 id
        assert_eq!(
            dst2.primary_provider_id("claude").unwrap().as_deref(),
            Some("local-1")
        );
    }

    #[test]
    fn import_rejects_garbage_and_unknown_versions() {
        let s = Store::open_in_memory().unwrap();
        assert!(import_config(&s, "not json").is_err());
        assert!(import_config(&s, r#"{"kiwano_config": 99}"#).is_err());
    }
}
