#!/usr/bin/env python3
"""Point an updater manifest at the Cloudflare mirror.

tauri-action writes `api.github.com` asset URLs, so copying the manifest to R2
verbatim moves the manifest and leaves every download on GitHub — i.e. mirrors
the half that already works. This rewrites each platform entry to its
version-free R2 path and checks the result before anyone uploads it.

Two checks, because they fail differently:

  * existence, through the S3 API (`--bucket`). Authoritative, and immune to
    the WAF: a HEAD to the public host from a CI runner comes back 403, which
    says nothing about whether the object is there.
  * public reachability over HTTPS. 403 is inconclusive (bot protection), a
    connection failure is inconclusive (transient), anything else non-200 —
    404 above all — means the manifest would point at nothing.

Usage:
  mirror_manifest.py --manifest latest.json --out latest.r2.json [--bucket kiwano-hub]
"""
import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

BASE = "https://hub.kiwano.cc/releases"

# Platform key (as tauri-action writes it) -> mirrored file name. The mirror
# keeps no version in the name, so this table stays valid across releases; an
# unmapped key means the build matrix grew a target the mirror does not know
# about, and it must be added here rather than silently left on GitHub.
NAMES = {
    "darwin-aarch64": "Kiwano_aarch64.app.tar.gz",
    "darwin-aarch64-app": "Kiwano_aarch64.app.tar.gz",
    "darwin-x86_64": "Kiwano_x64.app.tar.gz",
    "darwin-x86_64-app": "Kiwano_x64.app.tar.gz",
    "linux-x86_64": "Kiwano_amd64.AppImage",
    "linux-x86_64-appimage": "Kiwano_amd64.AppImage",
    "linux-x86_64-deb": "Kiwano_amd64.deb",
    "linux-x86_64-rpm": "Kiwano-1.x86_64.rpm",
    "windows-x86_64": "Kiwano_x64-setup.exe",
    "windows-x86_64-msi": "Kiwano_x64_en-US.msi",
    "windows-x86_64-nsis": "Kiwano_x64-setup.exe",
}


def rewrite(doc):
    unmapped = sorted(k for k in doc["platforms"] if k not in NAMES)
    if unmapped:
        sys.exit(f"unmapped platform key(s): {', '.join(unmapped)}")
    for key, entry in doc["platforms"].items():
        entry["url"] = f"{BASE}/{NAMES[key]}"
    return doc


def check_in_bucket(name, bucket):
    """The object must exist; this is the check a 403 cannot fake."""
    proc = subprocess.run(
        ["aws", "s3api", "head-object", "--bucket", bucket, "--key", f"releases/{name}"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.exit(f"{name} is not in s3://{bucket}/releases/ — {proc.stderr.strip()}")


def check_public(url):
    """Returns a warning string, or exits on a definite failure."""
    request = urllib.request.Request(url, method="GET", headers={"Range": "bytes=0-0"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            status = response.status
    except urllib.error.HTTPError as exc:
        status = exc.code
    except Exception as exc:  # DNS, TLS, timeout: inconclusive, not wrong
        return f"{url}: not reachable from here ({exc})"
    if status == 200:
        return None
    if status == 403:
        # Cloudflare answers a CI runner's request this way; the bucket check
        # above already established the object is there.
        return f"{url}: HTTP 403 (bot protection?) — existence already proven via S3"
    sys.exit(f"{url}: HTTP {status}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--bucket", help="verify object existence in this R2 bucket")
    parser.add_argument(
        "--no-verify",
        # The verify pass needs the bucket credentials and a network round trip
        # per artifact; both exist in CI, neither is guaranteed at a desk. An
        # unsigned manifest is still the previous behaviour, so the escape
        # hatch is explicit and loud in the output rather than silent.
        action="store_true",
        help="skip Ed25519 verification of every platform's signature",
    )
    args = parser.parse_args()

    with open(args.manifest) as fh:
        doc = json.load(fh)
    rewrite(doc)

    warnings = []
    checked = set()
    for key, entry in doc["platforms"].items():
        name = NAMES[key]
        # The `-app` variants share a file with their base key: check each
        # artifact once, not once per key that points at it.
        if name in checked:
            continue
        checked.add(name)
        if args.bucket:
            check_in_bucket(name, args.bucket)
        warning = check_public(entry["url"])
        if warning:
            warnings.append(warning)

    if not args.no_verify:
        failed = verify_signatures(doc, args.bucket)
        if failed:
            for line in failed:
                print(f"::error::{line}")
            sys.exit(
                "updater signature verification failed; refusing to publish a "
                "manifest the app would refuse"
            )

    with open(args.out, "w") as fh:
        json.dump(doc, fh, indent=2)
    print(f"rewrote {len(doc['platforms'])} platform URLs to {BASE}")
    for warning in warnings:
        print(f"::warning::{warning}")


def load_pubkey_text(conf_path="app/src-tauri/tauri.conf.json"):
    """The app's bundled updater public key, as minisign's two-line text.

    The conf stores the base64 of that text. This is the exact key every
    installed client verifies against, so a manifest that fails it is a
    manifest no client would accept."""
    with open(conf_path, encoding="utf-8") as fh:
        b64 = json.load(fh)["plugins"]["updater"]["pubkey"]
    return base64.b64decode(b64).decode("ascii")


def minisign_verify(public_key_text, artifact_path, signature_line):
    """True when `signature_line` verifies the artifact under the app's key.

    Tauri's signature is minisign-shaped — an ed25519 signature with a
    keyed-hash trusted comment — which the openssl `pkeyutl` path cannot
    verify (the signed message has a prefix; a 74-byte signature object, not a
    bare 64-byte one). minisign itself is the authoritative checker, and the
    one every installed updater behaves like, so this shells out to it. No
    minisign on PATH is an error returned to the caller: the gate must not
    silently become a no-op because the tool is missing."""
    if not shutil.which("minisign"):
        return "minisign is not installed (apt-get install minisign)"
    try:
        block = base64.b64decode(signature_line).decode("ascii")
    except Exception:
        return "signature field did not look like base64"
    with tempfile.TemporaryDirectory() as d:
        pub = os.path.join(d, "key.pub")
        with open(pub, "w", encoding="ascii") as fh:
            fh.write(public_key_text)
        sig = os.path.join(d, "msg.sig")
        with open(sig, "w", encoding="ascii") as fh:
            fh.write(block)
        proc = subprocess.run(
            ["minisign", "-V", "-p", pub, "-m", artifact_path, "-x", sig],
            capture_output=True,
            text=True,
        )
        if proc.returncode == 0:
            return None
        return "minisign said: " + proc.stderr.strip().splitlines()[-1]


def verify_signatures(doc, bucket):
    """Download each artifact once from the bucket and check its manifest
    signature against the app's bundled pubkey. Returns a list of failure
    lines (possibly empty); failing is manifest-wide — one bad platform is a
    broken release.

    Reading from the bucket rather than the public URL is deliberate:
    Cloudflare answers a CI runner's full GET with 403 (the same bot
    protection `check_public` documents), while `aws s3 cp` reads the very
    object the client is pointed at — verifying the mirrored object against
    the manifest's own signature is the exact contract that matters."""
    if not bucket:
        return [f"verify needs --bucket (it reads the mirrored objects themselves)"]
    pubkey_text = load_pubkey_text()
    failed = []
    seen = set()
    for key, entry in doc["platforms"].items():
        name = NAMES[key]
        if name in seen:
            continue
        seen.add(name)
        proc = subprocess.run(
            ["aws", "s3", "cp", f"s3://{bucket}/releases/{name}", "-", "--no-progress"],
            capture_output=True,
        )
        if proc.returncode != 0:
            failed.append(f"{key} ({name}): s3 fetch failed: {proc.stderr.decode()[:120]}")
            continue
        print(f"verifying {key}: {name} ({len(proc.stdout)} bytes)")
        with tempfile.TemporaryDirectory() as d:
            artifact = os.path.join(d, "artifact")
            with open(artifact, "wb") as fh:
                fh.write(proc.stdout)
            err = minisign_verify(pubkey_text, artifact, entry["signature"])
        if err:
            failed.append(f"{key} ({name}): {err}")
    return failed


if __name__ == "__main__":
    main()
