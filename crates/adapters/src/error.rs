// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/error.rs
// Copied on 2026-09-07. Modified for Kiwano (dropped the rusqlite `From` impl
// and the skill-error formatting helper to avoid unneeded dependencies).

use std::path::Path;
use std::sync::PoisonError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Native files changed after CC Switch last read them.
    #[error("conflicting concurrent change: {0}")]
    Conflict(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{context}: {source}")]
    IoContext {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse JSON in {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot serialize JSON: {source}")]
    JsonSerialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot parse TOML in {path}: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("cannot take the lock: {0}")]
    Lock(String),
    #[error("MCP validation failed: {0}")]
    McpValidation(String),
    #[error("{0}")]
    Message(String),
    #[error("HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("{zh} ({en})")]
    Localized {
        key: &'static str,
        zh: String,
        en: String,
    },
    #[error("database error: {0}")]
    Database(String),
    #[error("the OMO config file does not exist")]
    OmoConfigNotFound,
    #[error("every provider is circuit-broken; no channel is left")]
    AllProvidersCircuitOpen,
    #[error("no provider is configured")]
    NoProvidersConfigured,
}

impl AppError {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn json(path: impl AsRef<Path>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn toml(path: impl AsRef<Path>, source: toml::de::Error) -> Self {
        Self::Toml {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn localized(key: &'static str, zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Localized {
            key,
            zh: zh.into(),
            en: en.into(),
        }
    }
}

impl<T> From<PoisonError<T>> for AppError {
    fn from(err: PoisonError<T>) -> Self {
        Self::Lock(err.to_string())
    }
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The messages are English, and this is a ported crate where they arrived
    /// in another language: a guard rather than a habit, since every one of these
    /// is read by a user whose UI is English (and the localized half of the
    /// dictionary is not where they live).
    #[test]
    fn the_error_messages_are_english() {
        let errors = [
            AppError::Config("x".into()),
            AppError::InvalidInput("x".into()),
            AppError::Conflict("x".into()),
            AppError::Lock("x".into()),
            AppError::McpValidation("x".into()),
            AppError::Database("x".into()),
            AppError::OmoConfigNotFound,
            AppError::AllProvidersCircuitOpen,
            AppError::NoProvidersConfigured,
        ];
        for error in errors {
            let message = error.to_string();
            assert!(
                !message
                    .chars()
                    .any(|c| matches!(c, '\u{4e00}'..='\u{9fff}')),
                "{message:?} is not English"
            );
        }
    }
}
