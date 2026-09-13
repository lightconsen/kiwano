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

### Changed

- **The command-line client is now `kiwano`, and the desktop app's executable is
  `kiwano-app`.** The name was previously split the other way round — `kiwano`
  was the GUI and `kiwano-cli` the command-line client — which is backwards for
  a tool you run on a server. What you install from the desktop bundles is
  unchanged: the app is still called Kiwano, still ships as `Kiwano.app`,
  `Kiwano_x64.dmg` and friends, and updates in place. Only the executable inside
  changes name, and on Linux and Windows that means the installed command
  (`/usr/bin/kiwano-app`, `kiwano-app.exe`).

  > **Windows, one-time:** if you had *Launch at login* enabled, the login item
  > still points at the old `kiwano.exe` filename, which the installer does not
  > remove. Reinstall once — or re-toggle the setting — to clear it. macOS has no
  > equivalent: its login item points at the app bundle.

- **`kiwano-cli` is gone**, with no compatibility alias. Everything it did is
  available under `kiwano`; the subcommands now nest (`keys …` under
  `providers keys …`), and its flags are otherwise unchanged.

- **`providers add --billing` uses the app's vocabulary.** `plan`, `payg` and
  `unl` are the canonical words; the old `subscription`, `metered` and
  `unlimited` are still accepted as aliases, so existing scripts keep working.
  What changes is the *plan* case: `--limit`, `--unit` and `--reset` are now
  **rejected** for a plan provider instead of being written to columns the app
  deliberately leaves empty. A plan's quota is tracked through the plan query,
  not a locally entered number. Passing those flags used to configure a limit
  that the app would never read.

- **`--json` output is now a single JSON document on stdout.** Diagnostics — the
  note about a route reload, warnings, errors — go to stderr. Previously a
  mutating command printed its reload note to stdout *after* the JSON, so
  `kiwano-cli --json providers add … | jq` failed to parse. Errors are still
  reported only on stderr, never as a JSON object on stdout, so a pipeline can
  tell "it failed" from "it printed something unexpected" by the exit code:
  0 success, 1 the answer is "no" (`status` with the gateway down), 2 usage
  error, 3 runtime error.

### Removed

- **The app no longer looks for a gateway on the pre-0.1.8 admin port.** 0.1.8
  moved the admin plane off loopback TCP onto a socket / named pipe, and kept a
  fallback that found and stopped a gateway from the older build on `:8310` (or
  whatever `KIWANO_ADMIN_PORT` named). No released version ever shipped a
  gateway on that transport — the population it reached was hand-built workspace
  binaries and stale `target/debug` copies — so the fallback is gone and the app
  speaks IPC alone. `KIWANO_ADMIN_PORT` is read nowhere now; `KIWANO_ADMIN_SOCKET`
  is the variable that moves the plane.

## [0.1.8] - 2026-09-12

### Security

- **The data plane no longer forwards a request it cannot attribute.** A
  request to :8317 whose API key was missing or unknown used to be attributed
  to an agent from the URL path and forwarded upstream on the operator's own
  credentials — so any program on the machine could spend through their
  providers. Such a request is now refused with 401 and is never forwarded; the
  only keys that route are the `kw-ag-…` placeholder keys Kiwano mints for
  agents it has taken over, and an agent that was not taken over talks to its
  real upstream directly and never touches the gateway.
- **The admin plane is no longer a port at all.** It was loopback TCP on :8310,
  where any local process could reach it. It is now a unix domain socket
  (`~/.kiwano/admin.sock` — 0700 directory, 0600 socket) or, on Windows, a
  per-user named pipe with an explicit ACL, so only a process running as you can
  reach it. `POST /reload` and `POST /shutdown` still require the token the
  gateway mints for itself on first run, but as a second layer rather than the
  only one: anything that can open the socket can also read the token out of the
  database, so it is the endpoint's permissions that keep other users out, and
  the token is what still applies if the two ever come apart. `GET /status`
  still answers without it — the app's liveness and version checks have to work
  against a gateway from an older build — but an unauthenticated caller gets
  only the gateway's identity, version and uptime, not the route table, provider
  ids or blocked reasons.
- **Credentials are stripped out of the request log.** The log keeps a row for
  every request the gateway routes, and several of its columns took whatever
  arrived. Query-string parameters were stored verbatim, so a `?key=…` — the
  shape several providers use — sat in the clear in a column the CSV export
  writes out. Headers were redacted only if their name was one of five, so a
  credential under any other name went through. And the upstream error text, the
  response header block, and the URL inside a transport-failure message each
  carried a copy of the query back in. All of them now go through one filter: a
  parameter or header whose name means "credential" has its value replaced, and
  the URL in an error message is the redacted one.
- **Credentials are stripped out of the body too.** Headers were already
  redacted, but the body was stored exactly as it arrived — so a key pasted into
  a prompt was written to the database in the clear and shown in the Logs detail
  panel. Bodies are now scrubbed: values under a secret-shaped key name, and
  anything matching a known credential shape (`sk-…`, the GitHub and Slack token
  prefixes, JWTs, and Kiwano's own `kw-ag-…` placeholders). Responses are
  scrubbed the same way — a provider that names the key it rejected puts it in
  the log by the same route a prompt would. It is a filter over what it
  recognises, not a detector: a credential in a shape it does not know is still
  stored as it was.
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

- **The interface speaks Simplified Chinese.** Settings → Language has offered
  only "English" since the beginning, disabled — it is a real choice now:
  *System*, *English* or *简体中文*. System follows the operating system, and is
  what a fresh install gets, so nothing changes for anyone whose machine is not
  in Chinese. The switch applies immediately and is remembered.
  What is translated is the app window: every screen, dialog, tooltip and
  screen-reader label. What is *not*, yet: text the app's background pieces
  produce — the tray menu, system notifications, error messages, the provider
  connection test, and the gateway's own status sentences. Those stay English
  for now. Product names (Claude Code, Codex, DeepSeek…) are names and do not
  change language with the interface.
- **The CSV export can include request and response bodies.** Bodies are always
  recorded — that is the point of the log — but a file you might share should not
  carry them unless you say so. The export asks; off, the file is the same 27
  metadata columns as before, and on, the two body columns are appended and the
  header row grows with them. (The setting that used to govern this was
  unreachable and only half-wired: turning it off silenced the request and left
  the response.)
- **A gateway left over from another version is replaced at startup.** The
  daemon deliberately outlives the app, so an upgrade can meet its predecessor
  still holding the port. A gateway whose version is not the app's own is now
  stopped over the private control channel — the graceful path, so the
  database's write-ahead log is checkpointed — and the bundled one takes its
  place. A gateway that matches is still adopted untouched.

### Changed

- **Settings no longer prints the Hub catalog URL.** It was a display-only row
  for a value nothing in the app can change, so it read as a setting you could
  edit and were not allowed to. The Hub card now says what the sync does and
  offers the button.

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
