//! Key-pool wire types.

use serde::Serialize;

#[derive(Serialize)]
pub struct ApiKeyVm {
    pub id: i64,
    /// Masked for display — never the key itself. The UI only ever needs to
    /// tell two entries apart, and every copy of the plaintext we do not hand
    /// out is one less copy sitting in a webview heap.
    pub masked: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}
