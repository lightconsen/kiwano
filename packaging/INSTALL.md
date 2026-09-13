# Running Kiwano on a server

This bundle contains the two pieces a headless host needs, and nothing else:

| File | What it is |
| --- | --- |
| `kiwano-gateway` | The daemon: the local router the agents actually talk to |
| `kiwano` | The command-line client that configures and inspects it |
| `kiwano-gateway.service` | A systemd unit, for a machine-wide install |
| `INSTALL.md` | This file |

There is no GUI here. The desktop app is a separate download.

## The quick way

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sh
```

That downloads the bundle, checks it against a digest published for this
release, installs both binaries to `~/.local/bin`, and starts the gateway as a
**service for the current user**. No root. If you would rather read it first —
which is reasonable for anything you pipe into a shell — it is a plain script:
fetch it, read it, then run it.

On macOS it installs the binaries and tells you to start the gateway by hand;
there is no systemd to register with, and most macOS users want the desktop app
anyway.

The rest of this file is the same install done by hand, plus the two decisions
the script makes for you.

## 1. Verify the download

Do this before unpacking. The tarball is what is signed and attested, not the
binaries inside it:

```sh
gh attestation verify kiwano-<target>.tar.gz --repo lightconsen/kiwano

# Or compare against the digest published on the GitHub release.
sha256sum -c --ignore-missing SHA256SUMS
```

## 2. Install the binaries

```sh
tar xzf kiwano-<target>.tar.gz -C /tmp/kiwano
install -m 0755 /tmp/kiwano/kiwano         ~/.local/bin/
install -m 0755 /tmp/kiwano/kiwano-gateway ~/.local/bin/
```

## 3. Run the gateway as your own user

This is the default, and it is not merely the convenient choice — see
[Who runs what](#who-runs-what) below.

```sh
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/kiwano-gateway.service <<'EOF'
[Unit]
Description=Kiwano gateway (local AI model router)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=KIWANO_DB_PATH=%h/.kiwano/kiwano.db
Environment=KIWANO_DATA_PORT=8317
ExecStart=%h/.local/bin/kiwano-gateway
KillSignal=SIGTERM
TimeoutStopSec=15
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now kiwano-gateway
sudo loginctl enable-linger "$USER"      # else it stops when you log out
```

Check it came up:

```sh
systemctl --user status kiwano-gateway
kiwano status
```

`KillSignal=SIGTERM` with `TimeoutStopSec=15` is deliberate: the gateway
checkpoints its SQLite write-ahead log and removes its socket on the way out,
and a process the OS kills outright does neither.

## 4. Configure it

```sh
kiwano catalog list --search deepseek
kiwano providers add --name deepseek --endpoint https://api.deepseek.com/anthropic \
    --key sk-… --protocol anthropic --bind claude
kiwano agents takeover claude
```

`agents takeover` is the one that matters: it rewrites the agent's own config to
point at the gateway and mints the placeholder key the gateway will accept.
Without that key the gateway refuses the agent — it only routes keys it issued
itself — so an agent pointed at the port by hand gets a 401 and nothing else.

## Who runs what

There are two accounts in play, and getting them the same is the whole trick:

- the account the **gateway** runs as, which owns `~/.kiwano/kiwano.db` (0600,
  and it holds every upstream key in the clear);
- the account the **agent** runs as, which owns the agent's config file.

`agents takeover` does both halves of the job: it reads the gateway's database
for the placeholder key, and it writes the agent's config under `$HOME`. So it
has to be able to do both — which means running as the account that owns the
agent config, with the gateway running as that same account.

Run the gateway as yourself (as above) and this is automatic: one user, one
`$HOME`, one database. That is why it is the default.

The alternative — a dedicated `kiwano` system user with the database in
`/var/lib/kiwano` — is tidier systemd hygiene and is described below. It also
splits the two accounts, and then `kiwano agents takeover` run as *you* cannot
read the database, while running it as *the service user* writes a root-owned or
`kiwano`-owned file into your `$HOME` that your agent can no longer write. Use
it when the agent also runs as that service user, or when agents are configured
some other way.

## Machine-wide install instead

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sudo sh -s -- --system
```

or by hand:

```sh
sudo useradd --system --home-dir /var/lib/kiwano --create-home \
    --shell /usr/sbin/nologin kiwano
sudo install -m 0755 /tmp/kiwano/kiwano-gateway /usr/local/bin/
sudo install -m 0644 /tmp/kiwano/kiwano-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now kiwano-gateway
```

The unit sets `KIWANO_DB_PATH=/var/lib/kiwano/kiwano.db` and
`StateDirectory=kiwano` explicitly. Without the first, a service with no `HOME`
resolves its database relative to `WorkingDirectory`, which is not the file the
CLI on the same host would read. It also hardens more than the minimum:
`NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=read-only` — the gateway
reads agent configs nowhere, so it has no business in anyone's home.

Administer it as the service user:

```sh
sudo -u kiwano kiwano --db /var/lib/kiwano/kiwano.db status
```

…and read [Who runs what](#who-runs-what) before running `agents takeover`.

## Do not use `kiwano gateway restart` here

The CLI can start and stop a gateway (`kiwano gateway start|stop|restart`),
which is useful on a development machine and in a container with no init. Under
systemd it is the wrong tool: stopping the daemon out from under the unit makes
systemd restart it per `Restart=on-failure`, so the two fight.

Use `systemctl --user restart kiwano-gateway` (or the system equivalent) on this
host.

## Updating

There is no self-update for the server bundle. Re-run the installer, or download
the new tarball, verify it, and install over the binaries.

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sh
```

The database is migrated in place on first start. The gateway runs every
migration before it begins serving, so a failed migration fails the start rather
than half-serving.
