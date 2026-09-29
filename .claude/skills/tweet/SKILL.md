---
name: tweet
description: Draft one promotional tweet for a Kiwano commit — the latest one by default, or a range/ref you name. Use after committing, when asked for a tweet/post about recent work, or to re-draft one that missed.
---

<!-- The automatic half of this flow is personal, not project, config: the
     maintainer's ~/.claude/hooks/tweet-after-commit.sh fires on `git commit`
     and points back here. To set that up on another machine, put that script
     in ~/.claude/hooks/ and register it as a PostToolUse hook on Bash in
     ~/.claude/settings.json (or a project's settings.local.json):

       {"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command",
         "command":"sh \"$HOME/.claude/hooks/tweet-after-commit.sh\""}]}]}}

     It stays silent unless the checkout carries this file, so it is harmless
     in other repositories. Contributors get no hook, which is the point: a
     commit is not an occasion to hand someone else's session a promo task. -->

# Draft a promotional tweet for a commit

One English post, ≤280 characters, ready to paste into X. The reader is a
developer who already runs a coding agent and has never heard of Kiwano.

## What to read first

Run `git show --stat <ref>` (and the diff when the message alone is thin) and
`docs/promo/04-x-thread.md` for the house voice. `git log --oneline -20` gives
the surrounding commits when several belong to one story — a range the user
names wins, otherwise the newest commit is the subject.

**Never draft for a commit that did not land** (a failed commit, a hook
rejection) and never for one that says nothing a reader would care about —
internal chores, dependency bumps, formatting. Say so instead of inventing
significance.

## Voice

Punchy, concrete, no marketing adjectives. Lead with what changed for the
reader, not with what the code does; put a number in only when it is real and
verifiable. Short lines, one idea, no thread unless the user asks for one.
Terse verbs beat hedges: "Keys stay on your machine", not "we believe in
local-first principles".

What to avoid, in order of how often it creeps in:

- Words the promo docs ban or overuse — game-changing, seamless, powerful,
  supercharge, unlock, revolutionise. Also avoid emoji strings and ALL CAPS.
- Restating the commit subject line verbatim; the commit wrote for a
  maintainer, the tweet writes for a stranger.
- Claiming what the code does not do. A feature that is implemented but
  unverified is not shipped — "now supports X" is only true once it is.
- Inventing numbers, users, or benchmarks. If nothing measurable changed,
  describe the shape of the change instead.

## Shape

- At most 1–2 hashtags, and only ones a developer actually follows:
  `#ClaudeCode`, `#Codex`, `#AIcoding`, `#opensource`.
- No links by default (X suppresses reach on them). The user can ask for one.
- Version numbers only for release commits.
- Hashtags after the thought, never woven into the syntax.

## Output

The tweet in a fenced code block on its own, then one line saying which commit
it covers. Nothing else — no preamble, no "here's your tweet".

Add a second, alternative opening line only when the first is a close call.
Offer to re-draft for a different angle; never post anywhere yourself.

## Examples

For a takeover commit adding an agent:

```
Kiwano now takes over deepseek-harness too — one local port for every coding
agent you run, originals restored when you switch it off. 25 agents and
counting. #AIcoding
```

For a gateway fix (no user-visible surface, so it says what it prevents):

```
Fixed: a stream that stalled mid-response used to hang the agent forever.
Now it fails fast with the error, and the request is replayed against the
next provider in the strategy. #ClaudeCode
```

Not this (commit restatement, no reader):

```
feat(takeover): omp joins the agent roster (#123)
```
