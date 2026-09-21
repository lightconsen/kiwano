//! Per-request cost: the parsed rates, and the peak / off-peak pair.
//!
//! Two axes meet here. The length band is chosen by the request — see
//! `request_input` — and the time tier by the clock, through
//! `peak_hours::is_peak`. The band is settled first and both numbers of the
//! pair use it; inside a band the schedule has nothing of its own to
//! discount, so there the pair is equal by construction.
//!
//! `cache_inclusive` describes how the vendor reported the count, not how big
//! the request was, and it is read in opposite directions: `request_input`
//! *adds* the cache buckets to measure the prompt, `cost_of` *subtracts* them
//! to bill only the fresh part.

use crate::model_pricing::peak_hours::is_peak;
use crate::model_pricing::types::ModelPriceEntry;

/// Parse a per-million price string; invalid decimals yield no cost.
fn parse_price(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A fully parsed set of per-million rates.
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_creation: f64,
}

/// Parse a set of rates; `None` when the input or output rate is not a number.
fn rates_of(input: &str, output: &str, cache_read: &str, cache_creation: &str) -> Option<Rates> {
    Some(Rates {
        input: parse_price(input)?,
        output: parse_price(output)?,
        cache_read: parse_price(cache_read)?,
        cache_creation: parse_price(cache_creation)?,
    })
}

/// The input tokens a request *sent*, which is what a length band is measured
/// against.
///
/// `cache_inclusive` describes how the vendor reported the count, not how big the
/// request was: OpenAI style folds the cache buckets into `input_tokens`,
/// Anthropic style reports fresh input alone. A band's `over` is a statement
/// about the prompt's size, so both spellings have to land on the same number —
/// which is why this cannot be left to `cost_of`, whose job is billing the fresh
/// part.
fn request_input(input: u64, cache_read: u64, cache_creation: u64, cache_inclusive: bool) -> u64 {
    if cache_inclusive {
        input
    } else {
        input
            .saturating_add(cache_read)
            .saturating_add(cache_creation)
    }
}

/// The cost of `tokens` at `rates`, rounded to 6 decimal places.
fn cost_of(
    rates: &Rates,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
    cache_inclusive: bool,
) -> f64 {
    let billable_input = if cache_inclusive {
        input
            .saturating_sub(cache_read)
            .saturating_sub(cache_creation)
    } else {
        input
    };

    let million = 1_000_000f64;
    let total = (billable_input as f64 * rates.input
        + output as f64 * rates.output
        + cache_read as f64 * rates.cache_read
        + cache_creation as f64 * rates.cache_creation)
        / million;

    (total * 1e6).round() / 1e6
}

/// Compute the request cost (in the entry's currency) from token counts.
///
/// `cache_inclusive` mirrors cc-switch's `calculate_for_app` semantics:
/// OpenAI style `input_tokens` already contain the cache buckets and
/// must be reduced before billing at the input rate; Anthropic's are fresh
/// input only. Result rounded to 6 decimal places.
///
/// This is the **peak** price: the row's own rates. `compute_cost_pair` is what
/// a caller with a clock wants.
pub fn compute_cost(
    entry: &ModelPriceEntry,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    cache_inclusive: bool,
) -> Option<f64> {
    let rates = rates_of(
        &entry.input,
        &entry.output,
        &entry.cache_read,
        &entry.cache_creation,
    )?;
    Some(cost_of(
        &rates,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        cache_inclusive,
    ))
}

/// The cost at the tier `at` (unix seconds) falls in, paired with what the same
/// tokens would have cost at the row's **off-peak** rates.
///
/// The pair is one call because the second number is a property of the first:
/// when the request was already off-peak, or the row publishes no tiers, the two
/// are equal by construction — which is what makes a report of their difference
/// a plain sum over every priced row, with no tier flag to keep in step.
///
/// An `off_peak` whose rates do not parse means the row has no off-peak tier
/// (peak-only), never zero rates: a broken discount must not become a free one.
///
/// Two axes meet here — the length band, chosen by the request, and the time
/// tier, chosen by the clock. The band is settled first and both numbers use it;
/// inside a band the schedule has nothing of its own to discount, so there the
/// pair is equal by construction.
pub fn compute_cost_pair(
    entry: &ModelPriceEntry,
    at: i64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    cache_inclusive: bool,
) -> Option<(f64, f64)> {
    // Which length band the request falls in is a question about the *request*,
    // answered once: what it cost and what it would have cost off-peak are two
    // answers about the same tokens, so they cannot disagree about how long it
    // was. `over` is a statement about the size of the prompt, so the comparison
    // uses the tokens the request sent — see `request_input`.
    let band = entry.long_context.as_ref().filter(|lc| {
        request_input(
            input_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            cache_inclusive,
        ) > lc.over
    });

    let peak = match band {
        Some(lc) => rates_of(&lc.input, &lc.output, &lc.cache_read, &lc.cache_creation)?,
        None => rates_of(
            &entry.input,
            &entry.output,
            &entry.cache_read,
            &entry.cache_creation,
        )?,
    };
    let cost = |rates: &Rates| {
        cost_of(
            rates,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            cache_inclusive,
        )
    };
    // A band publishes one set of rates and no schedule of its own, so inside one
    // there is nothing to compare against: the band is flat and the pair's two
    // numbers come out equal — the shape a row with no tiers already has, and
    // what keeps the difference between them a pure time-of-day figure. Reading
    // the row's off-peak rates as the band's counterpart instead would invent a
    // discount on a rate the vendor never discounted.
    let off_peak = match band {
        Some(_) => None,
        None => entry
            .off_peak
            .as_ref()
            .and_then(|o| rates_of(&o.input, &o.output, &o.cache_read, &o.cache_creation)),
    };

    let billed = match (&entry.peak_hours, &off_peak) {
        // A schedule and something to discount: outside the windows the off-peak
        // rates are the ones that apply.
        (Some(hours), Some(off)) if !is_peak(hours, at) => cost(off),
        // Everything else bills at the listed rates — including a row whose
        // off-peak rates do not parse, where a broken discount must not become
        // a free one, and a row whose schedule is unreadable, where charging the
        // peak is the conservative direction the document itself prescribes.
        _ => cost(&peak),
    };
    let off_peak_cost = off_peak.as_ref().map_or(billed, cost);

    Some((billed, off_peak_cost))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_pricing::test_support::{banded_entry, tiered_entry, usage_entry};
    use crate::model_pricing::types::{LongContextRates, ModelsDoc, OffPeakRates};

    // ---- cost calculation (ported from calculator.rs) --------------------

    #[test]
    fn tiered_cost_picks_the_rate_in_force() {
        let e = tiered_entry();
        let peak_at = 1_788_919_200; // Wed 10:00 Beijing
        let off_at = 1_788_930_000; // Wed 13:00, the lunch gap
        let million = 1_000_000;

        let (peak_cost, peak_off) =
            compute_cost_pair(&e, peak_at, million, 0, 0, 0, false).unwrap();
        let (off_cost, off_off) = compute_cost_pair(&e, off_at, million, 0, 0, 0, false).unwrap();
        assert!((peak_cost - 9.0).abs() < 1e-9, "peak: {peak_cost}");
        assert!((off_cost - 4.5).abs() < 1e-9, "off-peak: {off_cost}");
        // The second element is the same answer either way — what matters for
        // the report is that it does not depend on when the request ran.
        assert!((peak_off - 4.5).abs() < 1e-9, "off-peak side: {peak_off}");
        assert_eq!(off_off, off_cost);
        // A row without tiers has no discount to state, so both are its own rate.
        let plain = usage_entry();
        let (cost, off) = compute_cost_pair(&plain, peak_at, million, 0, 0, 0, false).unwrap();
        assert_eq!(cost, off);
        assert_eq!(cost, compute_cost(&plain, million, 0, 0, 0, false).unwrap());
    }

    /// A discount nobody can read is not a discount: the row bills at its
    /// listed rates rather than at zero.
    #[test]
    fn unreadable_off_peak_rates_are_ignored() {
        let e = ModelPriceEntry {
            off_peak: Some(OffPeakRates {
                input: "abc".into(),
                output: "13.5".into(),
                cache_read: "0.15".into(),
                cache_creation: "0".into(),
            }),
            ..tiered_entry()
        };
        let off_at = 1_788_930_000;
        let (cost, off) = compute_cost_pair(&e, off_at, 1_000_000, 0, 0, 0, false).unwrap();
        assert!(
            (cost - 9.0).abs() < 1e-9,
            "billed at the listed rate: {cost}"
        );
        assert_eq!(cost, off);
    }

    /// The band is chosen by how big the request was, and `over` itself is still
    /// the cheap band: the document reads "the row's own rates apply **up to**
    /// `over`, the block's above it".
    #[test]
    fn a_long_request_bills_the_published_band() {
        let e = banded_entry();
        for (tokens, rate) in [(512_000_u64, 2.10), (512_001, 4.20), (1_000_000, 4.20)] {
            let (billed, _) = compute_cost_pair(&e, 1_788_919_200, tokens, 0, 0, 0, false).unwrap();
            let expected = rate * tokens as f64 / 1_000_000.0;
            assert!(
                (billed - expected).abs() < 1e-6,
                "{tokens} tokens billed {billed}, expected {expected}"
            );
        }
    }

    /// The threshold is about the size of the prompt, so the two reporting styles
    /// agree about it: OpenAI-style `input_tokens` already contain the cache
    /// buckets, Anthropic's count fresh input alone.
    #[test]
    fn the_band_threshold_counts_the_whole_prompt() {
        let e = banded_entry();
        // 500k fresh + 20k cached is 520k sent, whichever way it is reported.
        let openai = compute_cost_pair(&e, 1_788_919_200, 520_000, 0, 20_000, 0, true)
            .unwrap()
            .0;
        let anthropic = compute_cost_pair(&e, 1_788_919_200, 500_000, 0, 20_000, 0, false)
            .unwrap()
            .0;
        assert_eq!(openai, anthropic, "one request, one bill");
        // And it is the band's rate that ran, not the listed one.
        let expected = 500_000.0 * 4.20 / 1e6 + 20_000.0 * 0.84 / 1e6;
        assert!((openai - expected).abs() < 1e-6, "{openai}");
    }

    /// Inside a band the pair's two numbers are equal. The block carries one set
    /// of rates and no schedule, so the clock has nothing of its own to discount;
    /// reading the row's off-peak rates as the band's counterpart would invent a
    /// discount on a rate the vendor never discounted — and would turn the
    /// Dashboard's "peak premium" into a figure that also moves with request
    /// length.
    #[test]
    fn a_band_has_no_off_peak_of_its_own() {
        // The DeepSeek schedule, whose off-peak rates are exactly half, plus a band.
        let e = ModelPriceEntry {
            long_context: Some(LongContextRates {
                over: 512_000,
                input: "4.20".into(),
                output: "16.80".into(),
                cache_read: "0.84".into(),
                cache_creation: "0".into(),
            }),
            ..tiered_entry()
        };

        // Off-peak by the clock (Wed 13:00 Beijing, the lunch gap) and over the
        // threshold: the band's own rate, and nothing to compare it against.
        let (billed, off_peak_cost) =
            compute_cost_pair(&e, 1_788_930_000, 600_000, 0, 0, 0, false).unwrap();
        assert_eq!(billed, off_peak_cost);
        assert!((billed - 4.20 * 600_000.0 / 1e6).abs() < 1e-6, "{billed}");

        // Below the threshold the schedule is untouched: off-peak bills half.
        let (short_off, _) = compute_cost_pair(&e, 1_788_930_000, 100_000, 0, 0, 0, false).unwrap();
        assert!(
            (short_off - 4.5 * 100_000.0 / 1e6).abs() < 1e-6,
            "{short_off}"
        );
        let (short_peak, short_peak_off) =
            compute_cost_pair(&e, 1_788_919_200, 100_000, 0, 0, 0, false).unwrap();
        assert!(
            (short_peak - 9.0 * 100_000.0 / 1e6).abs() < 1e-6,
            "{short_peak}"
        );
        assert!(
            short_peak > short_peak_off,
            "the difference is the time of day"
        );
    }

    /// A block that omits a cache rate means that rate is zero — not that the row
    /// has no band. Making the field required would fail the whole block on the
    /// published shape above, and every long request would silently bill at the
    /// row's cheap rates.
    #[test]
    fn a_band_whose_cache_rates_are_absent_is_still_a_band() {
        let doc: ModelsDoc = serde_json::from_str(
            r#"{"version": 1, "exchange_rates": {"USD": 1.0}, "models": [
                {"model_id": "m", "display_name": "M", "input": "2.10", "output": "8.40",
                 "cache_read": "0.42", "cache_creation": "0", "currency": "CNY",
                 "long_context": {"over": 512000, "in": "4.20", "out": "16.80",
                                  "cache_read": "0.84"}}]}"#,
        )
        .expect("the published shape parses, cache_creation and all");
        let e = &doc.models[0];
        let band = e.long_context.as_ref().expect("the band survives");
        assert_eq!(band.cache_creation, "0", "an absent rate is no charge");
        assert_eq!(band.cache_read, "0.84");

        let (billed, _) = compute_cost_pair(e, 1_788_919_200, 600_000, 0, 0, 0, false).unwrap();
        assert!((billed - 4.20 * 600_000.0 / 1e6).abs() < 1e-6, "{billed}");
    }

    #[test]
    fn cost_anthropic_semantics_keeps_input() {
        let e = usage_entry();
        let cost = compute_cost(&e, 1000, 500, 200, 100, false).unwrap();
        // 0.003 + 0.0075 + 0.00006 + 0.000375 = 0.010935
        assert!((cost - 0.010935).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cost_openai_semantics_deducts_cache_buckets() {
        let e = usage_entry();
        let cost = compute_cost(&e, 1000, 500, 200, 100, true).unwrap();
        // billable input 700 -> 0.0021; total 0.010035
        assert!((cost - 0.010035).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cost_grokbuild_no_double_billing() {
        let e = ModelPriceEntry {
            input: "10".into(),
            output: "0".into(),
            cache_read: "1".into(),
            cache_creation: "0".into(),
            ..usage_entry()
        };
        let cost = compute_cost(&e, 1000, 0, 600, 0, true).unwrap();
        assert!((cost - 0.0046).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cost_invalid_price_is_none() {
        let e = ModelPriceEntry {
            input: "abc".into(),
            ..usage_entry()
        };
        assert!(compute_cost(&e, 1, 1, 0, 0, false).is_none());
    }
}
