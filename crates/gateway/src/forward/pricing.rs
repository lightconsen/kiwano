//! What a metered sample costs: the user's declared table first, the Hub's
//! published one second.

use crate::forward::sample::UsageSample;
use crate::server::GatewayState;

/// Resolve the sample's price and compute its cost in the price entry's
/// currency, with the off-peak equivalent beside it. Unpriced / unknown models
/// yield `(None, None, None)` (row keeps NULL cost).
///
/// Two rungs, in this order:
///
/// 1. **What the user declared for this provider** (`providers.prices`), keyed by
///    the provider row's own id. It wins over the Hub's figures because it *is*
///    the user's statement about what this provider charges — and for a provider
///    the Hub publishes nothing for, it is the only statement anyone has made.
///    A miss here is a miss: the declared table is asked with `find_declared`,
///    which does not fall back to another provider's row, or a model the user
///    left out would be billed at a neighbour's rate.
/// 2. **The Hub's table**, asked for the *catalog* provider, since that is who
///    the Hub publishes prices for. A provider that never came from the shelf
///    asks for no provider and gets the general rate rather than nothing.
pub(crate) fn compute_sample_cost(
    state: &GatewayState,
    sample: &UsageSample,
) -> (Option<f64>, Option<f64>, Option<String>) {
    let Some(model) = sample.model.as_deref().filter(|m| !m.is_empty()) else {
        return (None, None, None);
    };
    let declared = state
        .declared
        .read()
        .expect("declared pricing lock poisoned");
    let pricing = state.pricing.read().expect("pricing lock poisoned");
    // The name the client asked for first, then **the name the reply said
    // answered**. A plan's own client asks for the plan's name (`kimi-for-coding`,
    // `moonshot-kimi-k3`), and those are not what the vendor's API sells — the
    // reply carries the sold name, and pricing it is the difference between a
    // number and a blank. Both other gateways price this way; the ledger still
    // records what was asked for.
    let mut entry = None;
    for name in std::iter::once(model).chain(
        sample
            .served_model
            .as_deref()
            .filter(|served| *served != model),
    ) {
        if let Some(found) = declared
            .find_declared(&sample.provider_id, name)
            .or_else(|| pricing.find(sample.catalog_id.as_deref().unwrap_or_default(), name))
        {
            entry = Some(found);
            break;
        }
    }
    let Some(entry) = entry else {
        return (None, None, None);
    };
    let pair = kiwano_adapters::model_pricing::compute_cost_pair(
        entry,
        sample.started_unix,
        sample.usage.input_tokens.max(0) as u64,
        sample.usage.output_tokens.max(0) as u64,
        sample.usage.cache_read_tokens.max(0) as u64,
        sample.usage.cache_creation_tokens.max(0) as u64,
        sample.cache_inclusive,
    );
    match pair {
        Some((cost, off_peak)) => (Some(cost), Some(off_peak), Some(entry.currency.clone())),
        None => (None, None, None),
    }
}
