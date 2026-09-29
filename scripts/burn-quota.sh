#!/usr/bin/env bash
# Burn a plan window on purpose: fire repeated one-shot prompts at an agent
# until its subscription quota is exhausted, watching the results in Kiwano's
# Dashboard (the plan ring climbs as the vendor's API reports usage; a
# limit_hit notification means the window is full).
#
# The loop runs the agent's headless mode, so every round is one fresh,
# metered request — the gateway meters exactly what the vendor bills. Each
# round's prompt is randomized (topic and length) so no two requests share a
# cacheable prefix: what gets measured is uncached, full-price traffic, the
# worst case for the vendor and the truest test of what a plan allows. For
# the other end of the spectrum — agentic sessions that live on cache reads —
# set BURN_CACHE to a KB figure and every round repeats one long shared
# preamble that vendors bill at their cache-read rate.
#
# The script asks for a typed "yes" before the first round: this consumes
# real quota, often all of it. Stop with Ctrl+C; the script also stops itself
# after N consecutive failures, which is what a full plan window looks like
# from the client side (429s or quota errors until the reset date).
#
# Usage: scripts/burn-quota.sh <agent> [rounds]
#   agent  — one of the built-ins below, or any other headless CLI via
#            BURN_CMD='your cmd with {prompt}'
#   rounds — number of prompts to fire (default 1000; the failure circuit
#            breaker usually ends the run before that)
#
# Built-in agents (their known single-shot modes):
#   claude · codex · gemini · qwen · opencode · kimi · crush · droid ·
#   goose · aider · codebuddy · mimo · mcode · copilot · cursor · pi · omp ·
#   dsh · commandcode (cmd / cmdc / command-code)
#   (GUI/IDE-only agents — claude-desktop, continue, workbuddy — have no
#   headless mode to drive; the same BURN_CMD escape hatch covers anything
#   whose flags drift from the built-ins, and agents with no single-shot
#   mode at all — Devin is server-driven over ACP, for one — cannot be
#   scripted this way.)
#
# Env: BURN_WORDS   base words requested per round (default 3000; each round
#                   varies it by ±50% to give the token counts a realistic spread)
#       BURN_CACHE   KB of repeated shared preamble per round to exercise the
#                   vendor's cache-read billing (unset = uncached by default)
#       BURN_SLEEP   seconds between rounds (default 2)
#       BURN_YES     set to 1 to skip the confirmation (for scripted runs)
#       BURN_CMD     full command template for agents outside the built-in
#                    list; `{prompt}` is replaced with the prompt

set -euo pipefail

AGENT="${1:-}"
ROUNDS="${2:-1000}"
WORDS="${BURN_WORDS:-3000}"
SLEEP="${BURN_SLEEP:-2}"
MAX_FAILS=5 # consecutive failures before calling the window full

# A stable cache-warm preamble: generated once, identical every round, so the
# vendor's prefix cache holds it and each round bills it as a cache read.
# Random-ish filler, since the content itself is irrelevant — only its length
# and stability matter.
CACHE_KB="${BURN_CACHE:-0}"
cache_preamble() {
    local need=$((CACHE_KB * 128)) # 1 KB ≈ 256 tokens ≈ 128 space-separated words
    awk -v need="$need" 'BEGIN {
        srand(42); # fixed seed: the preamble must be byte-identical across rounds
        split("gateway routing metering latency provider strategy token cache quota model agent session circuit breaker fallback health check", words, " ");
        line = "";
        while (need-- > 0) {
            line = line words[int(rand() * 28 + 1)] " ";
            if (need % 12 == 0) { print line; line = "" }
        }
        if (line != "") print line;
    }'
}
if ((CACHE_KB > 0)); then
    PREAMBLE="$(cache_preamble)"
    echo "burn-quota: cache mode — every round repeats a ${CACHE_KB}KB shared preamble (billed at the cache-read rate)"
fi

# Each entry is the agent's headless form; "{prompt}" is where the prompt
# goes. CLI flags drift between versions — override with BURN_CMD rather than
# editing here when one goes stale.
case "$AGENT" in
claude) CMD=(claude -p --max-turns 1 "{prompt}") ;;
codex) CMD=(codex exec --skip-git-repo-check "{prompt}") ;;
gemini) CMD=(gemini -p "{prompt}") ;;
qwen) CMD=(qwen -p "{prompt}") ;;
opencode) CMD=(opencode run "{prompt}") ;;
kimi) CMD=(kimi -p "{prompt}") ;;
codebuddy) CMD=(codebuddy -p --max-turns 1 "{prompt}") ;;
mimo) CMD=(mimo -p "{prompt}") ;;
mcode) CMD=(mcode -p "{prompt}") ;;
crush) CMD=(crush run "{prompt}") ;;
droid) CMD=(droid exec "{prompt}") ;;
goose) CMD=(goose run -t "{prompt}") ;;
aider) CMD=(aider --message "{prompt}" --yes-always --no-git) ;;
# Non-interactive mode needs the tools allowed or every round stalls on a
# permission prompt (verified against the installed CLI's --help).
copilot) CMD=(copilot -p --allow-all-tools "{prompt}") ;;
cursor) CMD=(cursor-agent -p "{prompt}") ;;
pi) CMD=(pi "{prompt}") ;;
omp) CMD=(omp "{prompt}") ;;
# The headless profile runs one task, prints the last assistant text and
# exits; launcher flags must precede the task, so --profile comes first.
dsh) CMD=(dsh --profile headless "{prompt}") ;;
# print mode runs one query and exits. The full binary name works on every
# platform (cmd is unclaimed on Windows, where the CLI installs as cmdc); its
# headless gate denies write tools by default, which a text-only burn wants.
commandcode | command-code | cmd | cmdc) CMD=(command-code -p "{prompt}") ;;
"")
    echo "usage: $0 <agent> [rounds]  (see the header of this script for the built-in list, or BURN_CMD=...)" >&2
    exit 1
    ;;
*)
    if [[ -n "${BURN_CMD:-}" ]]; then
        # Split the template on whitespace like a typed command line; the
        # {prompt} placeholder is substituted per round, same as the built-ins.
        CMD=()
        for word in $BURN_CMD; do
            CMD+=("$word")
        done
    else
        echo "unknown agent: $AGENT (pass BURN_CMD='... {prompt} ...' for other CLIs)" >&2
        exit 1
    fi
    ;;
esac

# Random topic list: real requests differ in subject, not just length, and
# variety here is what keeps every round's prompt a cache miss.
TOPICS=(
    "database indexing strategies"
    "HTTP connection pooling"
    "memory allocator design"
    "distributed consensus algorithms"
    "filesystem journaling"
    "TLS handshake optimization"
    "CPU cache coherence"
    "log-structured storage"
    "rate limiter architectures"
    "zero-copy I/O"
    "virtual memory paging"
    "columnar storage layouts"
    "consensus-based replication"
    "network congestion control"
    "hash table collision strategies"
)

# Build one round's prompt: a fresh topic and a fresh target length each time
# (±50% around the base), so no two rounds share a token prefix and the
# input/output spread looks like real traffic rather than one repeated stamp.
next_prompt() {
    local topic="${TOPICS[$((RANDOM % ${#TOPICS[@]}))]}"
    local words=$((WORDS * (50 + RANDOM % 101) / 100))
    local prompt="Write a ${words}-word technical essay about ${topic}. Do not summarize or stop early; fill the full length."
    if ((CACHE_KB > 0)); then
        prompt="${PREAMBLE}

Above is reference material from a long working session. ${prompt}"
    fi
    printf '%s' "$prompt"
}

# The command template is resolved once; only {prompt} is substituted, per round.
run_round() {
    local cmd=() arg
    for arg in "${CMD[@]}"; do
        cmd+=("${arg//\{prompt\}/$1}")
    done
    "${cmd[@]}"
}

echo "burn-quota: agent=$AGENT rounds=$ROUNDS words/round=${WORDS}±50%$( ((CACHE_KB > 0)) && echo " + ${CACHE_KB}KB cached preamble")"
echo "  command : ${CMD[0]} … (${#CMD[@]} args)"
echo "  this WILL consume real plan quota, possibly all of it, until the vendor's reset date."
if [[ "${BURN_YES:-0}" != "1" ]]; then
    read -r -p "  type yes to start: " reply
    if [[ "$reply" != "yes" && "$reply" != "y" ]]; then
        echo "burn-quota: aborted."
        exit 1
    fi
fi

echo "burn-quota: running — watch the plan ring in Kiwano's Dashboard (Ctrl+C to stop)"

fails=0
for i in $(seq 1 "$ROUNDS"); do
    echo "--- round $i ---"
    prompt="$(next_prompt)"
    if out=$(run_round "$prompt" 2>&1); then
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
