#!/usr/bin/env python3
"""Fold injected platform fields into a Secret's cfg-hash.

CLI:    platform_cfg_hash.py <secret_cfg_hash> <injected_json>
Stdout: hex SHA-256 digest, no trailing newline.
Exit:   0 on success, 1 when injected_json is not a JSON object.

The injected fields are part of the source config, so they are part of the
drift signal the cfg-hash tag carries; Airbyte masks a secret on read, so the
tag is the only signal a rotated one can move.
"""

import hashlib
import json
import sys


def platform_cfg_hash(secret_cfg_hash: str, injected: dict[str, str]) -> str:
    canonical = json.dumps({"platform": injected, "secret": secret_cfg_hash}, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("platform_cfg_hash: expected 2 args (secret_cfg_hash, injected_json)\n")
        return 2
    try:
        injected = json.loads(sys.argv[2] or "{}") or {}
    except json.JSONDecodeError as exc:
        sys.stderr.write(f"platform_cfg_hash: bad JSON: {exc}\n")
        return 1
    if not isinstance(injected, dict):
        sys.stderr.write("platform_cfg_hash: injected_json must be a JSON object\n")
        return 1

    sys.stdout.write(platform_cfg_hash(sys.argv[1], injected))
    return 0


if __name__ == "__main__":
    sys.exit(main())
