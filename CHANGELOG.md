# Changelog

All notable changes to Kiwano are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

**Write the entry before the tag.** `scripts/release.sh --tag <version>` refuses
to tag a version that has no section here — and refuses one whose section is not
yet committed, since the tag is cut from the commit — because a tag is a release,
and a release nobody can read is not one. The usual shape is to move the
`[Unreleased]` items under a new `## [x.y.z] - YYYY-MM-DD` heading, commit that,
then tag.

The section *is* the release: the tag-triggered workflow copies it into the
GitHub release body, so this is what someone reads before downloading.

Releases up to and including 0.1.5 predate this file; their tags carry them.

## [Unreleased]

### Security

- **The gateway's loopback planes no longer accept any local process.** A
  request to the data plane (:8317) whose API key was missing or unknown used to
  be attributed to an agent from the URL path and forwarded upstream on the
  operator's own credentials — so any program on the machine could spend through
  their providers. Such a request is now refused with 401 and is never
  forwarded; the only keys that route are the `kw-ag-…` placeholder keys
  Kiwano mints for agents it has taken over, and an agent that was not taken
  over talks to its real upstream directly and never touches the gateway.
  The admin plane (:8310) requires a token on `POST /reload` and
  `POST /shutdown`. The gateway mints it on first run and both sides read it
  from the database they already share, so there is nothing to configure.
  `GET /status` still answers without it — the app's liveness and version
  checks have to work against a gateway from an older build — but an
  unauthenticated caller now gets only the gateway's identity, version and
  uptime, not the route table, provider ids or blocked reasons. Both planes
  stay loopback-bound: the token stops local processes, the bind stops the
  network.
- **Credentials are stripped out of the request log.** Headers were already
  redacted, but the body was stored exactly as it arrived — so a key pasted into
  a prompt was written to the database in the clear and shown in the Logs detail
  panel. Bodies are now scrubbed: values under a secret-shaped key name, and
  anything matching a known credential shape (`sk-…`, the GitHub and Slack token
  prefixes, JWTs, and Kiwano's own `kw-ag-…` placeholders). Responses are
  scrubbed the same way — a provider that names the key it rejected puts it in
  the log by the same route a prompt would. It is a filter over what it
  recognises, not a detector — a credential in a shape it does not know is
  still stored as it was.
- **The database is owner-only on Windows too.** The Unix hardening (0700
  directory, 0600 file) has always been Unix-only, leaving the database that
  holds every provider key with inherited permissions on Windows. It now gets a
  protected ACL granting the current user alone.
- **Exporting a configuration no longer writes your API keys to the file.**
  The export is a dormant feature — no screen calls it yet — but it would have
  written every key in the clear the moment one did. Keys are left out unless
  explicitly asked for.

### Fixed

- **A fresh install now has a working gateway.** Every build up to 0.1.7 bundled
  only the app, so on a machine that had never compiled Kiwano from source the
  gateway daemon was simply absent: the app opened, showed the gateway as down,
  and retried a spawn that could never succeed. The gateway is now built as part
  of the app build and placed beside the app binary — `Contents/MacOS/` in the
  macOS app, `usr/bin/` in the Linux packages, next to `kiwano.exe` on Windows —
  so a first install works on a machine that has never had Kiwano on it. Anyone
  already affected is fixed by updating: the update check runs in the app and
  never needed the gateway.
- **Taking over is reported from what is actually in the config file, not from
  a flag written beside it.** The switch was recorded before the agent's config
  was rewritten, so the Apps list could say an agent was taken over while its
  config was still untouched — and, after an interrupted takeover, the reverse.
  The state now comes from the config itself.
- **Restoring always reports the truth.** Turning a takeover off used to
  succeed silently when the saved original was gone, leaving the agent pointed
  at the local gateway with a placeholder key while the app said it had been
  restored. It now falls back — to the current provider, then to stripping the
  gateway route — and if it cannot restore you, it says so instead of claiming
  success.
- **A Codex configuration that Codex would refuse to start on is rejected
  before it is written,** rather than written and then blamed on Codex. Codex
  rejects a whole config over a missing provider name, a stale reserved provider
  id, or a key with no provider table to carry it; those are now caught (and,
  where it is lossless, repaired) before the switch, and the error says which
  one it was and what to change.

### Added

- **A gateway left over from another version is replaced at startup.** The
  daemon deliberately outlives the app, so an upgrade can meet its predecessor
  still holding the port. A gateway whose version is not the app's own is now
  stopped over the private control channel — the graceful path, so the
  database's write-ahead log is checkpointed — and the bundled one takes its
  place. A gateway that matches is still adopted untouched.

## [0.1.7] - 2026-09-11

### Added

- **A refresh button on Models**, beside the search box: it re-syncs the catalog
  from the Hub, so a provider published there since launch shows up without a
  restart. The icon spins while it checks and a failure reports itself in red;
  the sync is conditional on the Hub's side, so an unchanged catalog is still a
  completed check.

### Changed

- **The macOS build is signed with a Developer ID and notarized.** The app opens
  on a double-click: no more right-click → Open, and no more "unidentified
  developer" — from macOS 15 a notarized app is the only kind that opens without
  a detour through System Settings. This covers the disk image as well as the
  app inside it, which are assessed separately. Windows builds are still
  unsigned, so SmartScreen keeps warning there until a certificate is in place.

## [0.1.6] - 2026-09-11

### Added

- **Light mode**, alongside dark.
- **A log file**, written by the app and the gateway alike, rotated daily with
  seven days kept, and an *Open folder* button in Settings to reach it. Both
  processes previously wrote to stdout only, which a packaged app — started by
  launchd, with no terminal — sends to nothing.
- **CSV export for the Logs card**, with the date range chosen in the export
  dialog rather than committed to up front.
- **Billing limits enforced by the gateway**, so they hold whether or not the
  desktop app is running. A metered provider's spending cap and a plan's percent
  ceilings both take the provider out of the route table once reached and put it
  back when the period rolls over. A request whose every candidate is over its
  limit fails with 429 `provider_over_limit` rather than being served anyway.
- **The Apps card says when the gateway is refusing to route** to a provider,
  in the status column, and the Dashboard's By-provider card marks it too.
- **Costs in the provider's own currency**, including the spending limit
  denominated in it.
- **A trend chart worth reading**: columns, one metric at a time (requests or
  tokens), a bar per hour for *Today* and per day for *30 days*, and a readout
  that follows the hovered bar. The By-provider donut gains the same readout.
- Taking a takeover down from Settings, in the row where it is listed.

### Changed

- **Licence: Apache-2.0 → GPL-3.0-or-later.**
- Costs and limits are converted with the Hub's published rates, not a snapshot
  compiled into the binary.

### Removed

- **Two claims about privacy that were not true.** An *Anonymous usage reporting*
  switch, backed by no collection at all — the setting was declared and read by
  nothing, so it promised a pipeline that did not exist and implied the choice
  mattered. And "API keys never leave your device", when a key goes to the
  provider on every request, which is the point of the app. Settings now says
  what actually happens: nothing about your usage is sent, the only outbound
  requests are the Hub catalog and the update check, and a key reaches the
  provider it belongs to and no one else.

### Fixed

- The **agent filter never reached the backend** — the invoke argument was named
  `agentId` where the command takes `agent`, so every stat on the page ignored
  it while the provider filter worked.
- The **footer's "Today" started at UTC midnight**, which dropped the first
  eight hours of the day for anyone east of UTC.
- A stat and its chart now cover the **same days on the user's clock**, and so
  does the quota strategy's "today" — which used to reset at 08:00 for a UTC+8
  user.
- The **By-agent table ignored the agent filter**, so its rows could total more
  than the headline above them.
- A **pay-as-you-go spending limit never reached the Apps card**: it was stored,
  and dropped on read.
- The **currency selector could strand you on USD**, because the list came from
  what models are priced in rather than what conversion can target.
- Selects now show the selected **item's label**, not its raw id.
- **`single` fails on a limit** instead of silently promoting the backup. It is
  the one strategy that does not fail over, and it stayed that way.
- The **gateway stops by closing its store**, so SQLite checkpoints and the
  database is no longer left with a write-ahead log beside it.
- The **watchdog backs off** when respawns keep failing, rather than retrying
  every five seconds for as long as the app runs.
- The **login item is no longer re-registered on every launch**, which was
  making macOS announce "Background Items Added" each time.
- A takeover **removes the config files it created** rather than emptying them.
