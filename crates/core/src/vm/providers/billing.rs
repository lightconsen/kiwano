//! Billing mapping (UI plan/payg/unl ↔ DB subscription/metered/unlimited).
//!
//! `billing_to_db` is what a write path stores and `billing_to_ui` is what a row
//! shows; an unrecognized tag is an error rather than a silent `Metered`, which
//! is why the two are a total pair over the tags the UI can send.

// `billing_to_db` moved to `kiwanod::api::views` — the daemon's `add_provider`
// holds the form to the same vocabulary, and a second copy of a total mapping is
// two vocabularies. The round-trip test below therefore calls the daemon's copy,
// which is the whole point of keeping it: the vocabulary is pinned, not copied.
// The implementation is the daemon's (`kiwanod::api::views`), which produces
// this field now — a second copy is how two vocabularies come to disagree
// (`migrate.local.md` §10.7). Re-exported so the paths here are unchanged, and
// the round-trip test below still pins the vocabulary by calling it.
pub use kiwanod::api::views::billing_to_ui;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_mapping_roundtrip() {
        use kiwanod::api::views::billing_to_db;
        assert_eq!(billing_to_ui(billing_to_db("plan").unwrap()), "plan");
        assert_eq!(billing_to_ui(billing_to_db("payg").unwrap()), "payg");
        assert_eq!(billing_to_ui(billing_to_db("unl").unwrap()), "unl");
    }

    #[test]
    fn billing_to_db_rejects_unknown_tag() {
        // An unknown tag must never silently become payg/metered.
        let err = kiwanod::api::views::billing_to_db("per-token").unwrap_err();
        assert!(err.contains("per-token"), "{err}");
        assert!(err.contains("plan|payg|unl"), "{err}");
        assert!(kiwanod::api::views::billing_to_db("").is_err());
        assert!(kiwanod::api::views::billing_to_db("PAYG").is_err());

        // `both` is a tag we *know*, and still refuse: the catalog is telling us
        // the vendor charges two ways, and the local row holds one. The message
        // has to point at the choice rather than at the catalog — "unknown
        // billing" would send the reader looking for a data defect.
        let err = kiwanod::api::views::billing_to_db("both").unwrap_err();
        assert!(err.contains("resolved"), "{err}");
        assert!(err.contains("plan or payg"), "{err}");
        assert!(!err.contains("unknown"), "{err}");
    }
}
