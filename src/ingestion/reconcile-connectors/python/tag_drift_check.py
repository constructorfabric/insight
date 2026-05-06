#!/usr/bin/env python3
"""Check whether existing tags satisfy the desired (`insight`, `cfg-hash:<h>`) set.

CLI:    tag_drift_check.py <existing_tags_json> <desired_hash>
Stdout: 'noop' or 'patch_tags'.
Exit:   0 always.
"""
import json
import sys


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("tag_drift_check: expected 2 args (tags, hash)\n")
        return 2
    existing = json.loads(sys.argv[1] or "[]")
    desired_hash = sys.argv[2]
    have_insight = False
    have_hash = False
    for t in existing:
        name = t.get("name", t) if isinstance(t, dict) else t
        if name == "insight":
            have_insight = True
        elif isinstance(name, str) and name == f"cfg-hash:{desired_hash}":
            have_hash = True
    print("noop" if (have_insight and have_hash) else "patch_tags")
    return 0


if __name__ == "__main__":
    sys.exit(main())
