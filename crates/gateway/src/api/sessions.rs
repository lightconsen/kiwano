//! The merged session view: both sources read, keyed by session id.
//!
//! One seam for the whole notion. The CLI, the app's history screen and any
//! later reader see the same list, so the merge — which side wins per field,
//! what `"both"` means, how a project filter interacts with traffic that has no
//! project — is decided once, here, rather than re-derived per caller. The types
//! and the reasoning behind each field are in `kiwano_api::sessions`; this module
//! is the two reads and the join.
//!
//! The two halves are [`Store::list_sessions`] (the `sessions` table, written by
//! the import) and [`Store::traffic_sessions`] (the `request_logs` aggregate).
//! Neither is added to the other: see the VM's own doc for why.

use crate::store::sessions::{SessionRow, TrafficSession};
use crate::store::Store;
use kiwano_api::client_keys::CurrencyAmountVm;
use kiwano_api::error::ApiError;
use kiwano_api::history::{HistoryCount, HistoryToolUse, HistoryToolUses};
use kiwano_api::sessions::{SessionListVm, SessionTokens, SessionVm};
use std::collections::{HashMap, HashSet};

/// Every session this store can see, newest first.
///
/// `agent` and `since` bound both halves; `project` bounds only the imported
/// half — and, therefore, the whole view: a traffic-only session has no project,
/// so under a project filter it cannot be said to match and is dropped rather
/// than shown as if it did.
/// One session, by id. `None` when neither ledger has it.
///
/// Lives here rather than in a module of its own because [`merge`] is private to
/// this file — and `merge` is private because this is the only thing that should
/// ever combine the two ledgers.
///
/// **When two imported rows share the id** (two agents can be recorded against
/// one session id) this takes the first, which `Store::session_rows_by_id` sorts
/// by agent. The list shows both as separate rows; a page about "this session"
/// can only be about one, and the tie-break is stated at the pick rather than
/// left to a hash map's order.
pub fn get_session(store: &Store, session_id: &str) -> Result<Option<SessionVm>, ApiError> {
    let imported = store
        .session_rows_by_id(session_id)
        .map_err(ApiError::failed)?;
    let traffic = store
        .traffic_session_by_id(session_id)
        .map_err(ApiError::failed)?;
    match (imported.first(), traffic.as_ref()) {
        (None, None) => Ok(None),
        (first, traffic) => Ok(Some(merge(first, traffic))),
    }
}

/// One page of the merged list, with the total beside it.
///
/// The slice happens **after** the merge, because the merge is what decides the
/// order (newest first, across both ledgers) — paging either side separately
/// would produce a list that is not sorted. The cost is that the daemon computes
/// the whole merge to answer a page, which is a map lookup per session against
/// the cost of shipping a machine's whole history to a client that asked for ten
/// rows.
pub fn list_sessions_page(
    store: &Store,
    project: Option<&str>,
    since: Option<&str>,
    agents: Option<&[String]>,
    page: i64,
    page_size: i64,
) -> Result<SessionListVm, ApiError> {
    let rows = list_sessions(store, project, since, agents)?;
    let total = rows.len() as i64;
    let page = page.max(1);
    let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
    let offset = (page - 1).saturating_mul(page_size) as usize;
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .collect();
    Ok(SessionListVm { rows, total })
}

/// The largest page a client may ask for. A `page_size` of zero is clamped up to
/// one rather than read as "everything": the parameter exists to bound a
/// response, and a sentinel that defeated that would be read as a feature.
pub const MAX_PAGE_SIZE: i64 = 500;

pub fn list_sessions(
    store: &Store,
    project: Option<&str>,
    since: Option<&str>,
    agents: Option<&[String]>,
) -> Result<Vec<SessionVm>, ApiError> {
    let imported = store
        .list_sessions(None, project, since)
        .map_err(ApiError::failed)?;
    let traffic = store
        .traffic_sessions(None, since)
        .map_err(ApiError::failed)?;

    // The ids the imported half matched. Under a project filter this is also the
    // set of sessions the view is allowed to show at all.
    let known: HashSet<&str> = imported.iter().map(|r| r.session_id.as_str()).collect();
    let traffic: Vec<&TrafficSession> = traffic
        .iter()
        .filter(|t| project.is_none() || known.contains(t.session_id.as_str()))
        .collect();
    let traffic_by_id: HashMap<&str, &TrafficSession> = traffic
        .iter()
        .map(|t| (t.session_id.as_str(), *t))
        .collect();

    let mut rows: Vec<SessionVm> = Vec::with_capacity(imported.len().max(traffic.len()));
    for row in &imported {
        let traffic = traffic_by_id.get(row.session_id.as_str()).copied();
        rows.push(merge(Some(row), traffic));
    }
    // Traffic sessions the files never recorded — a session that ran entirely
    // after the takeover, or one the import's watermark skipped.
    for traffic in &traffic {
        if !known.contains(traffic.session_id.as_str()) {
            rows.push(merge(None, Some(traffic)));
        }
    }

    // The agents the caller manages, when it named them. **Here, and server-side,
    // on purpose**: the list is paged, so a client that filtered its own page would
    // show ten rows under a pager counting fifty. An empty list is not "no filter" —
    // it is the honest answer to "which of the agents I manage", and it means none.
    if let Some(agents) = agents {
        rows.retain(|r| agents.iter().any(|a| a == &r.agent));
    }

    // Newest first, by the session's end. Ties break on the id so the order does
    // not depend on which side a row happened to come from.
    rows.sort_by(|a, b| {
        b.ended_at
            .cmp(&a.ended_at)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(rows)
}

/// One row out of whichever halves have it. At least one of the two is `Some`.
fn merge(imported: Option<&SessionRow>, traffic: Option<&TrafficSession>) -> SessionVm {
    let session_id = match (imported, traffic) {
        (Some(i), _) => i.session_id.clone(),
        (None, Some(t)) => t.session_id.clone(),
        (None, None) => unreachable!("merge is called with at least one side"),
    };
    let source = match (imported.is_some(), traffic.is_some()) {
        (true, true) => "both",
        (true, false) => "imported",
        (false, true) => "gateway",
        (false, false) => unreachable!("merge is called with at least one side"),
    };

    // The span: the files know the session's whole life; the traffic rows only
    // cover what the gateway saw. Prefer the imported span, and fall back to the
    // traffic one only when there are no imported rows to have a span.
    let (started_at, ended_at) = match imported {
        Some(i) => (i.started_at.clone(), i.ended_at.clone()),
        None => match traffic {
            Some(t) => (t.first_ts.clone(), t.last_ts.clone()),
            None => unreachable!("merge is called with at least one side"),
        },
    };

    // The agent label: stored on the imported row, derived from the traffic rows
    // otherwise (the single distinct label in practice; a comma-join would only
    // ever separate two labels one id was shared by, which the list shows whole).
    let agent = match imported {
        Some(i) => i.agent.clone(),
        None => traffic.map(|t| t.agents.join(", ")).unwrap_or_default(),
    };

    // Tokens: the traffic side's exact metered numbers when it exists, the
    // files' otherwise. **Not summed** — a retry is one file turn and several
    // traffic rows, so adding them would count the same work twice.
    let (input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens) = match traffic {
        Some(t) => (
            t.input_tokens,
            t.output_tokens,
            t.cache_read_tokens,
            t.cache_creation_tokens,
        ),
        None => match imported {
            Some(i) => (
                i.input_tokens,
                i.output_tokens,
                i.cache_read_tokens,
                i.cache_creation_tokens,
            ),
            None => unreachable!("merge is called with at least one side"),
        },
    };

    // Money, from the traffic side only and per currency. A bucket with no
    // currency is dropped: there is no money to show it in (see `SessionCurrencyCost`).
    let cost: Vec<CurrencyAmountVm> = traffic
        .map(|t| {
            t.cost
                .iter()
                .filter_map(|c| {
                    c.currency.as_ref().map(|currency| CurrencyAmountVm {
                        currency: currency.clone(),
                        amount: c.cost,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    SessionVm {
        session_id,
        source: source.to_string(),
        agent,
        project: imported.and_then(|i| i.project.clone()),
        // The files' own answer to "what was this about", and only theirs: a
        // traffic-only session has no file, so it has no title.
        title: imported.and_then(|i| i.title.clone()),
        // Which machine the files came from: the imported row's own label, and
        // nothing else — the traffic side has no files to have come from, so a
        // gateway-only session reads `None` (the display's "unnamed").
        machine: imported.and_then(|i| i.imported_from.clone()),
        started_at,
        ended_at,
        requests: traffic.map(|t| t.requests).unwrap_or(0),
        turns: imported.map(|i| i.turns).unwrap_or(0),
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        file_tokens: imported.map(|i| SessionTokens {
            input: i.input_tokens,
            output: i.output_tokens,
            cache_read: i.cache_read_tokens,
            cache_creation: i.cache_creation_tokens,
        }),
        cost,
        unpriced_rows: traffic.map(|t| t.unpriced_rows).unwrap_or(0),
        tool_calls: imported
            .map(|i| decode_counts(i.tool_calls.as_deref()))
            .unwrap_or_default(),
        tool_uses: decode_tool_uses(imported.and_then(|i| i.tool_uses.as_deref())),
        skills: imported
            .map(|i| decode_counts(i.skills.as_deref()))
            .unwrap_or_default(),
    }
}

/// A stored cell back into the typed list.
///
/// Two shapes are accepted, because a row imported before the list grew a tail
/// holds the older one — a bare array of calls, with no timestamps and no gap. It
/// is read as the head of the new shape rather than dropped: the page shows what
/// the machine has, and a re-import upgrades it in place.
///
/// A cell that will not parse decodes to nothing rather than failing the read:
/// the list is the least important thing on the page, and losing it must not cost
/// the session its numbers.
fn decode_tool_uses(raw: Option<&str>) -> HistoryToolUses {
    let Some(raw) = raw else {
        return HistoryToolUses::default();
    };
    if let Ok(uses) = serde_json::from_str::<HistoryToolUses>(raw) {
        return uses;
    }
    match serde_json::from_str::<Vec<HistoryToolUse>>(raw) {
        Ok(head) => HistoryToolUses {
            head,
            tail: Vec::new(),
            skipped: 0,
        },
        Err(_) => HistoryToolUses::default(),
    }
}

/// A stored `[{"name","count"}]` cell back into the typed list.
///
/// A cell that will not parse reads as empty rather than failing the whole view:
/// the counts sit beside a session's money, and one unreadable column must not
/// hide the cost of every session in the list.
fn decode_counts(json: Option<&str>) -> Vec<HistoryCount> {
    json.and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{sample_log, temp_store};
    use crate::store::ImportedSession;

    fn imported(agent: &str, session: &str, project: Option<&str>) -> ImportedSession {
        ImportedSession {
            import_key: format!("{agent}:{session}"),
            agent: agent.into(),
            project: project.map(str::to_string),
            title: None,
            session_id: session.into(),
            started_at: "2026-09-01T00:00:00+00:00".into(),
            ended_at: "2026-09-01T05:00:00+00:00".into(),
            turns: 12,
            input_tokens: 1_000,
            output_tokens: 100,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            tool_calls: Some(r#"[{"name":"Bash","count":4},{"name":"Read","count":9}]"#.into()),
            tool_uses: None,
            skills: Some(r#"[{"name":"tweet","count":1}]"#.into()),
            imported_from: None,
        }
    }

    /// A store holding two traffic sessions: `s-both` (a priced row and an
    /// unpriced one) and `s-gateway` (priced, and never in the files). The
    /// tempdir comes back with it, because the store is file-backed and the
    /// directory has to outlive every read.
    fn store_with_traffic() -> (tempfile::TempDir, crate::store::Store) {
        let (dir, store) = temp_store();
        for (session, ts, cost, currency, tokens) in [
            (
                "s-both",
                "2026-09-02T10:00:00+00:00",
                Some(2.0),
                Some("USD"),
                500,
            ),
            ("s-both", "2026-09-02T11:00:00+00:00", None, None, 0),
            (
                "s-gateway",
                "2026-09-03T10:00:00+00:00",
                Some(1.0),
                Some("USD"),
                100,
            ),
        ] {
            let mut log = sample_log(ts, Some("claude"), 200);
            log.session_id = Some(session.into());
            log.provider_id = Some("p-1".into());
            log.input_tokens = tokens;
            log.output_tokens = 0;
            log.cost = cost;
            log.cost_currency = currency.map(str::to_string);
            store.insert_request_log(&log).unwrap();
        }
        (dir, store)
    }

    /// A session in both ledgers is one row, and each field comes from the side
    /// that can know it — the files' span and project, the traffic's requests,
    /// tokens and money. Nothing is added.
    #[test]
    fn a_session_in_both_ledgers_is_one_row_with_each_side_supplying_its_own() {
        let (_dir, store) = store_with_traffic();
        store
            .upsert_imported_sessions(&[imported("claude", "s-both", Some("kiwano"))])
            .unwrap();

        let rows = list_sessions(&store, None, None, None).unwrap();
        let row = rows
            .iter()
            .find(|r| r.session_id == "s-both")
            .expect("the merged row");
        assert_eq!(row.source, "both");
        assert_eq!(row.project.as_deref(), Some("kiwano"), "the files' project");
        assert_eq!(row.turns, 12, "only the files count turns");
        assert_eq!(
            row.started_at, "2026-09-01T00:00:00+00:00",
            "the imported span wins"
        );
        assert_eq!(row.requests, 2, "only the traffic counts requests");
        assert_eq!(
            row.input_tokens, 500,
            "the traffic side's exact tokens win; the file's 1000 is not added"
        );
        assert_eq!(row.unpriced_rows, 1, "the costless traffic row is counted");
        assert_eq!(row.cost.len(), 1);
        assert_eq!(row.cost[0].currency, "USD");
        assert!((row.cost[0].amount - 2.0).abs() < 1e-9);
        assert_eq!(
            row.tool_calls
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Bash", "Read"],
            "the files' tools survive the JSON round trip"
        );
    }

    /// An imported-only session shows the files' numbers and no money: the files
    /// carry tokens, not cost.
    /// One session by id, from both ledgers or from one — and `None` for an id
    /// neither has, which is the answer the route turns into a `null` body.
    /// A caller that names the agents it manages gets those, and **only** those —
    /// including when it manages none, which is a real answer and not a missing
    /// filter.
    #[test]
    fn a_page_can_be_narrowed_to_the_agents_the_caller_manages() {
        let (_dir, store) = store_with_traffic();
        store
            .upsert_imported_sessions(&[
                imported("claude", "s-claude", Some("acme")),
                imported("codex", "s-codex", Some("acme")),
            ])
            .unwrap();

        let page = |agents: Option<&[String]>| {
            list_sessions_page(&store, None, None, agents, 1, 50).unwrap()
        };

        // No set: the ledger, everything in it — the fixture's two traffic-only
        // sessions and these two.
        assert_eq!(page(None).total, 4);

        let one = vec!["codex".to_string()];
        let narrowed = page(Some(&one));
        assert_eq!(narrowed.total, 1, "and the total counts the narrowed list");
        assert!(narrowed.rows.iter().all(|r| r.agent == "codex"));

        // Nobody manages nothing: the app sends this on a machine where no agent
        // has been taken over, and the answer is an empty page — not every row.
        let none: Vec<String> = Vec::new();
        assert_eq!(page(Some(&none)).total, 0);
    }

    #[test]
    fn a_session_can_be_read_by_id_from_either_ledger() {
        let (_dir, store) = store_with_traffic();
        store
            .upsert_imported_sessions(&[
                imported("claude", "s-both", Some("acme")),
                imported("claude", "s-imported", Some("blog")),
            ])
            .unwrap();

        let both = get_session(&store, "s-both").unwrap().expect("in both");
        assert_eq!(both.source, "both");
        assert_eq!(both.project.as_deref(), Some("acme"));
        assert_eq!(both.requests, 2, "the traffic side's count");

        let imported_only = get_session(&store, "s-imported").unwrap().expect("files");
        assert_eq!(imported_only.source, "imported");
        assert_eq!(imported_only.requests, 0);
        assert!(imported_only.cost.is_empty(), "the files carry no money");

        let gateway_only = get_session(&store, "s-gateway").unwrap().expect("traffic");
        assert_eq!(gateway_only.source, "gateway");
        assert_eq!(gateway_only.project, None);

        assert!(
            get_session(&store, "nothing-knows-this").unwrap().is_none(),
            "neither ledger has it, and that is None rather than an error"
        );
    }

    #[test]
    fn an_imported_only_session_has_no_requests_and_no_money() {
        let (_dir, store) = store_with_traffic();
        store
            .upsert_imported_sessions(&[imported("codex", "s-files", Some("acme"))])
            .unwrap();

        let rows = list_sessions(&store, None, None, None).unwrap();
        let row = rows
            .iter()
            .find(|r| r.session_id == "s-files")
            .expect("the imported row");
        assert_eq!(row.source, "imported");
        assert_eq!(row.requests, 0);
        assert!(row.cost.is_empty(), "no traffic means no money to show");
        assert_eq!(row.unpriced_rows, 0);
        assert_eq!(row.input_tokens, 1_000, "the files' own numbers are shown");
    }

    /// A gateway-only session shows the traffic's numbers and no project, turns
    /// or tools — the three things only the files know.
    #[test]
    fn a_gateway_only_session_has_no_project_and_no_turns() {
        let (_dir, store) = store_with_traffic();
        let rows = list_sessions(&store, None, None, None).unwrap();
        let row = rows
            .iter()
            .find(|r| r.session_id == "s-gateway")
            .expect("the traffic-only row");
        assert_eq!(row.source, "gateway");
        assert_eq!(row.project, None);
        assert_eq!(row.turns, 0);
        assert!(row.tool_calls.is_empty() && row.skills.is_empty());
        assert_eq!(row.requests, 1);
        assert_eq!(
            row.started_at, "2026-09-03T10:00:00+00:00",
            "the traffic span"
        );
    }

    /// v32: the machine a session was imported from reaches the view, and it
    /// comes from the imported side alone — a traffic-only session has no origin
    /// machine to show, so it reads `None` rather than borrowing another row's.
    #[test]
    fn the_machine_comes_from_the_imported_row_and_only_from_it() {
        let (_dir, store) = store_with_traffic();
        let mut frombox = imported("claude", "s-both", Some("kiwano"));
        frombox.imported_from = Some("buildbox".into());
        store.upsert_imported_sessions(&[frombox]).unwrap();

        let rows = list_sessions(&store, None, None, None).unwrap();
        let both = rows.iter().find(|r| r.session_id == "s-both").unwrap();
        assert_eq!(
            both.machine.as_deref(),
            Some("buildbox"),
            "the imported row's label survives the merge"
        );
        let gateway = rows.iter().find(|r| r.session_id == "s-gateway").unwrap();
        assert_eq!(gateway.machine, None, "traffic alone never names a machine");
    }

    /// Under a project filter a traffic-only session is not shown: it has no
    /// project, so it cannot be said to match rather than be assumed into it.
    #[test]
    fn a_project_filter_drops_the_sessions_the_files_did_not_place() {
        let (_dir, store) = store_with_traffic();
        store
            .upsert_imported_sessions(&[imported("claude", "s-both", Some("kiwano"))])
            .unwrap();

        let rows = list_sessions(&store, Some("kiwano"), None, None).unwrap();
        assert_eq!(rows.len(), 1, "only the placed session");
        assert_eq!(rows[0].session_id, "s-both");

        assert!(list_sessions(&store, Some("nothing"), None, None)
            .unwrap()
            .is_empty());
    }

    /// Newest first, by end, across both sources.
    #[test]
    fn the_rows_are_newest_first_across_both_sources() {
        let (_dir, store) = store_with_traffic();
        store
            .upsert_imported_sessions(&[imported("codex", "s-files", Some("acme"))])
            .unwrap();
        let rows = list_sessions(&store, None, None, None).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.session_id.as_str()).collect();
        // s-gateway ends 09-03, s-both 09-02, s-files 09-01.
        assert_eq!(ids, vec!["s-gateway", "s-both", "s-files"]);
    }
}
