//! Billing mapping (UI plan/payg/unl ↔ DB subscription/metered/unlimited).
//!
//! `billing_to_db` is what a write path stores and `billing_to_ui` is what a row
//! shows; an unrecognized tag is an error rather than a silent `Metered`, which
//! is why the two are a total pair over the tags the UI can send.

use kiwanod::store::Billing;

/// Map a UI billing tag onto the store vocabulary. Unrecognized tags are an
/// error, never a silent `Metered` fallback (an unknown tag would otherwise
/// persist as a wrong billing mode and mis-shape the quota columns).
pub(crate) fn billing_to_db(ui: &str) -> Result<Billing, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed the plan-quota cache (Aux KV, same shape plan_quota.rs writes) so

    #[test]
    fn billing_mapping_roundtrip() {
        assert_eq!(billing_to_ui(billing_to_db("plan").unwrap()), "plan");
        assert_eq!(billing_to_ui(billing_to_db("payg").unwrap()), "payg");
        assert_eq!(billing_to_ui(billing_to_db("unl").unwrap()), "unl");
    }

    #[test]
    fn billing_to_db_rejects_unknown_tag() {
        // An unknown tag must never silently become payg/metered.
        let err = billing_to_db("per-token").unwrap_err();
        assert!(err.contains("per-token"), "{err}");
        assert!(err.contains("plan|payg|unl"), "{err}");
        assert!(billing_to_db("").is_err());
        assert!(billing_to_db("PAYG").is_err());

        // `both` is a tag we *know*, and still refuse: the catalog is telling us
        // the vendor charges two ways, and the local row holds one. The message
        // has to point at the choice rather than at the catalog — "unknown
        // billing" would send the reader looking for a data defect.
        let err = billing_to_db("both").unwrap_err();
        assert!(err.contains("resolved"), "{err}");
        assert!(err.contains("plan or payg"), "{err}");
        assert!(!err.contains("unknown"), "{err}");
    }
}
