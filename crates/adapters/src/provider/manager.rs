//! The provider manager: the config file's id-keyed provider map and its
//! current selection. `IndexMap`, not `HashMap` — this order is the file's.

use crate::provider::Provider;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Provider manager
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderManager {
    pub providers: IndexMap<String, Provider>,
    pub current: String,
}

impl ProviderManager {
    /// Get all providers
    pub fn get_all_providers(&self) -> &IndexMap<String, Provider> {
        &self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderManager;
    use crate::provider::Provider;
    use serde_json::json;

    #[test]
    fn provider_manager_get_all_providers_returns_map() {
        let mut manager = ProviderManager::default();
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            json!({ "env": {} }),
            None,
        );
        manager.providers.insert("provider-1".to_string(), provider);

        assert_eq!(manager.get_all_providers().len(), 1);
        assert!(manager.get_all_providers().contains_key("provider-1"));
    }
}
