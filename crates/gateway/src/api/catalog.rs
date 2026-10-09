//! The Hub catalog: the shelf the app adds providers from, and the endpoint
//! matching that links a provider back to the entry it came from.
//!
//! Moved here from `kiwano_core::vm::catalog` when the daemon started serving
//! `list_catalog` and `add_provider` needed the same snapshot (`migrate.local.md`
//! §10.12). The payload it reads is the daemon's table now (`store::hub`), which
//! is what made the move possible at all: the client-side copy read it through
//! `Aux`, a connection the daemon cannot open.
//!
//! The wire types live here rather than in `kiwano-api` for the reason §10.10
//! recorded: `CatalogPriceRefVm` embeds `kiwano_adapters::model_pricing` types,
//! and `kiwano-api`'s defining property is depending on nothing local. `kiwanod`
//! is the crate both sides already see; `kiwano-core` re-exports from here.
//!
//! What stayed on the client: `SyncReportVm` (the sync still runs in the app),
//! and `stored_key_for`, which is about what *this* machine has saved for an
//! endpoint the user is editing.

use crate::store::{Provider, Store};
use kiwano_api::error::ApiError;
use kiwano_api::logo::palette_color;
use serde::{Deserialize, Serialize};

/// Catalog-side billing vocabulary (`plan` | `payg` | `unl`).
///
/// Serialized as the bare lowercase tag, so the wire shape is unchanged. An
/// unrecognized tag is *not* silently coerced to `payg`: it is preserved
/// verbatim in `Other` and written back byte-identically, which keeps the hub
/// cache round-trip stable. One bad row must not fail a whole sync — the
/// catalog is the primary resource, and a sync that refuses a payload over one
/// odd tag leaves the shelf on whatever it had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum CatalogBilling {
    Plan,
    Payg,
    Unl,
    /// A vendor that charges both ways at **one address**: Anthropic sells an API
    /// (metered) and Pro/Max (subscription), and both answer on
    /// `api.anthropic.com`. A local provider row can only be one of them, so this
    /// value never reaches the database — the user settles it when adding, and
    /// [`billing_to_db`] refuses it with a message that says so.
    ///
    /// It has to be a tag of its own rather than a second catalog entry because
    /// [`catalog_id_for`] matches a local provider to an entry by endpoint and
    /// gives up when two entries share one: a second Anthropic entry would leave
    /// every hand-added Anthropic provider unlinked, and unlinked ones are priced
    /// from someone else's rates.
    Both,
    /// Unrecognized tag, kept verbatim for lossless round-tripping.
    Other(String),
}

impl CatalogBilling {
    pub fn as_str(&self) -> &str {
        match self {
            CatalogBilling::Plan => "plan",
            CatalogBilling::Payg => "payg",
            CatalogBilling::Unl => "unl",
            CatalogBilling::Both => "both",
            CatalogBilling::Other(raw) => raw,
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag (`Other` is what the
    /// `From<String>` conversion falls back to).
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(CatalogBilling::Plan),
            "payg" => Some(CatalogBilling::Payg),
            "unl" => Some(CatalogBilling::Unl),
            "both" => Some(CatalogBilling::Both),
            _ => None,
        }
    }
}

impl From<String> for CatalogBilling {
    fn from(raw: String) -> Self {
        CatalogBilling::parse_str(&raw).unwrap_or(CatalogBilling::Other(raw))
    }
}

impl From<CatalogBilling> for String {
    fn from(b: CatalogBilling) -> Self {
        b.as_str().to_string()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogEntryVm {
    pub id: String,
    pub name: String,
    /// Letter-avatar colour, derived at load time from `name` with the same
    /// palette the locally-added providers use. The Hub used to curate a brand
    /// colour per provider; those are not derivable (they were hand-picked), so
    /// dropping the field traded them for one consistent palette everywhere.
    #[serde(default)]
    pub logo_color: String,
    /// Hub-relative logo path ("logos/<id>.<ext>"); the frontend resolves it
    /// against hub_url. Absent while the setting loads, and in the bundled
    /// snapshot, which falls back to the letter avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// Primary endpoint protocol — derived at load time from the first entry of
    /// `endpoints`. The Hub publishes one list, primary first.
    #[serde(default)]
    pub protocol: String,
    /// Primary endpoint URL — derived at load time, see `protocol`.
    #[serde(default)]
    pub endpoint: String,
    /// The models the primary endpoint serves — derived at load time, see
    /// `protocol`. It seeds the add modal's model picker.
    #[serde(default)]
    pub models: Vec<String>,
    /// Every endpoint *after* the primary. The Hub publishes them together with
    /// the primary in one list; the split happens here because every screen
    /// reads the primary from its own fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<CatalogEndpointVm>,
    pub tag: String,
    pub rating: f64,
    /// One line of prose about the provider — what the card shows under its
    /// name. It supersedes the old price_line/price_note/users/blurb/free_offer
    /// fields; absent for the entries that have nothing to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// The price of the model that represents this provider, projected at build
    /// time from the data repo's `flagship` flag. Absent for a provider that
    /// prices no model at all (eleven of them), which falls back to `desc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_ref: Option<CatalogPriceRefVm>,
    /// The currency this provider bills in. Its price rows — and therefore its
    /// spending limit — are denominated in it. Catalogs published before the
    /// field existed fall back to USD, the price table's base currency.
    #[serde(default = "default_catalog_currency")]
    pub currency: String,
    /// The provider's own site (`https://deepseek.com/`). Published since the
    /// Hub started carrying it; nothing local can stand in for it, so the field
    /// only ever comes from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    /// How to read this provider's plan usage, when the vendor's own endpoint
    /// can answer with nothing but the provider's key: `{"template": "<id>"}`,
    /// and never anything else — credentials are the user's, and a catalog is
    /// public.
    ///
    /// Only four entries carry it. `billing == Plan` does **not** imply it: the
    /// other fourteen plan providers publish no such endpoint, or want a second
    /// credential. That distinction is the whole point of the field — it is what
    /// separates "bills by plan" from "can be asked how much of the plan is
    /// spent", and only the latter is worth offering a ceiling for.
    ///
    /// Declaring it here is not bookkeeping: `sync` re-serializes the parsed
    /// catalog before caching it, so a field this struct does not name is gone by
    /// the end of the first sync and no frontend can read it afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_query: Option<serde_json::Value>,
    /// Billing mode; unknown Hub tags survive as `CatalogBilling::Other`.
    pub billing: CatalogBilling,
    /// Derived at load time from the local provider list (same endpoint =
    /// added). The Hub does not publish it.
    #[serde(default)]
    pub added: bool,
}

/// One model's price, as shown for a provider on the Models list: the terms a
/// shopper compares before adding a provider. Figures are per million tokens in
/// TEXT decimal form, the same shape the price table uses — the frontend parses
/// them where it needs arithmetic.
#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogPriceRefVm {
    pub model_id: String,
    /// The model's display name, e.g. "Claude Opus 5". Shown beside the figures
    /// so a row does not read as a bare pair of numbers.
    pub display_name: String,
    pub input: String,
    pub output: String,
    /// ISO-4217 code these figures are denominated in — the provider's own
    /// currency, so the frontend converts rather than assumes.
    pub currency: String,
    /// The rates in force outside `peak_hours`, when the provider publishes a
    /// schedule. The figures above are then the **peak** ones, and the panel
    /// says so — a reader who is only shown the peak would compare the wrong
    /// number.
    ///
    /// The Hub copies these from the entry's flagship model and publishes the
    /// two together or not at all, so they are present exactly when the entry's
    /// flagship is priced by time of day.
    /// The rate band that applies once a request's input passes `over`, on the
    /// same terms. This projection and the mirror's row are two independent
    /// paths to the same figures — the shelf reads this one for its price cell
    /// and the dialog reads the mirror — so a band has to be declared on both or
    /// half the screens would show it and half would not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_peak: Option<kiwano_adapters::model_pricing::OffPeakRates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_hours: Option<kiwano_adapters::model_pricing::PeakHours>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<kiwano_adapters::model_pricing::LongContextRates>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogEndpointVm {
    pub protocol: String,
    pub endpoint: String,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CatalogListVm {
    pub total: i64,
    pub entries: Vec<CatalogEntryVm>,
}

/// Provider currency assumed when a catalog entry does not declare one — the
/// same default `generate.mjs` applies on the Hub side.
pub fn default_catalog_currency() -> String {
    "USD".into()
}

/// Fill what the Hub does not publish, and split the endpoint list.
///
/// The Hub sends source-of-truth data only: identity, classification, billing,
/// and one endpoint list with the primary first. Two things this view model
/// carries are the app's business, so they are derived on the way in — the
/// letter-avatar colour (one palette shared with locally-added providers), and
/// the primary endpoint hoisted into its own fields (`protocol` / `endpoint` /
/// `models`), which is the shape every screen reads.
///
/// Idempotent: a payload that already carries `endpoint` was normalized by an
/// older build — or was cached before the Hub switched to one endpoint list —
/// so its `endpoints` are extras and stay untouched.
fn normalize_catalog_entry(e: &mut CatalogEntryVm) {
    e.logo_color = palette_color(&e.name).to_string();
    if e.endpoint.is_empty() && !e.endpoints.is_empty() {
        let primary = e.endpoints.remove(0);
        e.protocol = primary.protocol;
        e.endpoint = primary.endpoint;
        e.models = primary.models;
    }
}

/// The cached Hub catalog, parsed and normalized.
///
/// One reader for every path that has to see the same entries: the shelf, and
/// the endpoint matching that links a provider to its entry and marks the
/// shelf's "already added" badge. Normalizing here is what makes the primary
/// endpoint matchable at all — `normalize_catalog_entry` is what moves it out
/// of `endpoints`, and a matcher that saw the raw list would be looking at the
/// extras only.
pub fn catalog_snapshot(store: &Store) -> CatalogListVm {
    // No cache, or a cache we cannot parse, is an empty shelf. A parse failure
    // used to fall back to the bundled copy, which meant a corrupted cache was
    // answered with stale data instead of being visibly broken; the next sync
    // repairs this either way.
    let mut list = store
        .hub_cache()
        .and_then(|(payload, _)| serde_json::from_str::<CatalogListVm>(&payload).ok())
        .unwrap_or(CatalogListVm {
            total: 0,
            entries: Vec::new(),
        });
    for e in &mut list.entries {
        normalize_catalog_entry(e);
    }
    list
}

/// Every endpoint key a local provider answers on: its base URL and each
/// additional per-protocol endpoint.
pub fn provider_endpoint_keys(p: &Provider) -> Vec<String> {
    std::iter::once(&p.base_url)
        .chain(p.endpoints.iter().map(|e| &e.base_url))
        .map(|u| endpoint_key(u))
        .collect()
}

/// Every endpoint key a catalog entry advertises, its primary first.
fn catalog_endpoint_keys(e: &CatalogEntryVm) -> Vec<String> {
    std::iter::once(&e.endpoint)
        .chain(e.endpoints.iter().map(|x| &x.endpoint))
        .map(|u| endpoint_key(u))
        .collect()
}

/// Does a local endpoint name the same place as a catalog one?
///
/// Equal keys, or whichever is shorter being a *path* prefix of the other: the
/// user typed a bare host (`api.deepseek.com`) where the entry names a deeper
/// path (`api.deepseek.com/anthropic`), or imported an agent config that
/// carries the path (`api.deepseek.com/v1`) where the entry names the bare
/// host. Same host, same service.
///
/// The `/` boundary is what keeps a host from swallowing its neighbours:
/// `api.deepseek.com` matches neither `api.deepseek.com.evil.com` nor
/// `api.deepseek.com:8443`. A non-default port staying distinct is deliberate —
/// a replica on another port is not the vendor's endpoint.
///
/// An empty key matches nothing: `strip_prefix("")` would otherwise make it a
/// prefix of every key.
pub fn endpoint_matches(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
    long.strip_prefix(short)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// The catalog entry a provider's endpoints point at, when exactly one does.
///
/// The provider's extra per-protocol endpoints are part of the set, mirroring
/// `added`: an entry whose second-protocol URL is what the user typed is the
/// same entry. Protocol itself is not part of the test — filtering on it would
/// let the badge and this link disagree about which entry a URL names.
///
/// Two or more matching entries resolve to `None` rather than to a guess: two
/// entries on one host mean the endpoint cannot tell them apart, and picking one
/// is the silent mis-pricing this exists to prevent. A provider that matches
/// nothing — self-hosted, an aggregator the Hub does not list — is the same
/// `None`, and that is an answer, not a failure.
pub fn catalog_id_for(entries: &[CatalogEntryVm], p: &Provider) -> Option<String> {
    let local = provider_endpoint_keys(p);
    let matching = |e: &&CatalogEntryVm| {
        catalog_endpoint_keys(e)
            .iter()
            .any(|cat| local.iter().any(|l| endpoint_matches(l, cat)))
    };
    let mut found = entries.iter().filter(matching);
    let first = found.next()?;
    if found.next().is_some() {
        return None;
    }
    Some(first.id.clone())
}

/// The shelf: the cached catalog, with each entry marked as already added when
/// a local provider answers on one of its endpoints.
///
/// The badge and the link answer the same question — "is this entry one the user
/// already has?" — so they match endpoints by the same rule. A provider whose
/// link was inferred is then not offered for adding a second time, which is what
/// it would otherwise be.
pub fn load_catalog(store: &Store) -> CatalogListVm {
    let local: Vec<String> = store
        .list_providers()
        .unwrap_or_default()
        .iter()
        .flat_map(provider_endpoint_keys)
        .collect();
    let mut list = catalog_snapshot(store);
    for e in &mut list.entries {
        let catalog = catalog_endpoint_keys(e);
        e.added = catalog
            .iter()
            .any(|cat| local.iter().any(|l| endpoint_matches(l, cat)));
    }
    list
}

pub fn endpoint_key(s: &str) -> String {
    let t = s.trim().to_lowercase();
    let no_scheme = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .unwrap_or(&t);
    no_scheme.trim_end_matches('/').to_string()
}

/// Fill in the catalog link on providers that have none.
///
/// The link is what prices a request at its own provider's rate, and it is
/// written when a provider is added from the shelf — and only then, so
/// everything hand-added, added from the CLI, imported, or present before the
/// column existed has none. With the Hub pricing per entry, an unlinked
/// provider is costed at *another* entry's rate, so the link is inferred from
/// the endpoint wherever that is unambiguous ([`catalog_id_for`]).
///
/// Only `NULL`s are considered: a link is a fact that decides a price, so it is
/// never re-derived, never overwritten, never cleared — the same rule
/// `update_provider` follows for an edit that carries no catalog id. The row's
/// own `updated_at` rides along, so a backfill does not read as a user edit.
///
/// Returns how many rows were linked: a caller with a running gateway needs to
/// know whether the daemon's in-memory copy just went stale.
pub fn link_providers(store: &Store) -> Result<usize, ApiError> {
    let list = catalog_snapshot(store);
    // Never synced: nothing to infer from, and nothing to write.
    if list.entries.is_empty() {
        return Ok(0);
    }
    let mut linked = 0;
    for mut p in store.list_providers().map_err(ApiError::failed)? {
        if p.catalog_id.is_some() {
            continue;
        }
        let Some(id) = catalog_id_for(&list.entries, &p) else {
            continue; // a custom endpoint, or an ambiguous one
        };
        p.catalog_id = Some(id);
        store.update_provider(&p).map_err(ApiError::failed)?;
        linked += 1;
    }
    Ok(linked)
}

/// The credential stored for `provider_id`, but **only** when `endpoint` is one
/// of the endpoints that provider already answers on.
///
/// The add/edit form deliberately keeps the key out of the webview: it shows a
/// masked placeholder and sends a blank when the user has not typed a new one
/// (blank = keep the stored key). So a Test or a Fetch pressed from the edit
/// dialog arrives with nothing to authenticate with, and both used to fail —
/// the probe reported `auth`, and the model list refused outright — for a
/// provider whose key was sitting right there.
///
/// The endpoint has to match, and that is the point rather than a nicety: the
/// URL field is editable, so reading the stored key for whatever address is in
/// the box would turn a Test button into a way to post someone's credential to a
/// host of the typer's choosing. Matching the stored endpoints keeps the button
/// meaning "does my saved configuration still work".
pub fn stored_key_for(store: &Store, provider_id: &str, endpoint: &str) -> Option<String> {
    let p = store.get_provider(provider_id).ok().flatten()?;
    let wanted = endpoint_key(endpoint);
    let known = provider_endpoint_keys(&p);
    known.contains(&wanted).then_some(p.api_key)?
}
