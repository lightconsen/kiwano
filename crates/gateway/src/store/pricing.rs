//! The two price sources the gateway costs a request with: the GUI's
//! `model_pricing` mirror of the Hub's table, and the prices a user declared
//! on a provider of their own.
//!
//! The gateway only reads them; `upsert_model_pricing` exists for a test to
//! seed the table it reads back.

use crate::error::Result;
use crate::store::Store;
use kiwano_adapters::model_pricing::{DeclaredPrices, ModelPriceEntry};
#[cfg(test)]
use rusqlite::params;

impl Store {
    /// The `model_pricing` mirror, written by the GUI's seeder from the Hub's
    /// models.json. The gateway resolves prices in memory, so this is what
    /// feeds a Hub price update into cost recording.
    ///
    /// An empty result means nothing is priced: the seeder has not run, or the
    /// Hub published a table that prices nothing. There is no snapshot behind
    /// this table to fall back to, so callers read it as it is.
    pub fn load_model_pricing(&self) -> Result<Vec<ModelPriceEntry>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT provider_id, model_id, display_name, input, output,
                    cache_read, cache_creation, currency, tiers
             FROM model_pricing",
        )?;
        let rows = stmt.query_map([], |r| {
            let tiers: Option<String> = r.get(8)?;
            let mut entry = ModelPriceEntry {
                long_context: None,
                provider_id: r.get(0)?,
                model_id: r.get(1)?,
                display_name: r.get(2)?,
                input: r.get(3)?,
                output: r.get(4)?,
                cache_read: r.get(5)?,
                cache_creation: r.get(6)?,
                currency: r.get(7)?,
                off_peak: None,
                peak_hours: None,
            };
            // A blob this build cannot read leaves the row at its listed rates
            // rather than failing the read: `resolve_pricing` answers an error
            // with an empty table, which would blank every cost.
            entry.apply_tiers(tiers.as_deref());
            Ok(entry)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

impl Store {
    /// The prices users declared for their own providers, keyed by the provider
    /// row's **own** `id`.
    ///
    /// The second price source, beside the Hub's mirror: a provider that names no
    /// catalog entry has no published prices to be costed at, and this is what
    /// its requests are costed with instead. It is read into its own in-memory
    /// table and never merged with the mirror's, so a declared row and a catalog
    /// row cannot shadow each other by string equality — `Provider.id` is
    /// `<slug>-<hex>` and `catalog_id` is a Hub slug, and the two namespaces are
    /// exactly what `Provider.catalog_id`'s doc comment keeps apart.
    ///
    /// A blob this build cannot read is skipped, not raised: the caller answers a
    /// read failure with an empty table, which would cost *every* provider its
    /// declared prices over one bad row.
    pub fn load_declared_prices(&self) -> Result<Vec<ModelPriceEntry>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, prices FROM providers WHERE prices IS NOT NULL AND prices <> ''",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            let Some(declared) = DeclaredPrices::parse(&blob) else {
                continue;
            };
            out.extend(declared.entries(&id));
        }
        Ok(out)
    }

    /// Test-only seeding hook. The GUI owns every production write to
    /// `model_pricing` (it shares the file through its own connection), so the
    /// gateway has no upsert of its own — but a test needs one to prove that
    /// the table it reads is the table that gets served.
    #[cfg(test)]
    pub fn upsert_model_pricing(&self, e: &ModelPriceEntry) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO model_pricing (provider_id, model_id, display_name, input, output,
                                        cache_read, cache_creation, currency, source, tiers)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'test', ?9)
             ON CONFLICT(provider_id, model_id) DO UPDATE SET
                display_name = ?3, input = ?4, output = ?5,
                cache_read = ?6, cache_creation = ?7, currency = ?8, tiers = ?9",
            params![
                e.provider_id,
                e.model_id,
                e.display_name,
                e.input,
                e.output,
                e.cache_read,
                e.cache_creation,
                e.currency,
                e.tiers_json(),
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::test_support::{sample_provider, temp_store};
    use crate::store::types::Protocol;

    /// The prices a user declared survive a round trip, and `load_declared_prices`
    /// hands them back keyed by the provider row's **own** id — which is the key
    /// the gateway costs that provider's requests by.
    #[test]
    fn provider_declared_prices_roundtrip() {
        let (_dir, store) = temp_store();

        let blob = r#"{"currency":"CNY","models":[{"model_id":"kimi-k2","input":"1.5","output":"6","cache_read":"0.15","cache_creation":"1.8"}]}"#;
        let mut p = sample_provider("p-own", Protocol::OpenAI);
        p.prices = Some(blob.to_string());
        store.insert_provider(&p).unwrap();

        let got = store.get_provider("p-own").unwrap().unwrap();
        assert_eq!(got.prices.as_deref(), Some(blob));
        assert_eq!(
            store.list_providers().unwrap()[0].prices.as_deref(),
            Some(blob)
        );

        let rows = store.load_declared_prices().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "p-own");
        assert_eq!(rows[0].model_id, "kimi-k2");
        assert_eq!(rows[0].currency, "CNY");
        assert_eq!(rows[0].input, "1.5");

        // A provider that declares none contributes no rows: the column is the
        // only thing that puts a provider in this table.
        store
            .insert_provider(&sample_provider("p-hub", Protocol::OpenAI))
            .unwrap();
        assert_eq!(store.load_declared_prices().unwrap().len(), 1);

        // Clearing the column clears them — this is what a provider leaving
        // pay-as-you-go does.
        let mut cleared = got.clone();
        cleared.prices = None;
        store.update_provider(&cleared).unwrap();
        assert!(store.load_declared_prices().unwrap().is_empty());

        // A blob this build cannot read is skipped rather than raised: one bad
        // row must not cost every provider its declared prices.
        let mut broken = cleared.clone();
        broken.prices = Some("{not json".to_string());
        store.update_provider(&broken).unwrap();
        assert!(store.load_declared_prices().unwrap().is_empty());
        assert_eq!(
            store.get_provider("p-own").unwrap().unwrap().prices,
            Some("{not json".to_string())
        );
    }
}
