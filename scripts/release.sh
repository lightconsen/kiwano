#!/usr/bin/env bash
#
# Bump the Kiwano version everywhere it is recorded, then refresh Cargo.lock.
#
# The version lives in three files that must always agree:
#   package.json                -> "version"
#   src-tauri/tauri.conf.json   -> "version"   (drives bundle names + updater manifest)
#   Cargo.toml                  -> [workspace.package] version, inherited by every crate
#
# Cargo.lock is never edited by hand: the four local packages (kiwano, kiwano-adapters,
# kiwano-cli, kiwano-gateway) inherit the workspace version, so we let
# `cargo metadata --offline` rewrite the lock -- not `cargo update`, which would also
# move third-party dependencies.
#
# Usage: scripts/release.sh <version> [--dry-run] [--commit] [--tag]
#
#   --dry-run   print the planned changes and modify nothing
#   --commit    commit the bump (message: "chore: bump version to <version>")
#   --tag       create the annotated tag v<version>; requires --commit
#
# Nothing is ever pushed. Without --commit/--tag this is a local, reversible edit.

set -euo pipefail

PKG_JSON="package.json"
TAURI_CONF="src-tauri/tauri.conf.json"
CARGO_TOML="Cargo.toml"
CARGO_LOCK="Cargo.lock"
# Every file this script owns and may rewrite.
MANAGED_FILES="$PKG_JSON $TAURI_CONF $CARGO_TOML $CARGO_LOCK"
# Workspace members whose Cargo.lock entries must match the new version.
LOCAL_CRATES="kiwano kiwano-adapters kiwano-cli kiwano-gateway"

NEW_VERSION=""
DRY_RUN=0
DO_COMMIT=0
DO_TAG=0

# Version grammar: strict semver, prerelease and build metadata optional.
# Leading zeros are rejected, e.g. "01.2.3" is not a valid semver.
SEMVER_RE='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z][0-9A-Za-z.-]*)?(\+[0-9A-Za-z][0-9A-Za-z.-]*)?$'

BACKUP_DIR=""
MUTATED=0

step() { printf '\n==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
die() { printf 'release: error: %s\n' "$*" >&2; exit 1; }

# Print the header comment block above as the help text.
usage() {
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
}

# Remove the scratch directory, whatever the exit path.
cleanup() {
    if [ -n "$BACKUP_DIR" ] && [ -d "$BACKUP_DIR" ]; then
        rm -rf "$BACKUP_DIR"
    fi
}

# Put the tracked files back the way we found them after a partial edit.
restore() {
    [ "$MUTATED" -eq 1 ] || return 0
    printf '    rolling back file changes\n' >&2
    for f in $MANAGED_FILES; do
        if [ -f "$BACKUP_DIR/$(basename "$f")" ]; then
            cp -p "$BACKUP_DIR/$(basename "$f")" "$f"
        fi
    done
}

# Report a failure, undo any edit already applied, and stop.
fail() {
    printf 'release: error: %s\n' "$*" >&2
    restore
    exit 1
}

# --- readers -----------------------------------------------------------------

# First top-level "version" value of a 2-space-indented JSON config file.
json_version() {
    sed -n -E 's/^  "version"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/p' "$1" | head -n 1
}

# version = "..." inside the [workspace.package] table of the root manifest.
cargo_workspace_version() {
    awk '
        /^\[/ { in_section = ($0 == "[workspace.package]") }
        in_section && /^version[[:space:]]*=/ {
            sub(/^version[[:space:]]*=[[:space:]]*"/, "")
            sub(/".*$/, "")
            print
            exit
        }
    ' "$1"
}

# version of one [[package]] block in Cargo.lock, located by its name.
cargo_lock_version() {
    awk -v pkg="$1" '
        $0 == "name = \"" pkg "\"" {
            getline
            if ($0 ~ /^version = "/) {
                sub(/^version = "/, "")
                sub(/"$/, "")
                print
                exit
            }
        }
    ' "$CARGO_LOCK"
}

# --- version comparison ------------------------------------------------------

# version_gt A B -- true (exit 0) when A has higher semver precedence than B.
version_gt() {
    local core m1 n1 p1 pre1 m2 n2 p2 pre2
    core="${1%%+*}"
    case "$core" in
        *-*) pre1="${core#*-}"; core="${core%%-*}" ;;
        *)   pre1="" ;;
    esac
    IFS=. read -r m1 n1 p1 <<< "$core" || true
    core="${2%%+*}"
    case "$core" in
        *-*) pre2="${core#*-}"; core="${core%%-*}" ;;
        *)   pre2="" ;;
    esac
    IFS=. read -r m2 n2 p2 <<< "$core" || true
    m1=$((10#$m1)); n1=$((10#$n1)); p1=$((10#$p1))
    m2=$((10#$m2)); n2=$((10#$n2)); p2=$((10#$p2))

    if [ "$m1" -ne "$m2" ]; then [ "$m1" -gt "$m2" ]; return; fi
    if [ "$n1" -ne "$n2" ]; then [ "$n1" -gt "$n2" ]; return; fi
    if [ "$p1" -ne "$p2" ]; then [ "$p1" -gt "$p2" ]; return; fi
    # Same core version: a prerelease sorts below the final release.
    if [ -z "$pre1" ] && [ -n "$pre2" ]; then return 0; fi
    if [ -n "$pre1" ] && [ -z "$pre2" ]; then return 1; fi
    if [ -z "$pre1" ]; then return 1; fi
    [ "$pre1" \> "$pre2" ]
}

# --- writers -----------------------------------------------------------------

# Rewrite the single top-level "version" key, preserving indentation and the
# trailing comma. Refuses to guess when the file does not look as expected.
write_json_version() {
    local file="$1" label="$2" matches tmp
    matches=$(grep -cE '^  "version"[[:space:]]*:[[:space:]]*"' "$file" || true)
    [ "$matches" -eq 1 ] || fail "$label: expected exactly one top-level \"version\" key, found $matches"

    tmp="$file.kiwano-release.tmp"
    sed -E 's/^(  "version"[[:space:]]*:[[:space:]]*")[^"]*(".*)$/\1'"$NEW_VERSION"'\2/' "$file" > "$tmp" \
        || { rm -f "$tmp"; fail "$label: sed failed"; }
    grep -qF "\"version\": \"$NEW_VERSION\"" "$tmp" || { rm -f "$tmp"; fail "$label: rewrite had no effect"; }
    mv "$tmp" "$file"
    note "$label: \"version\" -> $NEW_VERSION"
}

write_cargo_toml_version() {
    local tmp="$CARGO_TOML.kiwano-release.tmp"
    awk -v v="$NEW_VERSION" '
        /^\[/ { in_section = ($0 == "[workspace.package]") }
        in_section && /^version[[:space:]]*=/ { print "version = \"" v "\""; found = 1; next }
        { print }
        END { if (!found) exit 1 }
    ' "$CARGO_TOML" > "$tmp" || { rm -f "$tmp"; fail "$CARGO_TOML: [workspace.package] version not found"; }
    mv "$tmp" "$CARGO_TOML"
    note "$CARGO_TOML: [workspace.package] version -> $NEW_VERSION"
}

# --- consistency check -------------------------------------------------------

# Verify all three version sources and the four Cargo.lock entries equal $1.
check_consistency() {
    local expected="$1" ok=1 v crate
    printf '    %-32s %s\n' "$PKG_JSON" "$(json_version "$PKG_JSON")"
    printf '    %-32s %s\n' "$TAURI_CONF" "$(json_version "$TAURI_CONF")"
    printf '    %-32s %s\n' "$CARGO_TOML [workspace.package]" "$(cargo_workspace_version "$CARGO_TOML")"
    for crate in $LOCAL_CRATES; do
        printf '    %-32s %s\n' "$CARGO_LOCK $crate" "$(cargo_lock_version "$crate")"
    done

    v=$(json_version "$PKG_JSON")
    [ "$v" = "$expected" ] || { printf 'release: error: %s has version %s, expected %s\n' "$PKG_JSON" "$v" "$expected" >&2; ok=0; }
    v=$(json_version "$TAURI_CONF")
    [ "$v" = "$expected" ] || { printf 'release: error: %s has version %s, expected %s\n' "$TAURI_CONF" "$v" "$expected" >&2; ok=0; }
    v=$(cargo_workspace_version "$CARGO_TOML")
    [ "$v" = "$expected" ] || { printf 'release: error: %s [workspace.package] has version %s, expected %s\n' "$CARGO_TOML" "$v" "$expected" >&2; ok=0; }
    for crate in $LOCAL_CRATES; do
        v=$(cargo_lock_version "$crate")
        [ "$v" = "$expected" ] || { printf 'release: error: Cargo.lock %s has version %s, expected %s\n' "$crate" "$v" "$expected" >&2; ok=0; }
    done
    [ "$ok" -eq 1 ]
}

# --- argument parsing --------------------------------------------------------

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        --commit)  DO_COMMIT=1 ;;
        --tag)     DO_TAG=1 ;;
        -h|--help) usage; exit 0 ;;
        -*)        die "unknown option: $arg (try --help)" ;;
        *)         [ -z "$NEW_VERSION" ] || die "version given more than once: $NEW_VERSION and $arg"; NEW_VERSION="$arg" ;;
    esac
done

if [ -z "$NEW_VERSION" ]; then
    usage >&2
    exit 1
fi

if [ "$DO_TAG" -eq 1 ] && [ "$DO_COMMIT" -eq 0 ]; then
    die "--tag requires --commit (tagging an uncommitted bump would tag the wrong tree)"
fi

cd "$(cd "$(dirname "$0")/.." && pwd)"

[[ "$NEW_VERSION" =~ $SEMVER_RE ]] || die "not a valid semver (expected MAJOR.MINOR.PATCH[-prerelease]): $NEW_VERSION"

for f in $MANAGED_FILES; do
    [ -f "$f" ] || die "missing file: $f"
done

# --- validate the current state ---------------------------------------------

step "Current version"
CURRENT_VERSION=$(json_version "$TAURI_CONF")
[ -n "$CURRENT_VERSION" ] || die "$TAURI_CONF: could not read the top-level \"version\" key"
printf '    %-32s %s\n' "$PKG_JSON" "$(json_version "$PKG_JSON")"
printf '    %-32s %s\n' "$TAURI_CONF" "$(json_version "$TAURI_CONF")"
printf '    %-32s %s\n' "$CARGO_TOML [workspace.package]" "$(cargo_workspace_version "$CARGO_TOML")"

# A bump can only be verified if the three sources start out in agreement.
if ! check_consistency "$CURRENT_VERSION" > /dev/null 2>&1; then
    printf 'release: error: version sources disagree before the bump; fix them first\n' >&2
    check_consistency "$CURRENT_VERSION" > /dev/null || true
    exit 1
fi

if [ "$NEW_VERSION" = "$CURRENT_VERSION" ]; then
    step "Nothing to do"
    note "already at $NEW_VERSION"
    exit 0
fi

version_gt "$NEW_VERSION" "$CURRENT_VERSION" \
    || die "new version $NEW_VERSION is lower than current $CURRENT_VERSION (downgrades are not supported)"

# --- dry run ----------------------------------------------------------------

if [ "$DRY_RUN" -eq 1 ]; then
    step "Dry run: $CURRENT_VERSION -> $NEW_VERSION (no files will be written)"
    note "$PKG_JSON: \"version\" -> $NEW_VERSION"
    note "$TAURI_CONF: \"version\" -> $NEW_VERSION"
    note "$CARGO_TOML: [workspace.package] version -> $NEW_VERSION"
    note "$CARGO_LOCK: would be refreshed with 'cargo metadata --format-version 1 --offline'"
    note "consistency check would then verify all three sources and the four local packages"
    if [ "$DO_COMMIT" -eq 1 ]; then note "would commit: chore: bump version to $NEW_VERSION"; fi
    if [ "$DO_TAG" -eq 1 ]; then note "would tag: v$NEW_VERSION"; fi
    printf '\nDry run complete; nothing was modified.\n'
    exit 0
fi

# --- apply ------------------------------------------------------------------

BACKUP_DIR=$(mktemp -d) || die "could not create a temporary directory"
trap cleanup EXIT
for f in $MANAGED_FILES; do
    cp -p "$f" "$BACKUP_DIR/$(basename "$f")"
done
MUTATED=1

command -v cargo > /dev/null 2>&1 || fail "cargo not found in PATH"

step "Updating version sources to $NEW_VERSION"
write_json_version "$PKG_JSON" "$PKG_JSON"
write_json_version "$TAURI_CONF" "$TAURI_CONF"
write_cargo_toml_version

step "Refreshing $CARGO_LOCK"
note "cargo metadata --format-version 1 --offline"
cargo metadata --format-version 1 --offline > /dev/null \
    || fail "cargo metadata failed; $CARGO_LOCK was not refreshed"

step "Consistency check"
check_consistency "$NEW_VERSION" || fail "version sources are inconsistent after the bump"

# --- commit / tag -----------------------------------------------------------

if [ "$DO_COMMIT" -eq 1 ]; then
    step "Committing"
    git add -- $MANAGED_FILES
    if git diff --cached --quiet -- $MANAGED_FILES; then
        note "no staged changes; skipping commit"
    else
        git commit -m "🔧 chore: bump version to $NEW_VERSION" -m \
"Sync the version across package.json, src-tauri/tauri.conf.json and the
Cargo workspace, and refresh Cargo.lock so the local crates match."
        note "committed: chore: bump version to $NEW_VERSION"
    fi
fi

if [ "$DO_TAG" -eq 1 ]; then
    step "Tagging"
    git tag -a "v$NEW_VERSION" -m "v$NEW_VERSION"
    note "created annotated tag v$NEW_VERSION"
fi

printf '\nVersion %s -> %s complete. Nothing was pushed.\n' "$CURRENT_VERSION" "$NEW_VERSION"
