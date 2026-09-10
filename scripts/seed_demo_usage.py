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

Shape: day offset d holds `5 + d % 7` requests — each 1000 in / 200 out / 100
cache-read tokens times a per-day scale, $0.02, latency 200 + d ms — spread
between two providers and two agents. Today's rows are placed in the recent
past so they cannot land after "now".

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
COST_USD, LATENCY_BASE = 0.02, 200
PROVIDERS = ["demo-alpha", "demo-beta"]
AGENTS = ["claude", "codex"]


def day_boundaries(days):
    """Noon local on each of the last `days` local dates, oldest first."""
    today = datetime.now(LOCAL).date()
    return [
        datetime.combine(today - timedelta(days=d), time(12), tzinfo=LOCAL).astimezone(timezone.utc)
        for d in range(days)
    ]


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

    now = datetime.now(timezone.utc)
    inserted = 0
    per_day: list[tuple[int, int, int, float]] = []
    for day_index, day in enumerate(day_boundaries(days)):
        count = rows_on(day_index)
        day_tokens = [0, 0, 0.0]  # in, out, cost
        for i in range(count):
            # Today's rows go in the recent past: local noon could still be
            # ahead of "now", and a future timestamp would count as today's.
            ts = now - timedelta(seconds=2 + i) if day_index == 0 else day + timedelta(minutes=i)
            stamp = ts.strftime("%Y-%m-%dT%H:%M:%SZ")
            provider = PROVIDERS[i % len(PROVIDERS)]
            agent = AGENTS[i % len(AGENTS)]
            latency = LATENCY_BASE + day_index
            scale = day_token_scale(day_index)
            in_tokens, out_tokens, cache_read = (
                IN_TOKENS * scale, OUT_TOKENS * scale, CACHE_READ * scale,
            )
            day_tokens[0] += in_tokens
            day_tokens[1] += out_tokens
            day_tokens[2] += COST_USD
            conn.execute(
                """INSERT INTO usage (ts, agent, provider_id, model, input_tokens, output_tokens,
                                      cache_read_tokens, cache_creation_tokens, latency_ms, status,
                                      cost, cost_currency)
                   VALUES (?,?,?,?,?,?,?,?,?, 'ok', ?, 'USD')""",
                (stamp, agent, provider, "demo-model", in_tokens, out_tokens, cache_read, 0,
                 latency, COST_USD),
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
        ("30 days", 30, "today + the 29 before it, 6 chart points (5-day buckets)"),
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
