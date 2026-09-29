#!/bin/sh
# PostToolUse hook on Bash: when the command that just ran was a commit, hand
# the drafting instructions back as context. It reads the tool call from
# stdin, matches nothing it is not about, and prints nothing then — a hook's
# silence is what keeps ordinary commands quiet.
#
# The drafting rules themselves live in .claude/skills/tweet/SKILL.md, so both
# this hook and a manual /tweet produce the same voice.

payload=$(cat)

# jq is present on this machine; if it ever is not, stay silent rather than
# break the tool call that triggered the hook.
command -v jq >/dev/null 2>&1 || exit 0

cmd=$(printf '%s' "$payload" | jq -r '.tool_input.command // ""' 2>/dev/null) || exit 0

case "$cmd" in
*git\ commit*) ;;
*) exit 0 ;;
esac

# The commit may have failed (a rejected hook, nothing staged). The result is
# in the transcript either way — this only says what to do when one landed.
cat <<'JSON'
{
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "additionalContext": "A git commit just ran. If it succeeded, draft one promotional tweet for it following .claude/skills/tweet/SKILL.md (English, single post, no link) and offer it after your normal summary of the commit. If it failed, skip the tweet."
  }
}
JSON
