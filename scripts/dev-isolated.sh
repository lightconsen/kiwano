#!/usr/bin/env bash
# Isolated dev instance: its own database, gateway port and admin socket, so
# it never shares state with the installed app. The rule behind it: Kiwano
# development runs ONLY through this script — the default environment attaches
# to the installed app's live gateway (its socket is where every process looks
# by default) and its real database, and the takeover switch writes real agent
# configs on top of that.
#
# What the two overrides do:
#   KIWANO_DB_PATH   — a dev database beside the real one; the admin socket is
#                      derived from the db path, so the admin plane moves with
#                      it for free
#   KIWANO_DATA_PORT — the gateway's HTTP plane, off the installed app's :8317
#
# What this does NOT isolate: takeover writes the real agents' config files
# (~/.claude/...). Never toggle a takeover from a dev instance — for that, run
# with HOME=/tmp/kiwano-dev-home so configs, the db and the gateway land in a
# sandbox (probes will red: no agents are installed in a fake home, which is
# the correct answer there).
#
# Usage: scripts/dev-isolated.sh   (env overrides: KIWANO_DEV_DB, KIWANO_DEV_PORT)

set -euo pipefail
cd "$(dirname "$0")/.."

KIWANO_DEV_DB="${KIWANO_DEV_DB:-$HOME/.kiwano-dev/kiwano.db}"
KIWANO_DEV_PORT="${KIWANO_DEV_PORT:-8318}"

mkdir -p "$(dirname "$KIWANO_DEV_DB")"

# A second dev stack on the same port would race the first one's daemon for
# the bind; refuse instead of letting the GUI show a red gateway it cannot fix.
if lsof -nP -iTCP:"$KIWANO_DEV_PORT" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "dev-isolated: port $KIWANO_DEV_PORT is already listening — another dev instance running?" >&2
    exit 1
fi

export KIWANO_DB_PATH="$KIWANO_DEV_DB" KIWANO_DATA_PORT="$KIWANO_DEV_PORT"
echo "dev-isolated: db=$KIWANO_DEV_DB · gateway :$KIWANO_DEV_PORT (the installed app is untouched)"
exec pnpm -C app tauri dev
