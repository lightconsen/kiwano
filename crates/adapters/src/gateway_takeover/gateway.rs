//! The entry we write: the provider id every additive agent files it
//! under, the label on it, and the placeholder model id a transform falls
//! back to when its config held no entry to copy a model name from.

/// Provider id used in every additive config for the local gateway entry.
pub const GATEWAY_PROVIDER_ID: &str = "kiwano-gateway";

pub(crate) const GATEWAY_LABEL: &str = "Kiwano Gateway";

/// The id an entry needs when it is created from nothing. The gateway forwards
/// model names verbatim, so this is a placeholder the user replaces with a
/// model their provider serves — only reachable when the agent's config held
/// no entry to copy from.
pub(crate) const PLACEHOLDER_MODEL_ID: &str = "kiwano";
