#!/usr/bin/env python3
"""Fill a scratch copy of the app database with a predictable usage pattern.

Verifying the dashboard means comparing what it shows against numbers you
already know, so this seeds a pattern whose per-window sums are printed at the
end — not plausible-looking noise you would have to recompute by hand.

Two traps it handles for you:

  * the headline request count comes from `request_logs`, while tokens, cost
    and latency come from `usage`. Seeding one table alone gives "0 requests,
    8.8M tokens".
  * costs are stored in the price table's currency (USD) and converted for
    display, so the printed expectations include the conversion into your
    preferred currency and the rate used.

Shape: day offset d holds `5 + d % 7` requests of 1000 in / 200 out / 100
cache-read tokens, $0.02, latency 200 + d ms, spread between two providers and
two agents. Today's rows are placed in the recent past so they cannot land
after "now".

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
from datetime import datetime, timedelta, timezone

ROWS_BASE = 5
ROWS_CYCLE = 7
IN_TOKENS, OUT_TOKENS, CACHE_READ = 1000, 200, 100
COST_USD, LATENCY_BASE = 0.02, 200
PROVIDERS = ["demo-alpha", "demo-beta"]
AGENTS = ["claude", "codex"]


def day_keys(days):
    now = datetime.now(timezone.utc)
    return [now - timedelta(days=d) for d in range(days)]


def rows_on(day_index):
    return ROWS_BASE + day_index % ROWS_CYCLE


def seed(db, days, clear):
    conn = sqlite3.connect(db)
    if clear:
        conn.execute("DELETE FROM usage")
        conn.execute("DELETE FROM request_logs")
        conn.commit()

    now = datetime.now(timezone.utc)
    inserted = 0
    for day_index, day in enumerate(day_keys(days)):
        count = rows_on(day_index)
        for i in range(count):
            # Today's rows go in the recent past: 12:00 UTC could still be
            # ahead of "now", and a future timestamp would count as today's.
            ts = now - timedelta(minutes=2 + i) if day_index == 0 else (
                day.replace(hour=12, minute=i)
            )
            stamp = ts.strftime("%Y-%m-%dT%H:%M:%SZ")
            provider = PROVIDERS[i % len(PROVIDERS)]
            agent = AGENTS[i % len(AGENTS)]
            latency = LATENCY_BASE + day_index
            conn.execute(
                """INSERT INTO usage (ts, agent, provider_id, model, input_tokens, output_tokens,
                                      cache_read_tokens, cache_creation_tokens, latency_ms, status,
                                      cost, cost_currency)
                   VALUES (?,?,?,?,?,?,?,?,?, 'ok', ?, 'USD')""",
                (stamp, agent, provider, "demo-model", IN_TOKENS, OUT_TOKENS, CACHE_READ, 0,
                 latency, COST_USD),
            )
            conn.execute(
                """INSERT INTO request_logs (ts, method, path, agent, attribution, provider_id, model,
                                            status_code, is_streaming, input_tokens, output_tokens,
                                            cache_read_tokens, cache_creation_tokens, latency_ms)
                   VALUES (?, 'POST', '/v1/messages', ?, 'kw-ag-demo', ?, 'demo-model',
                           200, 0, ?, ?, ?, 0, ?)""",
                (stamp, agent, provider, IN_TOKENS, OUT_TOKENS, CACHE_READ, latency),
            )
            inserted += 1
    conn.commit()
    conn.close()
    return inserted


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
    inserted = seed(args.db, args.days, args.clear)
    currency = preferred_currency(args.db)
    rate = rate_for(currency, repo)

    def sums(window_days):
        # Today is the current UTC day, not the last 24h — mirror that here.
        if window_days == 1:
            rows = rows_on(0)
        else:
            rows = sum(rows_on(d) for d in range(window_days))
        return rows, rows * IN_TOKENS, rows * OUT_TOKENS, rows * COST_USD

    print(f"seeded {inserted} request(s) over {args.days} days into {args.db}")
    print(f"display currency: {currency} (rate {rate} per USD)")
    print()
    print(f"{'window':8} {'requests':>9} {'in tokens':>11} {'out tokens':>11} {'cost':>12}")
    for label, days, note in (
        ("Today", 1, "current UTC day only"),
        ("7 days", 7, "rolling 7x24h, 7 chart points"),
        ("30 days", 30, "rolling 30x24h, 6 chart points (5-day buckets)"),
    ):
        r, i, o, c = sums(days)
        print(f"{label:8} {r:9} {i:11} {o:11} {c * rate:11.2f} {currency}")
        print(f"{'':8} {note}")
    print()
    print("The chart's daily numbers are the same pattern: 5,6,7,8,9,10,11 then repeat,")
    print("newest point on the right.")


if __name__ == "__main__":
    sys.exit(main())
