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

# Restart means restart, daemon included.
#
# The gateway is a separate process that **outlives the app**: relaunching the
# app connects to whatever daemon is already listening, so a change to the
# daemon's own code lands only if that process is replaced. That is not a
# theoretical trap — it produced three separate "I clicked and nothing happened"
# reports, each time with a new frontend talking to an old gateway.
#
# The daemon is identified by the **socket beside this database**, not by name or
# command line: its database path is in its environment, so `pgrep -f` cannot see
# it (`migrate.local.md` §14). Killing by socket means this only ever stops the
# daemon serving *this* stack — a second dev instance on the same port is still
# refused below, by the port check that follows.
DEV_SOCKET="$(dirname "$KIWANO_DEV_DB")/admin.sock"
stale_daemon="$(lsof -U 2>/dev/null | awk -v s="$DEV_SOCKET" '$NF == s { print $2; exit }')"
if [ -n "$stale_daemon" ]; then
    echo "dev-isolated: stopping the daemon already serving $DEV_SOCKET (pid $stale_daemon)"
    kill "$stale_daemon" 2>/dev/null || true
    for _ in $(seq 1 40); do
        kill -0 "$stale_daemon" 2>/dev/null || break
        sleep 0.25
    done
    rm -f "$DEV_SOCKET"
fi

# The app window is replaced too, for the same reason one level up: a stale one
# would be a window onto the old build, and the two would share a tray icon and a
# database. Matched on **this checkout's** binary path, so the installed app
# (`/Applications/Kiwano.app`) is never touched.
stale_app="$(pgrep -f "^$(pwd)/target/debug/kiwano-app$" || true)"
if [ -n "$stale_app" ]; then
    echo "dev-isolated: closing the dev app already running (pid $stale_app)"
    kill $stale_app 2>/dev/null || true
    sleep 1
fi

# A second dev stack on the same port would race the first one's daemon for
# the bind; refuse instead of letting the GUI show a red gateway it cannot fix.
if lsof -nP -iTCP:"$KIWANO_DEV_PORT" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "dev-isolated: port $KIWANO_DEV_PORT is already listening — another dev instance running?" >&2
    exit 1
fi

export KIWANO_DB_PATH="$KIWANO_DEV_DB" KIWANO_DATA_PORT="$KIWANO_DEV_PORT"
echo "dev-isolated: db=$KIWANO_DEV_DB · gateway :$KIWANO_DEV_PORT (the installed app is untouched)"
# The DEV ribbon on the tray icon comes from tauri.dev.conf.json — merged over
# the shipping config here and only here, so release builds never carry it.
exec pnpm -C app tauri dev --config src-tauri/tauri.dev.conf.json
