//! Client keys: mint, read, rotate, and what a key is allowed to do.
//!
//! The data plane's credential, managed. Two rules run through every function
//! here, and both are about what leaves this module:
//!
//! * a **read** never returns a secret. A key that is already pasted into an
//!   agent's config does not need to be shown again, and the masked form is
//!   enough to tell two rows apart;
//! * a **mutation names the key by its handle**, never by its value. The routes
//!   take `ck-…` for the same reason: an operator should not have to paste a live
//!   credential into a command line to edit it, and a secret in a URL is a secret
//!   in an access log.
//!
//! Everything else is the house rules for limits: the same units, the same
//! window boundary, and the same refusal to store a window of zero.

use crate::store::{ClientKey, ClientKeyLimit};
use kiwano_api::client_keys::{
    ClientKeyCreatedVm, ClientKeyLimitVm, ClientKeyPolicyInput, ClientKeySpendVm, ClientKeyVm,
    CurrencyAmountVm, NewClientKeyInput,
};
use kiwano_api::error::ApiError;

/// One stored key as a reader sees it: identity and policy, secret masked.
///
/// `last_used` is passed in rather than looked up here: a listing of N keys must
/// not become N queries, and the caller already has the one grouped answer
/// (`Store::client_key_last_used`).
fn key_vm(
    store: &crate::store::Store,
    key: ClientKey,
    last_used: Option<&str>,
    spend: Option<ClientKeySpendVm>,
) -> Result<ClientKeyVm, ApiError> {
    let limits = store
        .client_key_limits_for(&key.id)
        .map_err(ApiError::failed)?
        .into_iter()
        .map(|l| ClientKeyLimitVm {
            period: l.period,
            period_limit: l.period_limit,
            limit_unit: l.limit_unit,
        })
        .collect();
    Ok(ClientKeyVm {
        id: key.id,
        agent: key.agent,
        label: key.label,
        masked: kiwano_api::keys::mask_key(&key.key),
        model_allow: crate::limits::allowlist(key.model_allow.as_deref()),
        provider_allow: crate::limits::allowlist(key.provider_allow.as_deref()),
        limits,
        created_at: key.created_at,
        last_used_at: last_used.map(str::to_string),
        spend: spend.unwrap_or_default(),
    })
}

/// The per-key spend over `since`, folded from the store's per-currency rows.
///
/// Folding here rather than in SQL because the *shape* the reader wants is one
/// row per key with a currency list inside it, and because the unpriced count has
/// to survive the fold: a key whose rows could not all be costed understates, and
/// the caller is told by how much.
fn spend_by_key(
    store: &crate::store::Store,
    since: Option<&str>,
) -> Result<std::collections::HashMap<String, ClientKeySpendVm>, ApiError> {
    let rows = store.client_key_spend(since).map_err(ApiError::failed)?;
    let mut folded: std::collections::HashMap<String, ClientKeySpendVm> =
        std::collections::HashMap::new();
    for row in rows {
        let entry = folded.entry(row.key_id).or_default();
        entry.requests += row.requests;
        entry.tokens += row.tokens;
        entry.unpriced_rows += row.unpriced_rows;
        if let Some(currency) = row.currency {
            entry.cost.push(CurrencyAmountVm {
                currency,
                amount: row.cost,
            });
        }
    }
    for entry in folded.values_mut() {
        entry.cost.sort_by(|a, b| a.currency.cmp(&b.currency));
    }
    Ok(folded)
}

/// Every key, with what each spent over `since`.
///
/// `since` is the caller's window — the CLI's `--days` — and `None` is all time.
/// The figure travels with the window that produced it rather than being a bare
/// number: "spent 42" means nothing without knowing over what.
pub fn list_client_keys(
    store: &crate::store::Store,
    since: Option<&str>,
) -> Result<Vec<ClientKeyVm>, ApiError> {
    let last_used = store.client_key_last_used().map_err(ApiError::failed)?;
    let spend = spend_by_key(store, since)?;
    store
        .list_client_keys()
        .map_err(ApiError::failed)?
        .into_iter()
        .map(|k| {
            let at = last_used.get(&k.id).map(String::as_str);
            let spent = spend.get(&k.id).cloned();
            key_vm(store, k, at, spent)
        })
        .collect()
}

pub fn get_client_key(
    store: &crate::store::Store,
    id: &str,
    since: Option<&str>,
) -> Result<ClientKeyVm, ApiError> {
    let key = store
        .get_client_key(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("no client key `{id}`")))?;
    let last_used = store.client_key_last_used().map_err(ApiError::failed)?;
    let at = last_used.get(&key.id).map(String::as_str);
    let spent = spend_by_key(store, since)?.remove(&key.id);
    key_vm(store, key, at, spent)
}

/// Mint a key for an agent, with whatever policy came with it.
///
/// The agent is not checked against the registry: a key may be minted for an
/// agent the user defined itself, and a key for an agent with no provider bound
/// is a key whose requests will be refused with `no_provider_bound` — which says
/// exactly what is missing. Refusing the mint instead would mean the two
/// problems could not be told apart by their messages.
pub fn add_client_key(
    store: &crate::store::Store,
    input: &NewClientKeyInput,
) -> Result<ClientKeyCreatedVm, ApiError> {
    let agent = input.agent.trim();
    if agent.is_empty() {
        return Err(ApiError::invalid(
            "a key needs an agent: it is how its traffic routes",
        ));
    }
    // The same shape a takeover mints, so nothing downstream can tell where a
    // row came from.
    let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
    let key = format!("kw-ag-{agent}-{rand}");
    let id = store
        .upsert_client_key(&key, agent)
        .map_err(ApiError::failed)?;
    if let Some(label) = input.label.as_deref() {
        store
            .set_client_key_label(&id, Some(label.trim()))
            .map_err(ApiError::failed)?;
    }
    let (models, providers) = normalize_allowlists(&input.model_allow, &input.provider_allow);
    if models.is_some() || providers.is_some() {
        store
            .set_client_key_allowlists(&id, models.as_deref(), providers.as_deref())
            .map_err(ApiError::failed)?;
    }
    if !input.limits.is_empty() {
        let rows = limit_rows(&id, &input.limits);
        store
            .replace_client_key_limits(&id, &rows)
            .map_err(ApiError::failed)?;
    }
    Ok(ClientKeyCreatedVm {
        id,
        key,
        agent: agent.to_string(),
    })
}

pub fn delete_client_key(store: &crate::store::Store, id: &str) -> Result<bool, ApiError> {
    let removed = store.delete_client_key(id).map_err(ApiError::failed)?;
    if !removed {
        return Err(ApiError::not_found(format!("no client key `{id}`")));
    }
    Ok(true)
}

/// Give the key a new secret and keep everything else — including the spend
/// already recorded against its windows, which is the difference between
/// rotating a leaked key and resetting a budget.
pub fn rotate_client_key(
    store: &crate::store::Store,
    id: &str,
) -> Result<ClientKeyCreatedVm, ApiError> {
    let key = store
        .rotate_client_key(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("no client key `{id}`")))?;
    let row = store
        .get_client_key(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("no client key `{id}`")))?;
    Ok(ClientKeyCreatedVm {
        id: row.id,
        key,
        agent: row.agent,
    })
}

/// Replace a key's spend windows with exactly these; an empty list clears them.
pub fn set_client_key_limits(
    store: &crate::store::Store,
    id: &str,
    limits: Vec<ClientKeyLimitVm>,
) -> Result<(), ApiError> {
    if store
        .get_client_key(id)
        .map_err(ApiError::failed)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("no client key `{id}`")));
    }
    let rows = limit_rows(id, &limits);
    store
        .replace_client_key_limits(id, &rows)
        .map_err(ApiError::failed)
}

/// Replace a key's allowlists, and its label when one was sent.
///
/// Clearing is `None`, never `[]`: an empty list stored would read back as "no
/// restriction" anyway (see `limits::allowlist`), so writing one would be a lie
/// in the database that only shows up in a `sqlite3` session.
pub fn set_client_key_policy(
    store: &crate::store::Store,
    id: &str,
    input: &ClientKeyPolicyInput,
) -> Result<(), ApiError> {
    if store
        .get_client_key(id)
        .map_err(ApiError::failed)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("no client key `{id}`")));
    }
    let (models, providers) = normalize_allowlists(&input.model_allow, &input.provider_allow);
    store
        .set_client_key_allowlists(id, models.as_deref(), providers.as_deref())
        .map_err(ApiError::failed)?;
    if let Some(label) = input.label.as_deref() {
        let label = label.trim();
        store
            .set_client_key_label(id, (!label.is_empty()).then_some(label))
            .map_err(ApiError::failed)?;
    }
    Ok(())
}

/// Both allowlists as the JSON to store: trimmed, deduplicated case-insensitively,
/// and `None` — not `[]` — when nothing is left.
///
/// Case-insensitive dedup because model ids are matched that way everywhere else
/// (the declared-price path keys them the same), so storing `GPT-5.5` and
/// `gpt-5.5` as two entries would show a user two allowlist rows that behave as
/// one. The stored spelling is what the user typed the first time.
fn normalize_allowlists(
    models: &[String],
    providers: &[String],
) -> (Option<String>, Option<String>) {
    let clean = |xs: &[String]| -> Option<String> {
        let mut seen: Vec<String> = Vec::new();
        for x in xs {
            let x = x.trim();
            if x.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(x)) {
                continue;
            }
            seen.push(x.to_string());
        }
        (!seen.is_empty())
            .then(|| serde_json::to_string(&seen).unwrap_or_else(|_| "[]".to_string()))
    };
    (clean(models), clean(providers))
}

/// The windows to store, dropping the ones that mean nothing.
///
/// A window of zero is not a window of nothing: the evaluator reads it as "no
/// ceiling at all" (`limits::client_key_limit_usage`), so storing one would show
/// a limit that does not exist. Same filter, same reasoning as the agent
/// ceilings in `api::routes`.
fn limit_rows(key_id: &str, limits: &[ClientKeyLimitVm]) -> Vec<ClientKeyLimit> {
    let now = crate::store::now_rfc3339();
    limits
        .iter()
        .filter(|l| l.period_limit.is_finite() && l.period_limit > 0.0 && !l.period.is_empty())
        .map(|l| ClientKeyLimit {
            key_id: key_id.to_string(),
            period: l.period.clone(),
            period_limit: l.period_limit,
            limit_unit: l.limit_unit.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_agent(agent: &str) -> crate::store::Store {
        let s = crate::store::Store::open_in_memory().unwrap();
        s.upsert_strategy(agent, crate::store::StrategyType::Single, None)
            .unwrap();
        s
    }

    fn window(period: &str, limit: f64, unit: &str) -> ClientKeyLimitVm {
        ClientKeyLimitVm {
            period: period.into(),
            period_limit: limit,
            limit_unit: Some(unit.into()),
        }
    }

    #[test]
    fn a_listed_key_is_masked_and_names_its_windows() {
        let s = store_with_agent("claude");
        let created = add_client_key(
            &s,
            &NewClientKeyInput {
                agent: "claude".into(),
                model_allow: vec!["gpt-5.5".into(), "GPT-5.5".into(), " ".into()],
                provider_allow: vec![],
                label: Some("  office laptop ".into()),
                limits: vec![window("day", 100.0, "requests")],
            },
        )
        .unwrap();

        let listed = list_client_keys(&s, None).unwrap();
        assert_eq!(listed.len(), 1);
        let vm = &listed[0];
        assert_eq!(vm.id, created.id);
        assert_eq!(vm.agent, "claude");
        assert_eq!(vm.label.as_deref(), Some("office laptop"));
        // The secret is never in a read. The mask is a truncation with the
        // middle elided (`kiwano_api::keys::mask_key`), so the property worth
        // asserting here is that what comes back is strictly shorter and is not
        // the credential.
        assert_ne!(vm.masked, created.key);
        assert!(vm.masked.len() < created.key.len(), "{:?}", vm.masked);
        assert!(vm.masked.contains('…'));
        // One entry, spelled as the user typed it first: the duplicate in the
        // other case and the blank are both dropped.
        assert_eq!(vm.model_allow, vec!["gpt-5.5".to_string()]);
        assert!(vm.provider_allow.is_empty());
        assert_eq!(vm.limits.len(), 1);
        assert_eq!(vm.limits[0].period, "day");

        // And the clearing case writes NULL rather than an empty list.
        set_client_key_policy(
            &s,
            &created.id,
            &ClientKeyPolicyInput {
                model_allow: vec![],
                provider_allow: vec![],
                label: Some(String::new()),
            },
        )
        .unwrap();
        let after = list_client_keys(&s, None).unwrap();
        assert!(after[0].model_allow.is_empty());
        assert_eq!(after[0].label, None);
        let stored = s.get_client_key(&created.id).unwrap().unwrap();
        assert_eq!(stored.model_allow, None);
        assert_eq!(stored.label, None);
    }

    #[test]
    fn a_window_of_zero_is_not_stored() {
        let s = store_with_agent("claude");
        let created = add_client_key(
            &s,
            &NewClientKeyInput {
                agent: "claude".into(),
                model_allow: vec![],
                provider_allow: vec![],
                label: None,
                limits: vec![window("day", 0.0, "requests"), window("", 5.0, "requests")],
            },
        )
        .unwrap();
        assert!(s.client_key_limits_for(&created.id).unwrap().is_empty());
    }

    #[test]
    fn rotating_keeps_the_policy_and_hands_back_a_new_secret() {
        let s = store_with_agent("claude");
        let created = add_client_key(
            &s,
            &NewClientKeyInput {
                agent: "claude".into(),
                model_allow: vec!["gpt-5.5".into()],
                provider_allow: vec![],
                label: Some("laptop".into()),
                limits: vec![window("day", 100.0, "requests")],
            },
        )
        .unwrap();

        let rotated = rotate_client_key(&s, &created.id).unwrap();
        assert_eq!(rotated.id, created.id);
        assert_ne!(rotated.key, created.key);
        assert!(s.client_key_by_value(&created.key).unwrap().is_none());
        let row = s.get_client_key(&created.id).unwrap().unwrap();
        assert_eq!(row.key, rotated.key);
        assert_eq!(row.label.as_deref(), Some("laptop"));
        assert_eq!(row.model_allow.as_deref(), Some(r#"["gpt-5.5"]"#));
        assert_eq!(s.client_key_limits_for(&created.id).unwrap().len(), 1);
    }

    #[test]
    fn a_key_needs_an_agent_and_an_unknown_id_is_not_found() {
        let s = store_with_agent("claude");
        let err = add_client_key(
            &s,
            &NewClientKeyInput {
                agent: "  ".into(),
                model_allow: vec![],
                provider_allow: vec![],
                label: None,
                limits: vec![],
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("needs an agent"), "{err}");

        assert!(get_client_key(&s, "ck-nope", None).is_err());
        assert!(rotate_client_key(&s, "ck-nope").is_err());
        assert!(delete_client_key(&s, "ck-nope").is_err());
        assert!(set_client_key_limits(&s, "ck-nope", vec![]).is_err());
    }
}
