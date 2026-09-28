#!/usr/bin/env bash
# Burn a plan window on purpose: fire repeated one-shot prompts at an agent
# until its subscription quota is exhausted, watching the results in Kiwano's
# Dashboard (the plan ring climbs as the vendor's API reports usage; a
# limit_hit notification means the window is full).
#
# The loop runs the agent's headless mode, so every round is one fresh,
# metered request — the gateway meters exactly what the vendor bills. Stop
# with Ctrl+C; the script also stops itself after N consecutive failures,
# which is what a full plan window looks like from the client side (429s or
# quota errors until the reset date).
#
# Usage: scripts/burn-quota.sh <agent> [rounds]
#   agent  — claude | codex | gemini | qwen | opencode (or BURN_CMD="your cmd"
#            to burn through any other headless CLI)
#   rounds — number of prompts to fire (default 1000; the failure circuit
#            breaker usually ends the run before that)
#
# Env: BURN_WORDS   words requested per round (default 3000 — the burn rate
#                   is output-bound, so longer output = more tokens per request)
#       BURN_SLEEP   seconds between rounds (default 2)
#       BURN_CMD     full command template when <agent> is not one of the
#                    known ones; `{prompt}` is replaced with the prompt

set -euo pipefail

AGENT="${1:-}"
ROUNDS="${2:-1000}"
WORDS="${BURN_WORDS:-3000}"
SLEEP="${BURN_SLEEP:-2}"
MAX_FAILS=5 # consecutive failures before calling the window full

PROMPT="Write a ${WORDS}-word technical essay about database indexing. Do not summarize or stop early; fill the full length."

case "$AGENT" in
claude) CMD=(claude -p --max-turns 1 "$PROMPT") ;;
codex) CMD=(codex exec --skip-git-repo-check "$PROMPT") ;;
gemini) CMD=(gemini -p "$PROMPT") ;;
qwen) CMD=(qwen -p "$PROMPT") ;;
opencode) CMD=(opencode run "$PROMPT") ;;
"")
    echo "usage: $0 <agent> [rounds]  (claude | codex | gemini | qwen | opencode, or BURN_CMD=...)" >&2
    exit 1
    ;;
*)
    if [[ -n "${BURN_CMD:-}" ]]; then
        # shellcheck disable=SC2086 # the template is meant to word-split
        CMD=(${BURN_CMD//\{prompt\}/"$PROMPT"})
    else
        echo "unknown agent: $AGENT (pass BURN_CMD='... {prompt} ...' for other CLIs)" >&2
        exit 1
    fi
    ;;
esac

echo "burn-quota: agent=$AGENT rounds=$ROUNDS words/round=$WORDS · watch the plan ring in Kiwano's Dashboard"

fails=0
for i in $(seq 1 "$ROUNDS"); do
    echo "--- round $i ---"
    if out=$("${CMD[@]}" 2>&1); then
        fails=0
        echo "$out" | tail -1
    else
        fails=$((fails + 1))
        echo "$out" | tail -3 >&2
        echo "round $i failed ($fails/$MAX_FAILS consecutive)" >&2
        if ((fails >= MAX_FAILS)); then
            echo "burn-quota: $MAX_FAILS consecutive failures — the plan window is most likely full (or the agent is broken). Check Kiwano's Dashboard." >&2
            exit 2
        fi
    fi
    sleep "$SLEEP"
done

echo "burn-quota: done after $ROUNDS rounds"
