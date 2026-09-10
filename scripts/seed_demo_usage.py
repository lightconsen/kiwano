#!/usr/bin/env python3
"""Fill a scratch copy of the app database with a predictable usage pattern.

Verifying the dashboard means comparing what it shows against numbers you
already know, so this seeds a pattern whose per-window sums are printed at the
end — not plausible-looking noise you would have to recompute by hand.

Two traps it handles for you:

  * the headline request count comes from `request_logs`, while tokens, cost
    and latency come from `usage`. Seeding one table alone gives "0 requests,
    8.8M tokens".
  * the Providers page rings a period limit against the period's cost, so a
    provider's cost is written in the currency its limit is denominated in.
    A USD figure under a CNY ceiling reads as a bug in the quota cell.

Shape: day offset d holds `5 + d % 7` requests — each 1000 in / 200 out / 100
cache-read tokens times a per-day scale, a per-request cost, latency 200 + d ms
— spread over every provider in the database and two agents. The demo
providers it creates cover one billing mode each (subscription, metered,
unlimited), so the Usage/quota column has one of every cell to draw. Today's
rows are spread over the local hours already elapsed, so the hourly "today"
chart has a shape to draw — and so none of them can land after "now".

Usage:
  cp ~/.kiwano/kiwano.db /tmp/kiwano-demo.db   # keeps providers + settings
  scripts/seed_demo_usage.py /tmp/kiwano-demo.db --clear
  KIWANO_DB_PATH=/tmp/kiwano-demo.db pnpm tauri dev
"""
import argparse
import json
import pathlib
import sqlite3
import sys
from datetime import datetime, time, timedelta, timezone

# Rows land at 12:00 *local* on each day: the app's day boundaries follow the
# user's clock (settings.tz_offset_minutes), so a UTC-noon row would belong to
# the wrong local day for most of the world.
LOCAL = datetime.now().astimezone().tzinfo

ROWS_BASE = 5
ROWS_CYCLE = 7
IN_TOKENS, OUT_TOKENS, CACHE_READ = 1000, 200, 100
# Per request, in the provider's own billing currency (see `currencies` below).
# Sized so a period's spend lands mid-range against the demo providers' limits:
# a ring pinned near zero demonstrates nothing about a limit cell.
COST, LATENCY_BASE = 1.0, 200
PROVIDERS = ["demo-alpha", "demo-beta", "demo-local"]
AGENTS = ["claude", "codex"]

# The providers the rows are written against, created here rather than expected
# in the copied database: usage naming a provider that does not exist is
# invisible on the Providers page, so the quota cells had nothing to draw — and
# one of each billing mode is what makes those cells testable.
#
#   demo-alpha  subscription — percent ceilings on the plan's own windows
#   demo-beta   metered      — a spending cap in the provider's own currency
#   demo-local  unlimited    — no metering at all
#
# A plan's limit is a *percentage* of the provider's own plan quota, not an
# amount: the Add-provider modal only offers percent fields for plan, and
# vm::add_provider clears period_limit/limit_unit for plan rows on save. An
# amount limit on a subscription row is legacy data the UI can no longer set.
#
# id, name, protocol, base_url, billing, period_limit, limit_unit, reset_period, plan_limits, plan_query
PROVIDER_ROWS = [
    ("demo-alpha", "Demo Alpha", "anthropic", "https://alpha.demo.invalid", "subscription",
     None, None, None, '{"five_hour":20,"weekly":60}',
     # Any template that is not `opencode_go`: plan_monthly_price only knows a
     # price for that one, and the demo has no subscription price to state.
     '{"template":"zhipu","fields":{}}'),
    ("demo-beta", "Demo Beta", "openai", "https://beta.demo.invalid", "metered",
     30.0, "CNY", "monthly", None, None),
    ("demo-local", "Demo Local", "openai", "http://127.0.0.1:11434/v1", "unlimited",
     None, None, None, None, None),
]

# Live plan report for demo-alpha, as its own endpoint would supply: 14% of the
# five-hour window (against the 20% ceiling above → a 70% ring) and 30% of the
# week (against 60%).
DEMO_PLAN_TIERS = [("five_hour", 14.0), ("weekly_limit", 30.0)]

# The Providers page rings a plan's *live* utilization, which it fetches only
# for providers that have a plan query — and the answer comes from a 5-minute
# cache. The demo has no endpoint to query, so the report is written straight
# into that cache, stamped a year ahead: the TTL compares `now - ts`, so a
# future stamp never expires. A ring that dies five minutes after seeding would
# make the fixture worse than one that lies about a timestamp.
CACHE_STAMP_AHEAD_MS = 365 * 24 * 60 * 60 * 1000


def seed_plan_quota_cache(conn, provider_id, tiers):
    now_ms = int(datetime.now(timezone.utc).timestamp() * 1000)
    report = {
        "provider_id": provider_id,
        "template": "zhipu",
        "success": True,
        "error": None,
        "note": None,
        "tiers": [
            {"name": name, "utilization": util, "resets_at": None,
             "used": None, "limit": None, "unit": None}
            for name, util in tiers
        ],
        "queried_at": now_ms,
        "cached": True,
    }
    conn.execute(
        """INSERT INTO app_settings (key, value) VALUES (?, ?)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value""",
        (f"plan_quota_cache:{provider_id}",
         json.dumps({"ts": now_ms + CACHE_STAMP_AHEAD_MS, "report": report})),
    )


def provider_currencies(conn):
    """provider id → the currency its limit is denominated in (USD otherwise)."""
    out = {}
    for pid, unit in conn.execute("SELECT id, limit_unit FROM providers"):
        out[pid] = unit if unit and len(unit) == 3 else "USD"
    return out


def provider_rotation(conn):
    """Every provider in the database, demo ones included.

    Traffic spread over only the demo providers leaves whatever was already in
    the copied database — usually the real ones — with an empty Usage/quota
    cell beside populated rows, which reads as a bug rather than as a fixture.
    """
    ids = [r[0] for r in conn.execute("SELECT id FROM providers ORDER BY id")]
    return ids or [row[0] for row in PROVIDER_ROWS]


def upsert_providers(conn):
    """Create the demo providers, leaving any existing row's key untouched."""
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    for pid, name, protocol, base_url, billing, limit, unit, reset, limits, query in PROVIDER_ROWS:
        conn.execute(
            """INSERT INTO providers (id, name, protocol, base_url, api_path, api_key,
                                      billing, period_limit, limit_unit, plan_query,
                                      reset_period, enabled, created_at, updated_at, plan_limits)
               VALUES (?,?,?,?,NULL,?,?,?,?,?,?,1,?,?,?)
               ON CONFLICT(id) DO UPDATE SET
                  name = excluded.name, billing = excluded.billing,
                  period_limit = excluded.period_limit, limit_unit = excluded.limit_unit,
                  reset_period = excluded.reset_period, plan_limits = excluded.plan_limits,
                  plan_query = excluded.plan_query,
                  updated_at = excluded.updated_at""",
            (pid, name, protocol, base_url, f"sk-demo-{pid}", billing, limit, unit,
             query, reset, now, now, limits),
        )
    seed_plan_quota_cache(conn, "demo-alpha", DEMO_PLAN_TIERS)


def day_boundaries(days):
    """Noon local on each of the last `days` local dates, oldest first."""
    today = datetime.now(LOCAL).date()
    return [
        datetime.combine(today - timedelta(days=d), time(12), tzinfo=LOCAL).astimezone(timezone.utc)
        for d in range(days)
    ]


def local_day_start(moment):
    """Midnight local on `moment`'s date, as the UTC instant the store sees."""
    midnight = datetime.combine(moment.astimezone(LOCAL).date(), time(0), tzinfo=LOCAL)
    return midnight.astimezone(timezone.utc)


def rows_on(day_index):
    return ROWS_BASE + day_index % ROWS_CYCLE


def day_token_scale(day_index):
    """Tokens per request, deliberately unrelated to the request count.

    A fixture whose two series rise and fall together draws one curve — which
    is exactly what made the chart look broken — so each day gets a
    deterministic scale that does not share the request cycle's period and
    cannot lock phase with it. Real traffic sits somewhere in between.
    """
    return 3 + (day_index * 7919) % 9


def seed(db, days, clear):
    conn = sqlite3.connect(db)
    if clear:
        conn.execute("DELETE FROM usage")
        conn.execute("DELETE FROM request_logs")
        conn.commit()
    upsert_providers(conn)
    conn.commit()
    providers = provider_rotation(conn)
    currencies = provider_currencies(conn)

    now = datetime.now(timezone.utc)
    # Today's rows spread across the local hours already elapsed, so the hourly
    # chart has a day's shape to draw. Stacking them in the seed's own second
    # (as this once did) leaves that chart a single bar with 23 empty ones.
    midnight = local_day_start(now)
    elapsed = (now - midnight).total_seconds()
    inserted = 0
    per_day: list[tuple[int, int, int, float]] = []
    for day_index, day in enumerate(day_boundaries(days)):
        count = rows_on(day_index)
        day_tokens = [0, 0, 0.0]  # in, out, cost
        for i in range(count):
            ts = (
                # Fractions stay strictly inside (0, 1), so every row is after
                # midnight and before "now" — a future one would still count as
                # today's, and local noon can be ahead of the seed.
                midnight + timedelta(seconds=elapsed * (i + 1) / (count + 1))
                if day_index == 0
                else day + timedelta(minutes=i)
            )
            stamp = ts.strftime("%Y-%m-%dT%H:%M:%SZ")
            provider = providers[i % len(providers)]
            agent = AGENTS[i % len(AGENTS)]
            latency = LATENCY_BASE + day_index
            scale = day_token_scale(day_index)
            in_tokens, out_tokens, cache_read = (
                IN_TOKENS * scale, OUT_TOKENS * scale, CACHE_READ * scale,
            )
            # The cost has to be in the currency the provider's limit is
            # denominated in, or the quota cell rings a USD figure against a CNY
            # ceiling. A provider with no currency limit bills as recorded (USD).
            currency = currencies.get(provider, "USD")
            day_tokens[0] += in_tokens
            day_tokens[1] += out_tokens
            day_tokens[2] += COST
            conn.execute(
                """INSERT INTO usage (ts, agent, provider_id, model, input_tokens, output_tokens,
                                      cache_read_tokens, cache_creation_tokens, latency_ms, status,
                                      cost, cost_currency)
                   VALUES (?,?,?,?,?,?,?,?,?, 'ok', ?, ?)""",
                (stamp, agent, provider, "demo-model", in_tokens, out_tokens, cache_read, 0,
                 latency, COST, currency),
            )
            conn.execute(
                """INSERT INTO request_logs (ts, method, path, agent, attribution, provider_id, model,
                                            status_code, is_streaming, input_tokens, output_tokens,
                                            cache_read_tokens, cache_creation_tokens, latency_ms)
                   VALUES (?, 'POST', '/v1/messages', ?, 'kw-ag-demo', ?, 'demo-model',
                           200, 0, ?, ?, ?, 0, ?)""",
                (stamp, agent, provider, in_tokens, out_tokens, cache_read, latency),
            )
            inserted += 1
        per_day.append((count, day_tokens[0], day_tokens[1], day_tokens[2]))
    conn.commit()
    conn.close()
    return inserted, per_day


def preferred_currency(db):
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        row = conn.execute(
            "SELECT json_extract(value, '$.preferred_currency') FROM app_settings WHERE key = 'ui'"
        ).fetchone()
        # The field is only stored once the user touches it; the app falls back
        # to CNY (vm::default_preferred_currency), and so does this.
        return (row[0] if row and row[0] else "CNY")
    finally:
        conn.close()


def rate_for(currency, repo):
    doc = json.loads((pathlib.Path(repo) / "crates/adapters/resources/models.json").read_text())
    return doc["exchange_rates"].get(currency, 1.0)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("db")
    parser.add_argument("--days", type=int, default=30)
    parser.add_argument("--clear", action="store_true", help="drop existing usage/logs first")
    args = parser.parse_args()

    repo = pathlib.Path(__file__).resolve().parent.parent
    inserted, per_day = seed(args.db, args.days, args.clear)
    currency = preferred_currency(args.db)
    rate = rate_for(currency, repo)

    def sums(window_days):
        # Today is the current local day; "7 days" is today plus the six before
        # it — the same whole days the chart draws a point for. Summed from
        # what was inserted, not from the constants, since tokens vary per row.
        window = per_day[:window_days]
        rows = sum(d[0] for d in window)
        return rows, sum(d[1] for d in window), sum(d[2] for d in window), sum(d[3] for d in window)

    print(f"seeded {inserted} request(s) over {args.days} days into {args.db}")
    print(f"display currency: {currency} (rate {rate} per USD)")
    print()
    print(f"{'window':8} {'requests':>9} {'in tokens':>11} {'out tokens':>11} {'cost':>12}")
    for label, days, note in (
        ("Today", 1, "current local day only"),
        ("7 days", 7, "today + the 6 before it, 7 chart points"),
        ("30 days", 30, "today + the 29 before it, 30 chart points"),
    ):
        r, i, o, c = sums(days)
        print(f"{label:8} {r:9} {i:11} {o:11} {c * rate:11.2f} {currency}")
        print(f"{'':8} {note}")
    print()
    print("Per-day shape (the two series pull against each other on purpose):")
    print(f"{'date':>12} {'requests':>9} {'tokens':>10}")
    dates = [d.astimezone(LOCAL).date().isoformat() for d in day_boundaries(args.days)]
    for date, (count, tok_in, _tok_out, _cost) in zip(dates, per_day):
        print(f"{date:>12} {count:9d} {tok_in:10d}")


if __name__ == "__main__":
    sys.exit(main())
