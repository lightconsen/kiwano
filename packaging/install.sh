#!/bin/sh
# Kiwano server installer.
#
#   curl -fsSL https://hub.kiwano.cc/install.sh | sh
#
# Installs the command-line client and the gateway daemon, and starts the
# gateway as a service for the current user. It needs no root, and that is
# deliberate: `kiwano agents takeover` rewrites an agent's own config under
# $HOME, so the CLI and the agent have to be the same user. A user-level service
# is the layout where they are.
#
# Nothing is unpacked until the download has been checked against a checksum
# published for this release — see "Verifying" below.
#
#   --system          install for the whole machine instead (needs root)
#   --no-service      install the binaries, do not touch any service manager
#   --prefix DIR      where the binaries go (default ~/.local/bin;
#                     /usr/local/bin with --system)
#   --version TAG     install a specific release instead of the latest
#   --mirror URL      download from somewhere else
#   -h, --help

set -eu

MIRROR="${KIWANO_MIRROR:-https://hub.kiwano.cc/releases}"
GITHUB="${KIWANO_GITHUB:-https://github.com/lightconsen/kiwano/releases}"
DATA_PORT="${KIWANO_DATA_PORT:-8317}"

SYSTEM=0
SERVICE=1
PREFIX=""
VERSION=""

say() { printf '%s\n' "$*"; }
note() { printf '  %s\n' "$*"; }
die() {
    printf 'install.sh: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    # The header comment is the help text, minus the shebang.
    sed -n '2,/^$/p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//' ||
        say "see https://github.com/lightconsen/kiwano"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --system) SYSTEM=1 ;;
        --no-service) SERVICE=0 ;;
        --prefix) PREFIX="${2:?--prefix needs a directory}" && shift ;;
        --prefix=*) PREFIX="${1#*=}" ;;
        --version) VERSION="${2:?--version needs a tag like v0.1.8}" && shift ;;
        --version=*) VERSION="${1#*=}" ;;
        --mirror) MIRROR="${2:?--mirror needs a URL}" && shift ;;
        --mirror=*) MIRROR="${1#*=}" ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unknown argument: $1 (try --help)" ;;
    esac
    shift
done

# ── what are we on ──────────────────────────────────────────────────────────
#
# The triples are the ones this project actually publishes. Linux is x86_64
# only; refusing by name beats downloading something that will not run.

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Linux)
        case "$arch" in
            x86_64 | amd64) triple=x86_64-unknown-linux-gnu ;;
            aarch64 | arm64)
                die "there is no Linux arm64 build yet — see $GITHUB/latest"
                ;;
            *) die "unsupported Linux architecture: $arch" ;;
        esac
        ;;
    Darwin)
        case "$arch" in
            arm64) triple=aarch64-apple-darwin ;;
            x86_64) triple=x86_64-apple-darwin ;;
            *) die "unsupported macOS architecture: $arch" ;;
        esac
        ;;
    *)
        die "this installer is for Linux and macOS — on Windows use the installer from $GITHUB/latest"
        ;;
esac

# ── where things go ─────────────────────────────────────────────────────────

if [ "$SYSTEM" = 1 ]; then
    [ "$(id -u)" = 0 ] ||
        die "--system installs for the whole machine and needs root:
  curl -fsSL https://hub.kiwano.cc/install.sh | sudo sh -s -- --system"
    bin_dir="${PREFIX:-/usr/local/bin}"
else
    bin_dir="${PREFIX:-$HOME/.local/bin}"
fi

# ── download ────────────────────────────────────────────────────────────────

command -v curl >/dev/null 2>&1 ||
    command -v wget >/dev/null 2>&1 ||
    die "neither curl nor wget is available"

# Timeouts are not decoration. The networks this installer is meant to work on
# are the ones where github.com does not refuse, it *stalls* — and a fetch with
# no deadline would hang the install rather than fall through to the mirror.
CONNECT_TIMEOUT=10
FETCH_TIMEOUT=30

download() { # url dest
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --connect-timeout "$CONNECT_TIMEOUT" "$1" -o "$2"
    else
        wget -qO "$2" --timeout="$FETCH_TIMEOUT" "$1"
    fi
}

fetch() { # url -> stdout; empty on any failure
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --connect-timeout "$CONNECT_TIMEOUT" --max-time "$FETCH_TIMEOUT" \
            "$1" 2>/dev/null || true
    else
        wget -qO- --timeout="$FETCH_TIMEOUT" "$1" 2>/dev/null || true
    fi
}

if [ -n "$VERSION" ]; then
    asset_url="$GITHUB/download/$VERSION/kiwano-$triple.tar.gz"
    sums_github="$GITHUB/download/$VERSION/SHA256SUMS"
else
    # The mirror serves version-free names, so this URL is stable across
    # releases — which is the point of the mirror.
    asset_url="$MIRROR/kiwano-$triple.tar.gz"
    sums_github="$GITHUB/latest/download/SHA256SUMS"
fi
asset="kiwano-$triple.tar.gz"

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t kiwano)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "Kiwano installer"
note "platform $os/$arch → $triple"
note "from     $asset_url"
say ""

if ! download "$asset_url" "$tmp/$asset"; then
    die "could not download $asset_url"
fi

# ── verifying ───────────────────────────────────────────────────────────────
#
# Two sources, tried in that order:
#
#   1. GitHub, for the digest only. The bytes come from the mirror and the
#      expected hash from a different host, so compromising one of them is not
#      enough to pass this check. This is the one that means something.
#   2. The mirror's own per-platform checksum file, when GitHub cannot be
#      reached — which is common on exactly the networks the mirror exists for.
#      It still catches a truncated or corrupted download; it does not catch a
#      tampered mirror, and the script says so rather than pretending otherwise.
#
#      It is per-platform (`kiwano-<triple>.sha256`) because the mirror is
#      written by four build legs in parallel: one shared SHA256SUMS would be
#      four uploads racing to overwrite each other, and the survivor would
#      describe a single platform.

digest_for() { # manifest text -> digest for $asset
    printf '%s\n' "$1" | awk -v want="$asset" '$2 == want { print $1; exit }'
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        printf ''
    fi
}

expected=""
source_desc=""
manifest=$(fetch "$sums_github")
if [ -n "$manifest" ]; then
    expected=$(digest_for "$manifest")
    [ -n "$expected" ] && source_desc="GitHub"
fi

if [ -z "$expected" ] && [ -z "$VERSION" ]; then
    manifest=$(fetch "$MIRROR/kiwano-$triple.sha256")
    if [ -n "$manifest" ]; then
        expected=$(digest_for "$manifest")
        [ -n "$expected" ] && source_desc="mirror"
    fi
fi

[ -n "$expected" ] ||
    die "no checksum available for $asset (tried GitHub and the mirror).
Refusing to install an unverified binary. Check your network, or download by
hand from $GITHUB/latest"

actual=$(sha256_of "$tmp/$asset")
[ -n "$actual" ] ||
    die "no sha256sum or shasum on this machine, so the download cannot be verified"

if [ "$actual" != "$expected" ]; then
    die "checksum mismatch for $asset
  expected $expected ($source_desc)
  actual   $actual
Nothing was installed. This means the download did not arrive intact, or it is
not the file this release published — either way, do not use it."
fi

if [ "$source_desc" = "mirror" ]; then
    note "checksum ok — but against the *mirror's* own manifest (GitHub was"
    note "unreachable). That proves the download is intact, not that it is"
    note "untampered. Re-run on a network that reaches GitHub to check properly."
else
    note "checksum ok (verified against GitHub)"
fi
say ""

# ── retiring the old name ───────────────────────────────────────────────────
#
# The daemon was `kiwano-gateway` before this release. An install that predates
# the rename still holds that binary and, under systemd, a unit pointing at it —
# and two units both want port 8317, so the old one has to be stopped before the
# new one starts, not left to lose a race at boot.
#
# Deliberately silent on a machine that never had the old name: this prints
# nothing at all there. Where it does find something it says what it is doing,
# because "your installer deleted a binary" should never be a surprise.
#
# The stop comes first so the port is free before the new unit is enabled.

old_unit=""
if [ "$SYSTEM" = 1 ]; then
    old_unit=/etc/systemd/system/kiwano-gateway.service
else
    old_unit="$HOME/.config/systemd/user/kiwano-gateway.service"
fi

if [ -f "$old_unit" ]; then
    note "retiring the old service: kiwano-gateway"
    if [ "$os" = Linux ] && command -v systemctl >/dev/null 2>&1; then
        if [ "$SYSTEM" = 1 ]; then
            systemctl disable --now kiwano-gateway >/dev/null 2>&1 || true
        else
            systemctl --user disable --now kiwano-gateway >/dev/null 2>&1 || true
        fi
    fi
    rm -f "$old_unit"
fi

if [ -f "$bin_dir/kiwano-gateway" ]; then
    note "removing the old binary: $bin_dir/kiwano-gateway"
    rm -f "$bin_dir/kiwano-gateway"
fi

# ── install ─────────────────────────────────────────────────────────────────

tar xzf "$tmp/$asset" -C "$tmp" || die "could not unpack $asset"
[ -f "$tmp/kiwano" ] && [ -f "$tmp/kiwanod" ] ||
    die "the archive does not contain the expected binaries"

mkdir -p "$bin_dir"
install -m 0755 "$tmp/kiwano" "$bin_dir/kiwano"
install -m 0755 "$tmp/kiwanod" "$bin_dir/kiwanod"
note "installed $bin_dir/kiwano"
note "installed $bin_dir/kiwanod"

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) note "note: $bin_dir is not on your PATH — add it for this shell with:"
       note "  export PATH=\"$bin_dir:\$PATH\"" ;;
esac

# ── the service ─────────────────────────────────────────────────────────────
#
# The gateway is what makes the CLI mean anything, so it is started rather than
# left as an exercise. On a machine without systemd there is nothing to install
# into, and saying that beats writing a unit nobody will read.

if [ "$SERVICE" = 1 ]; then
    if [ "$os" = Linux ] && command -v systemctl >/dev/null 2>&1; then
        if [ "$SYSTEM" = 1 ]; then
            id -u kiwano >/dev/null 2>&1 ||
                useradd --system --home-dir /var/lib/kiwano --create-home \
                    --shell /usr/sbin/nologin kiwano
            install -m 0644 "$tmp/kiwanod.service" /etc/systemd/system/
            systemctl daemon-reload
            systemctl enable --now kiwanod
            note "service kiwanod enabled (system, user kiwano)"
            note "database /var/lib/kiwano/kiwano.db"
        else
            unit_dir="$HOME/.config/systemd/user"
            mkdir -p "$unit_dir"
            # The shipped unit is the system one: a fixed user, a fixed database
            # under /var/lib. For a user service both of those follow from who
            # is running it, so the unit is generated with %h rather than copied.
            cat >"$unit_dir/kiwanod.service" <<EOF
[Unit]
Description=Kiwano gateway (local AI model router)
Documentation=https://github.com/lightconsen/kiwano
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=KIWANO_DB_PATH=%h/.kiwano/kiwano.db
Environment=KIWANO_DATA_PORT=$DATA_PORT
ExecStart=$bin_dir/kiwanod
KillSignal=SIGTERM
TimeoutStopSec=15
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF
            systemctl --user daemon-reload
            systemctl --user enable --now kiwanod
            note "service kiwanod enabled (user $USER, port $DATA_PORT)"

            # Without lingering, the service stops the moment the last session
            # ends: it belongs to the user manager, not to the machine.
            if command -v loginctl >/dev/null 2>&1; then
                if loginctl enable-linger "$(id -un)" 2>/dev/null; then
                    note "lingering enabled — the gateway starts at boot"
                else
                    note "could not enable lingering; the gateway will stop when you"
                    note "log out. Fix it with:  sudo loginctl enable-linger $(id -un)"
                fi
            fi
        fi
    else
        note "no systemd here, so nothing was registered as a service."
        note "Start the gateway yourself:"
        note "  $bin_dir/kiwanod &"
    fi
fi

# ── what now ────────────────────────────────────────────────────────────────

say ""
say "Next:"
note "kiwano status                          is the gateway up?"
note "kiwano providers add --name … --endpoint … --key … --bind claude"
note "kiwano agents takeover claude          route an agent through it"
say ""
note "The takeover rewrites the agent's own config under \$HOME, so run it as"
note "the user that agent runs as — and leave the gateway running as that user"
note "too. Same account, and the two just work."
say ""
note "Full reference: https://github.com/lightconsen/kiwano/blob/main/docs/cli.md"
