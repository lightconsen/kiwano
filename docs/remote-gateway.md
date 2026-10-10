# A gateway on another machine

Kiwano runs as two halves: a **gateway** (the daemon that owns the database,
routes your agents' traffic and meters it) and a **client** (the desktop app or
the CLI) that manages it. By default both are on the same computer, and the
client reaches the gateway through a socket beside the database.

This page is about the other arrangement: the gateway on one machine, the client
on another. It is for the shape where the machine that is always on is not the
one you are sitting at — a home server, a mini PC, a box in a cupboard — and you
want to add providers, watch usage and take agents over from your laptop.

Nothing here is required for a single-machine install. Every setting on this page
is unset by default and the default behaviour is what it always was.

## Two planes, and they are separate on purpose

The gateway listens on two:

| Plane | What travels on it | Where a client reaches it |
|---|---|---|
| **Admin** | provider rows, routes, logs, quotas, probes — everything the app and the CLI read or write | `/status` and every `/api/…` endpoint |
| **Data** | your agents' actual requests and responses | the URL your agents are configured with |

They are configured separately, and that is the point: they carry different
things. The admin plane carries credentials — the keys you stored. The data
plane carries the requests themselves — your prompts and the answers, in the
clear, because **Kiwano does not do TLS** (see [the deployment
requirement](#the-deployment-requirement) below).

Each plane also has its **own credential**, and they are not the same one:

| Plane | What a caller presents | How many |
|---|---|---|
| Admin | `KIWANO_DAEMON_TOKEN` — the gateway's admin token | one per gateway, for the operator |
| Data | a **client key** (`kw-ag-…`), one per agent | one per agent, and more per agent if you want them |

The admin token is one secret for one operator. A client key is what an agent's
config carries, it is minted *for* that agent, and it can be limited: what it may
spend, which models it may name, which providers it may use. That is the section
below, and it is why a leaked client key is bounded in a way a leaked admin token
is not.

## On the machine that runs the gateway

Give the admin plane an address to listen on, and the data plane one too:

```sh
KIWANO_ADMIN_ADDR=100.64.0.5:8318 \   # management: host:port
KIWANO_DATA_ADDR=100.64.0.5 \         # agents' traffic: an address, not host:port
KIWANO_DATA_PORT=8317 \               # optional; 8317 is the default
kiwanod
```

- **`KIWANO_ADMIN_ADDR`** — unset means no TCP listener at all: the admin plane
  is a socket beside the database, which only this machine can reach.
- **`KIWANO_DATA_ADDR`** — unset means `127.0.0.1`: the agents on this machine,
  and nobody else's. Its value is an **IP address, not `host:port`**, because the
  port has its own variable ([`KIWANO_DATA_PORT`](#ports)) and two sources for
  one number is how they come to disagree.
- Both refuse an address that is reachable from anywhere (a public address, or
  `0.0.0.0`) unless you say you mean it — see [the two switches](#the-two-switches).

## On the machine you manage from

```sh
KIWANO_DAEMON_ADDR=100.64.0.5:8318 \
KIWANO_DAEMON_TOKEN=<the gateway's admin token> \
kiwano status
```

The same two variables point the **desktop app** at a remote gateway, with one
practical caveat: a GUI app started by double-clicking does not inherit your
shell's environment. Launch it from a terminal, or set the variables where
macOS looks for them (`launchctl setenv KIWANO_DAEMON_ADDR …` before launching,
or an `LSEnvironment` entry in its `Info.plist`).

**Where the token comes from.** The gateway mints it on first start and keeps it
in its own database. On the gateway's machine:

```sh
sqlite3 ~/.kiwano/kiwano.db \
  "select value from gateway_settings where key = 'gateway.admin_token'"
```

There is no command that prints it; that one-liner, or `scp`, is the path.

Two rules about that token, both deliberate:

- **It is only ever read from the environment when a remote is named.** A remote
  client never falls back to the token row on its *own* machine — that row
  belongs to a different gateway, and sending it to another host is a credential
  handed to the wrong place.
- **A malformed address is refused, not fallen back from.** `KIWANO_DAEMON_ADDR`
  that is not `host:port` stops the command with an error rather than quietly
  talking to the local gateway, which would be a client silently using a
  different daemon than the one you named.

## What a client key may do

A key is the data plane's credential. It names one agent (that is what routing
needs), and it can carry three restrictions, all of them checked **before the
request reaches a provider**:

```sh
kiwano clients list                                  # handles, agents, what each may do
kiwano clients add --agent claude --label "laptop"   # the key is printed once
kiwano clients limits ck-1a2b3c4d --window day:200:requests --window monthly:20:USD
kiwano clients policy ck-1a2b3c4d --model claude-sonnet-4-5 --provider p-work
kiwano clients rotate ck-1a2b3c4d                    # new secret, same handle and budget
```

- **spend windows** — `day` / `weekly` / `monthly` / `yearly` / `all`, in
  `requests`, `wan_tokens` (万 tokens), or a currency. Measured against the same
  local-calendar boundaries an agent's ceiling uses. Over any window: **429**, with
  a `Retry-After` naming the rest of it.
- **models** — only the listed model ids, matched case-insensitively. A model
  outside the list: **403**. A request whose model cannot be read while a list is
  in force is refused too, because an allowance that cannot be checked is not one.
- **providers** — only the listed provider ids. A key that may not use the agent's
  primary is routed to one it may (multi-provider strategies) or refused (403)
  when the strategy is `single` — "this one provider, no failover" is not a
  statement a key can promote its way out of.

Two keys for one agent are two clients with two budgets, which is the point: a
laptop and a desktop running the same agent can be limited separately. Rotating a
key keeps the handle, the policy and the spend already recorded, so it is not a
way to clear a ceiling.

The key itself is stored the way a provider's key is — in the clear, because it is
in the clear in the agent's config file too — and it is scrubbed out of captured
bodies like every other credential. A management surface never prints a stored
key: only `add` and `rotate` show one, once, when they mint it.

## The two switches

Addresses that stop at your own network — RFC1918 (`10.`, `192.168.`, `172.16.`–`172.31.`),
loopback, link-local, and the `100.64/10` range Tailscale uses — are accepted as
they are. An address reachable from anywhere needs a second variable:

```sh
KIWANO_ADMIN_ALLOW_ANY=1   # the admin plane carries credentials
KIWANO_DATA_ALLOW_ANY=1    # the data plane carries your prompts and completions
```

**They are separate, and one does not imply the other.** An operator who has
decided their admin plane may face the network has not thereby decided that the
traffic plane may — what leaks there is different in kind. It is the one-character
mistake (`0.0.0.0` for `127.0.0.1`) that these catch.

## The deployment requirement

The cross-machine transport is **plaintext with a token**. That is a deliberate
decision, not an omission, and it comes with a requirement:

> The gateway's listeners **must not be exposed on a network you do not control**.
> For that, use a VPN — Tailscale, WireGuard, anything — and give the gateway the
> VPN address.

The reasoning: TLS with a self-signed certificate that is not verified is not
encryption; a self-signed certificate that *is* pinned cannot be rotated and has
to be re-paired on every reinstall. Both cost something real and buy protection
against an attacker the model does not contain — you, on your own network. Under
that model, the honest answer is a private network and a token, rather than
cryptography against nobody.

So: `100.64.0.5` (a tailnet address) is the value these variables are designed
for. A public address needs both switches *and* a good reason.

A client key does not change that requirement, but it does bound what a leak
costs: the holder can spend what that key's windows allow, on the models and
providers it names, as that one agent — rather than holding the admin token, which
reads your stored keys and edits everything.

## Ports

| Variable | Default | What it names |
|---|---|---|
| `KIWANO_ADMIN_ADDR` | unset (no TCP listener) | the admin plane's `host:port` |
| `KIWANO_DATA_ADDR` | `127.0.0.1` | the interface the data plane binds |
| `KIWANO_DATA_PORT` | `8317` | the data plane's port |
| `KIWANO_DAEMON_ADDR` | unset (local socket) | client → the gateway's admin `host:port` |
| `KIWANO_DAEMON_TOKEN` | unset | client → that gateway's admin token |

A client does **not** need to be told the data port. `/status` reports both the
address and the port the gateway's data plane is on, and a takeover writes *that*
into the agent's config — which is how an agent on your laptop ends up pointed at
the gateway on your server rather than at its own loopback.

## Checking it

On the machine you manage from:

```sh
kiwano status
```

`gateway: running` with the remote's version means the admin plane is reachable
and authenticated. Then take one agent over — `kiwano agents takeover claude` —
which rewrites that agent's config to the address the gateway reported, and send
it a request. If the agent answers, both planes are working.

If `kiwano status` says the gateway is not running while the daemon is up, the
address or the token is wrong, not the network: the CLI says which of the two it
could not use.
