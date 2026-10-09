//! The rules that turn a name into an id.
//!
//! Here because **both sides mint ids**: the app names a provider when it adds
//! one (`migrate.local.md` §6.1 — the client generates it, so a replayed add is
//! the same id and therefore the same provider), and the daemon mints a custom
//! agent's id from the label it was given. One rule, one place: two copies would
//! be two spellings of "what a slug is", and ids are what bindings, strategies,
//! keys and usage rows point at.

/// A name as an id stem: lowercase alphanumerics, everything else collapsed to
/// `-`, edges trimmed.
///
/// The empty fallback is `provider`, which is a name no id should ever be
/// derived from by accident — callers that are not naming a provider must use
/// [`agent_id_stem`], whose fallback is its own.
pub fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "provider".into()
    } else {
        trimmed.to_string()
    }
}

/// The id stem for a user-defined agent: the label's slug, or `custom` when the
/// label has nothing slug-able in it (an all-CJK name).
///
/// The check is on the label, not on `slug`'s output, so `slug`'s own fallback
/// cannot leak into an agent id — an agent called `长任务批处理` is `custom-…`,
/// not `provider-…`.
pub fn agent_id_stem(label: &str) -> String {
    if label.chars().any(|c| c.is_ascii_alphanumeric()) {
        slug(label)
    } else {
        "custom".to_string()
    }
}

/// A fresh id for a user-defined agent: the stem plus six hex characters.
///
/// The suffix is what makes a **label the user repeats** its own agent, which is
/// the behaviour the Apps screen documents ("same name, two agents"). It is also
/// what makes the id the identity of the *operation* rather than of the name:
/// whoever mints it decides how many agents a label produces, and a retry that
/// sends the id it already minted produces none (`migrate.local.md` §6.1).
pub fn mint_agent_id(label: &str) -> String {
    format!(
        "{}-{}",
        agent_id_stem(label),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    )
}

/// A fresh id for a provider: the slug plus six hex characters.
///
/// **The client mints it, which is what makes a retried add idempotent**
/// (`migrate.local.md` §6.1): nothing else in the request identifies the
/// operation — the name is just a label — and an id the daemon has already seen
/// is answered with the provider that exists. The suffix is what lets two
/// providers share a name, the same way a custom agent's id does.
pub fn mint_provider_id(name: &str) -> String {
    format!(
        "{}-{}",
        slug(name),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    )
}
