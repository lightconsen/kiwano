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

### Fixed

- **Saving a provider no longer strips the scheme from its endpoint.** The dialog
  is handed the endpoint as the app *shows* it — `https://` removed, the api path
  folded in — and hands that same value back on save, and nothing downstream put
  the scheme back: the gateway composes the upstream URL by concatenation and
  `reqwest` refuses a relative one. So opening a provider and saving it again
  quietly made it unroutable — every request to it failed at the transport layer
  while the row still read like a working provider, because the display strips
  the scheme either way. A stored endpoint is now an absolute URL, at every
  writer the dialog has (a new provider, an edited one, and the per-protocol
  endpoint rows) plus the takeover's import of an agent's own config, which is
  how a bare host arrives in the first place. `https://` unless the host is
  loopback, where the likely answer is a local server and the scheme is `http://`:
  guessing TLS at one fails at the handshake, before anything can say why.

### Added

- **A provider's latency is back in the Status column, read from traffic rather
  than probed.** A background prober used to ask every endpoint every 30 seconds
  and the row showed `Healthy 287ms`; that went this morning, because it bought a
  request per provider per half-minute for a number that routed nothing. The
  number now comes from the requests the provider actually served — the average
  round trip over the last 24 hours, which the usage table already holds — and
  the probe only asks about providers with no traffic of their own in that
  window: one just added, or one idle since yesterday. The loop therefore costs a
  request a minute for the rows whose answer is not already on screen, and
  nothing for the rows in use.

  The two are not the same claim, and the cell says which it is showing: your own
  round trips went through the gateway with your key, while the probe is an
  unsigned `GET` to the endpoint — it proves something answers there, not that
  the key works — so each carries its own tooltip, and the probe's says when it
  ran. An endpoint that does not answer reads `No answer` rather than a latency;
  a parked provider still reads `Disabled`; and a provider with neither traffic
  nor a verdict yet says nothing at all, as before.

  The table the prober writes to comes back as migration v21 rather than by
  undoing v18: that migration's `DROP` has already run on every install, and the
  `CREATE` only ever lived in v1, which never runs again.

## [0.1.10] - 2026-09-15

### Changed

- **The Models shelf prices in the provider's currency.** It converted every
  rate into the display currency from Settings, which turned a published figure
  into a derived one that moved with a preference and hid what the provider
  actually charges. Prices now show as published — `$0.27 / $1.10`, not
  `¥1.92 / ¥7.81` — and a group's price range is shown only when the rates it
  covers are in one currency. The display currency is the Dashboard's, where
  rolling mixed figures into a single number is the point.

- **The Models shelf's Category column is an icon.** A shield for official,
  layers for aggregate, boxes for third-party, a gift for the free tier — the
  glyphs the filter chips already carry, which is what makes an icon enough:
  the legend is the row directly above the table. The name stays as the
  tooltip and as the accessible name, and the width it gives back goes to the
  price.

- **The Models shelf says less per row.** The protocol column spelled every
  protocol out — `openai anthropic gemini` — in a table whose price cell is the
  one that truncates. Each protocol is now one letter (`O`, `A`, `G`) in its own
  colour, with the full name on hover and as the accessible name, and the width
  it gives back goes to the price.

- **Model prices now come only from the Hub.** They used to come from a table
  compiled into the binary as well, and the two had drifted apart: a machine
  that had never synced showed no providers on the shelf — that snapshot is
  already gone — yet still costed 192 models, 153 of them vendor rates the
  catalog had deliberately dropped. The compiled table is removed, so prices
  are seeded from the Hub's `models.json` and nothing else; until the first
  sync there are no rates and every cost reads "—". The seed also deletes rows
  the published document no longer carries, so a client that synced the larger
  table stops costing what was withdrawn — and if the Hub ever publishes an
  empty one, the app says so in its log rather than silently clearing it.

### Added

- **A provider can carry the prices you type for it.** The Hub prices the models
  of *its* catalog entries and a forwarded request is costed by looking that
  entry up, so a provider typed in by hand had no rate of its own: it was costed
  at whatever the general table happened to say about that model — or recorded
  unpriced, which left its spending limit with nothing to measure. The Custom
  add form now asks, on a pay-as-you-go provider, for the models it serves and
  their rates per million tokens (input, output, cache read, cache write).

  A declared rate outranks the Hub's, since it is the user's statement about
  what *this* provider charges, and it is consulted before the catalog entry the
  provider may also be linked to. A model left out still falls through to the
  Hub's table; a cache rate left blank is zero rather than the input rate —
  charging a cache read at the input price would invent a charge the vendor may
  not make, and would overstate the spend a limit is measured against. Prices
  are denominated in the currency the limit is in (one picker, because both are
  about what the provider bills), and a currency this machine cannot convert is
  refused rather than stored: the limit is compared against these numbers.

  The figures travel on the provider row, so a shared config carries them, and
  they are re-validated for the machine importing it the way a limit's currency
  already was. The CLI has no flag for them yet — `providers edit` leaves them
  alone rather than clearing them.

- **The Models shelf can be read by model.** The page lists providers; a
  toggle in its header now switches to the same catalog grouped by model, each
  model holding the providers that serve it — cheapest first — so "what does
  this cost elsewhere?" is a glance instead of 23 searches. A group appears when
  more than one provider serves the model (or when a search asked for it), a
  provider's price shows only when the entry prices *that* model, and the header
  states how many providers serve it and how many publish a price. It carries a
  price range only when the prices actually differ.

  Membership is the union of a provider's model lists and the model it prices:
  15 of the catalog's 71 priced entries price a model their own list spells
  differently (`openrouter` prices `gpt-5.2` while listing `openai/gpt-5.2`), so
  matching the lists alone would leave those prices in no group at all.

  Worth knowing before reading it as a comparison: today every provider charges
  the same for the same model, so most groups show one price repeated, and two
  thirds of the rows are providers that publish no price for that model. The
  view is what the per-provider pricing work was for — it starts showing real
  differences when the Hub starts publishing them.

- **The Models shelf says how you pay.** Every row has a Billing column —
  `Plan`, `Pay as you go` or `Unlimited` — and the header sorts it, ranked so
  that a click gathers the subscriptions at the top. Category does not answer
  that question on its own: the official entries split 14 pay-as-you-go to 16
  plan, and "which of these do I subscribe to?" is what that split is.

  For a plan provider the price cell now leads with the entry's one-liner —
  "from ¥49 /mo" — because that is what you would actually buy, while the
  per-token rates after it are what the same models cost metered. Fourteen of
  the nineteen plan entries were showing the rate and hiding the offer.

- **A price can belong to a provider.** The price table was keyed by model
  alone, so a model could carry exactly one price whoever served it, and a
  document that priced one model differently at two providers could not be
  loaded at all — which is why the Hub refused to publish one. Rows now name
  the catalog entry they belong to, and a forwarded request is costed at the
  provider it actually went through.

  A provider added from the shelf records which catalog entry it came from.
  That link has to be its own column: a local row is named `<slug>-<hex>`, so
  `Kimi (Moonshot)` is `kimi-moonshot-4f2a1c` while the catalog calls it
  `kimi`, and renaming a provider changes the name but not the id. A provider
  that no catalog entry prices falls back in a fixed order — the general price
  first, then the lowest-priced entry — instead of to whichever row was seeded
  last, which is what the old table did: a cost could move without any price
  moving.

  Providers that never went through the shelf — added by hand, from the command
  line, imported, or present before the column existed — are linked on their own
  where the answer is unambiguous: `providers add` infers the entry from the
  endpoint, and the app and `catalog sync` fill in the rest afterwards, once per
  provider. An endpoint naming no entry (self-hosted, an aggregator the Hub does
  not list) or two of them is left alone rather than guessed at, and a link that
  already exists is never re-derived. Note that linking an existing provider
  changes *future* costs only, and can therefore move a spend limit, without any
  price having changed.

## [0.1.9] - 2026-09-13

### Added

- **The command-line client can do everything the app can.** `kiwano-cli` had
  seven subcommands — providers, keys, usage, status, reload. `kiwano` covers
  the whole surface: agent takeover and restore, routing strategies (failover,
  roundrobin, timewindow, quota), candidate weights and time windows, request
  logs with filtering and CSV export, plan quota, the dashboard aggregates,
  settings, config export/import, catalog sync and cc-switch migration. See
  [docs/cli.md](docs/cli.md).

  The one that matters most on a server is `kiwano agents takeover`, which was
  not possible from a shell at all before: the gateway only routes keys it
  minted itself, so pointing an agent at the local port by hand produced a 401
  and nothing else.

- **A server bundle in every release.** `kiwano-<target>.tar.gz` contains the
  CLI, the gateway daemon, a systemd unit and install notes — no GUI, no
  display. Attested and checksummed like every other artifact.

- **`kiwano` gains a library.** The client is now `kiwano_cli` as well as a
  binary, so the command tree can be driven in-process by tests — 41
  integration tests exercise it against a temporary database, asserting on the
  exit code and on stdout and stderr separately.

- **The Models shelf shows what a provider actually costs.** The catalog has
  carried `desc` (one line of prose) and `price_ref` (the representative model's
  rates) for a while; this release starts reading them. The price column shows
  the model beside its input and output rates, converted into your display
  currency, and falls back to the description for the eleven providers that
  price no model at all. Sorting by price uses the input rate, and a provider
  with no published price stays last whichever way the arrow points — "no price"
  is a category of its own, not the cheapest one.

  The default order is providers you already have, then tag rank, then name.
  Alphabetical alone put whichever aggregators happened to be called `9527code`
  and `a6api` on the first screen, so a new user's whole impression of the
  catalog was decided by naming luck. Your sort choice is remembered.

  The detail dialog reads the description and drops the rows that held users /
  blurb / free offer — three of five prose fields the Hub was publishing empty
  in all 82 entries.

### Changed

- **The gateway daemon is now `kiwanod`.** It was `kiwano-gateway` — a name that
  read as a description rather than as a program, and one that made the server
  bundle look as though it shipped two things called Kiwano. The crate, its
  library and the binary all take the new name; the systemd unit is
  `kiwanod.service`.

  **Nothing else changes.** The admin socket, the data port, the database, the
  CLI's `kiwano gateway …` subcommands and the `KIWANO_GATEWAY_BIN` override are
  all untouched — `kiwanod` is the program, "gateway" is still what it does.

  **Installing over an existing server install is handled:** `install.sh` stops
  and removes the old unit and binary before starting the new one, and reports
  what it removed. Installing by hand needs that step done manually; see
  `INSTALL.md`.

  **One identifier deliberately keeps the old spelling.** The provider entry
  Kiwano writes into an agent's own config (for opencode, openclaw, hermes and
  pi) is still `kiwano-gateway`. That string is persisted in files under `$HOME`
  on every machine already taken over, and both the restore path and the
  takeover readers match on it exactly. Renaming it would have left those entries
  in place and unfindable — the agent still pointed at the gateway, with no way
  to restore it.

- **The privacy wording now describes what the code does.** The README, the app's
  own UI strings and the marketing site all said API keys are kept in the **OS
  keychain**. There is no keychain integration anywhere in this project — keys
  are stored in the clear in `~/.kiwano/kiwano.db`, protected by file
  permissions (directory `0700`, database `0600`, and the same on its `-wal` and
  `-shm`). A promise like that is read *before* someone downloads, so it should
  describe the shipped behaviour rather than the intended one; the wording now
  says keys are stored locally and readable only by your user.

  **Nothing about how keys are handled has changed — only the claim.** If you
  relied on the keychain statement, this is the correction to read. Keychain
  storage remains a sensible thing to build; it is not built.

- **The command-line client is now `kiwano`, and the desktop app's executable is
  `kiwano-app`.** The name was previously split the other way round — `kiwano`
  was the GUI and `kiwano-cli` the command-line client — which is backwards for
  a tool you run on a server. What you install from the desktop bundles is
  unchanged: the app is still called Kiwano, still ships as `Kiwano.app`,
  `Kiwano_x64.dmg` and friends, and updates in place. Only the executable inside
  changes name, and on Linux and Windows that means the installed command
  (`/usr/bin/kiwano-app`, `kiwano-app.exe`).

  > **Windows, one-time:** if you had *Launch at login* enabled, the login item
  > still names the app's executable as it was before this release —
  > `kiwano.exe`, which is now the name of the *command-line client*. Read that
  > carefully: the login item points at the old **desktop app**, sitting in the
  > install directory under a filename it no longer uses. The CLI is not
  > involved and never was (it was `kiwano-cli.exe` back then, and the app never
  > registered it). Left alone, you may get an old build of the app starting at
  > login.
  >
  > Reinstall once, or toggle *Launch at login* off and on, to re-register it
  > against `kiwano-app.exe`. macOS has no equivalent: its login item points at
  > the app bundle, which did not change.

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
