# The `kiwano` command line

`kiwano` manages providers, agents and routes without the desktop app. It talks
to the same SQLite store the app and the gateway use, and to the gateway's admin
plane for status and hot reload, so it works on a host with no display at all.

It shares its implementation with the app rather than reimplementing it: every
command in this document is a call into `kiwano-core`, the crate the desktop app
also links. A provider added here is the same row, written the same way, as one
added there.

## Installing

On a Linux or macOS server, one command does the whole thing — binaries,
checksum verification, and the gateway registered as a service for your user:

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sh
```

It needs no root, and that is deliberate rather than convenient: `agents
takeover` rewrites an agent's config under its own `$HOME`, so the CLI and the
agent must be the same account. See [`packaging/INSTALL.md`](../packaging/INSTALL.md)
for what it does step by step, and for the machine-wide alternative.

## Globals

| Flag | Meaning |
| --- | --- |
| `--db <PATH>` | The SQLite file. Default `~/.kiwano/kiwano.db`, or `KIWANO_DB_PATH` |
| `--admin-socket <TARGET>` | Where the gateway's admin plane is: a socket path on unix, a pipe name on Windows. Default: `admin.sock` beside the database, or `KIWANO_ADMIN_SOCKET` |
| `--json` | Machine-readable output on stdout |
| `--quiet` | Suppress informational notes |
| `--no-reload` | Do not ask a running gateway to reload after a change |
| `--data-port <PORT>` | The port an agent's config is pointed at when it is taken over. Default 8317, or `KIWANO_DATA_PORT` |
| `--home <PATH>` | The root agent config files live under. Default `$HOME` |

Globals are position-independent: `kiwano --json providers list` and
`kiwano providers list --json` are the same command.

## Output and exit codes

**stdout carries the payload and nothing else.** Under `--json` that is exactly
one JSON document, so `kiwano --json providers list | jq` works. Diagnostics —
the note after a route reload, warnings, errors — go to stderr, *including*
under `--json`.

Listings come out as tables, and the columns of numbers (request counts, token
counts, costs, latencies) are aligned to their right edge so they can be read by
magnitude:

```
+-------------------------+----------+--------+------------------+---------+---------------------------+
| ID                      | NAME     | PROTO  | ENDPOINT         | BILLING | AGENTS                    |
+-------------------------+----------+--------+------------------+---------+---------------------------+
| api-deepseek-com-271eb4 | DeepSeek | openai | api.deepseek.com | payg    | codex*,demo-route-00b40a* |
+-------------------------+----------+--------+------------------+---------+---------------------------+
```

A table is laid out to fit the terminal it is printed to: the widest columns
give up space, down to a floor of six characters, and cells that no longer fit
are elided with `…`. When the output is not a terminal — a pipe, a file — there
is no width to fit, so the table is written whole; set `COLUMNS` to say one
yourself (`COLUMNS=100 kiwano providers list`), which is also how a script gets
the narrow layout deliberately.

Errors are **not** emitted as a JSON object on stdout. Putting one there would
make a partially-successful pipeline ambiguous about which document it was
reading, and the exit code already carries the answer:

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | The answer is "no" — `status` with the gateway down. Not a failure: the store was read fine, there is simply no gateway to report on |
| `2` | Usage or validation error (clap's own parse failures use this too) |
| `3` | Runtime error: store, IO, admin plane, network |

## Command reference

Run `kiwano <command> --help` for the full flag list. The map below is the
shape, not every flag.

### Status

```
kiwano status              gateway, store, today's totals, and the blocked list
kiwano reload              ask a running gateway to rebuild its route table
kiwano gateway start       start one, adopting an already-running gateway
kiwano gateway stop        ask the running gateway to stop (graceful: it checkpoints its WAL)
kiwano gateway restart     stop and start
```

`status` includes the providers the gateway is currently **refusing to route
to**, with its reason — `capped-1  1.00 of 1.00 requests this period`. That is
derived state which exists only while the gateway runs, so this is the only
place a shell can see it, and it is usually the answer to "why is nothing
routing".

On a systemd host, use `systemctl restart kiwanod` instead of the
`gateway` subcommands — see INSTALL.md.

### Providers

```
kiwano providers list [--agent AGENT]
kiwano providers add --name N --endpoint URL [--key K] [--protocol P]
                     [--billing plan|payg|unl] [--limit N --unit U] [--reset R]
                     [--bind AGENT]...
kiwano providers edit <ID> [any of the above]
kiwano providers use <ID> --agent AGENT      switch an agent, forcing single strategy
kiwano providers enable <ID>                 make it the current route, keeping the strategy
kiwano providers remove <ID>
kiwano providers quota <ID> [--force]        plan quota windows
kiwano providers probe latency|endpoint|models
```

Both `add` and `edit` also take the forwarding and quota options:

```
--timeout SECS                 upstream wait for response headers (1–3600)
--retries N                    same-provider attempts before failover (0–5)
--header 'Name: value'         repeatable; merged after credential injection,
                               so these can override the injected credentials
--endpoint-extra PROTO=URL     repeatable; the same vendor on another protocol
--plan-limit-5h PCT            plan: share of the five-hour window
--plan-limit-weekly PCT        plan: share of the weekly window
--plan-query JSON              {"template":"kimi","fields":{…}} — this is what
                               `providers quota` reads
```

`edit` additionally takes `--no-headers` and `--clear-plan-query`.

`providers edit` keeps the provider's **id**, which is why it exists: bindings,
rotating keys and usage rows all reference it, so remove-and-add is a different
operation. Only the flags you give are changed; everything else is carried over
from the stored row.

That carry-over is load-bearing for `--timeout`, `--retries`, `--header`,
`--endpoint-extra` and `--plan-limit-*`: each is *part* of an object the
underlying API recomputes as a whole, so an edit that names one still sends the
others. Renaming a provider does not clear its timeout.

Clearing is always explicit — `--no-headers`, `--clear-plan-query` — because an
absent flag means keep. `--key` works the same way: omitting it keeps the stored
key, it does not blank it.

`--plan-limit-*` is refused unless the provider is a plan (the underlying layer
silently drops the value otherwise, and a flag that does nothing is worse than
one that refuses). `--plan-query` is what makes `providers quota` able to ask
anything at all.

`--unit` says what a `--limit` counts: `requests`, `wan_tokens`, or a currency the
Hub publishes a rate for. That last one is checked rather than stored, and exits 2
with the currencies it does know: a limit is measured against costs priced in other
currencies, and one without a rate would be added to them at 1:1. A machine that
has never synced has no table at all, and takes `USD` and `CNY` — the two the Hub
publishes rates against.

### Rotating keys

```
kiwano keys list <PROVIDER_ID>            masked; the key is never printed in full
kiwano keys add <PROVIDER_ID> --key K [--label L]
kiwano keys remove <KEY_ID>
```

Rotating keys are tried after the provider's primary key, per request.

### Agents

```
kiwano agents detect                    which agents are installed
kiwano agents versions                  version strings (slow: one subprocess each)
kiwano agents takeover <AGENT>          route it through the gateway, backing up its config
kiwano agents restore <AGENT>           put the original config back
kiwano agents list                      built-in and user-defined agents, with keys
kiwano agents add --name NAME [--note TEXT] [--protocol P]
                                        define your own agent: a named route with its own key
kiwano agents remove <ID>               delete it and its key (its usage history stays)
```

`takeover` is the command that makes a server usable. It imports the provider
the agent is currently configured with, mints the agent's placeholder key, and
rewrites the agent's own config to point at the gateway. The key matters: the
data plane only routes keys it issued itself and refuses anything else with a
401, so pointing an agent at the gateway by hand does not work.

It prints the placeholder key, because that is the one part of the result that
is invisible from outside.

`takeover` writes under `--home`, so run it as the user whose config you mean —
or pass `--home` explicitly for a service account.

`agents add` is the other way to get a route: an agent Kiwano cannot detect and
does not rewrite anything for. You get a name, an id, a key and a strategy to
bind providers to — nothing on disk changes, so point a client at the gateway
with the printed key. `agents list` prints `-` for the key of an agent that has
none yet, and `--json` carries every field on the row.

`--protocol` records what that agent's clients speak: `anthropic`, `openai` or
`gemini`. Omitting it means the agent does not say, which is a different answer
from any of the three and is what `agents list` shows as `-`. It is a **label** —
nothing routes, converts or validates by it; the wire format of a request is
decided by the path it arrives on.

### Routes

```
kiwano routes list
kiwano routes strategy <AGENT> single|failover|roundrobin|timewindow|quota
                               [--limit N --unit requests|tokens]
kiwano routes reorder <AGENT> <PROVIDER_ID>...
kiwano routes apply --from SOURCE --to TARGET
kiwano routes binding add|remove <AGENT> <PROVIDER_ID>
kiwano routes binding set <AGENT> <PROVIDER_ID> [--weight N] [--window HH:MM-HH:MM] [--no-window]
```

`routes reorder` takes the candidate order as arguments: the order you write
them in becomes priority 0, 1, 2…

The quota payload is validated by the gateway's own parser, so a config the CLI
writes cannot be one the engine would silently reinterpret. `--limit` on a
strategy that ignores one is rejected rather than accepted as a no-op.

### Usage and logs

```
kiwano usage [--days N] [--agent AGENT] [--provider ID]
kiwano dashboard [--window today|7d|30d|all] [--provider ID] [--agent AGENT]
kiwano insights [--days N] [--agent AGENT]
kiwano alerts [--mark-notified]

kiwano logs list [--agent A] [--provider P] [--status ok|error]
                 [--from RFC3339] [--to RFC3339] [--page N] [--page-size N]
kiwano logs show <ID>
kiwano logs export --out PATH [same filters] [--include-bodies]
kiwano logs clear --yes
kiwano logs dir
```

`--from` is inclusive and `--to` exclusive, matching the store's half-open
range.

`insights` is the one-page report about *how* the tokens are being spent: a
per-agent scorecard (cache hit rate, session context growth, reasoning share,
retries), rule findings tagged `cache` / `bloat` / `retry` / `overhead`, and
the sessions whose context grew the most. Every finding carries the
`request_logs` ids behind it — reopen one with `kiwano logs show <ID>` — and
everything is computed locally from the log: bodies are sampled to measure
the fixed tools/system payload, but they never leave this machine, and no
amounts appear anywhere (cost is the dashboard's job). The global `--json`
flag yields the same report for machine consumption.

`alerts` is **read-only by default**. The dedup key it would otherwise write is
the same one the desktop app consults before raising a notification, so a
scripted poll would silently swallow the alert you were waiting for. Pass
`--mark-notified` if you genuinely want the CLI to take over delivery.

`logs clear` requires `--yes`: the app confirms with a dialog, and a flag is the
equivalent for a shell with no TTY.

### Settings, config and catalog

```
kiwano settings get
kiwano settings set --key KEY=VALUE [--key ...] [--patch JSON]

kiwano config export --out PATH [--include-keys]
kiwano config import --file PATH
kiwano import cc-switch

kiwano catalog list [--tag official|aggregate|third|free] [--search QUERY]
kiwano catalog sync
kiwano catalog currency
```

Values to `settings set` are parsed as JSON when they parse, so
`--key cost_alert=false` is a boolean and `--key log_retention_days=30` a
number.

`config export --include-keys` writes live credentials and the file is created
owner-only (`0600`) for that reason. `config export` without the flag is safe to
keep in version control; the two round-trip through `config import`, which
merges by name and endpoint rather than overwriting.

`import cc-switch` reads `~/.cc-switch/cc-switch.db` (the v3.20+ SQLite, opened
for reading) or the older `~/.cc-switch/config.json`, and writes only to
Kiwano's own database. Nothing under `~/.cc-switch` is modified or deleted, so
the two can sit side by side while you decide.

It maps all eight apps it knows — claude, claude-desktop, codex, grokbuild,
opencode, openclaw, hermes and pi — onto the agent of the same name, and
whichever provider was *current* in cc-switch becomes that agent's primary
binding, so what migrates is the configuration you were actually running rather
than a list of candidates to pick from again.

It is safe to run again, and safe to run next to a Kiwano you have already
been using. A provider already here for the same endpoint and protocol — one
you hand-added, or an earlier run's import — is *reused* rather than added a
second time, and its name, key and billing are left alone: the import only
fills in what was never there. An agent that already has a route is left on
it; only an agent with no route yet is imported onto, so a re-run can never
move an agent back onto something you have since switched it away from.

Two cases are reported instead of guessed at. cc-switch's `gemini` rows are
skipped: they target Gemini CLI, their settings are Gemini-shaped, and this
gateway speaks no protocol to import them as. A provider with no base URL or no
key is listed as skipped rather than created empty. Every skip carries its
reason.

`catalog list` reads the Hub cache and nothing else — there is no bundled copy, so
a machine that has never synced lists nothing rather than a stale catalog. Run
`catalog sync` first; that is what a fresh server should do before adding anything
from the shelf, and after it the cache serves offline like any other.

**There is no `catalog add`.** The shelf's one-click "Add" opens the provider
form prefilled from the entry — it does not call an API of its own, so the
capability is `providers add`, and the values come from `catalog list --json`:

```sh
kiwano catalog list --json --search deepseek
kiwano providers add --name DeepSeek --endpoint https://api.deepseek.com/anthropic \
    --protocol anthropic --billing payg --limit 50 --unit CNY --bind claude
```

The prefilling rules (a `payg` limit of 50, and the agent prebind some catalog
entries carry) live in the app's form, not in a shared layer. If that mapping
moves into `kiwano-core` later, this becomes a one-command shortcut — until
then it is a convenience the CLI does without, not a capability it lacks.

What the CLI *does* record is which entry a provider corresponds to, since that
is what prices its requests at the entry's own published rates. It is inferred
from the endpoint rather than asked for: `providers add` links the provider when
its endpoint names exactly one entry, and `catalog sync` does the same for
providers that have no link yet. Nothing is guessed — an endpoint that matches
no entry, or more than one (two entries sharing a host), stays unlinked and is
priced at the general rate.

## How this maps onto the app

Every user-facing capability of the desktop app is reachable here except the
inherently graphical ones (tray, window, theme, notifications, i18n, in-app
self-update).

| App capability | Command |
| --- | --- |
| Gateway status + today's totals | `status` |
| Provider list / add / edit / delete | `providers list\|add\|edit\|remove` |
| Make a provider current | `providers enable`, `providers use` |
| Latency / endpoint / model probes | `providers probe …` |
| Plan quota rings | `providers quota` |
| Rotating API keys | `keys list\|add\|remove` |
| Agent routes + strategy | `routes list`, `routes strategy` |
| Candidate order, weights, time windows | `routes reorder`, `routes binding …` |
| Copy a route between agents | `routes apply` |
| Agent detection and versions | `agents detect`, `agents versions` |
| Takeover / restore | `agents takeover`, `agents restore` |
| Your own agents (named routes) | `agents list\|add\|remove` |
| Usage totals | `usage` |
| Dashboard trends | `dashboard` |
| Usage alerts | `alerts` |
| Request logs, detail, CSV export, clear | `logs …` |
| Log file location | `logs dir` |
| Settings | `settings get\|set` |
| Config export / import | `config export\|import` |
| cc-switch migration | `import cc-switch` |
| Catalog shelf + Hub sync + currencies | `catalog list\|sync\|currency` |
| Daemon lifecycle | `gateway start\|stop\|restart` |

Not covered anywhere: the update check, the update download, and the progress
event stream. Those are properties of the desktop app's installer, and the
server bundle is updated by installing a new one.

## Migrating from `kiwano-cli`

`kiwano-cli` is gone and there is no alias. Everything it did is under `kiwano`,
with two differences worth knowing:

1. **The billing vocabulary is the app's.** `--billing plan|payg|unl` is
   canonical; `subscription|metered|unlimited` still parse as aliases. For a
   **plan** provider, `--limit`, `--unit` and `--reset` are now rejected — a
   plan's quota comes from its plan query, and those flags used to write columns
   the app never reads.

2. **`--json` output is a single document on stdout.** Diagnostics moved to
   stderr, which fixes mutating commands: `kiwano-cli --json providers add … |
   jq` could not parse before, because the reload note was printed to stdout
   after the JSON.

Subcommands that moved:

| Was | Now |
| --- | --- |
| `kiwano-cli status` | `kiwano status` |
| `kiwano-cli providers use <ID> --agent A` | `kiwano providers use <ID> --agent A` (unchanged) |
| `kiwano-cli keys list\|add\|remove` | `kiwano keys list\|add\|remove` (unchanged) |
| `kiwano-cli usage --days N` | `kiwano usage --days N` (unchanged) |

The exit codes are new but the important one is unchanged in spirit: `status`
still exits `1` when the gateway is down.
