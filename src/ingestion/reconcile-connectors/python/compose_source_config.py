#!/usr/bin/env python3
"""Compose an Airbyte source config from a K8s Secret's data plus platform fields.

CLI:
  compose_source_config.py --tenant-id T --source-id S [--injected JSON]

Stdin:  the Secret's decoded stringData object (extract_secret_data.py output).
Stdout: the source config as JSON.
Exit:   0 on success, 1 on JSON decode error.

The Secret carries connector-specific credentials only. Identity
(`insight_tenant_id`, `insight_source_id`) and any `--injected` fields are
platform-owned: they are added here so an operator never duplicates them into
the Secret payload.
"""

import argparse
import json
import sys


def compose(secret_data: dict, tenant_id: str, source_id: str, injected: dict) -> dict:
    return {
        **secret_data,
        **injected,
        "insight_tenant_id": tenant_id,
        "insight_source_id": source_id,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tenant-id", required=True)
    parser.add_argument("--source-id", required=True)
    parser.add_argument("--injected", default="{}")
    args = parser.parse_args()

    try:
        secret_data = json.loads(sys.stdin.read() or "{}") or {}
        injected = json.loads(args.injected or "{}") or {}
    except json.JSONDecodeError as exc:
        print(f"compose_source_config: bad JSON: {exc}", file=sys.stderr)
        return 1

    print(json.dumps(compose(secret_data, args.tenant_id, args.source_id, injected)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
