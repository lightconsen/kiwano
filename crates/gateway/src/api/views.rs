//! The small view rules the provider rows are built from — the display form of
//! an endpoint, the billing mapping, the declared currency.
//!
//! Moved here from `kiwano_core::vm::providers` so the daemon can build the
//! same `ProviderVm` the app shows (`migrate.local.md` §10.13). `ProviderVm`
//! itself stays in `kiwano-api` — it is a wire type — but the rules that fill
//! it are not, and this is the rule §10.10 applied: they live where they are
//! produced.
//!
//! **The `String` errors are deliberate.** These functions are also called from
//! paths that are still the client's, and a `String` is what those paths' error
//! type is. The daemon's `api::providers_add` maps them into [`ApiError`] at
//! its own boundary — the same split as `keys`: a rule returns what it can say,
//! the transport decides what kind that is.

use crate::store::{Billing, Provider};
use kiwano_api::providers::{ProviderAdvancedVm, ProviderEndpointVm};

use super::catalog::CatalogEntryVm;

pub fn display_endpoint(p: &Provider) -> String {
    display_base(&p.base_url, &p.api_path)
}

pub fn endpoint_note(p: &Provider) -> String {
    let mut note = protocol_label(p.protocol).to_string();
    // Additional endpoints surface in the same subtitle: "OpenAI-compatible · +Anthropic".
    for e in &p.endpoints {
        let tag = match e.protocol {
            crate::store::Protocol::OpenAI => "OpenAI",
            crate::store::Protocol::Anthropic => "Anthropic",
            crate::store::Protocol::Gemini => "Gemini",
        };
        note.push_str(" · +");
        note.push_str(tag);
    }
    note
}

pub fn vm_endpoints(p: &Provider) -> Vec<ProviderEndpointVm> {
    p.endpoints
        .iter()
        .map(|e| ProviderEndpointVm {
            protocol: e.protocol.as_str().to_string(),
            endpoint: display_base(&e.base_url, &e.api_path),
        })
        .collect()
}

pub fn provider_currency(p: &Provider, entries: &[CatalogEntryVm]) -> String {
    if let Some(c) = p
        .prices
        .as_deref()
        .and_then(|s| {
            serde_json::from_str::<kiwano_adapters::model_pricing::DeclaredPrices>(s).ok()
        })
        .map(|d| d.currency)
        .filter(|c| !c.is_empty())
    {
        return c;
    }
    p.catalog_id
        .as_deref()
        .and_then(|id| entries.iter().find(|e| e.id == id))
        .map(|e| e.currency.clone())
        .unwrap_or_else(|| "USD".to_string())
}

pub fn advanced_vm(p: &Provider) -> Option<ProviderAdvancedVm> {
    if p.timeout_secs.is_none() && p.retries.is_none() && p.headers.is_none() {
        return None;
    }
    let headers = p
        .headers
        .as_deref()
        .and_then(|raw| {
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(raw).ok()
        })
        .unwrap_or_default();
    Some(ProviderAdvancedVm {
        timeout_secs: p.timeout_secs,
        retries: p.retries,
        headers,
    })
}

pub fn billing_to_db(ui: &str) -> Result<Billing, String> {
    match ui {
        "plan" => Ok(Billing::Subscription),
        "unl" => Ok(Billing::Unlimited),
        "payg" => Ok(Billing::Metered),
        // Known, and still refused: the catalog says this vendor charges two
        // ways, and a local row holds one. A distinct message because "unknown
        // billing" would send whoever reads it looking for a broken catalog.
        "both" => {
            Err("billing \"both\" must be resolved to plan or payg before saving".to_string())
        }
        other => Err(format!(
            "unknown billing \"{other}\" (expected plan|payg|unl)"
        )),
    }
}

pub fn billing_to_ui(db: Billing) -> &'static str {
    match db {
        Billing::Subscription => "plan",
        Billing::Unlimited => "unl",
        Billing::Metered => "payg",
    }
}

fn display_base(base_url: &str, api_path: &Option<String>) -> String {
    let stripped = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    match api_path {
        Some(path) if !path.is_empty() => format!("{stripped}{path}"),
        _ => stripped.to_string(),
    }
}

pub fn protocol_label(p: crate::store::Protocol) -> &'static str {
    match p {
        crate::store::Protocol::OpenAI => "OpenAI-compatible",
        crate::store::Protocol::Anthropic => "Anthropic",
        crate::store::Protocol::Gemini => "Gemini",
    }
}
