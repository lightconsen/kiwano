#!/usr/bin/env python3
"""Generate crates/adapters/resources/models.json from the cc-switch pricing DB.

One-off regeneration helper: reads the seeded model_pricing table from the
local cc-switch SQLite database (~/.cc-switch/cc-switch.db) and emits the
bundled price table Kiwano ships. All cc-switch prices are USD per million
tokens, stored as TEXT decimals and preserved verbatim.

Usage: python3 scripts/gen_models_json.py
"""

import json
import os
import sqlite3
import sys
from datetime import date

DB_PATH = os.path.expanduser("~/.cc-switch/cc-switch.db")
OUT_PATH = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "crates", "adapters", "resources", "models.json",
)

# Snapshot rates used for UI display conversion (provider-currency -> amount).
# Kept in the bundle so the frontend can convert without a network call.
EXCHANGE_RATES = {"USD": 1.0, "CNY": 7.1}


def main() -> int:
    if not os.path.exists(DB_PATH):
        print(f"cc-switch database not found: {DB_PATH}", file=sys.stderr)
        return 1

    conn = sqlite3.connect(DB_PATH)
    rows = conn.execute(
        "SELECT model_id, display_name, input_cost_per_million,"
        " output_cost_per_million, cache_read_cost_per_million,"
        " cache_creation_cost_per_million FROM model_pricing ORDER BY model_id"
    ).fetchall()
    conn.close()

    models = [
        {
            "model_id": r[0],
            "display_name": r[1],
            "input": r[2],
            "output": r[3],
            "cache_read": r[4],
            "cache_creation": r[5],
            "currency": "USD",
        }
        for r in rows
    ]

    doc = {
        "version": 1,
        "generated_at": date.today().isoformat(),
        "exchange_rates": EXCHANGE_RATES,
        "models": models,
    }

    os.makedirs(os.path.dirname(OUT_PATH), exist_ok=True)
    with open(OUT_PATH, "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, indent=2)
        f.write("\n")

    print(f"wrote {len(models)} models -> {OUT_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
