#!/usr/bin/env python3
"""Rewrite an image reference's registry through the install's mirrors.

RECONCILE_IMAGE_MIRRORS is a JSON object {"<registry>": "<mirror>"} (chart
value ingestion.reconcile.imageMirrors). Only the leading registry component
is matched; a reference whose registry is not mapped comes back unchanged.
Works on a full reference and on a bare repository alike.

CLI: mirror_image_ref.py <ref>
Stdout: the mirrored reference.
"""

import json
import os
import sys


def mirrors_from_env() -> dict[str, str]:
    return json.loads(os.environ.get("RECONCILE_IMAGE_MIRRORS") or "{}")


def mirror(ref: str, mirrors: dict[str, str]) -> str:
    registry, sep, rest = ref.partition("/")
    target = mirrors.get(registry)
    return f"{target.rstrip('/')}/{rest}" if sep and target else ref


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("mirror_image_ref: expected 1 arg\n")
        return 2
    sys.stdout.write(mirror(sys.argv[1], mirrors_from_env()) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
