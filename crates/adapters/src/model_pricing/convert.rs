//! Currency conversion for display and for the gateway's limit check.
//!
//! Both planes need this arithmetic, so it lives in the adapter they share
//! rather than beside either caller: two copies would eventually disagree,
//! and a spending limit that disagrees with the total beside it is the
//! failure the shared `limits` module exists to prevent.
//!
//! A leaf: it reads the rates it is handed and nothing else.

use std::collections::HashMap;

/// Convert an amount between currencies using the document's rates
/// (`rates[currency]` = units per 1 USD; USD pivots).
///
/// An unknown currency returns the amount unchanged. There is no honest rate for
/// it, and passing the raw number through is at least the number the reader was
/// already looking at — inventing one would be worse than not converting.
///
/// This lives here rather than beside its callers because both planes need it:
/// the app converts for display, and the gateway converts to compare usage
/// against a spending limit. Two copies of this arithmetic would eventually
/// disagree, and a limit that disagrees with the total beside it is the failure
/// the shared `limits` module exists to prevent.
pub fn convert_amount(amount: f64, from: &str, to: &str, rates: &HashMap<String, f64>) -> f64 {
    if from == to {
        return amount;
    }
    let (Some(per_usd_from), Some(per_usd_to)) = (rates.get(from), rates.get(to)) else {
        return amount;
    };
    if *per_usd_from == 0.0 {
        return amount;
    }
    let usd = amount / per_usd_from;
    usd * per_usd_to
}

/// Sum per-currency cost buckets into one amount denominated in `to`.
///
/// The buckets are what `SUM(cost) GROUP BY cost_currency` returns, so a provider
/// whose usage spans two currencies arrives as two entries and has to be
/// converted before it is added: summing them raw adds USD to CNY at 1:1, which
/// silently turns a `¥50` limit into a ceiling of nothing in particular. A
/// provider reaches that state honestly — its own models are priced in its own
/// currency, but a model it does not price is billed from the general row, which
/// may be denominated in someone else's.
///
/// Rows with no currency at all are dropped rather than counted as zero: they
/// are unpriced, not free.
pub fn convert_cost_buckets(
    buckets: &[(Option<String>, f64)],
    to: &str,
    rates: &HashMap<String, f64>,
) -> f64 {
    // Folded from `0.0` rather than `.sum()`ed: Rust's `Sum` for floats starts
    // at `-0.0`, so an empty bucket list — a window whose rows are all unpriced
    // — yields a *negative* zero, and `{:.4}` prints its sign. `-0.0000` in a
    // column of costs reads as a bug because it looks like one.
    buckets
        .iter()
        .fold(0.0, |total, (currency, cost)| match currency {
            Some(c) => total + convert_amount(*cost, c, to, rates),
            None => total,
        })
}
