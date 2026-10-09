//! Request logs: the list, one detail row, the CSV export, and the clear.

use kiwanod::api::logs as daemon;
use kiwanod::store::{RequestLogDetail, RequestLogEntry, RequestLogFilter, Store};
use serde::Serialize;

// ── Request logs (request_logs + request_bodies, migration V5) ──

// The list payload is built by the daemon now (`kiwanod::api::logs`), so it
// lives there — it embeds a store row, which `kiwano-api` cannot name without
// depending on this crate's gateway (§10.6's rule applied to a type that
// carries one). Re-exported so `crate::vm::RequestLogListVm` still resolves.
pub use kiwanod::api::logs::RequestLogListVm;

pub fn list_request_logs(
    store: &Store,
    page: i64,
    page_size: i64,
    filter: RequestLogFilter<'_>,
) -> Result<RequestLogListVm, String> {
    daemon::list_request_logs(store, page, page_size, filter).map_err(|e| e.to_string())
}

/// What an export wrote. Stays here with the export itself: this is the half of
/// the logs module the client keeps (§7 batch 3), because the file it writes is
/// the user's own.
#[derive(Serialize)]
pub struct RequestLogExportVm {
    pub rows_written: usize,
    /// The slice was larger than `EXPORT_ROW_CAP`, so the file is short of it.
    pub truncated: bool,
}

/// Every log row the filter matches, as CSV — what the Logs card's export
/// writes. Unpaged, unlike `list_request_logs`: the page size is a display
/// concern and must not cap what lands in the file.
///
/// The bodies ride along, always. There is no metadata-only export: the log
/// exists so a request can be looked at later, and both ends of the trip — the
/// audit trail here and the file someone carries out — want the same payload,
/// so there was nothing for a switch to decide. The cost is that every export
/// now reads `request_bodies`, which the metadata-only read used to skip; that
/// is the trade this makes deliberately.
pub fn export_request_logs_csv(
    store: &Store,
    path: &str,
    filter: RequestLogFilter<'_>,
) -> Result<RequestLogExportVm, String> {
    // The rows and their spelling are the daemon's; the **file** is this side's,
    // because the path came from the user's save dialog (`migrate.local.md`
    // §10.17). The daemon hands back the CSV text and the two counts.
    let (csv, written, truncated) =
        daemon::export_request_logs_csv(store, filter).map_err(|e| e.to_string())?;
    write_request_log_export(path, &csv, written, truncated)
}

/// The file half of an export, on its own.
///
/// Split out because the two clients reach the CSV differently — this crate's
/// caller has a store, the CLI asks the daemon over the wire — and the half
/// they share is this one. Writing it twice would put the error message in two
/// places, which is where the two would eventually disagree about what went
/// wrong with the user's path.
pub fn write_request_log_export(
    path: &str,
    csv: &str,
    rows_written: usize,
    truncated: bool,
) -> Result<RequestLogExportVm, String> {
    std::fs::write(path, csv).map_err(|e| format!("cannot write {path}: {e}"))?;
    Ok(RequestLogExportVm {
        rows_written,
        truncated,
    })
}

/// Detail view (metadata + bodies); re-exported for the command signature.
pub use kiwanod::store::RequestLogDetail as RequestLogDetailVm;

/// Served by the daemon (`kiwanod::api::logs::get_request_log`).
pub fn get_request_log(store: &Store, id: i64) -> Result<Option<RequestLogDetail>, String> {
    daemon::get_request_log(store, id).map_err(|e| e.to_string())
}

/// Served by the daemon. Emptying a trail that is already empty is not an error.
pub fn clear_request_logs(store: &Store) -> Result<(), String> {
    daemon::clear_request_logs(store).map_err(|e| e.to_string())
}

// ── Credential-watch banner (spec: dlp findings surface in-app) ──

/// The newest credential-watch finding the user has not acknowledged — the
/// banner's poll. Served by the daemon, which holds the ack marker in the same
/// `app_settings` KV it keeps its admin token in, so neither function needs the
/// auxiliary connection any more.
pub fn check_credential_finding(store: &Store) -> Result<Option<RequestLogEntry>, String> {
    daemon::check_credential_finding(store).map_err(|e| e.to_string())
}

/// Banner dismissed or clicked: acknowledge that log id.
pub fn ack_credential_finding(store: &Store, id: i64) -> Result<(), String> {
    daemon::ack_credential_finding(store, id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::test_support::{seed_usage_rows, store};
    use crate::vm::time::{rfc3339, unix_now};
    use kiwanod::store::RequestLogFilter;

    #[test]
    fn export_writes_the_filtered_slice_as_csv() {
        let s = store();
        let now = unix_now();
        // seed_usage_rows writes the request_logs twin too, which is the table
        // the export reads.
        seed_usage_rows(&s, now - 90, 3, 1_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.csv");
        let path = path.to_str().unwrap();

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default()).unwrap();
        assert_eq!(out.rows_written, 3);
        assert!(!out.truncated);

        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.starts_with('\u{feff}'), "Excel needs the BOM");
        assert_eq!(text.lines().count(), 4, "a header and one line per row");
        assert!(
            text.starts_with("\u{feff}id,ts,method,path"),
            "header first"
        );
        assert!(text.contains("claude"), "the row's agent is in there");

        // The same filter the table gets: a slice with no rows writes a
        // header-only file rather than the whole table.
        let empty = export_request_logs_csv(
            &s,
            path,
            RequestLogFilter {
                agent: Some("codex"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(empty.rows_written, 0);
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
    }

    /// The export carries the bodies. There is no metadata-only shape to ask
    /// for any more, so the markers have to actually be in the file.
    #[test]
    fn the_export_carries_the_bodies() {
        let s = store();
        // One row, with markers that need no CSV quoting, so "is it in the
        // file" is a plain substring test.
        s.insert_request_log(&kiwanod::store::RequestLogNew {
            ts: rfc3339(unix_now() - 90),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: Some("claude".into()),
            attribution: Some("key".into()),
            provider_id: Some("demo-alpha".into()),
            model: Some("demo-model".into()),
            status_code: 200,
            error_kind: None,
            error_message: None,
            session_id: None,
            is_streaming: false,
            input_tokens: 10,
            output_tokens: 2,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms: Some(100),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: Some("request-body-marker".into()),
            response_body: Some("response-body-marker".into()),
            request_size: 19,
            response_size: 20,
            truncated: false,
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
            request_notes: None,
        })
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.csv");
        let path = path.to_str().unwrap();

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default()).unwrap();
        assert_eq!(out.rows_written, 1);
        let csv = std::fs::read_to_string(path).unwrap();
        assert!(csv.contains("request-body-marker"), "{csv}");
        assert!(csv.contains("response-body-marker"), "{csv}");
        assert!(csv.contains("request_body,response_body"), "header matches");
    }

    /// The banner's contract: an unacked finding is returned, acking hides it,
    /// and a *newer* finding shows again. Ids do the ordering.
    ///
    /// Kept on this side, through the wrappers, because it is the app's promise:
    /// what the banner does across polls and restarts. The ack marker now lives
    /// in the KV the daemon already writes its own settings to, so this no
    /// longer needs an auxiliary connection — that is the one change.
    #[test]
    fn credential_finding_hides_on_ack_and_returns_on_a_newer_one() {
        let s = store();
        let insert = |notes: Option<&str>| {
            let log = kiwanod::store::RequestLogNew {
                ts: rfc3339(unix_now()),
                method: "POST".into(),
                path: "/v1/messages".into(),
                query: None,
                agent: Some("claude".into()),
                attribution: None,
                provider_id: Some("demo-alpha".into()),
                model: Some("demo-model".into()),
                status_code: 200,
                error_kind: None,
                error_message: None,
                session_id: None,
                is_streaming: false,
                input_tokens: 10,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
                usage_missing: false,
                latency_ms: None,
                first_token_ms: None,
                request_headers: None,
                response_headers: None,
                request_body: None,
                response_body: None,
                request_size: 0,
                response_size: 0,
                truncated: false,
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
                request_notes: notes.map(str::to_string),
            };
            s.insert_request_log(&log).unwrap()
        };

        assert_eq!(check_credential_finding(&s).unwrap(), None);

        insert(Some("thinking: removed invalid value"));
        assert_eq!(
            check_credential_finding(&s).unwrap(),
            None,
            "shim notes are not findings"
        );

        let first = insert(Some("dlp: github-token ×1"));
        assert_eq!(
            check_credential_finding(&s).unwrap().map(|r| r.id),
            Some(first)
        );

        // A read is not an ack: polling again returns the same finding.
        assert_eq!(
            check_credential_finding(&s).unwrap().map(|r| r.id),
            Some(first),
            "the banner survives its own poll"
        );

        ack_credential_finding(&s, first).unwrap();
        assert_eq!(check_credential_finding(&s).unwrap(), None);

        let second = insert(Some("dlp: openai-key ×1"));
        assert_eq!(
            check_credential_finding(&s).unwrap().map(|r| r.id),
            Some(second),
            "a newer finding re-raises the banner"
        );
    }
}
