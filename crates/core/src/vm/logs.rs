//! Request logs: the list, one detail row, the CSV export, and the clear.

use crate::vm::e2s;
use kiwanod::store::{
    RequestLogDetail, RequestLogEntry, RequestLogExportRow, RequestLogFilter, Store, EXPORT_ROW_CAP,
};
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
/// `include_bodies` is the export's own choice, not a capture setting: bodies
/// are always recorded, and the file either carries them or does not. It also
/// picks the read — the bodyless one never touches `request_bodies` at all,
/// so a metadata-only export of a large log does not load every body.
pub fn export_request_logs_csv(
    store: &Store,
    path: &str,
    filter: RequestLogFilter<'_>,
    include_bodies: bool,
) -> Result<RequestLogExportVm, String> {
    // Ask for one row more than the cap will allow, so "exactly at the cap"
    // and "more than the cap" are distinguishable.
    let (mut rows, truncated) = if include_bodies {
        let rows = store
            .export_request_logs_with_bodies(filter, EXPORT_ROW_CAP + 1)
            .map_err(e2s)?;
        let truncated = rows.len() as i64 > EXPORT_ROW_CAP;
        (rows, truncated)
    } else {
        let rows = store
            .export_request_logs(filter, EXPORT_ROW_CAP + 1)
            .map_err(e2s)?;
        let truncated = rows.len() as i64 > EXPORT_ROW_CAP;
        (
            rows.into_iter()
                .map(RequestLogExportRow::from_entry)
                .collect(),
            truncated,
        )
    };
    rows.truncate(EXPORT_ROW_CAP as usize);
    crate::csv::write_csv(path, &rows, include_bodies).map_err(e2s)?;
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

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default(), false).unwrap();
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
            false,
        )
        .unwrap();
        assert_eq!(empty.rows_written, 0);
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
    }

    /// The export's body flag is the only place a body can be withheld, so it
    /// has to actually withhold one — and actually produce one.
    #[test]
    fn export_includes_bodies_only_when_asked() {
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

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default(), false).unwrap();
        assert_eq!(out.rows_written, 1);
        let without = std::fs::read_to_string(path).unwrap();
        assert!(!without.contains("request-body-marker"), "{without}");
        assert!(!without.contains("response-body-marker"), "{without}");
        assert!(!without.contains("request_body"), "no body column either");

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default(), true).unwrap();
        assert_eq!(out.rows_written, 1);
        let with = std::fs::read_to_string(path).unwrap();
        assert!(with.contains("request-body-marker"), "{with}");
        assert!(with.contains("response-body-marker"), "{with}");
        assert!(
            with.contains("request_body,response_body"),
            "header matches"
        );
    }
}
