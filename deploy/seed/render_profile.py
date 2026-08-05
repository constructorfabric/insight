#!/usr/bin/env python3
"""Generate (or verify) the committed `PROFILE.md`.

    python3 deploy/seed/render_profile.py            # write PROFILE.md
    python3 deploy/seed/render_profile.py --check    # fail if it is stale

Both render against `manifest.CANONICAL_ENV`, not the ambient environment, so
the committed page is a function of committed bytes: it does not embed one
developer's `DEV_USER_EMAIL` or one stand's issuer, and the check gives the
same verdict for everybody.

The runtime `manifest.json` a seed run writes is a different document, built
from the real environment. This tool never writes it.

Needs no database, no docker and no third-party packages, so it is usable as
a cheap drift gate.
"""

from __future__ import annotations

import argparse
import difflib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import manifest
import profile_md


def _rendered() -> str:
    # The canonical stand is one that ran every seed step, so the catalogue
    # section describes what `seed all` writes rather than reporting it absent.
    # Only the static half — see `manifest.canonical_catalogue`.
    doc = manifest.build_manifest(manifest.CANONICAL_ENV, catalogue=manifest.canonical_catalogue())
    manifest.assert_no_credentials(doc)
    return profile_md.render_profile(doc)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if the committed PROFILE.md differs from a fresh render",
    )
    args = parser.parse_args(argv)

    want = _rendered()
    path = profile_md.profile_path()

    if not args.check:
        path.write_text(want, encoding="utf-8", newline="\n")
        print(f"wrote {path}")
        return 0

    if not path.exists():
        print(f"{path} is missing; run: {profile_md.REGEN_COMMAND}", file=sys.stderr)
        return 1

    have = path.read_text(encoding="utf-8")
    if have == want:
        print(f"{path.name} is up to date")
        return 0

    print(f"{path} is STALE — regenerate with: {profile_md.REGEN_COMMAND}\n", file=sys.stderr)
    sys.stderr.writelines(
        difflib.unified_diff(
            have.splitlines(keepends=True),
            want.splitlines(keepends=True),
            fromfile=f"{path.name} (committed)",
            tofile=f"{path.name} (regenerated)",
        )
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
