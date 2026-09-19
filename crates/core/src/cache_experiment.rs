//! The cache-shaping offline experiment (Features panel,
//! docs/request-logs-applications.md §3.1): would normalizing request bodies
//! — sorted JSON keys, volatile fields removed — make adjacent turns of a
//! session share a longer prefix, and therefore hit the provider's prompt
//! cache more often?
//!
//! Read-only by construction: it answers from the captured bodies and changes
//! nothing. The answer decides whether a forward-path normalizer is worth
//! building at all; shaping traffic on a hunch is how a gateway breaks
//! prompts.
//!
//! Metric: for each session's adjacent turn pair, the shared-prefix ratio of
//! the two request bodies (common prefix bytes over the shorter body). The
//! cache's question is exactly "how much of this prefix have I seen", so the
//! byte prefix is the honest proxy — tokenization differences are noise the
//! ratio mostly absorbs.

use std::collections::BTreeMap;

use kiwanod::store::RequestLogExportRow;
use serde::Serialize;

/// One adjacent-turn comparison, kept for the distribution.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairSample {
    /// Shared-prefix ratio of the raw bodies.
    pub raw: f64,
    /// …of the canonicalized bodies (keys sorted, whitespace canonical).
    pub canonical: f64,
    /// …canonicalized after the volatile-field whitelist (metadata dropped,
    /// tools sorted by name).
    pub stripped: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExperimentReport {
    pub days: i64,
    /// Sessions with at least one comparable pair.
    pub sessions: usize,
    pub pairs: usize,
    pub skipped_truncated: usize,
    pub skipped_no_body: usize,
    /// Pairs whose bodies did not parse as JSON — unnormalizable by design.
    pub skipped_unparseable: usize,
    /// Mean shared-prefix ratio over all pairs, per variant.
    pub raw_mean: f64,
    pub canonical_mean: f64,
    pub stripped_mean: f64,
    pub raw_median: f64,
    pub canonical_median: f64,
    pub stripped_median: f64,
    /// Share of pairs whose canonical prefix beats the raw one by ≥5 points —
    /// the pairs shaping would actually rescue.
    pub improved_share: f64,
}

/// The shared-prefix ratio of two strings: common prefix bytes over the
/// shorter length. 1.0 when one is a prefix of the other (or they are equal).
pub fn prefix_ratio(a: &str, b: &str) -> f64 {
    let common = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    common as f64 / a.len().min(b.len()).max(1) as f64
}

/// Canonicalize with the volatile-field whitelist applied first: `metadata`
/// (which agents stuff with per-turn state) dropped at the top level, and the
/// `tools` array sorted by name — a reorder carries no semantics and is the
/// classic prefix-breaker.
fn canonical_stripped(body: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("metadata");
        if let Some(tools) = obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
            tools.sort_by_key(|t| {
                t.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string()
            });
        }
    }
    Some(
        kiwano_adapters::proxy::json_canonical::canonical_json_string(
            &kiwano_adapters::proxy::json_canonical::canonicalize_value(value),
        ),
    )
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut sorted = xs.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

/// Run the experiment over an export window. Rows arrive newest-first from
/// the store; the session grouping re-sorts them oldest-first so "adjacent"
/// means what the cache saw.
pub fn run(days: i64, rows: &[RequestLogExportRow]) -> ExperimentReport {
    let mut sessions: BTreeMap<String, Vec<&RequestLogExportRow>> = BTreeMap::new();
    let mut skipped_truncated = 0;
    let mut skipped_no_body = 0;
    for row in rows {
        let Some(session) = row.entry.session_id.clone() else {
            continue; // not a skip we count: the row simply belongs to no session
        };
        if row.entry.truncated {
            skipped_truncated += 1;
            continue;
        }
        if row.request_body.is_none() {
            skipped_no_body += 1;
            continue;
        }
        sessions.entry(session).or_default().push(row);
    }

    let mut samples: Vec<PairSample> = Vec::new();
    let mut skipped_unparseable = 0;
    let mut comparable_sessions = 0;
    for (_, mut turns) in sessions {
        turns.sort_by_key(|r| r.entry.id);
        let mut paired = false;
        for pair in turns.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let (raw_a, raw_b) = (
                a.request_body.as_deref().expect("checked above"),
                b.request_body.as_deref().expect("checked above"),
            );
            let (Some(stripped_a), Some(stripped_b)) =
                (canonical_stripped(raw_a), canonical_stripped(raw_b))
            else {
                skipped_unparseable += 1;
                continue;
            };
            let canon_a =
                kiwano_adapters::proxy::json_canonical::canonicalize_json_string_if_parseable(
                    raw_a,
                );
            let canon_b =
                kiwano_adapters::proxy::json_canonical::canonicalize_json_string_if_parseable(
                    raw_b,
                );
            samples.push(PairSample {
                raw: prefix_ratio(raw_a, raw_b),
                canonical: prefix_ratio(&canon_a, &canon_b),
                stripped: prefix_ratio(&stripped_a, &stripped_b),
            });
            paired = true;
        }
        if paired {
            comparable_sessions += 1;
        }
    }

    let col = |pick: fn(&PairSample) -> f64| samples.iter().map(pick).collect::<Vec<_>>();
    let (raw, canonical, stripped) = (col(|s| s.raw), col(|s| s.canonical), col(|s| s.stripped));
    let improved = samples
        .iter()
        .filter(|s| s.canonical - s.raw >= 0.05)
        .count();
    ExperimentReport {
        days,
        sessions: comparable_sessions,
        pairs: samples.len(),
        skipped_truncated,
        skipped_no_body,
        skipped_unparseable,
        raw_mean: mean(&raw),
        canonical_mean: mean(&canonical),
        stripped_mean: mean(&stripped),
        raw_median: median(&raw),
        canonical_median: median(&canonical),
        stripped_median: median(&stripped),
        improved_share: if samples.is_empty() {
            0.0
        } else {
            improved as f64 / samples.len() as f64
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_ratio_counts_the_shared_prefix() {
        assert_eq!(prefix_ratio("abcdef", "abcxyz"), 0.5);
        assert_eq!(prefix_ratio("same", "same"), 1.0);
        assert_eq!(prefix_ratio("", "anything"), 0.0);
        // One string a full prefix of the other is a perfect score.
        assert_eq!(prefix_ratio("short", "shorter"), 1.0);
    }

    #[test]
    fn the_whitelist_drops_metadata_and_sorts_tools() {
        let body = r#"{"metadata":{"user_id":"turn-42"},"tools":[{"name":"Bash"},{"name":"Read"}],"messages":[]}"#;
        let stripped = canonical_stripped(body).unwrap();
        assert!(!stripped.contains("metadata"));
        assert!(stripped.find("Bash").unwrap() < stripped.find("Read").unwrap());
        // An unparseable body is unshapable, and the experiment must say so
        // rather than pretend the raw text is a measurement.
        assert!(canonical_stripped("{not json").is_none());
    }
}
