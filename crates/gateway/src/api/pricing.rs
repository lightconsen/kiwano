//! Model prices: the mirror the gateway charges with.
//!
//! One function, because this resource is one read — but it belongs here for the
//! same reason the rest do: the daemon is the side that serves it, and the client
//! keeps a wrapper so its call sites do not move (`migrate.local.md` §10.8).
//!
//! Its neighbour in the same screen, `currency_meta`, is **not** here: it reads
//! the app-scoped settings for the preferred currency and the rate table, which
//! is the half batch 2 owns — the same line `add_provider` and `list_catalog`
//! fall on (§10.9, §10.11).

use crate::store::Store;
use kiwano_adapters::model_pricing::ModelPriceEntry;
use kiwano_api::error::ApiError;

pub fn list_model_prices(store: &Store) -> Result<Vec<ModelPriceEntry>, ApiError> {
    store.load_model_pricing().map_err(ApiError::failed)
}
