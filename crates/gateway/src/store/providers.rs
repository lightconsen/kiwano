//! Provider rows: CRUD, the extra per-protocol endpoint set, and the two
//! row readers the rest of the module reuses.

use crate::error::Result;
use crate::store::time::now_rfc3339;
use crate::store::types::{Billing, Protocol, Provider, ProviderEndpoint};
use crate::store::Store;
use rusqlite::{params, Connection, OptionalExtension};

impl Store {
    // ---- providers ------------------------------------------------------

    pub fn insert_provider(&self, p: &Provider) -> Result<()> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO providers (id, name, catalog_id, protocol, base_url, api_path, api_key,
                                    billing, period_limit, limit_unit, plan_query,
                                    plan_limits, timeout_secs, retries, headers,
                                    reset_period, enabled, created_at, updated_at, model_default,
                                    prices)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
                p.id,
                p.name,
                p.catalog_id,
                p.protocol.as_str(),
                p.base_url,
                p.api_path,
                p.api_key,
                p.billing.as_str(),
                p.period_limit,
                p.limit_unit,
                p.plan_query,
                p.plan_limits,
                p.timeout_secs,
                p.retries,
                p.headers,
                p.reset_period,
                p.enabled as i64,
                p.created_at,
                p.updated_at,
                p.model_default,
                p.prices,
            ],
        )?;
        write_endpoints(&tx, &p.id, &p.endpoints)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<Provider>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, protocol, base_url, api_path, api_key, billing,
                    period_limit, limit_unit, plan_query, plan_limits,
                    timeout_secs, retries, headers,
                    reset_period, enabled, created_at, updated_at, catalog_id,
                    model_default, prices
             FROM providers WHERE id = ?1",
        )?;
        let mut provider = stmt.query_row(params![id], provider_from_row).optional()?;
        if let Some(p) = &mut provider {
            p.endpoints = read_endpoints(&conn, &p.id)?;
        }
        Ok(provider)
    }

    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, protocol, base_url, api_path, api_key, billing,
                    period_limit, limit_unit, plan_query, plan_limits,
                    timeout_secs, retries, headers,
                    reset_period, enabled, created_at, updated_at, catalog_id,
                    model_default, prices
             FROM providers ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], provider_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            let mut p = row?;
            p.endpoints = read_endpoints(&conn, &p.id)?;
            out.push(p);
        }
        Ok(out)
    }

    /// Update an existing provider; refreshes `updated_at`. The additional
    /// endpoint set is replaced wholesale (add/update pass the full list).
    pub fn update_provider(&self, p: &Provider) -> Result<()> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let updated = if p.updated_at.is_empty() {
            now_rfc3339()
        } else {
            p.updated_at.clone()
        };
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE providers SET name = ?2, catalog_id = ?3, protocol = ?4, base_url = ?5,
                    api_path = ?6, api_key = ?7, billing = ?8, period_limit = ?9,
                    limit_unit = ?10, plan_query = ?11, plan_limits = ?12,
                    timeout_secs = ?13, retries = ?14, headers = ?15,
                    reset_period = ?16, enabled = ?17, updated_at = ?18, model_default = ?19,
                    prices = ?20
             WHERE id = ?1",
            params![
                p.id,
                p.name,
                p.catalog_id,
                p.protocol.as_str(),
                p.base_url,
                p.api_path,
                p.api_key,
                p.billing.as_str(),
                p.period_limit,
                p.limit_unit,
                p.plan_query,
                p.plan_limits,
                p.timeout_secs,
                p.retries,
                p.headers,
                p.reset_period,
                p.enabled as i64,
                updated,
                p.model_default,
                p.prices,
            ],
        )?;
        write_endpoints(&tx, &p.id, &p.endpoints)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM providers WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }
}

fn provider_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Provider> {
    let protocol_str: String = row.get(2)?;
    let billing_str: String = row.get(6)?;
    Ok(Provider {
        id: row.get(0)?,
        name: row.get(1)?,
        catalog_id: row.get(18)?,
        protocol: Protocol::parse_str(&protocol_str).unwrap_or(Protocol::Anthropic),
        base_url: row.get(3)?,
        api_path: row.get(4)?,
        endpoints: Vec::new(),
        api_key: row.get(5)?,
        billing: Billing::parse_str(&billing_str).unwrap_or(Billing::Metered),
        period_limit: row.get(7)?,
        limit_unit: row.get(8)?,
        plan_query: row.get(9)?,
        plan_limits: row.get(10)?,
        timeout_secs: row.get(11)?,
        retries: row.get(12)?,
        headers: row.get(13)?,
        reset_period: row.get(14)?,
        enabled: row.get::<_, i64>(15)? != 0,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
        model_default: row.get(19)?,
        prices: row.get(20)?,
    })
}

/// Additional per-protocol endpoints of one provider, ordered by protocol for
/// stable output.
fn read_endpoints(conn: &Connection, provider_id: &str) -> Result<Vec<ProviderEndpoint>> {
    let mut stmt = conn.prepare(
        "SELECT protocol, base_url, api_path FROM provider_endpoints
         WHERE provider_id = ?1 ORDER BY protocol",
    )?;
    let rows = stmt.query_map(params![provider_id], |row| {
        let protocol_str: String = row.get(0)?;
        Ok(ProviderEndpoint {
            protocol: Protocol::parse_str(&protocol_str).unwrap_or(Protocol::OpenAI),
            base_url: row.get(1)?,
            api_path: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Replace-all write of a provider's additional endpoints (inside the caller's
/// transaction; rows cascade away with the provider).
fn write_endpoints(
    tx: &rusqlite::Transaction<'_>,
    provider_id: &str,
    endpoints: &[ProviderEndpoint],
) -> Result<()> {
    tx.execute(
        "DELETE FROM provider_endpoints WHERE provider_id = ?1",
        params![provider_id],
    )?;
    for e in endpoints {
        tx.execute(
            "INSERT INTO provider_endpoints (provider_id, protocol, base_url, api_path)
             VALUES (?1, ?2, ?3, ?4)",
            params![provider_id, e.protocol.as_str(), e.base_url, e.api_path],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{sample_provider, temp_store};
    use crate::store::types::Binding;

    /// v7: additional per-protocol endpoints round-trip with the provider row
    /// and cascade away on delete.
    #[test]
    fn provider_endpoints_roundtrip() {
        let (_dir, store) = temp_store();

        let mut p = sample_provider("p-dual", Protocol::OpenAI);
        p.endpoints = vec![ProviderEndpoint {
            protocol: Protocol::Anthropic,
            base_url: "https://api.example.com/anthropic".into(),
            api_path: None,
        }];
        store.insert_provider(&p).unwrap();

        let got = store.get_provider("p-dual").unwrap().unwrap();
        assert_eq!(got.endpoints.len(), 1);
        assert_eq!(got.endpoints[0].protocol, Protocol::Anthropic);
        assert_eq!(
            got.endpoints[0].base_url,
            "https://api.example.com/anthropic"
        );
        // list reads them too
        assert_eq!(store.list_providers().unwrap()[0].endpoints.len(), 1);

        // update replaces the whole set: making Anthropic primary moves the
        // extra endpoint over to OpenAI rather than appending a second row
        let mut updated = got.clone();
        updated.protocol = Protocol::Anthropic;
        updated.endpoints = vec![ProviderEndpoint {
            protocol: Protocol::OpenAI,
            base_url: "https://api.example.com/openai/v2".into(),
            api_path: None,
        }];
        store.update_provider(&updated).unwrap();
        let got = store.get_provider("p-dual").unwrap().unwrap();
        assert_eq!(got.protocol, Protocol::Anthropic);
        assert_eq!(got.endpoints.len(), 1);
        assert_eq!(got.endpoints[0].protocol, Protocol::OpenAI);
        assert_eq!(
            got.endpoints[0].base_url,
            "https://api.example.com/openai/v2"
        );

        // empty list clears every additional endpoint
        let mut cleared = got.clone();
        cleared.endpoints.clear();
        store.update_provider(&cleared).unwrap();
        assert!(store
            .get_provider("p-dual")
            .unwrap()
            .unwrap()
            .endpoints
            .is_empty());

        // rows cascade away with the provider
        let mut p2 = sample_provider("p-cascade", Protocol::OpenAI);
        p2.endpoints = vec![ProviderEndpoint {
            protocol: Protocol::Anthropic,
            base_url: "https://x.example.com/anthropic".into(),
            api_path: None,
        }];
        store.insert_provider(&p2).unwrap();
        store.delete_provider("p-cascade").unwrap();
        let conn = store.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM provider_endpoints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    /// v8: per-provider advanced forwarding settings (timeout / retries /
    /// custom headers) round-trip through insert, get, list, and update.
    #[test]
    fn provider_advanced_settings_roundtrip() {
        let (_dir, store) = temp_store();

        let mut p = sample_provider("p-adv", Protocol::OpenAI);
        p.timeout_secs = Some(120);
        p.retries = Some(2);
        p.headers = Some(r#"{"api-key":"azure-key"}"#.to_string());
        store.insert_provider(&p).unwrap();

        // get reads all three
        let got = store.get_provider("p-adv").unwrap().unwrap();
        assert_eq!(got.timeout_secs, Some(120));
        assert_eq!(got.retries, Some(2));
        assert_eq!(got.headers.as_deref(), Some(r#"{"api-key":"azure-key"}"#));
        // list reads them too (everything here is positional SQL)
        let listed = &store.list_providers().unwrap()[0];
        assert_eq!(listed.timeout_secs, Some(120));
        assert_eq!(listed.retries, Some(2));
        assert_eq!(
            listed.headers.as_deref(),
            Some(r#"{"api-key":"azure-key"}"#)
        );

        // update clears them (None = cleared, not "keep")
        let mut cleared = got.clone();
        cleared.timeout_secs = None;
        cleared.retries = None;
        cleared.headers = None;
        store.update_provider(&cleared).unwrap();
        let got = store.get_provider("p-adv").unwrap().unwrap();
        assert_eq!(got.timeout_secs, None);
        assert_eq!(got.retries, None);
        assert_eq!(got.headers, None);
    }

    #[test]
    fn provider_crud_roundtrip() {
        let (_dir, store) = temp_store();
        let p = sample_provider("p1", Protocol::Anthropic);
        store.insert_provider(&p).unwrap();

        let got = store.get_provider("p1").unwrap().expect("exists");
        assert_eq!(got, p);

        let mut updated = p.clone();
        updated.name = "renamed".to_string();
        updated.enabled = false;
        updated.updated_at = String::new(); // triggers fresh updated_at
        store.update_provider(&updated).unwrap();
        let got = store.get_provider("p1").unwrap().unwrap();
        assert_eq!(got.name, "renamed");
        assert!(!got.enabled);
        assert_ne!(got.updated_at, p.updated_at);

        let all = store.list_providers().unwrap();
        assert_eq!(all.len(), 1);

        // Duplicate insert must fail (primary key).
        assert!(store.insert_provider(&p).is_err());

        assert!(store.delete_provider("p1").unwrap());
        assert!(store.get_provider("p1").unwrap().is_none());
        assert!(!store.delete_provider("p1").unwrap());
    }

    #[test]
    fn deleting_provider_cascades_bindings() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("p", Protocol::Anthropic))
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store.delete_provider("p").unwrap();
        assert!(store.bindings_for_agent("claude").unwrap().is_empty());
    }
}
