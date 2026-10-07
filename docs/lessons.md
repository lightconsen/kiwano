# Lessons

Mistakes this project has already made once, written down so the next change
does not have to make them again. Internal: this file is not in the docs
`PAGES` manifest, so nothing here is published.

It is a log, not a style guide.

## How this file works

- **One entry per lesson**, and an entry is only worth writing when something
  cost time. General advice belongs in [CONTRIBUTING.md](../CONTRIBUTING.md);
  this file is for what actually went wrong here.
- **Each entry carries its evidence**: the commit that paid for it, the issue, or
  the command whose output settled it. A rule with no evidence is a preference.
- **`Seen N×` counts sightings.** When a lesson is seen a third time, the rule
  moves into CONTRIBUTING.md as a rule a change has to satisfy, and the entry
  here is marked `Graduated` and kept for its evidence.
- **Add to it when you lose time**, not when you finish something.

## A hand-written copy of a value drifts — `Seen 3×` · Graduated

**Rule**: a fact recorded in two places must be *derived* in one of them, not
typed in both. The derived form is the one that ages well.

**Why**: the copies do not drift visibly. They go stale quietly, and nothing
fails, because a wrong version string or licence name is still a valid string.

**Evidence**:
- `scripts/sync-site.mjs`'s own comment: the licence link "had gone stale, the
  licence by two releases" — which is why that script exists at all.
- The same script's target list was two hand-written paths until `ff86c09`; the
  version line in a *new* language's dictionary would have sat at the old
  version forever, with no test able to notice (it is prose, not logic). Now the
  list is read off the directory, and the check is `sync-site.mjs --check`.
- `bb7442e`: the demo's status bar read a hand-written `v0.1.2` while the page
  embedding the demo read `v0.2.10`. Now it reads `app/package.json`, the file
  `scripts/release.sh` bumps.

**Where it lives now**: CONTRIBUTING.md, "Rules that came from mistakes".

## A guard must prove the work happened, not that nothing was left over

**Rule**: when a step can *silently do nothing*, the assertion has to check what
the step was supposed to produce. "Nothing left over" passes just as happily
when the input was destroyed.

**Why**: the failure mode of a destructive step is not a residue of the old
thing — it is the absence of the new one, which looks exactly like success.

**Evidence**: `106a8d2`. Stripping comments from the landing-page template also
stripped `/*__SITE_CSS__*/` and `/*__SITE_JS__*/`, because the markers *are*
comments — so the substitution had nothing to replace and the build shipped a
50 KB page with no stylesheet and no script, at exit 0. The existing guard
checked for markers *left over*; a deleted marker is not a leftover. It now
asserts the tail of each payload is present, verified by putting the bug back
(it exits 2).

## Green locally is not evidence about the deployed thing

**Rule**: when the thing under test is served — a static page, a bundle, a
deploy — check it *as served* before believing a local pass. A local copy
differs in ways that matter: caching, cache-miss responses, the origin's
headers, the network between.

**Why**: every one of these was found by looking at the deployed shape, and none
of them was reproducible locally.

**Evidence**:
- `a652219`: the hero's live demo was pinned by Cloudflare's analytics beacon,
  which only hangs from networks that cannot reach it. Locally the beacon
  answered.
- `7690f21`: a language switch during the demo's load was dropped — a race the
  disk-local demo never lost.
- `d493b2f`: `/src/index.html` was a publicly reachable page serving the
  *template*, unstyled and scriptless. Found by fetching the deployed paths.

## A verification that says the same "wrong" three times is more likely wrong than the code

**Rule**: when a check reports a failure you cannot see with your own eyes,
suspect the check before changing the code. Print the raw numbers it is judging.

**Why**: three separate false alarms in one afternoon, each of which nearly
caused a "fix" to working code.

**Evidence**: `5839324` → `4ead67c`. The one-line check compared `top` values to
decide whether a row had wrapped — but CJK and Latin line boxes differ by a
pixel, and the current-language entry was an unpadded `<span>` beside padded
links, so the row read as wrapped at every width while rendering on one line.
Measuring each child's `top`/`height`/`padding` settled it in one query.

## Know what the file actually is before editing it

**Rule**: check *how* the text you are editing is embedded before typing into
it, and check what already outranks it before adding a rule.

**Why**: both mistakes produce an error that points somewhere other than the
cause, which is the expensive kind.

**Evidence**: `4ead67c`. The docs stylesheet is a JS template literal, so a
backtick inside a CSS *comment* ended the string — `SyntaxError: Unexpected
identifier` a long way from the line that caused it. And `.side a` sits later in
the same stylesheet at the same specificity as the new `.langs a`, so the new
rule lost and the row wrapped in a 216px column while identical markup fitted
one line on the hubs.

## Measure the window before building the mechanism

**Rule**: before writing the machinery for a failure mode, count how often that
failure mode has actually happened — in the logs you already keep. "It can
happen" is not a reason to touch a working hot path.

**Why**: a mechanism that fires zero times still costs code in the path every
request takes, and it is the kind of code that is hardest to change later.

**Evidence**: this conversation (2026-10-07). The hold-until-first-content
pattern holds a streaming response back until its first content arrives, so a
candidate that answers `200` and then dies can still fail over — a real gap in
Kiwano, whose plan loop replays on status alone
(`crates/gateway/src/server/data.rs`). Then the query:

```sql
SELECT count(*) FROM request_logs WHERE is_streaming = 1 AND first_token_ms IS NULL;
```

0, across 92 requests (54 streaming). Every failure in that window was a
configuration error (`502 protocol_mismatch` ×20), which fails before the
headers and is already replayed. So it was not built — the window is recorded
here with its trigger instead: **if that count goes non-zero, or a provider
starts answering 200 and then reporting an error inside the stream, build it.**
