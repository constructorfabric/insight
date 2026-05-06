#!/usr/bin/env python3
"""Compute a stable SHA-256 hash of a JSON document on stdin.

Stdin:  JSON document (typically a Secret stringData object).
Stdout: hex SHA-256 digest, no trailing newline.
Exit:   0 on success, 1 on JSON decode error.

Used by lib/discover.sh to derive the cfg_hash annotation stored on
Airbyte sources.
"""
import hashlib, json, sys


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError as e:
        print(f"compute_cfg_hash: bad JSON on stdin: {e}", file=sys.stderr)
        return 1
    canonical = json.dumps(data, sort_keys=True, separators=(",", ":"))
    sys.stdout.write(hashlib.sha256(canonical.encode("utf-8")).hexdigest())
    return 0


if __name__ == "__main__":
    sys.exit(main())
