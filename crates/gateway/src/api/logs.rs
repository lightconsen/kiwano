//! The request-log audit trail: the paged list, one entry in full, the clear —
//! and the credential-watch banner that reads it.
//!
//! Moved here from `kiwano_core::vm::logs` when the daemon started serving it
//! (see this module's parent). What did **not** move, and why:
//!
//! - the CSV export stays in `kiwano-core` for now: it writes to a path the user
//!   picked in a save dialog, which is the client's business, and splitting the
//!   two halves is `migrate.local.md` §7 batch 3's work;
//! - `open_log_folder` reveals a directory in the OS file manager and is a
//!   statement about *this machine* — it stays on the client forever (§10.6).
//!
//! # The one signature that changed
//!
//! The two banner functions took an `&Aux` connection. The marker they read and
//! write — the last acknowledged finding id — lives in the `app_settings` KV,
//! which the daemon's own `Store` reads and writes for the admin token. Taking
//! `&Store` removes the daemon's need for a second connection type to reach a
//! table it already owns a handle on; the row, the key and the semantics are
//! unchanged, and the banner's contract test still runs.

use crate::store::{RequestLogDetail, RequestLogEntry, RequestLogFilter, Store};
use kiwano_api::error::ApiError;
use serde::{Deserialize, Serialize};

// ── Request logs (request_logs + request_bodies, migration V5) ──

/// A page of the audit trail, as the Logs screen reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogListVm {
    pub rows: Vec<RequestLogEntry>,
    pub total: i64,
}

pub fn list_request_logs(
    store: &Store,
    page: i64,
    page_size: i64,
    filter: RequestLogFilter<'_>,
) -> Result<RequestLogListVm, ApiError> {
    let (rows, total) = store
        .list_request_logs(page, page_size, filter)
        .map_err(ApiError::failed)?;
    Ok(RequestLogListVm { rows, total })
}

pub fn get_request_log(store: &Store, id: i64) -> Result<Option<RequestLogDetail>, ApiError> {
    store.get_request_log(id).map_err(ApiError::failed)
}

/// Empty the audit trail. Not an error when it is already empty: the caller
/// asked for a state, and that is the state.
pub fn clear_request_logs(store: &Store) -> Result<(), ApiError> {
    store
        .clear_request_logs()
        .map_err(ApiError::failed)
        .map(drop)
}

// ── Credential-watch banner (spec: dlp findings surface in-app) ──

/// The filtered slice as CSV text — the export's **content** half. The file is
/// the client's (it writes the path the user picked); what the rows are, and
/// how they are spelled, is the daemon's, because it is the side that holds
/// them (`migrate.local.md` §10.17).
///
/// Unpaged, unlike `list_request_logs`: the page size is a display concern and
/// must not cap what lands in the file. `truncated` says the slice was larger
/// than the cap, so the caller can say so rather than implying a complete file.
pub fn export_request_logs_csv(
    store: &Store,
    filter: RequestLogFilter<'_>,
) -> Result<(String, usize, bool), ApiError> {
    // Ask for one row more than the cap will allow, so "exactly at the cap"
    // and "more than the cap" are distinguishable.
    let mut rows = store
        .export_request_logs_with_bodies(filter, crate::store::EXPORT_ROW_CAP + 1)
        .map_err(ApiError::failed)?;
    let truncated = rows.len() as i64 > crate::store::EXPORT_ROW_CAP;
    rows.truncate(crate::store::EXPORT_ROW_CAP as usize);
    let written = rows.len();
    Ok((super::csv::to_csv(&rows), written, truncated))
}

/// The `app_settings` key holding the last log id the user acknowledged via the
/// banner (dismiss or click both acknowledge). Same KV family the cost alerts
/// dedup with.
const DLP_FINDING_ACKED_KEY: &str = "dlp_finding_acked";

/// The newest credential-watch finding the user has not yet acknowledged — what
/// the banner polls. Read-only: the banner stays up across polls (and restarts)
/// until the user explicitly acks, so nothing is marked here.
pub fn check_credential_finding(store: &Store) -> Result<Option<RequestLogEntry>, ApiError> {
    let Some(row) = store
        .latest_credential_finding()
        .map_err(ApiError::failed)?
    else {
        return Ok(None);
    };
    let acked: i64 = store
        .app_setting(DLP_FINDING_ACKED_KEY)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // Ids are rowids: strictly increasing, so `>` covers "a newer finding
    // arrived since the ack" without a list of acked ids.
    Ok((row.id > acked).then_some(row))
}

/// Acknowledge a finding: banner dismissed or clicked. The next poll hides it; a
/// finding with a higher log id shows again.
pub fn ack_credential_finding(store: &Store, id: i64) -> Result<(), ApiError> {
    store
        .set_app_setting(DLP_FINDING_ACKED_KEY, &id.to_string())
        .map_err(ApiError::failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A finding row with the notes the banner keys on. `dlp:` marks it a
    /// credential finding; anything else is a shim note and not one.
    fn finding(store: &Store, notes: Option<&str>) -> i64 {
        store
            .insert_request_log(&crate::store::RequestLogNew {
                ts: crate::store::now_rfc3339(),
                method: "POST".into(),
                path: "/v1/messages".into(),
                query: None,
                agent: Some("claude".into()),
                attribution: None,
                provider_id: Some("p-1".into()),
                model: Some("m".into()),
                status_code: 200,
                error_kind: None,
                error_message: None,
                session_id: None,
                is_streaming: false,
                input_tokens: 1,
                output_tokens: 1,
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
            })
            .unwrap()
    }

    /// Clearing is a state, not an event: clearing an empty trail is fine, and
    /// the banner has nothing left to read.
    #[test]
    fn clearing_empties_the_trail_and_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        finding(&store, Some("dlp: github-token ×1"));
        assert_eq!(
            list_request_logs(&store, 1, 50, Default::default())
                .unwrap()
                .total,
            1
        );

        clear_request_logs(&store).unwrap();
        assert_eq!(
            list_request_logs(&store, 1, 50, Default::default())
                .unwrap()
                .total,
            0
        );
        clear_request_logs(&store).unwrap();

        assert_eq!(check_credential_finding(&store).unwrap(), None);
        assert!(get_request_log(&store, 1).unwrap().is_none());
    }
}
