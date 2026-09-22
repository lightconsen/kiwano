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

### Added

- **A credential watch on the data plane.** The gateway has always scrubbed the
  credentials it knows out of the request log — its own provider keys by value,
  anything under a secret-shaped name, anything carrying a listed prefix. That
  is an allowlist and the module says so; a credential it had never seen, in a
  shape nobody listed, went to the provider untouched. It now reads the outgoing
  request for the shapes themselves and records what it found as a note on the
  request-log row: vendor key prefixes (OpenAI, Anthropic, GitHub, GitLab,
  Slack, AWS, Stripe, npm, PyPI, Docker, Google), JWTs, and PEM private-key
  headers. **It reports and never blocks.** By the time a finding exists the
  request has already been sent, so the honest description is "you find out it
  left" — nothing in this path can hold a request up or fail one. A finding
  carries the rule and a count and never the matched text: the body is stored
  anyway, and a note that repeated what it caught would be a second copy of the
  thing it warns about. `Settings → Local gateway → Credential watch` sets it to
  **Alert** (the default) or **Off**. Four limits, all deliberate and all
  written down where the code is: a credential in a format nobody listed passes;
  a payload encoded before sending passes; only traffic through the gateway is
  seen; and the finding is written into the request log, so with request logging
  off the watch has nowhere to report and stays quiet.

### Changed

- **The CSV export always carries the request and response bodies.** Bodies
  were recorded either way; the export was the one place they could be
  withheld — `logs export --include-bodies`, off unless asked, and a switch in
  the Logs export dialog. Both are gone. The argument is the one that removed
  the capture-side switch earlier: the log exists so a request can be looked at
  later, so a file that cannot show the body is that look failing, and both
  ends of the trip want the same payload. A row whose body was never stored
  still gets its two cells, empty, so every record in the file has the same
  column count and no reader has to branch. Two costs come with it, both
  deliberate: an export now reads `request_bodies`, which the metadata-only
  read used to skip, and a file that leaves the machine carries prompt text
  without being asked. **`logs export` no longer accepts `--include-bodies`** —
  a script passing it will fail on the unknown flag rather than silently
  exporting less.

- **A log row's note heading reads "Gateway notes" rather than "Sanitizer".**
  That field now carries a credential-watch finding as well as the compat shim's
  rewrites, and the old heading would have been a lie in front of the one place
  a user reads security information. The CSV column keeps its name, because
  renaming a column breaks whoever parses the file.

## [0.2.3] - 2026-09-21

### Added

- **A compat shim on the passthrough path.** When an agent and a provider
  speak the same protocol, the gateway forwarded the request untouched —
  and a client that speaks a newer dialect than the upstream's parser got
  a bare 400 for its trouble. Claude Code's `thinking: adaptive` against
  a relay that only knows `enabled`/`disabled`; thinking blocks in
  assistant history replayed against a provider that did not produce them;
  Codex's `null` tool schemas against a strict JSON-Schema parser. Five
  separate failure reports, one missing layer. The shim now sanitizes each
  passthrough request by the provider's own protocol: unsupported thinking
  parameters are removed (never rewritten — a mapping would silently change
  generation behavior), thinking history is stripped whenever the client is
  not asking for thinking, and a `null` tool schema becomes the minimal
  object schema. Nothing else moves: a body with nothing to sanitize is
  forwarded byte-for-byte, so upstream prompt caches keep their prefix.
  And every change is recorded — the request log carries a notes line per
  action (the CSV export a column), so "what did the gateway do to my
  request" has an answer in the row itself. One switch in Settings turns
  the whole thing off; off means the client's exact bytes reach the
  upstream, 400s and all.

## [0.2.2] - 2026-09-21

### Added

- **A release smoke job.** When a release publishes, a workflow downloads
  each packaged artifact on its own platform, installs it (Windows
  silently; Linux runs the AppImage under Xvfb), launches the app, and
  proves the gateway came up — a probe to the data plane on every platform,
  the admin plane's `/status` matched against the released version on
  Linux, and the daemon's own `ready` log line. "Does the installed app
  actually run?" finally has an owner. A window screenshot rides along as
  evidence, never a gate.
- **A copy button beside the site's install command.** The one command a
  visitor is here to take away no longer needs a careful text selection.
  The icon is the state — copy, then a check for a beat — and the label
  follows the page language.

### Fixed

- **The Linux installer refuses what it cannot run.** The Linux bundles were
  built on Ubuntu 24.04, so an Ubuntu 20.04 server installed cleanly and then
  failed at first run with five bare `GLIBC_2.xx not found` lines — a real
  user hit exactly that. The Linux build now pins to 22.04, which widens
  support from 24.04-only to 22.04+ (glibc 2.35 is the floor; the desktop
  bundles build there too, since jammy ships webkit2gtk 4.1); the installer
  checks glibc — and names musl honestly — before it downloads anything; and
  README / INSTALL.md state the floor, with a from-source path for older
  systems, whose desktop app cannot work at all (20.04 has no webkit2gtk 4.1
  to link against).
- **An in-app update could be offered, downloaded, and then refused by its
  own signature.** v0.2.0 and v0.2.1's update manifest carried a wrong
  signature for the generic Windows entry — the updater reported "signature
  verification failed" however many times it retried, because it was handed
  the same bad pair every time. The manifests were rebuilt with each
  platform's own `.sig` asset (the bytes every installed copy actually
  verifies against), and both the GitHub release and the Cloudflare mirror
  now carry the corrected file.

  Two gates keep the whole class out, not just this instance:

  - **The mirror verifies before it publishes.** Every platform's signature
    is checked against the app's bundled updater key — minisign itself, the
    checker the updater behaves like — before anything reaches the mirror;
    a manifest a client would refuse is never allowed to be offered.
  - **The version-free mirror objects cache for at most a minute.** A
    version-free name is *reused* across releases, so the same URL can serve
    one version's bytes today and another's tomorrow. When the edge held the
    previous version's bytes longer than the gap between "new manifest
    published" and "new object fresh", a client was fed old bytes against a
    new signature. One minute bounds that window instead of the hours the
    edge was falling back to.

## [0.2.1] - 2026-09-20

### Fixed

- **No more cmd window at every launch (Windows).** `kiwanod.exe` is a console
  program, and a console child whose GUI parent has no console gets a fresh
  cmd window to carry its streams on Windows — which is what opened every time
  the app started. The daemon's stdout and stderr now go to NUL: no window is
  created, and the daemon still logs to its own files beside the database, so
  only the visible artifact of the streams disappears.

## [0.2.0] - 2026-09-20

### Added

- **The product speaks for the agents it serves.** The site's hero now reads
  *"By your agents, for your agents"*, and the feature story on the site and
  the README carries agent intelligence — `kiwano insights` scorecards and
  tuning advice, rule injection that writes picked findings back into each
  agent's own `CLAUDE.md` / `AGENTS.md`, and opt-in MCP self-query — beside
  the gateway's live numbers, conversation-draining strategies, and money
  ceilings written in the currency actually spent.

- **Custom headers that name an auth header say what they do.** A
  `Authorization` / `x-api-key` / `x-goog-api-key` custom header overrides
  the credential the gateway injects — the feature an Azure-style endpoint
  needs, and otherwise a silent way to send the wrong key upstream. The form
  warns the moment such a header is named.

- **Community plumbing.** A security policy, a contributor guide, issue
  templates, freshly captured UI screenshots, a demo poster, and the promo
  runbook that ships a cold-start launch.

### Fixed

- **Importing cc-switch reuses, and never overrides.** A provider already
  here for the same endpoint and protocol is reused rather than grown into a
  duplicate, and an agent that already has a route is left on it — re-running
  the migration is now safe against a Kiwano you have been using.

- **No home, no silent fallback.** `get_home_dir` used to fall back to the
  current directory, placing the shared database wherever the process
  happened to start; it now fails with an error that names the environment.
  And a gateway service running as a user other than the home's owner warns
  at startup, because the takeovers it writes land in files that user cannot
  read.

## [0.1.16] - 2026-09-20

### Added

- **A Features panel**, and the capabilities behind it. Seven switches on a new
  Settings card, all off by default: cost forecast, anomaly alerts, agent-budget
  alerts, MCP self-query, rule injection, tuning advice and the cache-shaping
  experiment. They are the applications mapped in
  `docs/request-logs-applications.md` that have landed, and each is gated by its
  own flag — the ones that write are reversible, and the way out of any of them
  works whether or not its flag is on.

- **Three more alerts.** The cost alert gains a **forecast** that warns before
  the month's spend slope passes a limit rather than after; an **anomaly** check
  that measures the last completed hour's error rate, mean latency and request
  volume against the trailing 7-day baseline (a storm shows up in the requests
  that never reached a provider); and **agent budgets**, so an agent that has
  spent its ceiling notifies instead of only being refused in silence. All three
  dedup through the same store the cost alert uses, so a restart does not
  re-notify.

- **`kiwano insights`** — how the agents spend their tokens, read back out of
  the request log. A per-agent scorecard (cache hit rate with writes in the
  denominator, median session context growth, reasoning share, retries), the
  definitions printed under the table, and four rules that turn the numbers into
  findings tagged `cache` / `bloat` / `retry` / `overhead` — each carrying the
  `request_logs` ids behind it, reopenable with `kiwano logs show`. Two honesty
  rules the first real run taught: an agent whose window is mostly errors earns
  no cache finding, and reasoning reads `–` rather than 0% when a provider never
  reports it. No amounts anywhere; cost stays the dashboard's job.

- **`kiwano mcp`** — a stdio MCP server so an agent can query its own stats:
  `get_usage_summary`, `get_insights` and `get_session_growth`, aggregates only.
  Request bodies never cross stdout.

- **`kiwano rules apply|remove|status`** — the insights findings that can be
  taught become instructions, appended to an agent's own `CLAUDE.md` /
  `AGENTS.md` inside a marker block. `remove` strips the block and restores the
  file from a backup byte for byte, the same machinery a takeover uses and under
  its own key, so injection is never part of the takeover state machine whose
  disable means "give the agent its whole config back".

- **Tuning advice**, two more findings in the same report: a provider whose
  error rate dwarfs the agent's other candidates, and a provider whose retries
  almost never rescue the request. Advice only — a finding never changes a
  route, and these carry no `rule`: they advise the user, not the agent.

- **`kiwano cache-experiment`** — the cache-shaping measurement from
  `docs/request-logs-applications.md` §3.1: the window's captured bodies grouped
  by session, and for each adjacent turn pair the shared-prefix ratio of the raw
  bodies against their canonicalized and whitelist-stripped forms. It closes
  with a verdict line so the read is a decision rather than a histogram.

- **The app's numbers move on their own.** A request that finished while someone
  was looking at Apps landed in the database and sat there until the next read.
  The gateway now ticks once per recorded request and streams those ticks to the
  app, which re-reads what is on screen — no polling. Cells whose numbers moved
  tint briefly (the tint holds while a burst is still moving, rather than
  strobing per request), and the tint is judged on the numbers themselves: one
  request re-reads every provider on screen, and a row that did not move stays
  quiet.

- **A dashboard "All" window, and deltas that are real.** The window picker
  gains **All** — everything the store holds — with the chart finding its own
  left edge and summing a week per bar once the span outgrows 62 days. The two
  percentages beside Requests and Avg Latency were hardcoded zeros, which
  rendered as a permanent "↑ 0%": an unimplemented figure wearing the clothes of
  a finished one. Both now compare the window with the one before it, of the
  same length, and the three cases are kept apart — a real 0%, and no comparable
  window at all (All, an empty earlier window, a latency nothing measured, which
  now draws nothing).

- **A money ceiling is written in the currency being spent.** The agent limit's
  unit picker offered the *display* currency, a preference about how numbers
  read, so an agent whose providers bill in CNY offered the one unit that is not
  its own. It now offers the currencies the agent's providers actually bill in.
  A new window starts in the agent's own currency when they agree on one, and
  the panel says what the gateway does when comparing: usage is converted into
  the ceiling's unit at the Hub's rates, the ceiling itself never converted.

- **Agent takeover moved to where the agent is.** Settings listed every built-in
  agent with its key and a switch — a list that grew with the registry, and
  whose rows mostly carried something that page could not do. What is left there
  is one summary line. The per-agent controls live in the agent's own settings
  in Apps, beside the config files they rewrite, and turning a takeover off is
  now a two-step control (the first click arms it) since it restores the agent's
  configuration.

- **A running conversation keeps its provider.** Quota and time-window used to
  move the whole agent the moment their condition flipped — mid-conversation,
  for whoever was talking — and that conversation paid for the switch by
  rebuilding a prompt cache its provider had already charged for once. Their
  condition now decides where a conversation *starts*; one already running
  finishes where it is. A candidate list pruned of providers over a limit still
  wins over a conversation in flight, and requests that name no session cannot
  be held (nothing distinguishes them from a new one).

- **A cache hit rate column** on the provider tables: the share of input-side
  tokens served from cache, over the same 7-day window as the usage cell. A
  window with no input-side tokens dashes rather than reading 0% — "nothing
  cached yet" is not "the cache never hit".

- **Two columns the request log was missing.** `reasoning_tokens` is a slice of
  `output_tokens`, never added to it: what it buys is the one number that
  separates "thought for a while" from "wrote a lot". `usage_missing` marks rows
  where the upstream reported no usage at all — written as zeros since the
  beginning, and indistinguishable from a request that genuinely used nothing.
  Both ride through the CSV export and the copied row.

- **An attempt that was retried, and a stream a client walked away from, both
  leave rows.** An attempt that failed after the upstream did work cost real
  money that appeared nowhere, and a client hanging up mid-stream produced no
  row at all. Each retry now writes its own row (the status the client never
  saw, and `usage_missing` when it cannot know), and an abandoned stream closes
  out with what the scanner read, marked `truncated`.

- **A chat client that never asked for usage gets metered anyway.** OpenAI chat
  completions only reports token counts when the request asked for them, so such
  a client's spend could not be seen. The gateway asks on its behalf and takes
  the extra chunk back out of the stream, line by line; the bytes a client that
  *did* ask for are untouched, byte for byte.

- **The log keeps everything until you say otherwise.** Retention and body-cap
  are now opt-in limits rather than defaults.

### Fixed

- **Session affinity read almost none of what the agents state.** Claude Code's
  `x-claude-code-session-id`, grok-build's, OpenCode's, and Codex's header and
  two body spellings were all ignored, so every conversation was a session of
  one — which is the cache loss the sticky table exists to prevent. An agent
  that states nothing at all now gets a fingerprint derived from its opening
  (system + first user turn), prefixed `derived:` so a guess is never mistaken
  for an id the client stated. Two takeovers needed work before their agents
  could send anything: OpenClaw and Hermes hand their base URL to an SDK that
  appends `/chat/completions`, so both now get the `/v1` suffix they were
  written without.

- **The sticky table remembered a position, not a provider**, so pruning one
  provider shifted every session pinned behind it onto a different one — the
  exact cache loss it exists to prevent. It holds provider ids now. And
  `roundrobin`'s table had no ceiling: one entry per session, kept for the life
  of the process, which stopped being free the moment Codex's own session id
  began to be read. It holds 4096, dropping the quietest quarter in one pass —
  least recently used, so eviction cannot take the slot out from under a live
  conversation.

- **The cache bucket has three spellings and only one was read**, so cache hits
  reported by two of the three flavours were counted as new input — which is
  what the cache hit rate column and every insights rule read.

- **The log's redaction covered one side of the exchange.** A credential echoed
  back in a response body was stored as it came.

- **UI**: the Cache column's header said "Cache" while its cell showed a rate
  (it reads "Cached" now); the strategy hint truncated exactly at the clause
  saying a running session keeps its provider, and wraps instead; and the window
  editor flagged a 22:00–06:00 window as invalid and silently dropped it, though
  the engine and the CLI both read start > end as crossing midnight.

## [0.1.15] - 2026-09-17

### Added

- **An agent can say which protocol it speaks.** A built-in agent carries the
  list of protocols its own clients use; a user-defined one is asked when it is
  created, and can change its answer in its dialog afterwards. The values are
  read off the wire format this app already writes into each tool's config —
  `wire_api = "responses"` for Codex, `api: "openai-completions"` for OpenClaw
  and WorkBuddy, `providers[].type = "openai"` for Kimi — so they are evidence
  rather than a catalogue of what each vendor also offers. It is a label:
  nothing routes, converts or refuses by it, the gateway still reads an
  inbound's protocol from the path it was called on, and an agent defined
  before the field existed reads "not specified" rather than a default nobody
  chose. `kiwano agents add --protocol` and a PROTOCOL column in `agents list`
  carry the same answer on the command line.

- **Cline CLI**, the fourteenth agent, over its own config file
  (`~/.cline/data/settings/providers.json`, moved by `CLINE_DIR`,
  `CLINE_DATA_DIR` or `CLINE_PROVIDER_SETTINGS_PATH`). It is the first agent
  that is neither additive nor an exclusive switch: Cline selects a provider
  slot by provider *id*, so there is no entry of ours to add and select. The
  takeover replaces the slot its own custom-endpoint option writes to, keeping
  that entry's model id and leaving the user's other slots alone, and imports
  the provider the agent was already using — named from its endpoint — so the
  first request after switching works without retyping anything.

  This is the Cline **CLI**. The VS Code extension keeps its state in VS Code's
  globalStorage and the OS keychain, which is not a file Kiwano can back up or
  restore, so that one is a user-defined agent — which is what that feature is
  for, and what `docs/custom-agents.md` now says.

### Fixed

- **A candidate row's two controls are 6px apart, not 40.** On the row that
  *is* the primary, the Pin renders invisible — it keeps its slot so the rows
  of one route stay aligned, which is deliberate — but it sat between the
  latency test and the remove button, so the row showed a button-wide hole
  between them.

## [0.1.14] - 2026-09-17

### Added

- **Four more agents: WorkBuddy, CodeBuddy Code, Kimi Code CLI and Qwen
  Code.** All four route through the gateway the same way the additive agents
  do — a `kiwano-gateway` entry written into the config the tool already has,
  with the user's own providers left in place and the original bytes restored
  on switch-off. Kimi takes over both generations (`~/.kimi/config.toml` and
  its successor's `~/.kimi-code/config.toml`, whose protocol name differs);
  Qwen's key goes in its own `env` block inside `settings.json`, so nothing
  touches the user's shell; WorkBuddy and CodeBuddy take over the first entry
  of their model list, keeping its id because that id is the model name the
  request goes upstream with.

  Not every agent can be routed this way, and the two that cannot are not
  listed rather than listed-and-broken: Antigravity CLI reads its endpoint
  from environment variables only (and its OAuth path is closed to third-party
  gateways), and Trae Agent keeps its config in the project directory it is
  launched from, which is not a place a per-agent takeover can reach.

- **Gemini CLI, over its own protocol.** Gemini CLI speaks Google's native
  Gemini API (`/v1beta/models/{model}:{action}`, `x-goog-api-key` auth), which
  the gateway now carries as a third protocol. It is passed through, not
  translated: the request reaches a Gemini provider in the shape it was sent,
  while Kiwano meters it, logs it and enforces the same limits as any other
  route. The agent is back in the registry with detection and one-click
  takeover — Gemini CLI reads its base URL and key from `~/.gemini/.env`, and
  that is the file takeover rewrites — and a provider can be added with
  `--protocol gemini` or from the dialog, Test button and model list included.

  What it does not do is convert. A Gemini inbound can only be forwarded to a
  provider that speaks Gemini, so this routes Gemini CLI at Google's own API; it
  does not make Gemini CLI reach DeepSeek or Kimi. cc-switch's `gemini` rows are
  still skipped on import — their settings are Gemini-CLI-shaped rather than a
  provider document — so a Gemini provider is added by hand.

  Schema v23 widens both protocol CHECK constraints to accept `gemini` again.
  It deletes nothing: the rows v15 removed stay removed.

- **Pointing Kiwano at an agent it cannot find.** A built-in agent's tab appears
  only when the detector finds its command, and there are installs it has no way
  to find: a packaged app inherits a narrower `PATH` than your shell, a tool can
  live outside every well-known prefix, and Windows ships `.cmd` shims. The `+`
  at the end of the agent strip opens a menu now — defining your own agent is
  still its first item — and under it are the built-ins the walk could not find.
  Picking one asks for the directory holding its command, which the detector
  then searches after the locations it is sure of and before `PATH`.

  The dialog shows the directories it looked in (the walk's own list, rather
  than a second copy that could drift), carries a **Check again** button for the
  case where you have just installed the tool, and checks a directory the moment
  you pick one — naming the file it found and the version that file printed.
  What it confirms is that the command runs; it cannot confirm that it is that
  agent's, since `--version` output has no shape shared across tools, and the
  dialog says so rather than implying a verification it cannot make.

- **An agent's config directory is read from your own shell.** A takeover writes
  into the file the agent actually reads, and several of these tools let an
  environment variable move it: Claude's `CLAUDE_CONFIG_DIR`, Codex's
  `CODEX_HOME`, OpenCode's `XDG_CONFIG_HOME`, plus the five the registry already
  named. Those five did nothing in the app — they were read from the app's own
  environment, which is launchd's and not the one your shell configures — so a
  relocated config was written to the default path instead, and the agent went
  on reading its own. Kiwano now asks your login shell what it exports, which is
  the only place that knows.

- **The agent strip reads in the order you would.** Agents already routed
  through the gateway, then the ones merely installed, then your own — and
  OpenCode wears its supplied mark rather than the registry's placeholder.
  A user-defined agent's name can be edited where its settings are, instead of
  deleting it and defining another (which lost its route, its key and its
  history).

### Fixed

- **The Windows home directory.** Kiwano read `HOME`, which Git/Cygwin/MSYS
  inject and which need not be the user profile — so on some Windows setups
  every agent config, and the database itself, were resolved from the wrong
  root. It asks the OS now, as the config readers always did, and the app, the
  CLI and the gateway share one resolver rather than three that had begun to
  differ.

- **A refusal says why.** Both places that enable a takeover discarded the
  backend's answer: the agent stayed exactly as it was and the user watched a
  spinner stop. Codex's own refusal — configure a custom provider before
  takeover — had been invisible for the same reason.

- **A takeover that replaces a config field says so.** Nine places in the
  rewriters replaced a field that was present but not the shape they needed. A
  user with `"provider": "anthropic"` in `opencode.json` — a plausible thing to
  have written — would find it replaced by an object holding the gateway entry,
  and nothing anywhere would have said so.

- **A variable that names no one place is refused rather than guessed at.** A
  relative `CODEX_HOME` is resolved by the tool against whatever directory it is
  run in, so it names not another location but one location per invocation, and
  writing the default instead was a takeover that read as done while the agent
  read something else. A probe that finds nothing also says so now, rather than
  leaving the click looking ignored.

- **The login-shell probe no longer needs the shell's syntax.** It asked the
  shell to run a `for` loop, and a loop is the one construct these shells do not
  write the same way — fish ends a block with `end`, not `done` — so for a fish
  user the probe failed silently and only the well-known directories were ever
  searched. It asks for the shell's environment instead, one word every shell
  runs the same way, and resolves the tools here.

- **The ported adapter messages are English.** They reached dialogs, request
  logs and the log file untranslated — the localized half of the dictionary is
  not where they live — so an English-speaking user read Chinese exactly where a
  failure was being explained. The adapter crate's warnings also reach the CLI's
  terminal now instead of being discarded.


## [0.1.13] - 2026-09-16

### Fixed

- **Timewindows follow the timezone you saved, not the host's (KIW-FUNC-001).**
  The Timewindow strategy picked its candidate with `chrono::Local`, and the
  Apps view mirrored the same mistake — so a gateway running on a UTC host for
  a UTC+8 user could hit its window at the wrong hour, and the UI would agree
  with the wrong answer. Both now read the persisted `tz_offset_minutes`, the
  same value quota and billing already used, and a cross-timezone test pins
  that a window resolves against the stored offset rather than the host zone.

- **Quota over-limit no longer pretends the first backup is serving
  (KIW-FUNC-002).** The route-planner keeps backups that are over the quota in
  a separate `fallback` list, the Apps view marks them with a `Fallback` badge
  (its tooltip says the runtime breaker decides, which is the honest contract),
  and the CLI's `*` marker follows the same `serving_agents` list.

- **Plan-quota refreshes run in parallel (KIW-OPS-001).** `refresh_plan_reports`
  awaited providers one by one, each up to its 15s timeout, so N unreachable
  providers delayed the new limit snapshot by N×15s while old limits kept
  serving. It now queries with `join_all` and publishes one evaluation, with
  tests that the queries do not serialize and that parked or query-less
  providers are skipped.

- **A refresh re-reads the screen in front of you, and reloads that can fail
  say so (KIW-UX-001/002/003).** The reload registry went from a single slot to
  a `Set`, so an embedded panel registering after its parent screen no longer
  overrides it — the Dashboard's Request Logs re-reads with the status-bar ⟳
  now. The Providers refresh errors go through the retryable error state
  instead of an unhandled rejection, stale rows stay visible under a banner,
  and the refresh scope is stated as "this screen": settings and agent
  detection are read at launch, not on every click.

### Security

- **`/metrics` redacts the identifiers and can require a token
  (KIW-PRIV-001).** The Prometheus endpoint stays open to tools that cannot
  hold the app's credentials, but its per-agent labels are now SHA-256-hashed
  unless the daemon runs with `KIWANO_METRICS_TOKEN` set — then it serves the
  full exposition only to `Authorization: Bearer <token>` and 401s everyone
  else. Constant-time compare, and the token itself never appears in logs.
  `/health` carries no identifiers and is unchanged.

- **The installer will not trust a mirror's self-checksum silently
  (KIW-SUP-001).** When github.com is unreachable, the mirror's own digest can
  prove a download is intact but not untampered, so installing against it is
  now opt-in (`--insecure-mirror-checksum`); without it the install refuses
  and names the two ways out.

- **Provider icons are pinned to local assets (KIW-SEC-001).** A registry
  contract test locks the icon table to build-time local imports and bare SVGs
  free of scripts and event handlers, and the README records that remote or
  Hub-served content must be sanitized and gated by a CSP before it renders.

## [0.1.12] - 2026-09-16

### Changed

- **The CLI prints tables.** Listings came out as columns of text separated by
  spaces, which is only readable while the columns happen to line up: a long
  provider id pushed everything after it, and there was no way to tell where one
  column ended and the next began. They are now MySQL-shaped — a `+---+` rule
  under the header and after the last row, `|` between the cells — and the
  columns of numbers (request counts, token counts, costs, latencies) hug their
  right edge, so they can be read by magnitude. The `usage` and `dashboard`
  splits come with them: they were hand-padded to the same shape, and are now
  the same renderer as everything else.

  A table is laid out to fit the terminal it is printed to. The widest columns
  give up space down to a floor of six characters and the cells that no longer
  fit are elided, so a narrow window gets a narrower table rather than a wrapped
  line. A pipe or a file has no width to fit, so nothing is narrowed and the
  table is written whole; `COLUMNS` overrides both, which is how a script asks
  for a specific layout. `--json` and stderr are untouched: this is the text
  branch of the same commands.

- **A refresh reads the data again instead of rebuilding the screen.** The status
  bar's ⟳ — and the Apps page's, which does the same work plus a forced quota
  query — used to remount whatever screen was open, so it reloaded that screen's
  data and threw away everything else with it: the Models shelf's category and
  search, the Dashboard's window and filters, where you had scrolled to. Each
  screen now publishes how to re-read itself and the button awaits that, so what
  came back changes and what you had chosen does not. It also shows that it is
  working, and that it finished: the glyph spins and the button disables until
  the last of the reads lands, then turns into a check for a moment and goes
  back. That acknowledgement is the point — the work is tens of milliseconds of
  local reads, so the spinner is a blink, and when nothing has changed the screen
  afterwards is identical to before. A refresh that found nothing new used to
  read exactly like a click that did nothing.

- **Hub sync and plan-quota queries are asynchronous.** Both went through
  `reqwest`'s *blocking* client, which is why each needed a thread of its own:
  the app's startup sync ran on a `std::thread`, and the gateway's quota patrol
  wrapped the refresh in `spawn_blocking` so a blocking client never had to be
  dropped inside the runtime. They now use the async client the rest of the
  daemon does, so both escapes are gone and the gateway no longer parks a worker
  thread on an HTTP call every time it checks a plan quota. Nothing to see from
  the outside: the same commands, with the same answers.

- **The latency test records what it measured.** An Apps row's Test button has
  always sent a real prompt and shown the number on the button, and that was the
  whole life of the measurement — which left the one case it could have settled
  with nowhere to go: a Status cell reading `No answer` while the endpoint plainly
  answers. The click now writes its verdict as that provider's health, beside the
  prober's and the provider's own traffic, and the row is re-read, so the cell
  shows what the click just found. It is also the only one of the three that works
  on demand — the traffic average needs traffic, and the prober only asks about
  providers that have none. The button is on an agent's rows as well as the All
  row's, since it measures the *provider* rather than the binding and both tabs
  read the one verdict; parking stays on the All tab, where it reads as what it
  is — an act on every agent at once.

  The three readings stay apart, and the cell says which it is showing: your own
  requests, the gateway's unsigned `GET`, or the test you ran. A verdict now
  carries the reason when there is one, and a refusal is the case that needed it:
  a 401 is the vendor answering and saying no to your key, which is not the same
  fact as nobody being home. It reads `Key refused`, with the vendor's own message
  in the tooltip, rather than joining `No answer` — reading it as silence sends
  the reader looking at the network instead of at the key.

## [0.1.11] - 2026-09-15

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

- **`kiwano import cc-switch` is documented.** The command reference now says what
  the import reads (`~/.cc-switch/cc-switch.db`, or the older `config.json`) and
  what it refuses to guess at — cc-switch's `gemini` rows, and a provider with no
  base URL or key — so the two questions a reader has before running it ("does it
  touch my cc-switch", "what happens to the rows it cannot map") are answered
  where they are asked. It stays out of the README's feature list on purpose:
  worth knowing before running the command, not before deciding what the project
  is.

### Changed

- **The landing page opens in English.** It carried Chinese inline and overrode it
  from an `en` dictionary, so a visitor whose browser asks for English got Chinese
  first and had to press a button to fix it. The default is now the other way
  round — the html is the English source and the dictionary carries Chinese — and
  the toggle still works in both directions; this flips which language is the
  fallback, it does not drop one. What describes the page rather than its content
  moved with it: the `lang` attribute, the title, the description, the og tags,
  and the two image `alt` texts, which were the only prose a screen reader got in
  a language the page no longer defaults to.

### Fixed

- **Four claims on the landing page did not survive being checked against the
  code**, and the page is what a visitor reads before downloading. Gemini CLI was
  named in the hero, the takeover card, step 02 and the meta description, and the
  gateway was said to normalize `anthropic / openai / gemini` — that protocol went
  with the `Protocol` enum before 0.1.8, so those now name Grok Build, the third
  agent takeover actually offers. The shelf was said to aggregate 177 providers
  where the Hub serves 19: the count is dropped rather than corrected, because the
  Hub owns that number and a new one only schedules the same drift. The macOS card
  said "Universal" while the release ships `aarch64` and `x64` separately, which
  the two buttons beneath it already said. And Windows now carries the SmartScreen
  warning the README has always given — an unsigned installer that stops on first
  launch reads as malware to anyone who was not told to expect it.

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

### Changed

- **The landing page opens in English.** It carried Chinese inline and overrode it
  from an `en` dictionary, so a visitor whose browser asks for English got Chinese
  first and had to press a button to fix it. The default is now the other way
  round — the html is the English source and the dictionary carries Chinese — and
  the toggle still works in both directions. What describes the page rather than
  its content moved with it: the `lang` attribute, the title, the description, the
  og tags, and the two image `alt` texts, which were the only prose a screen
  reader got in a language the page no longer defaults to.

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
