//! The small view rules the provider rows are built from — the display form of
//! an endpoint, the billing mapping, the declared currency.
//!
//! Moved here from `kiwano_core::vm::providers` so the daemon can build the
//! same `ProviderVm` the app shows. `ProviderVm`
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

/// The Hub-hosted brand mark of the catalog entry this provider belongs to, as
/// an absolute URL — or `None` when it belongs to none, or when the entry
/// carries no logo.
///
/// The entry is the one the provider is **linked** to (`catalog_id`), so the
/// picture and the price agree about which entry this row is: a provider whose
/// endpoint happens to look like a vendor's is not dressed as that vendor
/// unless the link says so. A row added before the catalog was ever synced has
/// no link yet, and then the endpoint is asked the same unambiguous question
/// the add path asks — a hand-typed provider is matched to its entry by URL.
///
/// The path is resolved against `hub_url` here rather than in the webview: the
/// catalog publishes it relative to the Hub, and both sides resolve it with
/// [`super::sync::hub_asset_url`] — the same rule, so the app and the shelf
/// cannot disagree about where a logo lives.
pub fn catalog_logo(p: &Provider, entries: &[CatalogEntryVm], hub_url: &str) -> Option<String> {
    let id = match p.catalog_id.as_deref() {
        Some(id) => id.to_string(),
        None => super::catalog::catalog_id_for(entries, p)?,
    };
    let path = entries.iter().find(|e| e.id == id)?.logo.as_deref()?;
    super::sync::hub_asset_url(hub_url, path).ok()
}

pub fn advanced_vm(p: &Provider) -> Option<ProviderAdvancedVm> {
    if p.timeout_secs.is_none() && p.retries.is_none() && p.headers.is_none() && p.proxy.is_none() {
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
        proxy: p.proxy.clone(),
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

/// One colour per id, distinct within the list: the id picks the starting slot
/// (so a provider keeps its colour while the roster holds still) and a taken
/// slot steps to the next free one. Only past eight entries do colours repeat.
/// Categorical colours for charts: spread around the hue wheel and held at a
/// lightness that reads on both themes.
const CHART_COLORS: [&str; 8] = [
    "#4D6BFE", // blue
    "#0F9D58", // green
    "#F55036", // orange-red
    "#9333EA", // purple
    "#0EA5E9", // cyan
    "#EAB308", // amber
    "#EC4899", // pink
    "#14B8A6", // teal
];

pub fn chart_palette(ids: &[String]) -> Vec<&'static str> {
    let mut taken = [false; CHART_COLORS.len()];
    ids.iter()
        .map(|id| {
            let h: u64 = id.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
            let mut i = (h as usize) % CHART_COLORS.len();
            for _ in 0..CHART_COLORS.len() {
                if !taken[i] {
                    break;
                }
                i = (i + 1) % CHART_COLORS.len();
            }
            taken[i] = true;
            CHART_COLORS[i]
        })
        .collect()
}

/// Token formatting, mirroring `src/lib/format.ts`.
pub fn fmt_tokens(v: i64) -> String {
    if v >= 1_000_000 {
        let m = v as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{}M", m.round() as i64)
        } else {
            format!("{}M", (m * 10.0).round() / 10.0)
        }
    } else if v >= 1_000 {
        format!("{}k", (v as f64 / 1_000.0).round() as i64)
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, endpoint: &str, logo: Option<&str>) -> CatalogEntryVm {
        let logo = logo
            .map(|l| format!(r#""logo":"{l}","#))
            .unwrap_or_default();
        serde_json::from_str(&format!(
            r#"{{"id":"{id}","name":"{id}","tag":"aggregate","rating":4,
                {logo}
                "billing":"payg","currency":"USD",
                "endpoints":[{{"protocol":"openai","endpoint":"{endpoint}"}}]}}"#
        ))
        .expect("catalog entry fixture")
    }

    fn provider(catalog_id: Option<&str>, endpoint: &str) -> Provider {
        use crate::store::Protocol;
        let mut p = crate::store::test_support::sample_provider("p", Protocol::OpenAI);
        p.catalog_id = catalog_id.map(str::to_string);
        p.base_url = endpoint.to_string();
        p
    }

    const HUB: &str = "https://hub.example/catalog.json";

    /// The mark comes from the entry the row is linked to, resolved against the
    /// Hub base — the same resolver `manifest.json` uses.
    #[test]
    fn a_linked_entry_lends_its_logo_as_an_absolute_url() {
        let entries = vec![entry(
            "commandcode",
            "https://api.commandcode.ai/provider/v1",
            Some("logos/commandcode.png"),
        )];
        let p = provider(
            Some("commandcode"),
            "https://api.commandcode.ai/provider/v1",
        );
        assert_eq!(
            catalog_logo(&p, &entries, HUB).as_deref(),
            Some("https://hub.example/logos/commandcode.png")
        );
    }

    /// A row added before the catalog was synced has no link yet; the endpoint
    /// answers the same unambiguous question the add path asks it.
    #[test]
    fn an_unlinked_row_is_matched_by_its_endpoint() {
        let entries = vec![entry(
            "deepseek",
            "https://api.deepseek.com",
            Some("logos/deepseek.svg"),
        )];
        let p = provider(None, "https://api.deepseek.com");
        assert_eq!(
            catalog_logo(&p, &entries, HUB).as_deref(),
            Some("https://hub.example/logos/deepseek.svg")
        );
    }

    /// The link wins over the URL: a row the user linked to one entry is not
    /// dressed as whichever entry its endpoint happens to resemble. And an
    /// entry with no published logo lends nothing, rather than a guess.
    #[test]
    fn the_link_decides_and_an_entry_without_a_logo_lends_nothing() {
        let entries = vec![
            entry(
                "one",
                "https://api.shared.example/v1",
                Some("logos/one.svg"),
            ),
            entry("two", "https://api.other.example/v1", None),
        ];
        // Linked to `two` — which publishes no logo — even though the endpoint
        // also matches `one`. The link is the answer, so there is no mark.
        let p = provider(Some("two"), "https://api.shared.example/v1");
        assert_eq!(catalog_logo(&p, &entries, HUB), None);

        // Nothing matches and nothing is linked: the letter avatar is next.
        let p = provider(None, "https://relay.internal");
        assert_eq!(catalog_logo(&p, &entries, HUB), None);
    }
}
