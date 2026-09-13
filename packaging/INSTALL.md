# Running Kiwano on a server

This bundle contains the two pieces a headless host needs, and nothing else:

| File | What it is |
| --- | --- |
| `kiwano-gateway` | The daemon: the local router the agents actually talk to |
| `kiwano` | The command-line client that configures and inspects it |
| `kiwano-gateway.service` | A systemd unit for the daemon |
| `INSTALL.md` | This file |

There is no GUI here. The desktop app is a separate download and is not part of
a server install.

## 1. Verify the download

Do this before unpacking. The tarball is what is signed and attested, not the
binaries inside it:

```sh
# Proves the file came from this repository's release workflow at the tagged
# commit, via GitHub's transparency log — it does not depend on trusting the
# host you downloaded from.
gh attestation verify kiwano-<target>.tar.gz --repo lightconsen/kiwano

# Or compare against the digest published on the GitHub release.
sha256sum -c --ignore-missing SHA256SUMS
```

## 2. Install the binaries

```sh
tar xzf kiwano-<target>.tar.gz -C /tmp/kiwano
sudo install -m 0755 /tmp/kiwano/kiwano-gateway /usr/local/bin/
sudo install -m 0755 /tmp/kiwano/kiwano         /usr/local/bin/
```

## 3. Create the service user

The gateway owns a SQLite database that holds every upstream API key in the
clear. It is protected by file permissions alone — the directory is `0700` and
the file `0600` — so the user it runs as is the boundary that matters.

```sh
sudo useradd --system --home-dir /var/lib/kiwano --create-home --shell /usr/sbin/nologin kiwano
```

## 4. Install and start the unit

```sh
sudo install -m 0644 /tmp/kiwano/kiwano-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now kiwano-gateway
```

Check it came up. The daemon prints a `ready data=… admin=…` line once both
planes are bound, and logs beside its database:

```sh
systemctl status kiwano-gateway
sudo -u kiwano kiwano --db /var/lib/kiwano/kiwano.db status
```

## 5. Configure it

The CLI is the only interface, and it needs the same `--db` the service uses:

```sh
K="sudo -u kiwano kiwano --db /var/lib/kiwano/kiwano.db"

# Add a provider from the shelf, or by hand:
$K catalog list --search deepseek
$K providers add --name deepseek --endpoint https://api.deepseek.com/anthropic \
    --key sk-… --protocol anthropic --bind claude

# Route an agent through the gateway. This rewrites the agent's own config and
# mints the placeholder key the gateway will accept — without it the gateway
# refuses the agent's requests.
$K agents takeover claude
```

Then run the agent **as the same user whose config was taken over**. The
takeover rewrites a file under that user's `$HOME`; a different user has a
different `$HOME` and therefore a different config. If the agent runs as a
system service of its own, give that service the CLI's `--home` explicitly:

```sh
kiwano --home /home/deploy agents takeover claude
```

## Running the CLI as another user

The admin socket sits beside the database in a `0700` directory, so only the
service user (and root) can reach it. `sudo -u kiwano …` is the straightforward
answer. If you would rather not, point the CLI at a socket it *can* reach with
`--admin-socket`, and move the gateway's to match with `KIWANO_ADMIN_SOCKET` in
the unit — but then the access control is whatever you set on that path.

## Do not use `kiwano gateway restart` here

The CLI can start and stop a gateway (`kiwano gateway start|stop|restart`),
which is useful on a development machine and in a container with no init. Under
systemd it is the wrong tool: stopping the daemon out from under the unit makes
systemd restart it per `Restart=on-failure`, so the two fight.

Use `systemctl restart kiwano-gateway` on this host. The CLI commands remain
correct and are worth knowing about — just not here.

## What "the gateway outlives the GUI" means for you

On a desktop, the daemon deliberately survives the app closing, and the app
adopts a running gateway at startup. On a server the daemon is simply a service,
and the CLI never needs to adopt anything: it talks to whatever is listening on
the admin socket and reports honestly when nothing is.

## Updating

There is no self-update for the server bundle. Download the new tarball, verify
it, install over the binaries, and restart the unit. The database is migrated in
place on first start; the gateway has already run every migration by the time it
begins serving, so a failed migration fails the start rather than half-serving.
