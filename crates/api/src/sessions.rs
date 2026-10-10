//! The merged session view: one row per session id, from two sources.
//!
//! The gateway has two notions of "session" and this is where they meet.
//!
//! * The **traffic** side is `request_logs.session_id`: the precise record of
//!   what the gateway routed — provider, cost, tokens, status — keyed by the id
//!   an agent named on its request.
//! * The **imported** side is the `sessions` table, written from the agents' own
//!   files. It knows the **project**, the **turns** and the tools, which the
//!   traffic side can never observe: the gateway sees requests, not the checkout
//!   they came from.
//!
//! The same session can be in both. One that ran before Kiwano was installed is
//! only in the files; one that ran after is in both (the import's watermark
//! skips the *usage* rows of traffic it already metered, but still writes the
//! session row). So a row here says which source(s) it came from, and **the two
//! sides are never added together**: they describe overlapping-but-not-equal
//! sets — a retry or a failover is one file turn and several traffic rows — so
//! where both exist this prefers one side per field and says which in the
//! field's own comment.

use crate::client_keys::CurrencyAmountVm;
use crate::history::HistoryCount;
use serde::{Deserialize, Serialize};

/// One session, merged from the gateway's traffic and the agents' files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionVm {
    pub session_id: String,
    /// `"gateway"` | `"imported"` | `"both"` — which sides this id was found in.
    /// A plain string, not an enum: the wire types in this crate are serde-only,
    /// and this is the shape a reader switches on.
    pub source: String,
    /// The agent the session ran as. Taken from the imported row when there is
    /// one (a stored field there); otherwise the traffic rows' agent — the single
    /// distinct one in practice.
    pub agent: String,
    /// The project label, **imported side only**. The gateway sees requests and
    /// never learns which checkout they came from, so a traffic-only session has
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Which machine's files this session was imported from, **imported side
    /// only** — the same label as the row it came from (`imported_from`,
    /// migration v32), `None` for a traffic-only session or one whose machine
    /// could not name itself. A **label, not an identity**: two machines can
    /// share it, so a reader is told rather than a key being joined on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// When the session ran. The **imported** span wins when there is one: it is
    /// the session's whole life, where the traffic span only covers what the
    /// gateway saw — and starts at the takeover, not at the session.
    pub started_at: String,
    pub ended_at: String,
    /// Requests the gateway routed. 0 for an imported-only session — the files
    /// count turns, and a turn is not a request.
    pub requests: i64,
    /// Turns the files recorded. 0 for a traffic-only session.
    pub turns: i64,
    /// The four token buckets, kept apart (never summed here). When both sides
    /// exist the **traffic** numbers win: they are the exact ones the gateway
    /// metered, while the file numbers are the same work seen a second time.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// What the traffic cost, per currency — **the traffic side only**, and
    /// never converted. Empty for an imported-only session: the files carry
    /// tokens, not money, and an amount invented from them would be a guess.
    #[serde(default)]
    pub cost: Vec<CurrencyAmountVm>,
    /// Traffic rows with no cost. They are part of `requests`; `cost` is short
    /// by them, and this is how a reader is told rather than shown a small total.
    #[serde(default)]
    pub unpriced_rows: i64,
    /// Tools the files recorded, as `{name, count}`. Imported only.
    #[serde(default)]
    pub tool_calls: Vec<HistoryCount>,
    /// Skills the files recorded. Imported only.
    #[serde(default)]
    pub skills: Vec<HistoryCount>,
}
