//! Schema migrations: the SQL bodies and the driver that applies them.
//!
//! The `MIGRATION_V*` constants are in the order the module grew, not in
//! numeric order ([`MIGRATION_V23`] sits after [`MIGRATION_V26`]) — what
//! matters is the order `Store::migrate` calls them in, which is the
//! sequence a database is brought through. `SCHEMA_VERSION` is the version
//! `migrate` stamps at the end: `execute_batch` does not bump
//! `user_version` itself.
//!
//! `Store::open` lives here with them because the open contract is this
//! file: open (or create) the connection, configure it and migrate, then
//! harden the file permissions `-wal`/`-shm` picked up.

use crate::error::Result;
use crate::store::permissions::harden_permissions;
use crate::store::Store;
#[cfg(test)]
use rusqlite::params;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// Current schema version tracked via `PRAGMA user_version`.
pub const SCHEMA_VERSION: i32 = 26;

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS providers (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL DEFAULT 'anthropic'
                 CHECK (protocol IN ('anthropic','openai')),
    base_url     TEXT NOT NULL,
    api_path     TEXT,
    api_key      TEXT,
    billing      TEXT NOT NULL DEFAULT 'metered'
                 CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit REAL,
    reset_period TEXT
                 CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_strategies (
    agent  TEXT PRIMARY KEY,
    type   TEXT NOT NULL DEFAULT 'single'
           CHECK (type IN ('single','failover','roundrobin','timewindow','quota')),
    config TEXT
);

CREATE TABLE IF NOT EXISTS agent_bindings (
    agent       TEXT NOT NULL,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    priority    INTEGER NOT NULL DEFAULT 0,
    weight      INTEGER NOT NULL DEFAULT 1,
    win_start   TEXT,
    win_end     TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (agent, provider_id)
);

CREATE TABLE IF NOT EXISTS placeholder_keys (
    key        TEXT PRIMARY KEY,
    agent      TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS usage (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    ts                    TEXT NOT NULL,
    agent                 TEXT NOT NULL,
    provider_id           TEXT NOT NULL,
    model                 TEXT,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms            INTEGER,
    status                TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok','error'))
);

CREATE INDEX IF NOT EXISTS idx_usage_ts            ON usage(ts);
CREATE INDEX IF NOT EXISTS idx_usage_provider_ts   ON usage(provider_id, ts);
CREATE INDEX IF NOT EXISTS idx_usage_agent_ts      ON usage(agent, ts);

CREATE TABLE IF NOT EXISTS provider_health (
    provider_id          TEXT PRIMARY KEY REFERENCES providers(id) ON DELETE CASCADE,
    status               TEXT NOT NULL DEFAULT 'unknown'
                         CHECK (status IN ('healthy','degraded','down','unknown')),
    last_latency_ms      INTEGER,
    last_check_at        TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0
);
"#;

/// v2: unit for the user-entered period cap (spending alerts / ring percentage, tech.md §2.4 A).
/// NULL rows predate the column and are read as 'requests'.
const MIGRATION_V2: &str = r#"
ALTER TABLE providers ADD COLUMN limit_unit TEXT
    CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens','cny'));
"#;

/// v3: extra API keys per provider (spec §4.1 P1 multi-key rotation).
/// providers.api_key stays the primary key of the pool; these rotate after it.
const MIGRATION_V3: &str = r#"
CREATE TABLE IF NOT EXISTS api_keys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    api_key     TEXT NOT NULL,
    label       TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_api_keys_provider ON api_keys(provider_id);
"#;

/// v4: Gemini protocol flavor (P1 Gemini CLI takeover). CHECK constraints can't be
/// altered in place, so `providers` is rebuilt with the widened protocol set;
/// bindings/keys survive via named-column copy (FKs off during the swap).
const MIGRATION_V4: &str = r#"
PRAGMA foreign_keys=OFF;
CREATE TABLE providers_new (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL DEFAULT 'anthropic'
                 CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url     TEXT NOT NULL,
    api_path     TEXT,
    api_key      TEXT,
    billing      TEXT NOT NULL DEFAULT 'metered'
                 CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit REAL,
    limit_unit   TEXT
                 CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens','cny')),
    reset_period TEXT
                 CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
INSERT INTO providers_new (id, name, protocol, base_url, api_path, api_key,
                           billing, period_limit, limit_unit, reset_period,
                           enabled, created_at, updated_at)
    SELECT id, name, protocol, base_url, api_path, api_key,
           billing, period_limit, limit_unit, reset_period,
           enabled, created_at, updated_at
    FROM providers;
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
PRAGMA foreign_keys=ON;
"#;

/// v5: complete request logging (bodies + metadata) — this is a local tool, so
/// full captures are kept. Metadata lives in `request_logs`; the (possibly
/// large) bodies live in a sibling table so list queries never drag blobs.
/// `gateway_settings` is the gateway-visible KV store (the GUI's `app_settings`
/// lives in the same file but is GUI-owned; the gateway only reads its own).
const MIGRATION_V5: &str = r#"
CREATE TABLE IF NOT EXISTS request_logs (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    ts                    TEXT NOT NULL,
    method                TEXT NOT NULL,
    path                  TEXT NOT NULL,
    query                 TEXT,
    agent                 TEXT,
    attribution           TEXT,
    provider_id           TEXT,
    model                 TEXT,
    status_code           INTEGER NOT NULL,
    error_kind            TEXT,
    error_message         TEXT,
    session_id            TEXT,
    is_streaming          INTEGER NOT NULL DEFAULT 0,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms            INTEGER,
    first_token_ms        INTEGER,
    request_headers       TEXT,
    response_headers      TEXT,
    request_size          INTEGER NOT NULL DEFAULT 0,
    response_size         INTEGER NOT NULL DEFAULT 0,
    truncated             INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_request_logs_ts          ON request_logs(ts);
CREATE INDEX IF NOT EXISTS idx_request_logs_agent_ts    ON request_logs(agent, ts);
CREATE INDEX IF NOT EXISTS idx_request_logs_provider_ts ON request_logs(provider_id, ts);

CREATE TABLE IF NOT EXISTS request_bodies (
    log_id        INTEGER PRIMARY KEY REFERENCES request_logs(id) ON DELETE CASCADE,
    request_body  TEXT,
    response_body TEXT
);

CREATE TABLE IF NOT EXISTS gateway_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- The GUI's KV, created here too. It used to be the GUI's alone, so the
-- gateway could assume it existed; it now reads and writes it (the plan-quota
-- cache, the tz offset), and on a fresh install the gateway can start first.
-- The definition must stay identical to `Aux::init_tables`'s.
CREATE TABLE IF NOT EXISTS app_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// v7: per-protocol endpoints on one provider — a single provider row can now
/// serve multiple inbound protocols, each with its own upstream URL (e.g. a
/// vendor exposing both an OpenAI-compatible and a native Anthropic endpoint).
/// The provider's primary protocol/endpoint stay in `providers`; this table
/// holds only the ADDITIONAL endpoints (one row per protocol).
const MIGRATION_V7: &str = r#"
CREATE TABLE IF NOT EXISTS provider_endpoints (
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    protocol    TEXT NOT NULL CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url    TEXT NOT NULL,
    api_path    TEXT,
    PRIMARY KEY (provider_id, protocol)
);
"#;

/// v8: per-provider advanced forwarding settings — NULL everywhere means the
/// gateway defaults apply. `timeout_secs` bounds the wait for upstream
/// response headers (never aborts an in-flight stream body); `retries` counts
/// same-provider re-attempts before the strategy layer moves on; `headers` is
/// a JSON object of custom request headers merged after credential injection.
const MIGRATION_V8: &str = r#"
ALTER TABLE providers ADD COLUMN timeout_secs INTEGER;
ALTER TABLE providers ADD COLUMN retries INTEGER;
ALTER TABLE providers ADD COLUMN headers TEXT;
"#;

/// v9: model pricing (port of cc-switch's price layer).
/// - `model_pricing` is the price table itself (TEXT decimals, currency per
///   row). The GUI seeds it from the Hub's models.json; the gateway builds its
///   in-memory table from these rows at startup and on `/reload`
///   (`server::resolve_pricing`) — an empty table is an empty table, there is
///   nothing behind these rows. A price update therefore affects costs recorded
///   *after* the reload — history is not rewritten.
/// - `usage`/`request_logs` gain `cost` + `cost_currency` (cost is stored in
///   the price entry's currency; NULL for unpriced models).
/// - `providers.plan_query` holds the token-plan quota query template +
///   credentials as JSON (`{"template":"kimi","fields":{...}}`).
/// - `limit_unit` widens from ('requests','wan_tokens','cny') to those two
///   units plus any 3-letter uppercase currency code. CHECK constraints can't
///   be altered in place, so `providers` is rebuilt (v4 pattern); legacy
///   lowercase units are preserved verbatim, bare currency codes uppercased.
const MIGRATION_V9: &str = r#"
CREATE TABLE IF NOT EXISTS model_pricing (
    model_id        TEXT PRIMARY KEY,
    display_name    TEXT NOT NULL,
    input           TEXT NOT NULL,
    output          TEXT NOT NULL,
    cache_read      TEXT NOT NULL DEFAULT '0',
    cache_creation  TEXT NOT NULL DEFAULT '0',
    currency        TEXT NOT NULL DEFAULT 'USD',
    source          TEXT
);

ALTER TABLE usage ADD COLUMN cost REAL;
ALTER TABLE usage ADD COLUMN cost_currency TEXT;

ALTER TABLE request_logs ADD COLUMN cost REAL;
ALTER TABLE request_logs ADD COLUMN cost_currency TEXT;

PRAGMA foreign_keys=OFF;
CREATE TABLE providers_new (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL DEFAULT 'anthropic'
                 CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url     TEXT NOT NULL,
    api_path     TEXT,
    api_key      TEXT,
    billing      TEXT NOT NULL DEFAULT 'metered'
                 CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit REAL,
    limit_unit   TEXT
                 CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens')
                        OR (length(limit_unit) = 3 AND upper(limit_unit) = limit_unit)),
    plan_query   TEXT,
    reset_period TEXT
                 CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    timeout_secs INTEGER,
    retries      INTEGER,
    headers      TEXT
);
INSERT INTO providers_new (id, name, protocol, base_url, api_path, api_key,
                           billing, period_limit, limit_unit, plan_query,
                           reset_period, enabled, created_at, updated_at,
                           timeout_secs, retries, headers)
    SELECT id, name, protocol, base_url, api_path, api_key,
           billing, period_limit,
           CASE WHEN limit_unit IN ('requests','wan_tokens') THEN limit_unit
                WHEN limit_unit IS NULL THEN NULL
                ELSE upper(limit_unit) END,
           NULL,
           reset_period, enabled, created_at, updated_at,
           timeout_secs, retries, headers
    FROM providers;
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
PRAGMA foreign_keys=ON;
"#;

/// v10: percent-of-window plan limits. `providers.plan_limits` holds the
/// plan-mode limit as JSON `{"five_hour":20,"weekly":60}` — per-window
/// utilization ceilings (percent of the vendor's rolling 5h / weekly window)
/// replacing the old number+unit+reset-cycle form for plan providers. Legacy
/// v9 columns (period_limit/limit_unit/reset_period) keep rendering for old
/// rows but are no longer written by the plan form; a plain nullable column
/// needs no rebuild.
const MIGRATION_V10: &str = r#"
ALTER TABLE providers ADD COLUMN plan_limits TEXT;
"#;

/// v11: prices become per-provider.
///
/// The Hub prices a model per catalog provider entry, so the same `model_id`
/// may appear once per provider at different rates (a subsidy, a margin) — and
/// the v9 key of `model_id` alone could hold only one of them. The primary key
/// becomes `(provider_id, model_id)`; a row whose `provider_id` is empty is the
/// general price, which is what every pre-v11 row is (nothing had a provider to
/// name) and what the app falls back to for a provider with no price of its own.
///
/// SQLite cannot alter a primary key in place, so the table is rebuilt (the v4
/// pattern). No indexes or foreign keys point at it.
const MIGRATION_V11: &str = r#"
PRAGMA foreign_keys=OFF;
CREATE TABLE model_pricing_new (
    provider_id     TEXT NOT NULL DEFAULT '',
    model_id        TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    input           TEXT NOT NULL,
    output          TEXT NOT NULL,
    cache_read      TEXT NOT NULL DEFAULT '0',
    cache_creation  TEXT NOT NULL DEFAULT '0',
    currency        TEXT NOT NULL DEFAULT 'USD',
    source          TEXT,
    PRIMARY KEY (provider_id, model_id)
);
INSERT INTO model_pricing_new (provider_id, model_id, display_name, input, output,
                               cache_read, cache_creation, currency, source)
    SELECT '', model_id, display_name, input, output,
           cache_read, cache_creation, currency, source
    FROM model_pricing;
DROP TABLE model_pricing;
ALTER TABLE model_pricing_new RENAME TO model_pricing;
PRAGMA foreign_keys=ON;
"#;

/// v12: `providers.catalog_id`.
///
/// The price table is keyed by the Hub's catalog entry id, and a local
/// provider's own id is not it: `vm::add_provider` names a row
/// `<slug>-<hex>` and a rename changes the name, not the id. This column is
/// the link that lets the gateway ask for the right provider's price.
///
/// NULL — every pre-v12 row, and every hand-added provider — means "no catalog
/// entry", which prices at the general rate.
const MIGRATION_V12: &str = r#"
ALTER TABLE providers ADD COLUMN catalog_id TEXT;
"#;

/// v13: time-of-day prices, and what they would have cost off-peak.
///
/// - `model_pricing.tiers` holds a row's `off_peak` rates and `peak_hours`
///   windows as one JSON blob, verbatim from the document — the shape belongs
///   to the Hub, and two more columns per rate would make this table's schema
///   the data side's problem. NULL means the row has no tiers; that spelling
///   (rather than `"{}"`) is what keeps a forced re-seed write-free.
/// - `usage.cost_off_peak` / `request_logs.cost_off_peak` are what the same
///   tokens would have cost at the row's off-peak rates — **equal to `cost`**
///   when the model publishes none. That equality is the whole point: it makes
///   "the peak premium" a plain `SUM(cost - cost_off_peak)` over every priced
///   row, with no tier flag to keep in step.
///
/// The backfill is load-bearing rather than cosmetic. Without it, every row
/// written before this migration has a `cost` and a NULL, so the two sums cover
/// different row sets and a user's headline and premium stop reconciling.
/// Setting them equal is also the honest answer: those requests were priced from
/// a table that had no tiers, so their off-peak equivalent never existed.
///
/// Both tables take plain `ADD COLUMN`s — no primary key or CHECK moves, which
/// is the only reason v9 and v11 had to rebuild.
/// v14: the model an add/edit form collected as a provider's default.
///
/// The form has asked for it since it existed and `NewProviderInput` has carried
/// it just as long, documented as "collected and never read" — no column, no view
/// field, so reopening a provider showed an empty box and the choice was quietly
/// thrown away. Storing it makes the round-trip match the form. Nothing consults
/// it yet: it is a remembered value, and `NULL` for every provider added before
/// this migration, which is exactly "not set".
const MIGRATION_V14: &str = r#"
ALTER TABLE providers ADD COLUMN model_default TEXT;
"#;

/// v15: the `gemini` protocol is gone, and so are the rows that speak it.
///
/// It had exactly two catalog entries (`google-ai-studio` as its only primary
/// protocol, `openrouter` as the only one adding it) and one consumer, the
/// Gemini CLI agent — which went with it, because a Gemini CLI can only reach a
/// provider that speaks Gemini. Keeping a branch per layer in the validators,
/// the adapters and the importers for that reach is not worth it.
///
/// Both tables are rebuilt, not just emptied, because the tag is still
/// *writable* on an existing database: v4 widened the CHECK for Gemini CLI, v9
/// carried the widening forward, and migrations replay in order — so narrowing
/// v1's text (done when the data side dropped the protocol) does not survive
/// them, and a fresh database ends up permissive too. Emptying the rows alone
/// would leave the tag insertable by anything that is not this build.
///
/// The rows themselves have to go rather than stay readable: once the Rust enum
/// loses the variant, `parse_str` has no answer for the tag and a stored
/// `gemini` row reads as Anthropic — requests leaving in the wrong shape,
/// failing at the vendor instead of here.
///
/// What the rebuild leaves behind is cleared by name, not by cascade: FKs are
/// off for the duration (the recipe requires it), so `ON DELETE CASCADE` does
/// not fire, and `model_pricing` never had a foreign key at all. Usage and
/// request logs are history and stay. The retired agent's own rows go too — a
/// binding naming `gemini` would reach the Apps screen as an agent id the
/// frontend no longer knows.
const MIGRATION_V15: &str = r#"
DELETE FROM model_pricing
    WHERE provider_id IN (SELECT id FROM providers WHERE protocol = 'gemini');
DELETE FROM agent_bindings
    WHERE agent = 'gemini'
       OR provider_id IN (SELECT id FROM providers WHERE protocol = 'gemini');
DELETE FROM api_keys
    WHERE provider_id IN (SELECT id FROM providers WHERE protocol = 'gemini');
DELETE FROM provider_health
    WHERE provider_id IN (SELECT id FROM providers WHERE protocol = 'gemini');
DELETE FROM agent_strategies WHERE agent = 'gemini';
DELETE FROM placeholder_keys WHERE agent = 'gemini';

PRAGMA foreign_keys=OFF;
CREATE TABLE providers_new (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    protocol      TEXT NOT NULL DEFAULT 'anthropic'
                  CHECK (protocol IN ('anthropic','openai')),
    base_url      TEXT NOT NULL,
    api_path      TEXT,
    api_key       TEXT,
    billing       TEXT NOT NULL DEFAULT 'metered'
                  CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit  REAL,
    limit_unit    TEXT
                  CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens')
                         OR (length(limit_unit) = 3 AND upper(limit_unit) = limit_unit)),
    plan_query    TEXT,
    reset_period  TEXT
                  CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    timeout_secs  INTEGER,
    retries       INTEGER,
    headers       TEXT,
    plan_limits   TEXT,
    catalog_id    TEXT,
    model_default TEXT
);
INSERT INTO providers_new (id, name, catalog_id, protocol, base_url, api_path, api_key,
                           billing, period_limit, limit_unit, plan_query, plan_limits,
                           timeout_secs, retries, headers, reset_period, enabled,
                           created_at, updated_at, model_default)
    SELECT id, name, catalog_id, protocol, base_url, api_path, api_key,
           billing, period_limit, limit_unit, plan_query, plan_limits,
           timeout_secs, retries, headers, reset_period, enabled,
           created_at, updated_at, model_default
    FROM providers WHERE protocol <> 'gemini';
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;

CREATE TABLE provider_endpoints_new (
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    protocol    TEXT NOT NULL CHECK (protocol IN ('anthropic','openai')),
    base_url    TEXT NOT NULL,
    api_path    TEXT,
    PRIMARY KEY (provider_id, protocol)
);
INSERT INTO provider_endpoints_new (provider_id, protocol, base_url, api_path)
    SELECT provider_id, protocol, base_url, api_path FROM provider_endpoints
    WHERE protocol <> 'gemini';
DROP TABLE provider_endpoints;
ALTER TABLE provider_endpoints_new RENAME TO provider_endpoints;
PRAGMA foreign_keys=ON;
"#;

/// v16: user-defined agents — a named route with no config file behind it.
///
/// The registry has been a constant in `kiwano-core` (`AGENTS`) since the first
/// release, and everything an agent *is* has always been in tables that never
/// heard of it: `agent_strategies`, `agent_bindings`, `placeholder_keys`, and
/// the agent columns of `usage` / `request_logs` are all plain TEXT, and the
/// gateway's route table is built from them without ever consulting a registry
/// (`RouteTable::load`). So a user-defined agent needs exactly this: a row to
/// name it and to hang a key on. No config to rewrite, nothing to detect, no
/// backup to restore — which is the whole point of it.
///
/// Its id is derived from the label and never changes (bindings, strategies,
/// keys and usage all reference it); the label can be edited. `note` is free
/// text for the user ("cheap by day, batch at night").
///
/// There is deliberately no `enabled` column: an agent you do not want is an
/// agent you delete, and its usage history stays either way.
const MIGRATION_V16: &str = r#"
CREATE TABLE IF NOT EXISTS custom_agents (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    note       TEXT,
    created_at TEXT NOT NULL
);
"#;

const MIGRATION_V17: &str = r#"
CREATE TABLE IF NOT EXISTS agent_limits (
    agent        TEXT PRIMARY KEY,
    period_limit REAL NOT NULL,
    limit_unit   TEXT,
    reset_period TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
"#;

const MIGRATION_V18: &str = r#"
DROP TABLE IF EXISTS provider_health;
"#;

// One row per window instead of one per agent. The old rows carry over as the
// window they were: a row with no reset period was "all time", which is `all`
// here. Rebuilding rather than editing `MIGRATION_V17` because that migration has
// already run on every database in existence — including this machine's — and a
// migration that has run is history.
//
// No CHECK on `period`, deliberately. The providers table has one
// (`reset_period IN ('monthly','weekly','yearly')`) and paid for it: "daily" had
// to arrive by rebuilding the table. A list of periods is a policy that grows —
// a five-hour window is the obvious next one — and a constraint that has to be
// dropped to extend it is a constraint that only ever costs a migration.
const MIGRATION_V19: &str = r#"
CREATE TABLE IF NOT EXISTS agent_limits_by_window (
    agent        TEXT NOT NULL,
    period       TEXT NOT NULL,
    period_limit REAL NOT NULL,
    limit_unit   TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (agent, period)
);
INSERT OR REPLACE INTO agent_limits_by_window
    (agent, period, period_limit, limit_unit, created_at, updated_at)
    SELECT agent, COALESCE(reset_period, 'all'), period_limit, limit_unit, created_at, updated_at
    FROM agent_limits;
DROP TABLE agent_limits;
ALTER TABLE agent_limits_by_window RENAME TO agent_limits;
"#;

/// v20: the prices a user declares for their own provider.
///
/// The Hub prices the models of *its* catalog entries, and the gateway resolves
/// a request's price through that entry id — so a provider typed in by hand,
/// which names no entry, is costed at the general rate for a model the Hub
/// happens to know and recorded unpriced for one it does not. The user is the
/// only one who can say what such a provider charges, so the add/edit form asks,
/// and this column holds the answer:
/// `{"currency":"CNY","models":[{"model_id":"kimi-k2","input":"1.5",…}]}`.
///
/// A JSON blob rather than rows beside `model_pricing`, where prices otherwise
/// live, because that table is *equal to the Hub's document*: the seeder prunes
/// every row the document does not carry (`pricing::prune_absent`), and that
/// prune is deliberately not scoped by `source`. Declared rows in there would be
/// deleted by the next sync, or would force that invariant open. Travelling on
/// the provider row also carries them into a config share, which serializes the
/// row itself.
const MIGRATION_V20: &str = r#"
ALTER TABLE providers ADD COLUMN prices TEXT;
"#;

/// v21: the health-probe verdict comes back, narrower than it left.
///
/// v18 dropped this table with the prober that wrote it, and a revert could not
/// have brought it back: the `CREATE` only ever lived in v1, a migration that has
/// already run everywhere. So this is a new migration rather than an undone one —
/// history stands, and a database that is already past v18 gets its table here.
///
/// The shape is not v1's. That one carried `degraded` in its CHECK and a
/// `consecutive_failures` streak, and between them exactly one was ever written
/// and neither was ever read. What the prober now writes is what the Status
/// column shows: `reachable` or `down`, the round trip it measured, and when.
///
/// What the prober *does* is narrower too — see `strategy::prober`: it only asks
/// about providers with no traffic of their own in the last day, which is the
/// only case where the answer is not already in the usage table. That is what
/// makes a background loop affordable again, a decision the v18 commit took the
/// other way round ("a background request per provider per half-minute, on a
/// laptop, for a badge") when it was the *only* source of the number.
const MIGRATION_V21: &str = r#"
CREATE TABLE IF NOT EXISTS provider_health (
    provider_id TEXT PRIMARY KEY REFERENCES providers(id) ON DELETE CASCADE,
    status      TEXT NOT NULL CHECK (status IN ('reachable','down')),
    latency_ms  INTEGER,
    checked_at  TEXT NOT NULL
);
"#;

/// v22: a health verdict says who took it and what came back.
///
/// The table arrived in v21 with the prober, and the Apps screen's own latency
/// test had no way to record what it measured — the button's number was the whole
/// life of the measurement, which left the one case it could have answered (a
/// stale or wrong verdict, with the endpoint plainly working) with nowhere to go.
///
/// `source` names the measurer, because the two are not the same claim: `probe`
/// is an unsigned GET that proves something answers at that address, `test` is a
/// real prompt sent with the provider's key. `error` carries the vendor's own
/// message when the answer was not a success — "invalid API key" — because that
/// is a different fact from silence, and the cell has to be able to say it.
///
/// No rebuild: the v21 CHECK still allows exactly the two status words, and a
/// refusal is not a third one. A 401 *is* reachable — the vendor answered — so
/// what went wrong belongs in `error`, not in the status.
const MIGRATION_V22: &str = r#"
ALTER TABLE provider_health ADD COLUMN source TEXT NOT NULL DEFAULT 'probe';
ALTER TABLE provider_health ADD COLUMN error TEXT;
"#;

/// v23: the Gemini protocol comes back (Gemini CLI's native API), so both
/// protocol CHECK constraints widen to accept it.
///
/// A rebuild rather than an edit of the older migrations: v4 and v9 widened the
/// same constraint for the same protocol before v15 narrowed it again, and
/// migrations replay in order — so the CHECK that survives on disk is whichever
/// rebuild ran last, and only a new one can widen what v15 closed.
///
/// Unlike v15 this deletes nothing: no row is retired here, and the column list
/// is v15's plus `prices` (added by v20 — the only column change since), or the
/// rebuild would silently drop it.
/// v24: a user-defined agent can say which protocol it speaks.
///
/// Additive, and deliberately unconstrained: the vocabulary is three words
/// today and may grow, and v19 already paid for the lesson — a constraint that
/// has to be dropped to extend it is a constraint that only ever costs a
/// migration (`provider_health.period` has none for the same reason). NULL is
/// "the user did not say", which is every row that existed before this ran, and
/// it is a different statement from any of the three words.
const MIGRATION_V24: &str = r#"
ALTER TABLE custom_agents ADD COLUMN protocol TEXT;
"#;

/// v25: two more things a request can say about itself.
///
/// `reasoning_tokens` is a slice of `output_tokens`, not an addition to it —
/// the providers that report it bill it as output, and the ones that do not
/// report nothing. Stored because it is the one number that separates "this
/// model thought for a long time" from "this model wrote a lot", and a tuning
/// question about latency or spend cannot tell those apart without it.
///
/// `usage_missing` marks the rows where the upstream said nothing about usage
/// at all. Those rows have always been written with zeros, which is
/// indistinguishable from a request that genuinely used nothing — and the
/// difference matters to anyone adding them up. It defaults to 0 because every
/// row written before this column existed was written by code that had no way
/// to say otherwise, and guessing now would put a claim in the data that
/// nothing supports.
const MIGRATION_V25: &str = r#"
ALTER TABLE request_logs ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE request_logs ADD COLUMN usage_missing INTEGER NOT NULL DEFAULT 0;
"#;

/// v26: what the compat shim changed about a request, in the request's own
/// row.
///
/// The shim (providers/shim.rs in adapters) edits a passthrough body before
/// it reaches an upstream — removing parameters the upstream's parser cannot
/// know, filling schemas it would refuse. Those edits are the gateway doing
/// something the client did not ask for, on the client's behalf, and a request
/// log that showed only the outcome would leave the middle unexplained. The
/// notes are the explanation: one line per action, NULL when nothing was
/// touched — which is almost every row, and deliberately a different
/// statement from any note.
///
/// Never folded into `error_kind`/`error_message`: those gate the UI's error
/// rendering, and a sanitized request that answered 200 is not an error.
const MIGRATION_V26: &str = r#"
ALTER TABLE request_logs ADD COLUMN request_notes TEXT;
"#;

const MIGRATION_V23: &str = r#"
PRAGMA foreign_keys=OFF;
CREATE TABLE providers_new (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    protocol      TEXT NOT NULL DEFAULT 'anthropic'
                  CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url      TEXT NOT NULL,
    api_path      TEXT,
    api_key       TEXT,
    billing       TEXT NOT NULL DEFAULT 'metered'
                  CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit  REAL,
    limit_unit    TEXT
                  CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens')
                         OR (length(limit_unit) = 3 AND upper(limit_unit) = limit_unit)),
    plan_query    TEXT,
    reset_period  TEXT
                  CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    timeout_secs  INTEGER,
    retries       INTEGER,
    headers       TEXT,
    plan_limits   TEXT,
    catalog_id    TEXT,
    model_default TEXT,
    prices        TEXT
);
INSERT INTO providers_new (id, name, catalog_id, protocol, base_url, api_path, api_key,
                           billing, period_limit, limit_unit, plan_query, plan_limits,
                           timeout_secs, retries, headers, reset_period, enabled,
                           created_at, updated_at, model_default, prices)
    SELECT id, name, catalog_id, protocol, base_url, api_path, api_key,
           billing, period_limit, limit_unit, plan_query, plan_limits,
           timeout_secs, retries, headers, reset_period, enabled,
           created_at, updated_at, model_default, prices
    FROM providers;
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;

CREATE TABLE provider_endpoints_new (
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    protocol    TEXT NOT NULL CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url    TEXT NOT NULL,
    api_path    TEXT,
    PRIMARY KEY (provider_id, protocol)
);
INSERT INTO provider_endpoints_new (provider_id, protocol, base_url, api_path)
    SELECT provider_id, protocol, base_url, api_path FROM provider_endpoints;
DROP TABLE provider_endpoints;
ALTER TABLE provider_endpoints_new RENAME TO provider_endpoints;
PRAGMA foreign_keys=ON;
"#;

const MIGRATION_V13: &str = r#"
ALTER TABLE model_pricing ADD COLUMN tiers TEXT;

ALTER TABLE usage ADD COLUMN cost_off_peak REAL;
ALTER TABLE request_logs ADD COLUMN cost_off_peak REAL;

UPDATE usage SET cost_off_peak = cost WHERE cost IS NOT NULL;
UPDATE request_logs SET cost_off_peak = cost WHERE cost IS NOT NULL;
"#;

impl Store {
    /// Open (creating if needed) and migrate a database file.
    pub fn open(path: impl AsRef<Path>) -> Result<Store> {
        let conn = Connection::open(path.as_ref())?;
        let store = Store {
            conn: Mutex::new(conn),
            path: Some(path.as_ref().to_path_buf()),
        };
        store.configure_and_migrate()?;
        // After migrate, not before: WAL mode is what creates -wal and -shm,
        // and they carry the same credential rows as the database until the
        // next checkpoint.
        harden_permissions(path.as_ref());
        Ok(store)
    }

    /// In-memory store, mainly for quick experiments (tests use tempfile files
    /// so WAL behaviour matches production).
    pub fn open_in_memory() -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        let store = Store {
            conn: Mutex::new(conn),
            path: None,
        };
        store.configure_and_migrate()?;
        Ok(store)
    }

    fn configure_and_migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_millis(5_000))?;
        self.migrate(&conn)?;
        Ok(())
    }

    /// Idempotent schema migration using `PRAGMA user_version`.
    fn migrate(&self, conn: &Connection) -> Result<()> {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(MIGRATION_V1)?;
        }
        if version < 2 {
            conn.execute_batch(MIGRATION_V2)?;
        }
        if version < 3 {
            conn.execute_batch(MIGRATION_V3)?;
        }
        if version < 4 {
            conn.execute_batch(MIGRATION_V4)?;
        }
        if version < 5 {
            conn.execute_batch(MIGRATION_V5)?;
        }
        if version < 6 {
            // Repair: early dev builds stamped user_version ahead of the final
            // migration bodies, so a database can claim v5 while missing the v3
            // api_keys table, the v4 gemini-capable providers shape, or the v5
            // request-log tables. V3/V5 are IF NOT EXISTS and V4 is a lossless
            // named-column swap, so replaying all three on a complete schema
            // (or a partial one) converges to the target state.
            conn.execute_batch(MIGRATION_V3)?;
            conn.execute_batch(MIGRATION_V4)?;
            conn.execute_batch(MIGRATION_V5)?;
        }
        if version < 7 {
            conn.execute_batch(MIGRATION_V7)?;
        }
        if version < 8 {
            conn.execute_batch(MIGRATION_V8)?;
        }
        if version < 9 {
            conn.execute_batch(MIGRATION_V9)?;
        }
        if version < 10 {
            conn.execute_batch(MIGRATION_V10)?;
        }
        if version < 11 {
            conn.execute_batch(MIGRATION_V11)?;
        }
        if version < 12 {
            conn.execute_batch(MIGRATION_V12)?;
        }
        if version < 13 {
            conn.execute_batch(MIGRATION_V13)?;
        }
        if version < 14 {
            conn.execute_batch(MIGRATION_V14)?;
        }
        if version < 15 {
            // Counted before the delete rather than after: a user with a gemini
            // provider should be able to find out where it went, and the only
            // other trace is a catalog entry that is no longer published.
            let dropped: i64 = conn.query_row(
                "SELECT COUNT(*) FROM providers WHERE protocol = 'gemini'",
                [],
                |r| r.get(0),
            )?;
            conn.execute_batch(MIGRATION_V15)?;
            if dropped > 0 {
                eprintln!(
                    "kiwano: removed {dropped} provider(s) whose protocol (gemini) is no longer supported"
                );
            }
        }
        if version < 16 {
            conn.execute_batch(MIGRATION_V16)?;
        }
        if version < 17 {
            conn.execute_batch(MIGRATION_V17)?;
        }
        if version < 18 {
            conn.execute_batch(MIGRATION_V18)?;
        }
        if version < 19 {
            conn.execute_batch(MIGRATION_V19)?;
        }
        if version < 20 {
            conn.execute_batch(MIGRATION_V20)?;
        }
        if version < 21 {
            conn.execute_batch(MIGRATION_V21)?;
        }
        if version < 22 {
            conn.execute_batch(MIGRATION_V22)?;
        }
        if version < 23 {
            conn.execute_batch(MIGRATION_V23)?;
        }
        if version < 24 {
            conn.execute_batch(MIGRATION_V24)?;
        }
        if version < 25 {
            conn.execute_batch(MIGRATION_V25)?;
        }
        if version < 26 {
            conn.execute_batch(MIGRATION_V26)?;
        }
        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::logs::RequestLogFilter;
    use crate::store::test_support::{sample_log, sample_provider, temp_store};
    use crate::store::types::{Billing, Binding, CustomAgent, Protocol};

    #[test]
    fn migration_creates_tables_and_wal() {
        let (_dir, store) = temp_store();
        let conn = store.conn.lock().unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "wal");

        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        for expected in [
            "providers",
            "agent_strategies",
            "agent_bindings",
            "placeholder_keys",
            "usage",
            "request_logs",
            "request_bodies",
            "gateway_settings",
            "provider_endpoints",
            "custom_agents",
        ] {
            assert!(tables.iter().any(|t| t == expected), "missing {expected}");
        }

        // Migration must be idempotent.
        drop(conn);
        let (_dir2, store2) = temp_store();
        store2.metrics().expect("re-migrate ok");
    }

    /// v3 → v4 rebuild: existing providers, bindings and keys survive.
    #[test]
    fn migration_v4_rebuilds_providers_and_keeps_their_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v3 database (pre-Gemini CHECK) with data in it.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-ant', 'Old', 'anthropic', 'https://api.anthropic.com', 'metered', 't0', 't0');
                 INSERT INTO agent_bindings (agent, provider_id, priority, weight, enabled)
                 VALUES ('claude', 'p-ant', 0, 1, 1);
                 INSERT INTO api_keys (provider_id, api_key, enabled, created_at)
                 VALUES ('p-ant', 'sk-x', 1, 't0');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let got = store.get_provider("p-ant").unwrap().expect("provider kept");
        assert_eq!(got.protocol, Protocol::Anthropic);
        assert_eq!(got.limit_unit, None);
        assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 1);
        assert_eq!(store.list_api_keys("p-ant").unwrap().len(), 1);

        // The tag is writable by the time every migration has run: v4 widened
        // this CHECK for Gemini CLI, v9 carried the widening forward, v15
        // rebuilt both tables narrow, and v23 — the last word — widened it back
        // (see the v15 test below for the rows written in between).
        let conn = Connection::open(&db).unwrap();
        assert!(
            conn.execute(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-gem', 'Gem', 'gemini', 'https://g.example.com', 'metered', 't0', 't0')",
                [],
            )
            .is_ok(),
            "the gemini tag is accepted again after v23"
        );
        conn.execute("DELETE FROM providers WHERE id = 'p-gem'", [])
            .unwrap();
    }

    /// v14 → v15: the gemini protocol's rows go, and the tag stops being
    /// writable — including on a database that was built while it was allowed.
    #[test]
    fn migration_v15_drops_the_gemini_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v14 database — the state a build with the protocol
        // leaves behind: a gemini provider with rows hanging off it, next to
        // one that must survive.
        {
            let conn = Connection::open(&db).unwrap();
            for migration in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
            ] {
                conn.execute_batch(migration).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, api_key, billing, created_at, updated_at)
                 VALUES ('p-gem', 'Gem', 'gemini', 'https://g.example.com', 'sk-g', 'metered', 't0', 't0'),
                        ('p-keep', 'Keep', 'openai', 'https://k.example.com', 'sk-k', 'metered', 't0', 't0');
                 INSERT INTO agent_bindings (agent, provider_id, priority, weight, enabled)
                 VALUES ('gemini', 'p-gem', 0, 1, 1), ('codex', 'p-keep', 0, 1, 1);
                 INSERT INTO agent_strategies (agent, type) VALUES ('gemini', 'single');
                 INSERT INTO api_keys (provider_id, api_key, enabled, created_at)
                 VALUES ('p-gem', 'sk-g2', 1, 't0');
                 INSERT INTO provider_endpoints (provider_id, protocol, base_url)
                 VALUES ('p-gem', 'gemini', 'https://g.example.com');
                 INSERT INTO model_pricing (provider_id, model_id, display_name, input, output)
                 VALUES ('p-gem', 'gemini-2.5-pro', 'Pro', '1', '2');
                 INSERT INTO placeholder_keys (key, agent, created_at)
                 VALUES ('kw-ag-gemini-old', 'gemini', 't0');
                 PRAGMA user_version = 14;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();

        // The gemini rows are gone, by name and by provider.
        assert!(store.get_provider("p-gem").unwrap().is_none());
        assert!(store.bindings_for_agent("gemini").unwrap().is_empty());
        assert!(store.get_strategy("gemini").unwrap().is_none());
        assert!(store
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != "gemini"));
        let conn = Connection::open(&db).unwrap();
        for (table, predicate) in [
            ("api_keys", "provider_id = 'p-gem'"),
            ("provider_endpoints", "provider_id = 'p-gem'"),
            ("model_pricing", "provider_id = 'p-gem'"),
        ] {
            let left: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(left, 0, "{table} still holds a dropped provider's rows");
        }

        // …and everything else is exactly as it was.
        assert!(store.get_provider("p-keep").unwrap().is_some());
        assert_eq!(store.bindings_for_agent("codex").unwrap().len(), 1);
        assert_eq!(store.list_api_keys("p-keep").unwrap().len(), 0);
        // v15 closed the tag, and v23 reopened it: writing a gemini provider
        // succeeds again, while the rows v15 deleted above stay deleted.
        assert!(conn
            .execute(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-gem2', 'Gem', 'gemini', 'https://g.example.com', 'metered', 't0', 't0')",
                [],
            )
            .is_ok());
        conn.execute("DELETE FROM providers WHERE id = 'p-gem2'", [])
            .unwrap();
    }

    /// v8 → v9 rebuild: providers survive with widened limit_unit CHECK
    /// (legacy 'cny' uppercased, units preserved), plan_query backfilled NULL,
    /// model_pricing created, usage/request_logs gain cost columns.
    // A migration that carries data is where data goes missing quietly, so the
    // carry-over gets its own case: a database already at v18, holding a ceiling
    // written when an agent could only have one.
    #[test]
    fn migration_v19_carries_a_stored_ceiling_over_as_its_window() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");
        {
            let conn = Connection::open(&db).unwrap();
            for migration in [MIGRATION_V1, MIGRATION_V17] {
                conn.execute_batch(migration).unwrap();
            }
            // This fixture hand-builds a v18 database, so `providers` has to
            // look like one: the columns the migrations between v1 and v18 add.
            // A later migration rebuilds that table and reads its columns by
            // name, so a providers table left at the v1 shape fails there
            // rather than here — with a `no such column` that says nothing
            // about agent ceilings. (v20 adds `prices`; that migration runs.)
            //
            // `custom_agents` is the same kind of debt for the same reason, and
            // it is not covered by running V17 alone: the table comes from v16,
            // and stamping 18 makes the ladder skip it (v16 runs only under
            // `version < 16`). v24 alters that table, so a database claiming to
            // be v18 has to have it — a real one would, having been through v16.
            //
            // `request_logs` is the third of these, and the one that shows why
            // the pattern is worth naming: v5 creates it, v25 alters it, and a
            // fixture that stamps 18 having skipped v5 fails at the second.
            // Anything a later migration touches has to exist here, in the shape
            // the migrations up to this version would have left it.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS request_logs (
                     id                    INTEGER PRIMARY KEY AUTOINCREMENT,
                     ts                    TEXT NOT NULL,
                     method                TEXT NOT NULL,
                     path                  TEXT NOT NULL,
                     query                 TEXT,
                     agent                 TEXT,
                     attribution           TEXT,
                     provider_id           TEXT,
                     model                 TEXT,
                     status_code           INTEGER NOT NULL,
                     error_kind            TEXT,
                     error_message         TEXT,
                     session_id            TEXT,
                     is_streaming          INTEGER NOT NULL DEFAULT 0,
                     input_tokens          INTEGER NOT NULL DEFAULT 0,
                     output_tokens         INTEGER NOT NULL DEFAULT 0,
                     cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
                     cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                     latency_ms            INTEGER,
                     first_token_ms        INTEGER,
                     request_headers       TEXT,
                     response_headers      TEXT,
                     request_size          INTEGER NOT NULL DEFAULT 0,
                     response_size         INTEGER NOT NULL DEFAULT 0,
                     truncated             INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS custom_agents (
                     id         TEXT PRIMARY KEY,
                     label      TEXT NOT NULL,
                     note       TEXT,
                     created_at TEXT NOT NULL
                 );",
            )
            .unwrap();
            conn.execute_batch(
                "ALTER TABLE providers ADD COLUMN limit_unit TEXT;
                 ALTER TABLE providers ADD COLUMN timeout_secs INTEGER;
                 ALTER TABLE providers ADD COLUMN retries INTEGER;
                 ALTER TABLE providers ADD COLUMN headers TEXT;
                 ALTER TABLE providers ADD COLUMN plan_limits TEXT;
                 ALTER TABLE providers ADD COLUMN plan_query TEXT;
                 ALTER TABLE providers ADD COLUMN catalog_id TEXT;
                 ALTER TABLE providers ADD COLUMN model_default TEXT;
                 CREATE TABLE IF NOT EXISTS provider_endpoints (
                     provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
                     protocol    TEXT NOT NULL CHECK (protocol IN ('anthropic','openai')),
                     base_url    TEXT NOT NULL,
                     api_path    TEXT,
                     PRIMARY KEY (provider_id, protocol)
                 );",
            )
            .unwrap();
            conn.execute_batch(
                "INSERT INTO agent_limits (agent, period_limit, limit_unit, reset_period, created_at, updated_at)
                 VALUES ('claude', 50.0, 'CNY', 'monthly', 't0', 't0'),
                        ('codex', 100.0, NULL, NULL, 't0', 't0');
                 PRAGMA user_version = 18;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();

        // Each row arrives as the window it was: the one that named a reset period
        // keeps it, and the one with none was "all time", which the new key spells
        // `all` — a nullable key is not a key, since SQLite treats every NULL as
        // distinct.
        let claude = store.agent_limits_for("claude").unwrap();
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].period, "monthly");
        assert_eq!(claude[0].period_limit, 50.0);
        assert_eq!(claude[0].limit_unit.as_deref(), Some("CNY"));

        let codex = store.agent_limits_for("codex").unwrap();
        assert_eq!(codex.len(), 1, "the row with no reset period survives too");
        assert_eq!(codex[0].period, "all");
        assert_eq!(codex[0].limit_unit, None);
    }

    #[test]
    fn migration_v9_rebuilds_providers_with_plan_query_and_cost_columns() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v8 database (pre-pricing) with data in it.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(MIGRATION_V4).unwrap();
            conn.execute_batch(MIGRATION_V5).unwrap();
            conn.execute_batch(MIGRATION_V7).unwrap();
            conn.execute_batch(MIGRATION_V8).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        period_limit, limit_unit, created_at, updated_at)
                 VALUES ('p-cny', 'Old', 'anthropic', 'https://api.example.com', 'metered',
                         10.0, 'cny', 't0', 't0'),
                        ('p-req', 'Old2', 'openai', 'https://api2.example.com', 'metered',
                         100.0, 'requests', 't0', 't0');
                 PRAGMA user_version = 8;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let cny = store.get_provider("p-cny").unwrap().expect("provider kept");
        // Legacy lowercase currency unit is uppercased; 'requests' preserved.
        assert_eq!(cny.limit_unit.as_deref(), Some("CNY"));
        assert_eq!(cny.plan_query, None);
        assert_eq!(
            store
                .get_provider("p-req")
                .unwrap()
                .unwrap()
                .limit_unit
                .as_deref(),
            Some("requests")
        );
        // Bindings survive the rebuild.
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p-cny".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 1);

        // New v9 shapes exist: model_pricing table + cost columns + 3-letter
        // currency units accepted.
        let conn = store.conn.lock().unwrap();
        let has_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_pricing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_table, 1);
        conn.execute_batch(
            "INSERT INTO model_pricing (model_id, display_name, input, output)
             VALUES ('m-x', 'X', '1', '2');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage (ts, agent, provider_id, status, cost, cost_currency)
             VALUES ('t0', 'claude', 'p-cny', 'ok', 0.5, 'USD')",
            [],
        )
        .unwrap();
        drop(conn);

        // Currency limit units pass the widened CHECK; plan_query round-trips.
        let mut p = sample_provider("p-usd", Protocol::OpenAI);
        p.limit_unit = Some("USD".to_string());
        p.plan_query = Some(r#"{"template":"kimi"}"#.to_string());
        store.insert_provider(&p).unwrap();
        let got = store.get_provider("p-usd").unwrap().expect("usd provider");
        assert_eq!(got.limit_unit.as_deref(), Some("USD"));
        assert_eq!(got.plan_query.as_deref(), Some(r#"{"template":"kimi"}"#));
    }

    /// v9 → v10: rows survive and gain a NULL plan_limits column; the percent
    /// limits JSON round-trips through insert/update/get.
    #[test]
    fn migration_v10_adds_plan_limits_column() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v9 database with a provider in it.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(MIGRATION_V4).unwrap();
            conn.execute_batch(MIGRATION_V5).unwrap();
            conn.execute_batch(MIGRATION_V7).unwrap();
            conn.execute_batch(MIGRATION_V8).unwrap();
            conn.execute_batch(MIGRATION_V9).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        period_limit, limit_unit, created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered',
                         10.0, 'requests', 't0', 't0');
                 PRAGMA user_version = 9;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        // v9 row survives with its data; plan_limits reads as NULL.
        let old = store.get_provider("p-old").unwrap().expect("kept");
        assert_eq!(old.limit_unit.as_deref(), Some("requests"));
        assert_eq!(old.plan_limits, None);

        // Percent limits round-trip; NULL clears.
        let mut p = sample_provider("p-plan", Protocol::Anthropic);
        p.billing = Billing::Subscription;
        p.plan_limits = Some(r#"{"five_hour":20,"weekly":60}"#.to_string());
        store.insert_provider(&p).unwrap();
        let got = store
            .get_provider("p-plan")
            .unwrap()
            .expect("plan provider");
        assert_eq!(
            got.plan_limits.as_deref(),
            Some(r#"{"five_hour":20,"weekly":60}"#)
        );
        p.plan_limits = None;
        store.update_provider(&p).unwrap();
        assert_eq!(
            store.get_provider("p-plan").unwrap().unwrap().plan_limits,
            None
        );
    }

    /// v10 → v11: the price table is rebuilt keyed by `(provider_id, model_id)`.
    /// A v10 row survives as the general price, and two providers can now price
    /// the same model — which the old key could not express at all.
    /// v14 adds the column the add/edit form has always collected and never
    /// stored. A row written before it reads back as "not set" — which is what
    /// it was — and the value round-trips once the column exists.
    #[test]
    fn migration_v14_adds_the_default_model_column() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v13 database with a provider in it.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered',
                         't0', 't0');
                 PRAGMA user_version = 13;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let old = store.get_provider("p-old").unwrap().expect("kept");
        assert_eq!(
            old.model_default, None,
            "a row written before the column has no default model"
        );

        // The value round-trips, and NULL clears it: an emptied box is "not
        // set" rather than the empty string.
        let mut p = sample_provider("p-model", Protocol::OpenAI);
        p.model_default = Some("deepseek-v4-pro".into());
        store.insert_provider(&p).unwrap();
        assert_eq!(
            store
                .get_provider("p-model")
                .unwrap()
                .unwrap()
                .model_default
                .as_deref(),
            Some("deepseek-v4-pro")
        );
        p.model_default = None;
        store.update_provider(&p).unwrap();
        assert_eq!(
            store
                .get_provider("p-model")
                .unwrap()
                .unwrap()
                .model_default,
            None
        );
    }

    /// v19 → v20: an existing provider gains the declared-prices column, NULL —
    /// "none declared", which is the state of every row that predates the form
    /// asking for them.
    #[test]
    fn migration_v20_adds_the_declared_prices_column() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v19 database with a provider in it.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered',
                         't0', 't0');
                 PRAGMA user_version = 19;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let old = store.get_provider("p-old").unwrap().expect("kept");
        assert_eq!(
            old.prices, None,
            "a row written before the column declares none"
        );
        assert!(store.load_declared_prices().unwrap().is_empty());
    }

    /// v22 → v23: the protocol CHECK widens for Gemini again, and the rebuild
    /// keeps every column and every row.
    ///
    /// A rebuild whose column list is stale drops what it forgets, silently —
    /// which is what this case exists to catch, `prices` in particular: v20
    /// added it *after* the shape v15 left behind, so a v23 that copied v15's
    /// column list would take the declared prices with it.
    #[test]
    fn migration_v23_widens_the_protocol_check_for_gemini() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v22 database holding a provider with everything set.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
                MIGRATION_V20,
                MIGRATION_V21,
                MIGRATION_V22,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at,
                                        updated_at, prices, model_default)
                 VALUES ('p-ant', 'Ant', 'anthropic', 'https://api.example.com', 'metered',
                         't0', 't0', '{\"m\":{\"input\":1.0,\"output\":2.0}}', 'claude-sonnet-4-5');
                 INSERT INTO agent_bindings (agent, provider_id, priority)
                 VALUES ('claude', 'p-ant', 0);
                 INSERT INTO provider_endpoints (provider_id, protocol, base_url)
                 VALUES ('p-ant', 'openai', 'https://api.example.com/v1');
                 PRAGMA user_version = 22;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();

        let kept = store.get_provider("p-ant").unwrap().expect("kept");
        assert!(
            kept.prices.is_some(),
            "the prices column survives the rebuild"
        );
        assert_eq!(kept.model_default.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(kept.endpoints.len(), 1, "the extra endpoint survives");
        assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 1);

        // And the widened CHECK: the tag is writable again.
        let conn = Connection::open(&db).unwrap();
        assert!(
            conn.execute(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-gem', 'Gem', 'gemini', 'https://generativelanguage.googleapis.com', 'metered', 't0', 't0')",
                [],
            )
            .is_ok(),
            "v23 accepts the gemini tag"
        );
    }

    /// v25 adds two columns to a table older builds already filled. The rows
    /// that were there have to come back with the column defaults rather than
    /// with a claim: nothing can now know whether a row written before the
    /// column existed had unknown usage, so the migration says "reported".
    #[test]
    fn migration_v25_adds_reasoning_and_the_usage_mark() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v24 database holding one request, logged by a build that
        // knew nothing about either column.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
                MIGRATION_V20,
                MIGRATION_V21,
                MIGRATION_V22,
                MIGRATION_V23,
                MIGRATION_V24,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO request_logs (ts, method, path, status_code, input_tokens,
                                           output_tokens, cache_read_tokens,
                                           cache_creation_tokens, request_size, response_size)
                 VALUES ('2026-09-07T10:00:00+00:00', 'POST', '/v1/messages', 200, 15302,
                         102, 15232, 0, 10, 20);
                 PRAGMA user_version = 24;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let (rows, _) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(rows.len(), 1, "the row an older build wrote is still there");
        assert_eq!(rows[0].input_tokens, 15302);
        assert_eq!(
            rows[0].reasoning_tokens, 0,
            "the new column reads its default on a row that predates it"
        );
        assert!(
            !rows[0].usage_missing,
            "and the mark says reported, because nothing can say otherwise"
        );

        // The columns are writable: a row written now carries both.
        let mut log = sample_log("2026-09-07T11:00:00+00:00", Some("codex"), 200);
        log.reasoning_tokens = 71;
        log.usage_missing = true;
        store.insert_request_log(&log).unwrap();
        let (rows, _) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(rows[0].reasoning_tokens, 71);
        assert!(rows[0].usage_missing);
    }

    #[test]
    fn migration_v26_adds_request_notes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v25 database holding one request, logged by a build
        // that knew nothing about the shim's notes.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
                MIGRATION_V20,
                MIGRATION_V21,
                MIGRATION_V22,
                MIGRATION_V23,
                MIGRATION_V24,
                MIGRATION_V25,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO request_logs (ts, method, path, status_code, input_tokens,
                                           output_tokens, cache_read_tokens,
                                           cache_creation_tokens, request_size, response_size)
                 VALUES ('2026-09-07T10:00:00+00:00', 'POST', '/v1/messages', 200, 15302,
                         102, 15232, 0, 10, 20);
                 PRAGMA user_version = 25;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let (rows, _) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(rows.len(), 1, "the row an older build wrote is still there");
        assert_eq!(
            rows[0].request_notes, None,
            "the new column reads its default on a row that predates it"
        );

        // The column is writable: a row written now carries its notes, and a
        // shim-free row stays NULL — not empty text, so the UI can tell
        // "nothing happened" from a note that has not arrived.
        let mut log = sample_log("2026-09-07T11:00:00+00:00", Some("codex"), 200);
        log.request_notes = Some("thinking: removed unsupported type \"adaptive\"".into());
        store.insert_request_log(&log).unwrap();
        let mut clean = sample_log("2026-09-07T12:00:00+00:00", Some("codex"), 200);
        clean.request_notes = None;
        store.insert_request_log(&clean).unwrap();
        let (rows, _) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(
            rows[1].request_notes.as_deref(),
            Some("thinking: removed unsupported type \"adaptive\""),
            "newest first: the clean 12:00 row reads [0]"
        );
        assert_eq!(rows[0].request_notes, None);
    }

    #[test]
    /// v24 adds the column without disturbing what was already there: an agent
    /// defined before it says nothing about its protocol, and saying nothing is
    /// not one of the three words.
    fn migration_v24_lets_a_custom_agent_name_its_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v23 database holding an agent defined before the column.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
                MIGRATION_V20,
                MIGRATION_V21,
                MIGRATION_V22,
                MIGRATION_V23,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO custom_agents (id, label, note, created_at)
                 VALUES ('long-tasks-3f9a', 'Long tasks', 'batch at night', 't0');
                 PRAGMA user_version = 23;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let agents = store.list_custom_agents().unwrap();
        assert_eq!(agents.len(), 1, "the migration keeps the row");
        assert_eq!(agents[0].label, "Long tasks");
        assert_eq!(agents[0].note.as_deref(), Some("batch at night"));
        assert_eq!(agents[0].protocol, None, "it never said");

        // A new agent can say it at creation…
        store
            .insert_custom_agent(&CustomAgent {
                id: "nightly-7c21".to_string(),
                label: "Nightly".to_string(),
                note: None,
                protocol: Some("gemini".to_string()),
                created_at: "t1".to_string(),
            })
            .unwrap();
        let created = store.get_custom_agent("nightly-7c21").unwrap().unwrap();
        assert_eq!(created.protocol.as_deref(), Some("gemini"));

        // …and an existing one can change its mind, or leave it alone: the
        // rename call is not about the protocol, so it travels back unchanged.
        store
            .update_custom_agent_label(
                "long-tasks-3f9a",
                "Long tasks",
                Some("batch at night"),
                Some("anthropic"),
            )
            .unwrap();
        let reread = store.get_custom_agent("long-tasks-3f9a").unwrap().unwrap();
        assert_eq!(reread.protocol.as_deref(), Some("anthropic"));
    }

    #[test]
    fn migration_v11_keys_prices_by_provider() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v10 database holding one price row and one provider.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(MIGRATION_V4).unwrap();
            conn.execute_batch(MIGRATION_V5).unwrap();
            conn.execute_batch(MIGRATION_V7).unwrap();
            conn.execute_batch(MIGRATION_V8).unwrap();
            conn.execute_batch(MIGRATION_V9).unwrap();
            conn.execute_batch(MIGRATION_V10).unwrap();
            conn.execute_batch(
                "INSERT INTO model_pricing (model_id, display_name, input, output,
                                            cache_read, cache_creation, currency, source)
                 VALUES ('m1', 'M1', '1', '2', '0', '0', 'USD', 'hub');
                 INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered', 't0', 't0');
                 PRAGMA user_version = 10;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();

        // The row is kept, as the general price: nothing before v11 had a
        // provider to name.
        let rows = store.load_model_pricing().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "");
        assert_eq!(rows[0].model_id, "m1");
        assert_eq!(rows[0].input, "1");

        // Two providers, one model: the pair is the key now.
        let mut kimi = rows[0].clone();
        kimi.provider_id = "kimi".into();
        kimi.input = "3".into();
        store.upsert_model_pricing(&kimi).unwrap();
        let mut rows = store.load_model_pricing().unwrap();
        rows.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
        assert_eq!(rows.len(), 2, "the general row and kimi's");
        assert_eq!(rows[1].provider_id, "kimi");
        assert_eq!(rows[1].input, "3", "and each keeps its own price");

        // v12 comes with it: a pre-v12 provider has no catalog entry …
        let old = store.get_provider("p-old").unwrap().expect("kept");
        assert_eq!(old.catalog_id, None);
        // … and one set now round-trips.
        let mut p = sample_provider("p-shelf", Protocol::Anthropic);
        p.catalog_id = Some("kimi".into());
        store.insert_provider(&p).unwrap();
        assert_eq!(
            store.get_provider("p-shelf").unwrap().unwrap().catalog_id,
            Some("kimi".to_string())
        );
        p.catalog_id = None;
        store.update_provider(&p).unwrap();
        assert_eq!(
            store.get_provider("p-shelf").unwrap().unwrap().catalog_id,
            None
        );
    }

    /// v12 → v13: the mirror gains a JSON blob, and the two cost tables gain the
    /// off-peak figure with a backfill. `ALTER TABLE ADD COLUMN` is all it takes
    /// — no key or CHECK moves — and the backfill is what keeps a window's
    /// headline and its peak premium comparable for rows written before the
    /// column existed: without it the two sums cover different row sets.
    #[test]
    fn migration_v13_backfills_the_off_peak_cost() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v12 database holding a priced usage row, an unpriced one,
        // a log row, and a price row.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO usage (ts, agent, provider_id, model, input_tokens, output_tokens,
                                    cache_read_tokens, cache_creation_tokens, latency_ms, status,
                                    cost, cost_currency)
                 VALUES ('t0', 'claude', 'p1', 'm1', 10, 2, 0, 0, 100, 'ok', 1.5, 'USD'),
                        ('t0', 'claude', 'p1', 'm2', 10, 2, 0, 0, 100, 'ok', NULL, NULL);
                 INSERT INTO request_logs (ts, method, path, agent, status_code, input_tokens,
                                           output_tokens, cache_read_tokens,
                                           cache_creation_tokens, cost, cost_currency)
                 VALUES ('t0', 'POST', '/v1/messages', 'claude', 200, 10, 2, 0, 0, 1.5, 'USD');
                 INSERT INTO model_pricing (provider_id, model_id, display_name, input, output,
                                            cache_read, cache_creation, currency, source)
                 VALUES ('', 'm1', 'M1', '1', '2', '0', '0', 'USD', 'hub');
                 PRAGMA user_version = 12;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let conn = store.conn.lock().unwrap();

        // The priced row says "off-peak would have cost the same" — the only
        // honest answer for a row priced before a schedule existed — and the
        // unpriced one stays NULL in both.
        let row = |id: i64| -> (Option<f64>, Option<f64>) {
            conn.query_row(
                "SELECT cost, cost_off_peak FROM usage WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(row(1), (Some(1.5), Some(1.5)));
        assert_eq!(row(2), (None, None));
        let logged: Option<f64> = conn
            .query_row("SELECT cost_off_peak FROM request_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, Some(1.5));

        // The mirror's new column starts NULL, and a row with no schedule reads
        // back with none.
        let tiers: Option<String> = conn
            .query_row("SELECT tiers FROM model_pricing", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tiers, None);
        drop(conn);
        let rows = store.load_model_pricing().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].off_peak.is_none() && rows[0].peak_hours.is_none());
    }

    /// A database stamped v5 by an early dev build but missing the v3 api_keys
    /// table, the v4 gemini providers shape, and the v5 request-log tables is
    /// repaired on open (version bumped to SCHEMA_VERSION, objects created,
    /// data kept).
    #[test]
    fn migrate_repairs_database_stamped_ahead_of_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-ant', 'Old', 'anthropic', 'https://api.anthropic.com', 'metered', 't0', 't0');
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            let version: i32 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version, SCHEMA_VERSION);

            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table'
                       AND name IN ('api_keys','request_logs','request_bodies','gateway_settings')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 4, "missing tables repaired");

            // providers rebuilt to the current shape: the repair replays v4,
            // which widened the protocol CHECK for Gemini CLI, v15 rebuilt both
            // tables narrow, and the run ends at v23, which widens it back for
            // the same protocol.
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='providers'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(ddl.contains("gemini"), "{ddl}");
        }
        assert_eq!(
            store
                .get_provider("p-ant")
                .unwrap()
                .expect("provider kept")
                .name,
            "Old"
        );
    }

    /// v21 → v22: a verdict already on file keeps its numbers and gains the two
    /// new columns with the values that describe it — it was the prober's.
    #[test]
    fn migration_v22_backfills_a_verdict_already_on_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
                MIGRATION_V20,
                MIGRATION_V21,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered',
                         't0', 't0');
                 INSERT INTO provider_health (provider_id, status, latency_ms, checked_at)
                 VALUES ('p-old', 'reachable', 42, '2026-09-15T00:00:00Z');
                 PRAGMA user_version = 21;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let rows = store.list_provider_health().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].latency_ms, Some(42), "the numbers survive");
        assert_eq!(
            rows[0].source, "probe",
            "a verdict from before this migration was only ever the prober's"
        );
        assert_eq!(rows[0].error, None, "nothing had recorded a reason");
    }

    /// v20 → v21: the health table comes back for a database that had it dropped.
    #[test]
    fn migration_v21_restores_the_health_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v20 database — the state a build without the prober
        // leaves behind, and the one every install is in right now.
        {
            let conn = Connection::open(&db).unwrap();
            for m in [
                MIGRATION_V1,
                MIGRATION_V2,
                MIGRATION_V3,
                MIGRATION_V4,
                MIGRATION_V5,
                MIGRATION_V7,
                MIGRATION_V8,
                MIGRATION_V9,
                MIGRATION_V10,
                MIGRATION_V11,
                MIGRATION_V12,
                MIGRATION_V13,
                MIGRATION_V14,
                MIGRATION_V15,
                MIGRATION_V16,
                MIGRATION_V17,
                MIGRATION_V18,
                MIGRATION_V19,
                MIGRATION_V20,
            ] {
                conn.execute_batch(m).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered',
                         't0', 't0');
                 PRAGMA user_version = 20;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        // The table exists and is empty; the provider is untouched.
        assert!(store.list_provider_health().unwrap().is_empty());
        store
            .upsert_provider_health("p-old", "reachable", 7, "probe", None)
            .unwrap();
        assert_eq!(store.list_provider_health().unwrap()[0].latency_ms, Some(7));
        assert_eq!(
            store.get_provider("p-old").unwrap().unwrap().name,
            "Old",
            "the migration is additive"
        );
    }
}
