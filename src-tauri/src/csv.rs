//! CSV for the Logs card's export.
//!
//! One row per request, every column the list view carries. Kept here rather
//! than in `vm.rs` so the formatting is a pure function of the rows and can be
//! unit-tested without touching a file.

use kiwano_gateway::store::RequestLogEntry;

/// Excel reads a BOM-less UTF-8 file as the system codepage, which turns any
/// non-ASCII (model ids, upstream error text) into mojibake. The BOM is the
/// difference between a file that opens right and one that doesn't.
const BOM: &str = "\u{feff}";

/// Column order is the contract for whoever parses the file — append, never
/// reorder.
const HEADERS: [&str; 27] = [
    "id",
    "ts",
    "method",
    "path",
    "query",
    "agent",
    "attribution",
    "provider_id",
    "model",
    "status_code",
    "error_kind",
    "error_message",
    "session_id",
    "is_streaming",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_creation_tokens",
    "latency_ms",
    "first_token_ms",
    "request_size",
    "response_size",
    "truncated",
    "cost",
    "cost_currency",
    "request_headers",
    "response_headers",
];

/// Quote a field only when it needs it (RFC 4180): at least the delimiter, a
/// quote, or a line break. Embedded quotes double.
fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn opt(field: &Option<String>) -> &str {
    field.as_deref().unwrap_or("")
}

/// A `None` numeric cell is empty rather than `0`: "we didn't measure it" and
/// "it was zero" are different facts, and the export should not merge them.
fn opt_num<T: std::fmt::Display>(field: Option<T>) -> String {
    field.map(|v| v.to_string()).unwrap_or_default()
}

fn row(entry: &RequestLogEntry) -> String {
    let cells = [
        entry.id.to_string(),
        entry.ts.clone(),
        entry.method.clone(),
        entry.path.clone(),
        opt(&entry.query).to_string(),
        opt(&entry.agent).to_string(),
        opt(&entry.attribution).to_string(),
        opt(&entry.provider_id).to_string(),
        opt(&entry.model).to_string(),
        entry.status_code.to_string(),
        opt(&entry.error_kind).to_string(),
        opt(&entry.error_message).to_string(),
        opt(&entry.session_id).to_string(),
        entry.is_streaming.to_string(),
        entry.input_tokens.to_string(),
        entry.output_tokens.to_string(),
        entry.cache_read_tokens.to_string(),
        entry.cache_creation_tokens.to_string(),
        opt_num(entry.latency_ms),
        opt_num(entry.first_token_ms),
        entry.request_size.to_string(),
        entry.response_size.to_string(),
        entry.truncated.to_string(),
        opt_num(entry.cost),
        opt(&entry.cost_currency).to_string(),
        opt(&entry.request_headers).to_string(),
        opt(&entry.response_headers).to_string(),
    ];
    cells.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",")
}

/// The whole file as a string, BOM and header included. Records end CRLF, as
/// RFC 4180 specifies — every parser worth the name accepts it.
///
/// Fields are quoted but not otherwise neutralised, so a value starting with
/// `=`, `+`, `-` or `@` stays exactly what was logged. Spreadsheets may read
/// such a cell as a formula; that is a real risk only when the file is opened
/// by someone else, and mangling the value with a leading apostrophe would
/// corrupt a local audit trail to guard against it.
pub fn to_csv(rows: &[RequestLogEntry]) -> String {
    let mut out = String::from(BOM);
    out.push_str(&HEADERS.join(","));
    out.push_str("\r\n");
    for entry in rows {
        out.push_str(&row(entry));
        out.push_str("\r\n");
    }
    out
}

pub fn write_csv(path: &str, rows: &[RequestLogEntry]) -> std::io::Result<()> {
    std::fs::write(path, to_csv(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> RequestLogEntry {
        RequestLogEntry {
            id: 7,
            ts: "2026-09-07T13:00:00Z".into(),
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
            input_tokens: 1000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(214),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_size: 512,
            response_size: 1024,
            truncated: false,
            cost: Some(0.02),
            cost_currency: Some("USD".into()),
        }
    }

    fn body(csv: &str) -> &str {
        csv.strip_prefix(BOM).expect("BOM")
    }

    #[test]
    fn header_and_bom_come_first() {
        let csv = to_csv(&[entry()]);
        assert!(csv.starts_with(BOM), "Excel needs the BOM to read UTF-8");
        assert_eq!(csv.matches(BOM).count(), 1, "and exactly one of them");
        let mut lines = body(&csv).lines();
        assert_eq!(lines.next().unwrap().split(',').count(), HEADERS.len());
        assert_eq!(lines.next().unwrap().split(',').count(), HEADERS.len());
        assert!(lines.next().is_none(), "no trailing blank row");
    }

    #[test]
    fn empty_cells_are_not_zeroes() {
        let csv = to_csv(&[entry()]);
        let row = body(&csv).lines().nth(1).unwrap();
        let cells: Vec<&str> = row.split(',').collect();
        // first_token_ms is absent, output_tokens is a real 200 — and they must
        // not look the same in the file.
        assert_eq!(
            cells[HEADERS.iter().position(|h| *h == "first_token_ms").unwrap()],
            ""
        );
        assert_eq!(
            cells[HEADERS.iter().position(|h| *h == "output_tokens").unwrap()],
            "200"
        );
        assert_eq!(
            cells[HEADERS.iter().position(|h| *h == "query").unwrap()],
            ""
        );
    }

    #[test]
    fn fields_that_need_quoting_get_it() {
        let mut e = entry();
        e.path = "/v1/a,b".into();
        e.error_message = Some("boom \"quoted\"".into());
        e.request_headers = Some("{\n  \"a\": 1\n}".into());
        // Assert against the whole file: a cell containing a line break makes
        // `lines()` split one record across several, which is the point.
        let csv = to_csv(&[e]);
        assert!(csv.contains("\"/v1/a,b\""), "comma forces quotes");
        assert!(
            csv.contains("\"boom \"\"quoted\"\"\""),
            "embedded quotes double"
        );
        assert!(
            csv.contains("\"{\n  \"\"a\"\": 1\n}\""),
            "a line break stays inside its quoted cell"
        );
        assert!(
            !csv.contains("\"\"/v1/a,b\"\""),
            "a field that needed quoting is not double-quoted"
        );
    }

    #[test]
    fn records_end_crlf() {
        let csv = to_csv(&[entry(), entry()]);
        assert_eq!(csv.matches("\r\n").count(), 3, "header + 2 rows");
        assert_eq!(
            body(&csv).matches('\n').count(),
            body(&csv).matches("\r\n").count(),
            "no bare LF: every newline is part of a CRLF"
        );
    }

    #[test]
    fn a_json_header_cell_survives_a_minimal_reader() {
        // The strongest escaping check available without a CSV parser: split on
        // commas that are not inside quotes, and get the original bytes back.
        let mut e = entry();
        e.request_headers = Some(r#"{"a":"b,c","d":"say \"hi\""}"#.into());
        let csv = to_csv(&[e]);
        let record = body(&csv).split("\r\n").nth(1).unwrap();

        let mut cells = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        let mut chars = record.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    cur.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => cells.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        cells.push(cur);

        let at = |name: &str| cells[HEADERS.iter().position(|h| *h == name).unwrap()].clone();
        assert_eq!(at("request_headers"), r#"{"a":"b,c","d":"say \"hi\""}"#);
        assert_eq!(at("path"), "/v1/messages");
    }

    #[test]
    fn bools_and_numbers_render_plainly() {
        let mut e = entry();
        e.is_streaming = true;
        e.truncated = true;
        let csv = to_csv(&[e]);
        let row = body(&csv).lines().nth(1).unwrap();
        let cells: Vec<&str> = row.split(',').collect();
        let at = |name: &str| cells[HEADERS.iter().position(|h| *h == name).unwrap()];
        assert_eq!(at("is_streaming"), "true");
        assert_eq!(at("truncated"), "true");
        assert_eq!(at("cost"), "0.02");
        assert_eq!(at("cost_currency"), "USD");
    }

    #[test]
    fn no_rows_still_gives_a_header() {
        let csv = to_csv(&[]);
        assert_eq!(body(&csv).lines().count(), 1);
    }
}
