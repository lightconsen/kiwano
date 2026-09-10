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
import json
import subprocess
import sys
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

    with open(args.out, "w") as fh:
        json.dump(doc, fh, indent=2)
    print(f"rewrote {len(doc['platforms'])} platform URLs to {BASE}")
    for warning in warnings:
        print(f"::warning::{warning}")


if __name__ == "__main__":
    main()
