//! Key-pool wire types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyVm {
    pub id: i64,
    /// Masked for display — never the key itself. The UI only ever needs to
    /// tell two entries apart, and every copy of the plaintext we do not hand
    /// out is one less copy sitting in a webview heap.
    pub masked: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// Display form of a key: a recognisable head and tail, never a usable secret.
///
/// The previous frontend-side masker printed the key verbatim when it was 12
/// characters or shorter, which is exactly the case where a mask matters most.
/// A masked key is a display string, not a credential — there is no input
/// length at which it hands the key back.
///
/// Here rather than in the app because both sides need it and only one of them
/// may hold the plaintext: the daemon masks what it serves, and a second copy of
/// the rule is how the two would come to disagree about what a reader sees.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let n = chars.len();
    if n > 12 {
        format!(
            "{}…{}",
            chars[..6].iter().collect::<String>(),
            chars[n - 4..].iter().collect::<String>()
        )
    } else if n > 4 {
        format!("…{}", chars[n - 4..].iter().collect::<String>())
    } else {
        "•".repeat(n)
    }
}

#[cfg(test)]
mod tests {
    use super::mask_key;

    #[test]
    fn mask_key_never_returns_the_whole_key() {
        assert_eq!(mask_key("sk-live-abcdefghijklmnop"), "sk-liv…mnop");
        // The frontend masker this replaced printed keys of 12 characters or
        // fewer verbatim — the case where masking matters most.
        assert_eq!(mask_key("123456789012"), "…9012");
        assert_eq!(mask_key("sk-short"), "…hort");
        assert_eq!(mask_key("abc"), "•••");
        assert_eq!(mask_key(""), "");
    }
}
