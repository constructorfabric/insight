#!/usr/bin/env python3
"""Check whether existing tags satisfy the desired (`insight`, `cfg-hash:<h>`) set.

CLI:    tag_drift_check.py <existing_tags_json> <desired_hash>
Stdout: 'noop' or 'patch_tags'.
Exit:   0 always.

Note: Airbyte caps tag-name length at 30 characters, so the cfg-hash tag is
written as `cfg-hash:<first-12-hex>` (see lib/reconcile.sh + lib/adopt.sh).
The drift check truncates the incoming hash the same way to stay consistent.
"""
import json
import sys

CFG_HASH_PREFIX_LEN = 12


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("tag_drift_check: expected 2 args (tags, hash)\n")
        return 2
    existing = json.loads(sys.argv[1] or "[]")
    desired_hash = sys.argv[2][:CFG_HASH_PREFIX_LEN]
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
