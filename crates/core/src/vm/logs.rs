//! Request logs: the list, one detail row, the CSV export, and the clear.

use crate::vm::e2s;
use kiwanod::store::{RequestLogDetail, RequestLogEntry, RequestLogFilter, Store, EXPORT_ROW_CAP};
use serde::Serialize;

// ── Request logs (request_logs + request_bodies, migration V5) ──

#[derive(Serialize)]
pub struct RequestLogListVm {
    pub rows: Vec<RequestLogEntry>,
    pub total: i64,
}

pub fn list_request_logs(
    store: &Store,
    page: i64,
    page_size: i64,
    filter: RequestLogFilter<'_>,
) -> Result<RequestLogListVm, String> {
    let (rows, total) = store
        .list_request_logs(page, page_size, filter)
        .map_err(e2s)?;
    Ok(RequestLogListVm { rows, total })
}

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
    // Ask for one row more than the cap will allow, so "exactly at the cap"
    // and "more than the cap" are distinguishable.
    let mut rows = store
        .export_request_logs_with_bodies(filter, EXPORT_ROW_CAP + 1)
        .map_err(e2s)?;
    let truncated = rows.len() as i64 > EXPORT_ROW_CAP;
    rows.truncate(EXPORT_ROW_CAP as usize);
    crate::csv::write_csv(path, &rows).map_err(e2s)?;
    Ok(RequestLogExportVm {
        rows_written: rows.len(),
        truncated,
    })
}

/// Detail view (metadata + bodies); re-exported for the command signature.
pub use kiwanod::store::RequestLogDetail as RequestLogDetailVm;

pub fn get_request_log(store: &Store, id: i64) -> Result<Option<RequestLogDetail>, String> {
    store.get_request_log(id).map_err(e2s)
}

pub fn clear_request_logs(store: &Store) -> Result<(), String> {
    store.clear_request_logs().map_err(e2s).map(drop)
}

// ── Credential-watch banner (spec: dlp findings surface in-app) ──

/// The app_settings key holding the last log id the user acknowledged via the
/// banner (dismiss or click both acknowledge). Same KV family the cost alerts
/// dedup with.
const DLP_FINDING_ACKED_KEY: &str = "dlp_finding_acked";

/// The newest credential-watch finding the user has not yet acknowledged —
/// what the banner polls. Read-only: the banner stays up across polls (and
/// restarts) until the user explicitly acks, so nothing is marked here.
pub fn check_credential_finding(
    store: &Store,
    aux: &crate::auxiliary::Aux,
) -> Result<Option<RequestLogEntry>, String> {
    let Some(row) = store.latest_credential_finding().map_err(e2s)? else {
        return Ok(None);
    };
    let acked: i64 = aux
        .get_setting(DLP_FINDING_ACKED_KEY)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // Ids are rowids: strictly increasing, so `>` covers "a newer finding
    // arrived since the ack" without a list of acked ids.
    Ok((row.id > acked).then_some(row))
}

/// Acknowledge a finding: banner dismissed or clicked. The next poll hides it;
/// a finding with a higher log id shows again.
pub fn ack_credential_finding(aux: &crate::auxiliary::Aux, id: i64) -> Result<(), String> {
    aux.set_setting(DLP_FINDING_ACKED_KEY, &id.to_string())
        .map_err(e2s)
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
    #[test]
    fn credential_finding_hides_on_ack_and_returns_on_a_newer_one() {
        let s = store();
        let aux = crate::vm::test_support::linkless_aux();
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

        assert_eq!(check_credential_finding(&s, &aux).unwrap(), None);

        insert(Some("thinking: removed invalid value"));
        assert_eq!(
            check_credential_finding(&s, &aux).unwrap(),
            None,
            "shim notes are not findings"
        );

        let first = insert(Some("dlp: github-token ×1"));
        assert_eq!(
            check_credential_finding(&s, &aux).unwrap().map(|r| r.id),
            Some(first)
        );

        // A read is not an ack: polling again returns the same finding.
        assert_eq!(
            check_credential_finding(&s, &aux).unwrap().map(|r| r.id),
            Some(first),
            "the banner survives its own poll"
        );

        ack_credential_finding(&aux, first).unwrap();
        assert_eq!(check_credential_finding(&s, &aux).unwrap(), None);

        let second = insert(Some("dlp: openai-key ×1"));
        assert_eq!(
            check_credential_finding(&s, &aux).unwrap().map(|r| r.id),
            Some(second),
            "a newer finding re-raises the banner"
        );
    }
}
