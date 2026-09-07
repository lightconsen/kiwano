// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/types.rs
// Copied on 2026-09-07. Modified for Kiwano (only the `OptimizerConfig` block
// was carried over — it is the sole `types.rs` item cache_injector.rs needs).

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// 请求优化器配置
///
/// 存储在 settings 表中，key = "optimizer_config"
/// 仅对 Bedrock provider 生效（CLAUDE_CODE_USE_BEDROCK = "1"）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerConfig {
    /// 总开关（默认关闭，用户需手动启用）
    #[serde(default)]
    pub enabled: bool,
    /// Thinking 优化子开关（总开关开启后默认生效）
    #[serde(default = "default_true")]
    pub thinking_optimizer: bool,
    /// Cache 注入子开关（总开关开启后默认生效）
    #[serde(default = "default_true")]
    pub cache_injection: bool,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            thinking_optimizer: true,
            cache_injection: true,
        }
    }
}
