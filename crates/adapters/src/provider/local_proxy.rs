//! Local proxy request overrides, applied after the route/protocol
//! transforms.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Local proxy request overrides applied after route/protocol transforms.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalProxyRequestOverrides {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl LocalProxyRequestOverrides {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body.is_none()
    }
}
