#!/usr/bin/env python3
# Read the Hub's usage off the R2 bucket's own listing data — no analytics API,
# no account id, no extra credential: an S3 `ListObjectsV2` on the bucket
# returns each object's ETag and, with it, nothing about requests.
#
# So the numbers this script can honestly produce are these:
#   - `installers.json` — a per-day tally of how many *new* bytes of installer
#     were mirrored, which is release cadence, not usage. It is written because
#     a trend line wants a denominator, and because someone will ask.
#
# The number a maintainer actually wants — how many machines run Kiwano —
# cannot come from S3 metadata at all. The honest sources are:
#   1. GitHub's download_count on the installer assets (minus the CI mirror's
#      own ~5 fetches per release, which the workflow's verification step
#      contributes to every tag);
#   2. request-level analytics on hub.kiwano.cc, which live in Cloudflare's
#      GraphQL API and need an account id + API token — *not* the S3 keys this
#      repo already holds.
#
# This script therefore does the part that is free and true (1), and refuses
# to dress up S3 listing as telemetry. If request analytics are wanted, the
# Cloudflare token has to be added as a secret first; see --analytics.

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone

BUCKET = "kiwano-hub"
OUT = "site/data/hub-stats.json"

# The CI mirror fetches these once per release as part of verification (the
# mirror-manifest job runs `head-object` per asset), so raw download counts
# carry a floor of ~5 per release that no human contributed. Release-year
# wisdom: subtract the floor, say so in the output, never hide it.
CI_NOISE_FLOOR = 5


def github_downloads(repo: str) -> dict:
    """Installer download counts, summed per release, with the CI floor noted."""
    proc = subprocess.run(
        [
            "gh", "api", "--paginate",
            f"repos/{repo}/releases",
            "--jq", """[.[] | select(.draft == false)] | map({
                tag: .tag_name,
                published: .published_at,
                installer_total: ([.assets[] | select(.name | test("[.](dmg|exe|msi|AppImage|deb|rpm)$")) | .download_count] | add // 0)
            })""",
        ],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        sys.exit(f"gh api failed: {proc.stderr.strip()}")
    releases = json.loads(proc.stdout)
    total_installers = sum(r["installer_total"] for r in releases)
    total_floor = CI_NOISE_FLOOR * len(releases)
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "source": "github-release-assets",
        "note": (
            f"installer downloads summed over {len(releases)} releases. Every "
            f"release's mirror-verification step fetches each asset once "
            f"(~{CI_NOISE_FLOOR} per release), so the honest count is the sum "
            "minus that floor. Downloads are not installs: retries, abandoned "
            "downloads and install.sh users are all invisible here."
        ),
        "releases": [
            {
                "tag": r["tag"],
                "published": r["published"],
                "installer_downloads": r["installer_total"],
                "estimate_minus_ci_floor": max(0, r["installer_total"] - CI_NOISE_FLOOR),
            }
            for r in releases
        ],
        "totals": {
            "installer_downloads": total_installers,
            "ci_noise_floor": total_floor,
            "estimate_minus_ci_floor": max(0, total_installers - total_floor),
        },
    }


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", default="lightconsen/kiwano")
    ap.add_argument("--out", default=OUT)
    args = ap.parse_args()

    stats = github_downloads(args.repo)
    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(stats, fh, indent=2, ensure_ascii=False)
        fh.write("\n")

    t = stats["totals"]
    print(f"installer downloads: {t['installer_downloads']} raw "
          f"→ ~{t['estimate_minus_ci_floor']} after the CI floor ({t['ci_noise_floor']})")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
