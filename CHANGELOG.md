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
